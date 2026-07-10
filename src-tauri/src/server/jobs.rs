//! Upload job queue.
//!
//! `POST /api/send` returns immediately with a job id; a single background
//! worker performs uploads one at a time (embroidery machines handle exactly
//! one request at once, and serializing also prevents two designs racing for
//! the same free memory). Clients poll `GET /api/jobs/{id}` for progress.

use crate::machine::{MachineError, UploadProgress, UploadRequest};
use crate::server::state::AppState;
use serde::Serialize;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JobState {
    Queued,
    Uploading,
    Done,
    Failed,
}

/// Publicly visible job record (everything the API returns).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobRecord {
    pub id: String,
    pub filename: String,
    pub ip: IpAddr,
    pub state: JobState,
    pub sent_bytes: u64,
    pub total_bytes: u64,
    /// Machine-assigned filename after a successful upload, when known.
    pub stored_as: Option<String>,
    /// Stable error code + message, present when `state == failed`.
    pub error_code: Option<String>,
    pub error: Option<String>,
    pub created_at_ms: u64,
    pub finished_at_ms: Option<u64>,
}

/// What travels through the queue: the record id plus the design bytes
/// (kept out of the record map so finished jobs don't pin file contents).
struct QueuedUpload {
    id: String,
    ip: IpAddr,
    filename: String,
    data: bytes::Bytes,
}

pub struct JobQueue {
    records: Mutex<HashMap<String, JobRecord>>,
    /// Insertion order, for "recent jobs" listings and pruning.
    order: Mutex<Vec<String>>,
    tx: mpsc::UnboundedSender<QueuedUpload>,
    rx: Mutex<Option<mpsc::UnboundedReceiver<QueuedUpload>>>,
    counter: AtomicU64,
}

const MAX_FINISHED_JOBS: usize = 100;

impl JobQueue {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            records: Mutex::new(HashMap::new()),
            order: Mutex::new(Vec::new()),
            tx,
            rx: Mutex::new(Some(rx)),
            counter: AtomicU64::new(0),
        }
    }

    /// Register a job and hand it to the worker. Returns the new record.
    pub fn enqueue(&self, ip: IpAddr, filename: String, data: bytes::Bytes) -> JobRecord {
        let id = format!(
            "job-{}-{}",
            now_ms(),
            self.counter.fetch_add(1, Ordering::Relaxed)
        );
        let record = JobRecord {
            id: id.clone(),
            filename: filename.clone(),
            ip,
            state: JobState::Queued,
            sent_bytes: 0,
            total_bytes: data.len() as u64,
            stored_as: None,
            error_code: None,
            error: None,
            created_at_ms: now_ms(),
            finished_at_ms: None,
        };
        {
            let mut records = self.records.lock().expect("jobs mutex poisoned");
            let mut order = self.order.lock().expect("jobs mutex poisoned");
            records.insert(id.clone(), record.clone());
            order.push(id.clone());
            prune(&mut records, &mut order);
        }
        // The worker holds the receiver for the lifetime of the app, so this
        // can only fail during shutdown, when nobody is watching anyway.
        let _ = self.tx.send(QueuedUpload {
            id,
            ip,
            filename,
            data,
        });
        record
    }

    pub fn get(&self, id: &str) -> Option<JobRecord> {
        self.records
            .lock()
            .expect("jobs mutex poisoned")
            .get(id)
            .cloned()
    }

    /// All known jobs, newest first.
    pub fn list(&self) -> Vec<JobRecord> {
        let records = self.records.lock().expect("jobs mutex poisoned");
        let order = self.order.lock().expect("jobs mutex poisoned");
        order
            .iter()
            .rev()
            .filter_map(|id| records.get(id).cloned())
            .collect()
    }

    /// Number of jobs waiting or in flight.
    pub fn pending_count(&self) -> usize {
        self.records
            .lock()
            .expect("jobs mutex poisoned")
            .values()
            .filter(|r| matches!(r.state, JobState::Queued | JobState::Uploading))
            .count()
    }

    fn update<F: FnOnce(&mut JobRecord)>(&self, id: &str, mutate: F) {
        if let Some(record) = self
            .records
            .lock()
            .expect("jobs mutex poisoned")
            .get_mut(id)
        {
            mutate(record);
        }
    }

    /// Spawn the single upload worker. Called once at startup.
    pub fn start_worker(state: Arc<AppState>) {
        let mut rx = state
            .jobs
            .rx
            .lock()
            .expect("jobs mutex poisoned")
            .take()
            .expect("start_worker must be called exactly once");
        tokio::spawn(async move {
            while let Some(job) = rx.recv().await {
                run_upload(&state, job).await;
            }
        });
    }
}

