//! Momentaufnahmen: der letzte bekannte Stand von Listen und Zuständen, die keine Zeitreihen sind
//! (Containerliste, Prozesse, Peers …). Sammler legen sie mit `core.snapshot(key, &daten)` ab,
//! die Oberfläche liest sie über `GET /api/v1/snap/{key}` oder abonniert sie im Live-Kanal.

use std::{collections::HashMap, sync::Arc};

use parking_lot::RwLock;
use serde_json::Value;
use tokio::sync::broadcast;

pub struct Snap {
    pub key: String,
    /// Unix-Millisekunden.
    pub ts: i64,
    pub data: Value,
}

pub struct Snaps {
    map: RwLock<HashMap<String, Arc<Snap>>>,
    tx: broadcast::Sender<Arc<Snap>>,
}

impl Default for Snaps {
    fn default() -> Self {
        Self { map: RwLock::new(HashMap::new()), tx: broadcast::channel(256).0 }
    }
}

impl Snaps {
    /// Ablegen und an Abonnenten schicken. Unveränderte Daten werden nicht noch einmal verschickt.
    pub fn put(&self, key: &str, ts: i64, data: Value) {
        let changed = self.map.read().get(key).is_none_or(|old| old.data != data);
        let snap = Arc::new(Snap { key: key.to_string(), ts, data });
        self.map.write().insert(key.to_string(), snap.clone());
        if changed {
            let _ = self.tx.send(snap);
        }
    }

    pub fn get(&self, key: &str) -> Option<Arc<Snap>> {
        self.map.read().get(key).cloned()
    }

    pub fn keys(&self) -> Vec<String> {
        let mut k: Vec<String> = self.map.read().keys().cloned().collect();
        k.sort();
        k
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Arc<Snap>> {
        self.tx.subscribe()
    }
}
