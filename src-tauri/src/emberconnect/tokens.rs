//! Per-dongle pairing tokens.
//!
//! Firmware 0.4.0+ requires a bearer token on every API call except
//! `/api/health` and `/api/pair`. Tokens are minted by the dongle when we
//! pair (allowed for a few minutes after its power-on, after a button tap,
//! or in setup mode) and stored here, keyed by dongle serial, in
//! `emberconnect-tokens.json` next to `config.json`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

pub struct TokenStore {
    path: PathBuf,
    map: RwLock<HashMap<String, String>>,
}

impl TokenStore {
    /// Load the store; a missing file is simply "no dongles paired yet".
    pub fn load(dir: &Path) -> Self {
        let path = dir.join("emberconnect-tokens.json");
        let map = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        Self {
            path,
            map: RwLock::new(map),
        }
    }

    pub fn get(&self, serial: &str) -> Option<String> {
        self.map.read().unwrap().get(serial).cloned()
    }

    /// Remember a freshly minted token. Persistence failures are logged, not
    /// fatal: the token still works for this session and pairing can rerun.
    pub fn set(&self, serial: &str, token: &str) {
        let mut map = self.map.write().unwrap();
        map.insert(serial.to_string(), token.to_string());
        if let Err(e) = persist(&self.path, &map) {
            tracing::warn!("could not persist dongle token: {e}");
        }
    }

    /// Drop a token the dongle no longer accepts (revoked / factory reset).
    pub fn forget(&self, serial: &str) {
        let mut map = self.map.write().unwrap();
        if map.remove(serial).is_some() {
            if let Err(e) = persist(&self.path, &map) {
                tracing::warn!("could not persist dongle token removal: {e}");
            }
        }
    }
}

/// Same discipline as the main config: temp file + rename, owner-only.
fn persist(path: &Path, map: &HashMap<String, String>) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(map).expect("string map serialization cannot fail");
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)
}