impl Default for JobQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// Drop the oldest finished jobs beyond the retention cap.
fn prune(records: &mut HashMap<String, JobRecord>, order: &mut Vec<String>) {
    let finished = order
        .iter()
        .filter(|id| {
            records
                .get(*id)
                .is_some_and(|r| matches!(r.state, JobState::Done | JobState::Failed))
        })
        .count();
    if finished <= MAX_FINISHED_JOBS {
        return;
    }
    let mut to_remove = finished - MAX_FINISHED_JOBS;
    order.retain(|id| {
        if to_remove > 0
            && records
                .get(id)
                .is_some_and(|r| matches!(r.state, JobState::Done | JobState::Failed))
        {
            records.remove(id);
            to_remove -= 1;
            false
        } else {
            true
        }
    });
}

/// Execute one queued upload against the machine, updating the record and
/// the application log along the way.
async fn run_upload(state: &Arc<AppState>, job: QueuedUpload) {
    let QueuedUpload {
        id,
        ip,
        filename,
        data,
    } = job;

    state.jobs.update(&id, |r| r.state = JobState::Uploading);
    state
        .logs
        .info(format!("Uploading \"{filename}\" ({} bytes) to {ip}", data.len()));

    // Resolve which backend owns the device at this address.
    let machine = match state.registry.identify(ip).await {
        Ok(Some((machine, _info))) => machine,
        Ok(None) => {
            fail(state, &id, "not_a_machine", format!("no supported embroidery machine found at {ip}"));
            return;
        }
        Err(e) => {
            fail_with(state, &id, e);
            return;
        }
    };

    let progress = {
        let jobs = state.clone();
        let id = id.clone();
        std::sync::Arc::new(move |p: UploadProgress| {
            jobs.jobs.update(&id, |r| {
                r.sent_bytes = p.sent_bytes;
                r.total_bytes = p.total_bytes;
            });
        }) as crate::machine::ProgressFn
    };

    match machine
        .upload(
            UploadRequest {
                filename: filename.clone(),
                data,
            },
            progress,
        )
        .await
    {
        Ok(receipt) => {
            state.jobs.update(&id, |r| {
                r.state = JobState::Done;
                r.sent_bytes = receipt.bytes_sent;
                r.stored_as = receipt.stored_as.clone();
                r.finished_at_ms = Some(now_ms());
            });
            let stored = receipt
                .stored_as
                .map(|n| format!(" (stored as {n})"))
                .unwrap_or_default();
            state
                .logs
                .info(format!("Upload of \"{filename}\" to {ip} finished{stored}"));
        }
        Err(e) => fail_with(state, &id, e),
    }
}

fn fail_with(state: &Arc<AppState>, id: &str, e: MachineError) {
    fail(state, id, e.code(), e.to_string());
}

fn fail(state: &Arc<AppState>, id: &str, code: &str, message: String) {
    state.jobs.update(id, |r| {
        r.state = JobState::Failed;
        r.error_code = Some(code.to_string());
        r.error = Some(message.clone());
        r.finished_at_ms = Some(now_ms());
    });
    state.logs.error(format!("Upload {id} failed: {message}"));
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
