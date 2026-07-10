//! HTTPS client for Brother embroidery machines.
//!
//! Transport characteristics (all mandatory, learned from packet captures and
//! the machine's TLS stack):
//!
//! * HTTPS on port 443 with a **self-signed certificate** (CN like
//!   `56;2;1;1.73.local`) — certificate validation must be disabled.
//! * **TLS 1.2 exactly.** The machine offers
//!   `TLS_RSA_WITH_AES_256_GCM_SHA384` — a static-RSA suite. This is why the
//!   crate uses reqwest's *native-tls* backend: rustls does not implement
//!   static-RSA key exchange and would fail the handshake.
//! * The embedded server (`debut/1.20`) is slow over Wi-Fi; requests get
//!   generous timeouts and reads are retried. Uploads are retried at most
//!   once more to avoid storing duplicate designs.

use super::models::{BrotherInfo, SewingResponse};
use super::protocol;
use crate::machine::{
    EmbroideryMachine, MachineCapabilities, MachineError, MachineIdentity, MachineInfo,
    ProgressFn, StorageStatus, UploadProgress, UploadReceipt, UploadRequest,
};
use async_trait::async_trait;
use rand::Rng;
use reqwest::header;
use std::net::IpAddr;
use std::time::Duration;

/// File formats Brother home machines load from memory.
pub const SUPPORTED_FORMATS: &[&str] = &["pes", "phc", "dst", "phx"];

/// Timeout for the initial TCP+TLS handshake.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(6);
/// Timeout for identification/status round-trips.
const READ_TIMEOUT: Duration = Duration::from_secs(25);
/// Timeout for a whole upload. Machine Wi-Fi is slow; be generous.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(120);
/// Short timeout used when probing during discovery.
const PROBE_TIMEOUT: Duration = Duration::from_secs(4);

/// Retry counts, mirroring the tolerances of the reference implementation.
const READ_RETRIES: u32 = 4;
const UPLOAD_RETRIES: u32 = 2;
const RETRY_DELAY: Duration = Duration::from_millis(1500);

/// A handle to one Brother machine. Cheap to create; owns a lazy HTTP client.
pub struct BrotherClient {
    ip: IpAddr,
    http: reqwest::Client,
}

impl BrotherClient {
    pub fn new(ip: IpAddr) -> Self {
        Self {
            ip,
            http: build_http_client(),
        }
    }

    fn url(&self, path: &str) -> String {
        // IPv6 literals need brackets in URLs; machines are IPv4 in practice
        // but there is no reason to fail on IPv6.
        match self.ip {
            IpAddr::V4(v4) => format!("https://{v4}{path}"),
            IpAddr::V6(v6) => format!("https://[{v6}]{path}"),
        }
    }

    /// `GET /info` with retries.
    pub async fn fetch_info(&self) -> Result<BrotherInfo, MachineError> {
        with_retries(READ_RETRIES, || self.fetch_info_once(READ_TIMEOUT)).await
    }

    /// Single-attempt `GET /info` with a short timeout; used by discovery.
    pub async fn probe_info(&self) -> Result<BrotherInfo, MachineError> {
        self.fetch_info_once(PROBE_TIMEOUT).await
    }

    async fn fetch_info_once(&self, timeout: Duration) -> Result<BrotherInfo, MachineError> {
        let response = self
            .http
            .get(self.url(protocol::INFO_PATH))
            .timeout(timeout)
            .send()
            .await
            .map_err(map_transport_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(MachineError::Protocol(format!(
                "GET /info answered HTTP {status}"
            )));
        }
        response
            .json::<BrotherInfo>()
            .await
            .map_err(|e| MachineError::Protocol(format!("/info is not valid JSON: {e}")))
    }

    /// Status/handshake call: `POST /sewing/sewing.cgi` with `appstate=2`.
    /// Returns memory usage and the current file list.
    pub async fn fetch_session(&self) -> Result<SewingResponse, MachineError> {
        with_retries(READ_RETRIES, || async {
            let response = self
                .http
                .post(self.url(protocol::SEWING_PATH))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .timeout(READ_TIMEOUT)
                .body(protocol::status_body())
                .send()
                .await
                .map_err(map_transport_error)?;
            let status = response.status();
            if !status.is_success() {
                return Err(MachineError::Protocol(format!(
                    "status call answered HTTP {status}"
                )));
            }
            let text = response
                .text()
                .await
                .map_err(|e| MachineError::Protocol(format!("failed reading status body: {e}")))?;
            let session = protocol::parse_sewing_response(&text)?;
            if session.error_code != 0 {
                return Err(MachineError::Rejected {
                    code: session.error_code,
                });
            }
            Ok(session)
        })
        .await
    }

