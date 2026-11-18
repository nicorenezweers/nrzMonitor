//! Einstellungen, die in der Oberfläche geändert werden (`<data>/settings.json`): je Bereich ein
//! Schlüssel (`dns`, `cloudflare`, `netbird`, `probes`, `alerts`, `storage` …) mit eigenem Typ aus
//! `shared`. Sammler lesen mit [`Settings::get`] und bekommen Änderungen über [`Settings::subscribe`].
//!
//! Die Datei enthält Zugangsdaten und wird nur für den Besitzer lesbar geschrieben. Endpunkte
//! geben Geheimnisse nie im Klartext zurück – das regelt jeder Bereich selbst.

use std::path::PathBuf;

use parking_lot::RwLock;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};
use tokio::sync::watch;

pub struct Settings {
    path: PathBuf,
    state: RwLock<Map<String, Value>>,
    tx: watch::Sender<u64>,
}

impl Settings {
    pub fn load(path: PathBuf) -> Self {
        let state = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Map<String, Value>>(&s).ok())
            .unwrap_or_default();
        Self { path, state: RwLock::new(state), tx: watch::channel(0).0 }
    }

    /// Einstellungen eines Bereichs; fehlt etwas oder passt es nicht, gilt der Standard des Typs.
    pub fn get<T: DeserializeOwned + Default>(&self, key: &str) -> T {
        self.state.read().get(key).cloned().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
    }

    pub fn set<T: Serialize>(&self, key: &str, value: &T) -> anyhow::Result<()> {
        let value = serde_json::to_value(value)?;
        let json = {
            let mut state = self.state.write();
            state.insert(key.to_string(), value);
            serde_json::to_string_pretty(&*state)?
        };
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&tmp, &self.path)?;
        self.tx.send_modify(|n| *n += 1);
        Ok(())
    }

    /// Meldet sich bei jeder Änderung (egal welcher Bereich).
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.tx.subscribe()
    }
}
