//! In-memory application log.
//!
//! A bounded ring buffer of user-relevant events (discovery, uploads,
//! errors), exposed through `GET /api/logs` and shown on the Logs page.
//! Entries carry a monotonically increasing sequence number so clients can
//! poll incrementally (`?afterSeq=`). Everything is mirrored to `tracing`
//! for terminal debugging.

use serde::Serialize;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const CAPACITY: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    /// Monotonic sequence number, 1-based.
    pub seq: u64,
    /// Milliseconds since the Unix epoch.
    pub timestamp_ms: u64,
    pub level: LogLevel,
    pub message: String,
}

#[derive(Default)]
pub struct LogBuffer {
    entries: Mutex<VecDeque<LogEntry>>,
    seq: AtomicU64,
}

impl LogBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn info(&self, message: impl Into<String>) {
        self.push(LogLevel::Info, message.into());
    }

    pub fn warn(&self, message: impl Into<String>) {
        self.push(LogLevel::Warn, message.into());
    }

    pub fn error(&self, message: impl Into<String>) {
        self.push(LogLevel::Error, message.into());
    }

    fn push(&self, level: LogLevel, message: String) {
        match level {
            LogLevel::Info => tracing::info!("{message}"),
            LogLevel::Warn => tracing::warn!("{message}"),
            LogLevel::Error => tracing::error!("{message}"),
        }
        let entry = LogEntry {
            seq: self.seq.fetch_add(1, Ordering::Relaxed) + 1,
            timestamp_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            level,
            message,
        };
        let mut entries = self.entries.lock().expect("log mutex poisoned");
        if entries.len() == CAPACITY {
            entries.pop_front();
        }
        entries.push_back(entry);
    }

    /// Entries with `seq > after_seq`, oldest first.
    pub fn since(&self, after_seq: u64) -> Vec<LogEntry> {
        let entries = self.entries.lock().expect("log mutex poisoned");
        entries
            .iter()
            .filter(|e| e.seq > after_seq)
            .cloned()
            .collect()
    }

    pub fn last_seq(&self) -> u64 {
        self.seq.load(Ordering::Relaxed)
    }
}
