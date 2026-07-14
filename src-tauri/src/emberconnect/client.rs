//! HTTP client for EmberConnect dongles.
//!
//! Plain HTTP on port 80 against the ESP32's `esp_http_server`. Transport
//! notes:
//!
//! * The server needs an explicit `Content-Length` — it does not implement
//!   chunked transfer encoding (uploads stream, but with the length header).
//! * The upload response arrives only after the dongle has written the card
//!   AND completed its USB re-plug cycle (~1 s), so upload timeouts include
//!   that tail.
//! * Few sockets on the ESP32: connections are not pooled.

use super::models::{DongleFile, DongleInfo, ErrorResponse, FileList, Health, UploadResponse};
use crate::machine::{
    EmbroideryMachine, MachineCapabilities, MachineError, MachineIdentity, MachineInfo,
    ProgressFn, StorageStatus, UploadProgress, UploadReceipt, UploadRequest,
};
use async_trait::async_trait;
use std::net::IpAddr;
use std::time::Duration;

/// Formats commonly readable from a USB stick across manufacturers. The
/// dongle itself stores anything; the machine behind it decides what it can
/// load, and we cannot see that machine — so advertise the broad set.
pub const COMMON_FORMATS: &[&str] = &["pes", "pec", "dst", "exp", "jef", "vp3", "hus", "vip", "xxx"];

const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
const READ_TIMEOUT: Duration = Duration::from_secs(10);
/// SD write + USB re-plug happen before the dongle answers an upload.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(180);
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

const READ_RETRIES: u32 = 3;
/// Uploads are overwrite-by-name on the dongle, so retrying is safe.
const UPLOAD_RETRIES: u32 = 2;
const RETRY_DELAY: Duration = Duration::from_millis(1000);

/// A handle to one EmberConnect dongle. Cheap to create.
pub struct EmberConnectClient {
    ip: IpAddr,
    port: u16,
    http: reqwest::Client,
}

impl EmberConnectClient {
    pub fn new(ip: IpAddr) -> Self {
        Self::with_port(ip, 80)
    }

    /// Non-default port; used by tests that run a mock dongle on localhost.
    pub fn with_port(ip: IpAddr, port: u16) -> Self {
        Self {
            ip,
            port,
            http: reqwest::Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                // The ESP32 http server has a handful of sockets; keeping
                // idle connections open starves other clients.
                .pool_max_idle_per_host(0)
                .build()
                .expect("static reqwest client configuration cannot fail"),
        }
    }

    fn url(&self, path: &str) -> String {
        match self.ip {
            IpAddr::V4(v4) => format!("http://{v4}:{}{path}", self.port),
            IpAddr::V6(v6) => format!("http://[{v6}]:{}{path}", self.port),
        }
    }

    /// Single-attempt `GET /api/health` with a short timeout; used by
    /// discovery/probing.
    pub async fn probe_health(&self) -> Result<Health, MachineError> {
        let response = self
            .http
            .get(self.url("/api/health"))
            .timeout(PROBE_TIMEOUT)
            .send()
            .await
            .map_err(map_transport_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(MachineError::Protocol(format!(
                "GET /api/health answered HTTP {status}"
            )));
        }
        response
            .json::<Health>()
            .await
            .map_err(|e| MachineError::Protocol(format!("/api/health is not valid JSON: {e}")))
    }

    async fn fetch_dongle_info(&self) -> Result<DongleInfo, MachineError> {
        with_retries(READ_RETRIES, || async {
            let response = self
                .http
                .get(self.url("/api/info"))
                .timeout(READ_TIMEOUT)
                .send()
                .await
                .map_err(map_transport_error)?;
            decode_json::<DongleInfo>(response).await
        })
        .await
    }

    async fn fetch_files(&self) -> Result<FileList, MachineError> {
        with_retries(READ_RETRIES, || async {
            let response = self
                .http
                .get(self.url("/api/files"))
                .timeout(READ_TIMEOUT)
                .send()
                .await
                .map_err(map_transport_error)?;
            decode_json::<FileList>(response).await
        })
        .await
    }

    pub(crate) fn to_machine_info(&self, health: &Health) -> MachineInfo {
        MachineInfo {
            identity: MachineIdentity {
                manufacturer: super::MANUFACTURER.to_string(),
                model: "EmberConnect dongle".to_string(),
                // Match the setup-hotspot / mDNS naming so users recognize it.
                name: health
                    .serial
                    .as_ref()
                    .filter(|s| s.len() >= 4)
                    .map(|s| format!("EmberConnect-{}", &s[s.len() - 4..])),
                firmware: health.version.clone(),
                serial: health.serial.clone(),
                ip: self.ip,
            },
            capabilities: MachineCapabilities {
                // The dongle cannot know the attached machine's hoop size.
                emb_width_mm: None,
                emb_height_mm: None,
                needles: None,
                // Bounded by card space, which is checked live per upload.
                max_file_bytes: None,
                formats: COMMON_FORMATS.iter().map(|s| s.to_string()).collect(),
            },
        }
    }
}

