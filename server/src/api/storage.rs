//! Speicher: Belegung und Einstellungen (`/api/v1/storage/…`).

use std::{sync::OnceLock, time::Duration};

use axum::{Json, Router, extract::State, routing::get};
use shared::storage::{StorageSettings, StorageStats};

use super::ApiResult;
use crate::core::Core;

pub fn routes() -> Router<Core> {
    Router::new().route("/storage/stats", get(stats)).route("/storage/settings", get(get_settings).put(put_settings))
}

async fn stats(State(core): State<Core>) -> ApiResult<StorageStats> {
    ensure_publisher(&core);
    let s = core.store.stats().await?;
    core.snapshot("storage.stats", &s);
    Ok(Json(s))
}

async fn get_settings(State(core): State<Core>) -> Json<StorageSettings> {
    Json(core.store.settings())
}

async fn put_settings(State(core): State<Core>, Json(s): Json<StorageSettings>) -> ApiResult<StorageSettings> {
    let s = s.sanitized();
    core.settings.set("storage", &s)?;
    core.store.set_settings(s.clone());
    Ok(Json(s))
}

/// Momentaufnahme `storage.stats` laufend ablegen. Der Kern hat für Bereich 1 keinen Start-Haken –
/// darum startet die erste Abfrage (jede Seite fragt Zeitreihen ab) diese Aufgabe einmalig.
pub fn ensure_publisher(core: &Core) {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    let core = core.clone();
    tokio::spawn(async move {
        loop {
            match core.store.stats().await {
                Ok(s) => core.snapshot("storage.stats", &s),
                Err(e) => tracing::warn!("Speicher-Kennzahlen: {e:#}"),
            }
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    });
}
