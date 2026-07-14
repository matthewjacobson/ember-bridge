//! Serial transport for the dongle's USB setup channel.
//!
//! A dongle plugged into this computer shows up as a CDC-ACM serial port
//! (alongside its mass-storage volume). The firmware speaks line-delimited
//! JSON on it: requests carry an `id` which the response echoes; lines with
//! an `event` field instead of an `id` are progress notifications.

use super::{SetupError, SetupResult};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Request ids are process-global, not per-connection: every command opens
/// the port fresh, and a response stranded by a previous session must never
/// be mistakable for the current one.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Espressif's development VID/PID, matching the firmware's descriptor.
/// Both sides must move to a product-specific PID before shipping.
const USB_VID: u16 = 0x303a;
const USB_PID: u16 = 0x4002;

/// How long the port may stay silent while we wait for a response line.
const READ_POLL: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DongleSummary {
    /// OS port path, e.g. `/dev/cu.usbmodem1101` or `COM5`.
    pub port: String,
    /// USB serial number — the dongle's MAC-derived serial.
    pub serial: Option<String>,
}

/// Every EmberConnect dongle currently plugged into this computer.
pub fn list() -> Vec<DongleSummary> {
    let ports = serialport::available_ports().unwrap_or_default();
    ports
        .into_iter()
        .filter_map(|p| match p.port_type {
            serialport::SerialPortType::UsbPort(usb)
                if usb.vid == USB_VID && usb.pid == USB_PID =>
            {
                Some(DongleSummary {
                    port: p.port_name,
                    serial: usb.serial_number,
                })
            }
            _ => None,
        })
        // macOS lists each device twice; the callout node is the right one.
        .filter(|d| !d.port.starts_with("/dev/tty.") || cfg!(not(target_os = "macos")))
        .collect()
}

pub struct DongleLink {
    port: Box<dyn serialport::SerialPort>,
    buf: Vec<u8>,
}

impl DongleLink {
    pub fn open(port_name: &str) -> SetupResult<Self> {
        // The baud rate is decorative — CDC over USB ignores it.
        let mut port = serialport::new(port_name, 115_200)
            .timeout(READ_POLL)
            .open()
            .map_err(|e| SetupError::new("port_open_failed", format!("{port_name}: {e}")))?;
        let _ = port.write_data_terminal_ready(true);
        // Drop anything the OS buffered from a previous session; the DTR
        // rise makes the firmware do the same on its side.
        let _ = port.clear(serialport::ClearBuffer::All);
        Ok(Self {
            port,
            buf: Vec::new(),
        })
    }

    /// Send `cmd` (plus arguments) and wait for its response, feeding any
    /// interleaved `event` lines to `on_event`. `timeout` bounds the whole
    /// exchange — provisioning legitimately takes tens of seconds.
    pub fn request(
        &mut self,
        cmd: &str,
        mut args: Value,
        timeout: Duration,
        mut on_event: impl FnMut(&Value),
    ) -> SetupResult<Value> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        args["cmd"] = json!(cmd);
        args["id"] = json!(id);

        let mut line = serde_json::to_vec(&args).expect("json values always serialize");
        line.push(b'\n');
        self.port
            .write_all(&line)
            .and_then(|_| self.port.flush())
            .map_err(|e| SetupError::new("port_write_failed", e.to_string()))?;

        let deadline = Instant::now() + timeout;
        loop {
            let Some(raw) = self.read_line(deadline)? else {
                return Err(SetupError::new(
                    "dongle_timeout",
                    format!("no response to \"{cmd}\" within {}s", timeout.as_secs()),
                ));
            };
            let Ok(value) = serde_json::from_str::<Value>(&raw) else {
                continue; // boot noise or a torn line; keep waiting
            };
            if value.get("event").is_some() {
                on_event(&value);
                continue;
            }
            if value.get("id").and_then(Value::as_u64) != Some(id) {
                continue; // stale response from an interrupted predecessor
            }
            if value.get("ok").and_then(Value::as_bool) == Some(true) {
                return Ok(value);
            }
            let code = value
                .pointer("/error/code")
                .and_then(Value::as_str)
                .unwrap_or("dongle_error");
            let message = value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("the dongle rejected the request");
            return Err(SetupError::new(code, message));
        }
    }

    /// Stream raw bytes (a firmware image) down the pipe.
    pub fn write_raw(&mut self, bytes: &[u8]) -> SetupResult<()> {
        self.port
            .write_all(bytes)
            .and_then(|_| self.port.flush())
            .map_err(|e| SetupError::new("port_write_failed", e.to_string()))
    }

    /// Wait for one further protocol line (event or response) — used while
    /// streaming an update, where responses arrive without a fresh request.
    pub fn next_line(&mut self, timeout: Duration) -> SetupResult<Option<Value>> {
        let deadline = Instant::now() + timeout;
        loop {
            let Some(raw) = self.read_line(deadline)? else {
                return Ok(None);
            };
            if let Ok(value) = serde_json::from_str::<Value>(&raw) {
                return Ok(Some(value));
            }
        }
    }

    /// One `\n`-terminated line, or `None` once `deadline` passes.
    fn read_line(&mut self, deadline: Instant) -> SetupResult<Option<String>> {
        loop {
            if let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
                let mut line: Vec<u8> = self.buf.drain(..=pos).collect();
                line.pop(); // the \n
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                return Ok(Some(String::from_utf8_lossy(&line).into_owned()));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            let mut chunk = [0u8; 512];
            match self.port.read(&mut chunk) {
                Ok(0) => return Err(SetupError::new("port_closed", "the dongle went away")),
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
                Err(e) => return Err(SetupError::new("port_read_failed", e.to_string())),
            }
        }
    }
}