    /// Upload a design: `POST /sewing/sewing.cgi`, multipart, `appstate=3`.
    /// Success is HTTP 204 (the capture) — 200 is also accepted.
    async fn send_design(
        &self,
        filename: &str,
        data: &[u8],
        progress: &ProgressFn,
    ) -> Result<(), MachineError> {
        let total = data.len() as u64;
        let report = |sent: u64| {
            progress(UploadProgress {
                sent_bytes: sent.min(total),
                total_bytes: total,
            })
        };

        let boundary = protocol::multipart_boundary(&random_hex12());
        let body = protocol::upload_body(&boundary, filename, data);

        with_retries(UPLOAD_RETRIES, || {
            let body = body.clone();
            let boundary = boundary.clone();
            // The request body stream must be 'static, so it owns its own
            // clone of the progress callback.
            let progress = progress.clone();
            async move {
                report(0);
                // Stream the body in chunks so we can observe transmission
                // progress. Progress reflects hand-off to the TLS layer, so it
                // slightly leads what is truly on the wire — good enough for a
                // progress bar.
                let overhead = (body.len() as u64).saturating_sub(total);
                let content_length = body.len() as u64;
                let chunks: Vec<bytes::Bytes> = body
                    .chunks(64 * 1024)
                    .map(bytes::Bytes::copy_from_slice)
                    .collect();
                let stream = futures::stream::iter(chunks.into_iter().scan(
                    0u64,
                    move |sent, chunk| {
                        *sent += chunk.len() as u64;
                        progress(UploadProgress {
                            sent_bytes: sent.saturating_sub(overhead).min(total),
                            total_bytes: total,
                        });
                        Some(Ok::<_, std::io::Error>(chunk))
                    },
                ));

                let response = self
                    .http
                    .post(self.url(protocol::SEWING_PATH))
                    .header(
                        header::CONTENT_TYPE,
                        protocol::upload_content_type(&boundary),
                    )
                    // Explicit Content-Length: the machine's embedded server
                    // predates chunked transfer encoding, and hyper would
                    // otherwise chunk a streaming body.
                    .header(header::CONTENT_LENGTH, content_length)
                    .header(header::ACCEPT_ENCODING, "gzip,deflate")
                    .header(header::CONNECTION, "Keep-Alive")
                    .timeout(UPLOAD_TIMEOUT)
                    .body(reqwest::Body::wrap_stream(stream))
                    .send()
                    .await
                    .map_err(map_transport_error)?;

                match response.status().as_u16() {
                    200 | 204 => {
                        report(total);
                        Ok(())
                    }
                    other => Err(MachineError::UploadFailed(other)),
                }
            }
        })
        .await
    }

    pub(crate) fn to_machine_info(&self, raw: &BrotherInfo) -> MachineInfo {
        MachineInfo {
            identity: MachineIdentity {
                manufacturer: "brother".to_string(),
                model: match raw.model {
                    Some(code) => format!("Brother (model {code})"),
                    None => "Brother".to_string(),
                },
                name: raw.name.clone(),
                firmware: raw.version.clone(),
                serial: raw.serial.clone(),
                ip: self.ip,
            },
            capabilities: MachineCapabilities {
                // The machine reports dimensions in 0.1 mm units.
                emb_width_mm: raw.features.embwidth.map(|v| v as f64 / 10.0),
                emb_height_mm: raw.features.embheight.map(|v| v as f64 / 10.0),
                needles: raw.features.needles,
                max_file_bytes: raw.features.postsize,
                formats: SUPPORTED_FORMATS.iter().map(|s| s.to_string()).collect(),
            },
        }
    }
}

