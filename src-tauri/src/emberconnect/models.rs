//! Serde types for the EmberConnect dongle's JSON API.
//!
//! The wire format is defined by the firmware (EmberConnect repo,
//! `firmware/main/http_api.c`); fields are camelCase.

use serde::Deserialize;

/// `GET /api/health`
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Health {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub name: String,
    pub version: Option<String>,
    pub serial: Option<String>,
}

impl Health {
    pub fn is_ember_connect(&self) -> bool {
        self.ok && self.name == "EmberConnect"
    }
}

/// `GET /api/info`
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DongleInfo {
    #[serde(default)]
    pub name: String,
    pub version: Option<String>,
    pub serial: Option<String>,
    pub storage: DongleStorage,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DongleStorage {
    #[serde(default)]
    pub total_bytes: u64,
    #[serde(default)]
    pub free_bytes: u64,
}

/// `GET /api/files`
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileList {
    #[serde(default)]
    pub files: Vec<DongleFile>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DongleFile {
    pub name: String,
    #[serde(default)]
    pub size: u64,
}

/// `POST /api/upload` success body.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadResponse {
    #[serde(default)]
    pub ok: bool,
    pub file: Option<DongleFile>,
}

/// Error envelope: `{"error": {"code": "...", "message": "..."}}`.
#[derive(Debug, Clone, Deserialize)]
pub struct ErrorResponse {
    pub error: ErrorBody,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ErrorBody {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_parses_and_identifies() {
        let health: Health = serde_json::from_str(
            r#"{"ok":true,"name":"EmberConnect","version":"0.1.0","serial":"A1B2C3D4E5F6"}"#,
        )
        .unwrap();
        assert!(health.is_ember_connect());
        assert_eq!(health.serial.as_deref(), Some("A1B2C3D4E5F6"));
    }

    #[test]
    fn foreign_devices_are_not_ours() {
        // Some random LAN thing answering 200 with JSON on port 80.
        let health: Health =
            serde_json::from_str(r#"{"ok":true,"name":"SmartToaster"}"#).unwrap();
        assert!(!health.is_ember_connect());
        // Non-JSON-matching shapes should still deserialize leniently.
        let health: Health = serde_json::from_str(r#"{}"#).unwrap();
        assert!(!health.is_ember_connect());
    }

    #[test]
    fn info_and_files_parse() {
        let info: DongleInfo = serde_json::from_str(
            r#"{"name":"EmberConnect","version":"0.1.0","serial":"A1B2C3D4E5F6",
                "ip":"192.168.1.50","storage":{"totalBytes":31914983424,"freeBytes":31914950656}}"#,
        )
        .unwrap();
        assert_eq!(info.storage.total_bytes, 31_914_983_424);

        let files: FileList =
            serde_json::from_str(r#"{"files":[{"name":"rose.pes","size":24576}]}"#).unwrap();
        assert_eq!(files.files[0].name, "rose.pes");
    }

    #[test]
    fn error_envelope_parses() {
        let err: ErrorResponse = serde_json::from_str(
            r#"{"error":{"code":"insufficient_storage","message":"not enough free space"}}"#,
        )
        .unwrap();
        assert_eq!(err.error.code, "insufficient_storage");
    }
}