#[async_trait]
impl EmbroideryMachine for EmberConnectClient {
    fn manufacturer(&self) -> &'static str {
        super::MANUFACTURER
    }

    fn ip(&self) -> IpAddr {
        self.ip
    }

    async fn info(&self) -> Result<MachineInfo, MachineError> {
        let health = with_retries(READ_RETRIES, || self.probe_health()).await?;
        if !health.is_ember_connect() {
            return Err(MachineError::Protocol(
                "device at this address is not an EmberConnect dongle".to_string(),
            ));
        }
        Ok(self.to_machine_info(&health))
    }

    async fn storage(&self) -> Result<StorageStatus, MachineError> {
        let info = self.fetch_dongle_info().await?;
        let files = self.fetch_files().await?;
        let total = info.storage.total_bytes;
        let free = info.storage.free_bytes;
        Ok(StorageStatus {
            total_bytes: total,
            free_bytes: free,
            used_bytes: total.saturating_sub(free),
            files: files.files.into_iter().map(|f| f.name).collect(),
        })
    }

    async fn upload(
        &self,
        request: UploadRequest,
        progress: ProgressFn,
    ) -> Result<UploadReceipt, MachineError> {
        let size = request.data.len() as u64;

        // Live free-space check. The dongle enforces this server-side too
        // (507), but checking first avoids streaming megabytes to a full
        // card — and unlike the dongle we can name the exact free figure.
        let info = self.fetch_dongle_info().await?;
        let free = info.storage.free_bytes;
        if size > free {
            return Err(MachineError::InsufficientStorage { size, free });
        }

        let receipt = with_retries(UPLOAD_RETRIES, || {
            let data = request.data.clone();
            let filename = request.filename.clone();
            let progress = progress.clone();
            async move {
                progress(UploadProgress {
                    sent_bytes: 0,
                    total_bytes: size,
                });

                // Stream in chunks to observe transmission progress; explicit
                // Content-Length because the ESP32 server cannot parse
                // chunked encoding.
                let chunks: Vec<bytes::Bytes> = data
                    .chunks(16 * 1024)
                    .map(bytes::Bytes::copy_from_slice)
                    .collect();
                let progress_stream = progress.clone();
                let stream = futures::stream::iter(chunks.into_iter().scan(
                    0u64,
                    move |sent, chunk| {
                        *sent += chunk.len() as u64;
                        progress_stream(UploadProgress {
                            sent_bytes: (*sent).min(size),
                            total_bytes: size,
                        });
                        Some(Ok::<_, std::io::Error>(chunk))
                    },
                ));

                let response = self
                    .http
                    .post(self.url("/api/upload"))
                    .query(&[("filename", filename.as_str())])
                    .header(reqwest::header::CONTENT_LENGTH, size)
                    .timeout(UPLOAD_TIMEOUT)
                    .body(reqwest::Body::wrap_stream(stream))
                    .send()
                    .await
                    .map_err(map_transport_error)?;

                let status = response.status();
                if !status.is_success() {
                    return Err(map_api_error(status.as_u16(), response, size).await);
                }
                let body: UploadResponse = response.json().await.map_err(|e| {
                    MachineError::Protocol(format!("upload response is not valid JSON: {e}"))
                })?;
                progress(UploadProgress {
                    sent_bytes: size,
                    total_bytes: size,
                });
                Ok(UploadReceipt {
                    bytes_sent: size,
                    stored_as: body.file.map(|f: DongleFile| f.name),
                })
            }
        })
        .await?;

        Ok(receipt)
    }
}

/// Decode a JSON success body, mapping HTTP failures to the neutral error.
async fn decode_json<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, MachineError> {
    let status = response.status();
    if !status.is_success() {
        return Err(MachineError::Protocol(format!("HTTP {status}")));
    }
    response
        .json::<T>()
        .await
        .map_err(|e| MachineError::Protocol(format!("invalid JSON from dongle: {e}")))
}

/// Translate the dongle's structured error envelope into the neutral type.
async fn map_api_error(status: u16, response: reqwest::Response, size: u64) -> MachineError {
    let body = response.text().await.unwrap_or_default();
    let code = serde_json::from_str::<ErrorResponse>(&body)
        .map(|e| e.error.code)
        .unwrap_or_default();
    match code.as_str() {
        "insufficient_storage" => MachineError::InsufficientStorage { size, free: 0 },
        "invalid_filename" => MachineError::Protocol(
            "the dongle rejected the filename (FAT-illegal characters?)".to_string(),
        ),
        _ => MachineError::UploadFailed(status),
    }
}

fn map_transport_error(e: reqwest::Error) -> MachineError {
    if e.is_timeout() {
        MachineError::Timeout
    } else if e.is_connect() {
        MachineError::Unreachable(concise_reqwest_error(&e))
    } else {
        MachineError::Protocol(concise_reqwest_error(&e))
    }
}

fn concise_reqwest_error(e: &reqwest::Error) -> String {
    let mut source: &dyn std::error::Error = e;
    while let Some(inner) = source.source() {
        source = inner;
    }
    source.to_string()
}

/// Same retry contract as the Brother backend: transport errors retry,
/// machine-side rejections do not.
async fn with_retries<T, F, Fut>(attempts: u32, mut attempt: F) -> Result<T, MachineError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, MachineError>>,
{
    let mut last = None;
    for i in 0..attempts {
        match attempt().await {
            Ok(value) => return Ok(value),
            Err(e @ (MachineError::Rejected { .. }
            | MachineError::FileTooLarge { .. }
            | MachineError::InsufficientStorage { .. }
            | MachineError::UnsupportedFormat { .. })) => return Err(e),
            Err(e) => {
                last = Some(e);
                if i + 1 < attempts {
                    tokio::time::sleep(RETRY_DELAY).await;
                }
            }
        }
    }
    Err(last.expect("attempts is at least 1"))
}