#[async_trait]
impl EmbroideryMachine for BrotherClient {
    fn manufacturer(&self) -> &'static str {
        "brother"
    }

    fn ip(&self) -> IpAddr {
        self.ip
    }

    async fn info(&self) -> Result<MachineInfo, MachineError> {
        let raw = self.fetch_info().await?;
        Ok(self.to_machine_info(&raw))
    }

    async fn storage(&self) -> Result<StorageStatus, MachineError> {
        let session = self.fetch_session().await?;
        let total = session.upload_size.unwrap_or(0);
        let free = session.upload_freesize.unwrap_or(0);
        Ok(StorageStatus {
            total_bytes: total,
            free_bytes: free,
            used_bytes: total.saturating_sub(free),
            files: session.files,
        })
    }

    async fn upload(
        &self,
        request: UploadRequest,
        progress: ProgressFn,
    ) -> Result<UploadReceipt, MachineError> {
        let size = request.data.len() as u64;

        // 1. Format gate: the machine would silently store-and-fail on
        //    formats it cannot load, so reject early.
        let extension = request
            .filename
            .rsplit('.')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if !SUPPORTED_FORMATS.contains(&extension.as_str()) {
            return Err(MachineError::UnsupportedFormat {
                format: extension,
                supported: SUPPORTED_FORMATS.join(", "),
            });
        }

        // 2. Hard size limit from /info (`postsize`).
        let info = self.fetch_info().await?;
        if let Some(limit) = info.features.postsize {
            if size > limit {
                return Err(MachineError::FileTooLarge { size, limit });
            }
        }

        // 3. Live free-memory check, which also doubles as the protocol
        //    handshake the official client performs before sending.
        let before = self.fetch_session().await?;
        if let Some(free) = before.upload_freesize {
            if size > free {
                return Err(MachineError::InsufficientStorage { size, free });
            }
        }

        // 4. Transmit.
        self.send_design(&request.filename, &request.data, &progress)
            .await?;

        // 5. Best-effort: ask again and report the machine-assigned name of
        //    the new file (the machine renames every upload).
        let stored_as = match self.fetch_session().await {
            Ok(after) => after
                .files
                .iter()
                .find(|f| !before.files.contains(f))
                .cloned(),
            Err(_) => None,
        };

        Ok(UploadReceipt {
            bytes_sent: size,
            stored_as,
        })
    }
}

/// Build the reqwest client with the transport quirks described above.
fn build_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        // Self-signed certificate with a nonsense CN: nothing to verify.
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true)
        // Pin TLS 1.2: the machine supports nothing newer, and pinning both
        // ends avoids a doomed 1.3 negotiation attempt.
        .min_tls_version(reqwest::tls::Version::TLS_1_2)
        .max_tls_version(reqwest::tls::Version::TLS_1_2)
        .connect_timeout(CONNECT_TIMEOUT)
        // The embedded server handles one request at a time; do not pool.
        .pool_max_idle_per_host(0)
        .user_agent(protocol::USER_AGENT)
        .default_headers({
            let mut headers = header::HeaderMap::new();
            headers.insert(
                header::ACCEPT_LANGUAGE,
                header::HeaderValue::from_static(protocol::ACCEPT_LANGUAGE),
            );
            headers.insert(
                header::CACHE_CONTROL,
                header::HeaderValue::from_static("no-cache"),
            );
            headers
        })
        .build()
        .expect("static reqwest client configuration cannot fail")
}

fn random_hex12() -> String {
    let mut rng = rand::rng();
    (0..12)
        .map(|_| format!("{:x}", rng.random_range(0..16)))
        .collect()
}

/// Classify a reqwest error into the neutral error type.
fn map_transport_error(e: reqwest::Error) -> MachineError {
    if e.is_timeout() {
        MachineError::Timeout
    } else if e.is_connect() {
        MachineError::Unreachable(concise_reqwest_error(&e))
    } else {
        MachineError::Protocol(concise_reqwest_error(&e))
    }
}

/// reqwest error strings nest sources ("error sending request for url ...:
/// ..."); walk to the root cause for a message a user can act on.
fn concise_reqwest_error(e: &reqwest::Error) -> String {
    let mut source: &dyn std::error::Error = e;
    while let Some(inner) = source.source() {
        source = inner;
    }
    source.to_string()
}

/// Run `attempt` up to `attempts` times with a fixed delay between tries.
/// Machine-side rejections are not retried — the machine meant it.
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
