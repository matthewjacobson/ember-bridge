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

use super::models::{
    DongleFile, DongleInfo, ErrorResponse, FileList, Health, PairResponse, UploadResponse,
};
use super::tokens::TokenStore;
use crate::machine::{
    EmbroideryMachine, MachineCapabilities, MachineError, MachineIdentity, MachineInfo,
    ProgressFn, StorageStatus, UploadProgress, UploadReceipt, UploadRequest,
};
use async_trait::async_trait;
use std::net::IpAddr;
use std::sync::Arc;
use std::sync::OnceLock;
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

/// How this computer introduces itself when pairing; the dongle shows the
/// name in its paired-clients list.
const PAIR_CLIENT_NAME: &str = "Ember Bridge";

/// A handle to one EmberConnect dongle. Cheap to create.
pub struct EmberConnectClient {
    ip: IpAddr,
    port: u16,
    http: reqwest::Client,
    tokens: Arc<TokenStore>,
    /// Serial learned from the first `/api/health` answer, so token lookups
    /// don't repeat the round-trip on every call.
    serial: OnceLock<Option<String>>,
}

impl EmberConnectClient {
    pub fn new(ip: IpAddr, tokens: Arc<TokenStore>) -> Self {
        Self::with_port(ip, 80, tokens)
    }

    /// Non-default port; used by tests that run a mock dongle on localhost.
    pub fn with_port(ip: IpAddr, port: u16, tokens: Arc<TokenStore>) -> Self {
        Self {
            ip,
            port,
            tokens,
            serial: OnceLock::new(),
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

    /// The dongle's serial, learned from `/api/health` once and cached.
    async fn serial(&self) -> Result<Option<String>, MachineError> {
        if let Some(serial) = self.serial.get() {
            return Ok(serial.clone());
        }
        let health = self.probe_health().await?;
        Ok(self.serial.get_or_init(|| health.serial).clone())
    }

    /// The token we hold for this dongle, if we've paired before.
    async fn stored_token(&self) -> Result<Option<String>, MachineError> {
        Ok(self
            .serial()
            .await?
            .and_then(|serial| self.tokens.get(&serial)))
    }

    /// Pair with the dongle and persist the minted token. Fails with
    /// [`MachineError::PairingRequired`] when the dongle's pairing window is
    /// closed — the user has to power-cycle it (or tap its button) first.
    async fn pair(&self) -> Result<String, MachineError> {
        let response = self
            .http
            .post(self.url("/api/pair"))
            .json(&serde_json::json!({ "name": PAIR_CLIENT_NAME }))
            .timeout(READ_TIMEOUT)
            .send()
            .await
            .map_err(map_transport_error)?;

        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            let hint = serde_json::from_str::<ErrorResponse>(&body)
                .map(|e| e.error.message)
                .ok()
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| {
                    "unplug and replug the dongle, then try again within 5 minutes".to_string()
                });
            return Err(MachineError::PairingRequired { hint });
        }

        let body: PairResponse = response.json().await.map_err(|e| {
            MachineError::Protocol(format!("pair response is not valid JSON: {e}"))
        })?;
        let serial = match body.serial {
            Some(s) => Some(s),
            None => self.serial().await?,
        };
        if let Some(serial) = serial {
            self.tokens.set(&serial, &body.token);
        }
        tracing::info!("paired with EmberConnect dongle at {}", self.ip);
        Ok(body.token)
    }

    /// Authenticated GET with transparent pairing: attach the stored token
    /// if any; on 401 (never paired, revoked, or factory-reset dongle) pair
    /// and retry once. Dongles on pre-auth firmware never answer 401, so
    /// they work unchanged.
    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, MachineError> {
        let mut request = self.http.get(self.url(path)).timeout(READ_TIMEOUT);
        if let Some(token) = self.stored_token().await? {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.map_err(map_transport_error)?;
        if response.status().as_u16() != 401 {
            return decode_json::<T>(response).await;
        }

        if let Some(serial) = self.serial().await? {
            self.tokens.forget(&serial); // the dongle no longer accepts it
        }
        let token = self.pair().await?;
        let response = self
            .http
            .get(self.url(path))
            .bearer_auth(token)
            .timeout(READ_TIMEOUT)
            .send()
            .await
            .map_err(map_transport_error)?;
        decode_json::<T>(response).await
    }

    async fn fetch_dongle_info(&self) -> Result<DongleInfo, MachineError> {
        with_retries(READ_RETRIES, || self.get_json::<DongleInfo>("/api/info")).await
    }

    async fn fetch_files(&self) -> Result<FileList, MachineError> {
        with_retries(READ_RETRIES, || self.get_json::<FileList>("/api/files")).await
    }

    pub(crate) fn to_machine_info(&self, health: &Health) -> MachineInfo {
        MachineInfo {
            identity: MachineIdentity {
                manufacturer: super::MANUFACTURER.to_string(),
                model: "EmberConnect dongle".to_string(),
                // The name the user gave the machine during setup; for
                // unnamed (or pre-0.5.0) dongles, fall back to the
                // setup-hotspot / mDNS naming so users still recognize it.
                name: Some(health.device_name.clone())
                    .filter(|n| !n.is_empty())
                    .or_else(|| {
                        health
                            .serial
                            .as_ref()
                            .filter(|s| s.len() >= 4)
                            .map(|s| format!("EmberConnect-{}", &s[s.len() - 4..]))
                    }),
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

                let mut request = self
                    .http
                    .post(self.url("/api/upload"))
                    .query(&[("filename", filename.as_str())])
                    .header(reqwest::header::CONTENT_LENGTH, size)
                    .timeout(UPLOAD_TIMEOUT)
                    .body(reqwest::Body::wrap_stream(stream));
                if let Some(token) = self.stored_token().await? {
                    request = request.bearer_auth(token);
                }
                let response = request.send().await.map_err(map_transport_error)?;

                let status = response.status();
                if status.as_u16() == 401 {
                    // Rare here — the free-space check above already paired —
                    // but the token can be revoked between the two calls.
                    // Pair now and let the retry loop re-send with it.
                    if let Some(serial) = self.serial().await? {
                        self.tokens.forget(&serial);
                    }
                    self.pair().await?;
                    return Err(MachineError::Protocol(
                        "dongle token was stale; re-paired".to_string(),
                    ));
                }
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
            | MachineError::UnsupportedFormat { .. }
            | MachineError::PairingRequired { .. })) => return Err(e),
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
