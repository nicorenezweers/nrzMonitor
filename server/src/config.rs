//! Einstellungen aus der Umgebung (`NRZMON_*`). Alles hat einen sinnvollen Standard.

use std::{
    env,
    net::SocketAddr,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct Config {
    /// Adressen der Weboberfläche und API (`NRZMON_BIND`, mehrere mit Komma).
    pub bind: Vec<SocketAddr>,
    /// Ordner für DuckDB, Parquet-Dateien und `settings.json` (`NRZMON_DATA_DIR`).
    pub data_dir: PathBuf,
    /// `/proc` des Hosts (`NRZMON_HOST_PROC`; im Container `/host/proc`).
    pub proc: PathBuf,
    /// `/sys` des Hosts (`NRZMON_HOST_SYS`).
    pub sys: PathBuf,
    /// Wurzel des Host-Dateisystems (`NRZMON_HOST_ROOT`; im Container `/host/root`, sonst `/`).
    pub root: PathBuf,
    /// Docker-Socket (`NRZMON_DOCKER_SOCK`).
    pub docker_sock: PathBuf,
    /// Sekunden zwischen zwei Messungen der lokalen Sammler (`NRZMON_INTERVAL`, 1–60).
    pub interval: u32,
    /// Sekunden in voller Auflösung im Arbeitsspeicher (`NRZMON_LIVE_WINDOW`).
    pub live_window: u32,
    /// Beispieldaten statt echter Sammler (`NRZMON_DEMO=1`; immer an, wenn es kein Linux ist).
    pub demo: bool,
    /// Passwort der Oberfläche (`NRZMON_PASSWORD`); leer = ohne Anmeldung.
    pub password: Option<String>,
    /// Oberfläche aus diesem Ordner statt der eingebauten ausliefern (`NRZMON_WEB_DIR`).
    pub web_dir: Option<PathBuf>,
    /// Obergrenze für DuckDBs Arbeitsspeicher (`NRZMON_DB_MEMORY`, z. B. `256MB`).
    pub db_memory: String,
    /// Threads von DuckDB (`NRZMON_DB_THREADS`).
    pub db_threads: u32,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let host = |var: &str, container: &str, native: &str| -> PathBuf {
            match env::var(var) {
                Ok(v) if !v.is_empty() => PathBuf::from(v),
                _ if Path::new(container).is_dir() => PathBuf::from(container),
                _ => PathBuf::from(native),
            }
        };
        let bind = var("NRZMON_BIND").unwrap_or_else(|| "0.0.0.0:8571".to_string());
        let bind = bind
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.parse::<SocketAddr>().map_err(|e| anyhow::anyhow!("NRZMON_BIND `{s}`: {e}")))
            .collect::<anyhow::Result<Vec<_>>>()?;
        anyhow::ensure!(!bind.is_empty(), "NRZMON_BIND ist leer");
        Ok(Self {
            bind,
            data_dir: PathBuf::from(var("NRZMON_DATA_DIR").unwrap_or_else(|| "data".to_string())),
            proc: host("NRZMON_HOST_PROC", "/host/proc", "/proc"),
            sys: host("NRZMON_HOST_SYS", "/host/sys", "/sys"),
            root: host("NRZMON_HOST_ROOT", "/host/root", "/"),
            docker_sock: PathBuf::from(var("NRZMON_DOCKER_SOCK").unwrap_or_else(|| "/var/run/docker.sock".to_string())),
            interval: num("NRZMON_INTERVAL", 1).clamp(1, 60),
            live_window: num("NRZMON_LIVE_WINDOW", 900).clamp(120, 86_400),
            demo: flag("NRZMON_DEMO") || !cfg!(target_os = "linux"),
            password: var("NRZMON_PASSWORD"),
            web_dir: var("NRZMON_WEB_DIR").map(PathBuf::from),
            db_memory: var("NRZMON_DB_MEMORY").unwrap_or_else(|| "256MB".to_string()),
            db_threads: num("NRZMON_DB_THREADS", 2).clamp(1, 16),
        })
    }
}

/// Umgebungsvariable, leer zählt als nicht gesetzt.
pub fn var(name: &str) -> Option<String> {
    env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

pub fn flag(name: &str) -> bool {
    matches!(var(name).as_deref(), Some("1" | "true" | "yes" | "on"))
}

pub fn num(name: &str, default: u32) -> u32 {
    var(name).and_then(|v| v.parse().ok()).unwrap_or(default)
}
