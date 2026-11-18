//! Typen des Bereichs `docker` (Momentaufnahmen, Einstellungen, Listen).
//! Besitzer und Inhalt: siehe `ARCHITECTURE.md`.
//!
//! Momentaufnahmen:
//! - `docker.containers` → `Vec<Container>` (Bestand und aktuelle Werte, jede Sekunde)
//! - `docker.apps` → `Vec<App>` (je Compose-Projekt)
//! - `docker.images` → `Vec<Image>`, `docker.volumes` → `Vec<Volume>`, `docker.networks` → `Vec<Network>`
//! - `docker.info` → `DockerInfo`

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Veröffentlichter Port.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Port {
    /// Adresse auf dem Host (`0.0.0.0`, `::`), leer = nicht veröffentlicht.
    pub ip: String,
    pub private: u16,
    /// 0 = nicht veröffentlicht.
    pub public: u16,
    /// `tcp`, `udp`.
    pub proto: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Mount {
    /// `bind`, `volume`, `tmpfs`.
    pub kind: String,
    /// Pfad auf dem Host oder Name des Volumes.
    pub source: String,
    pub destination: String,
    pub rw: bool,
}

/// Netzwerk, an dem ein Container hängt.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NetAttach {
    pub name: String,
    pub ip: String,
    pub ipv6: String,
    pub mac: String,
}

/// Ein Container mit Bestand (Docker-API) und aktuellen Werten (cgroups).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Container {
    /// Kurze ID (12 Zeichen).
    pub id: String,
    pub name: String,
    pub image: String,
    /// App = Compose-Projekt (`com.docker.compose.project`), sonst der Containername.
    pub app: String,
    /// Dienst im Compose-Projekt.
    pub service: String,
    /// `running`, `exited`, `paused`, `restarting`, `created`, `dead`, `removing`.
    pub state: String,
    /// `healthy`, `unhealthy`, `starting` oder leer (kein Healthcheck).
    pub health: String,
    /// Text von Docker, z. B. `Up 3 hours`.
    pub status: String,
    /// Unix-Sekunden (0 = unbekannt).
    pub created: i64,
    pub started: i64,
    pub finished: i64,
    pub exit_code: i64,
    pub restarts: u32,
    /// Letztes Ende durch Speichermangel.
    pub oom_killed: bool,
    /// Zähler `oom_kill` aus `memory.events` (seit Start des Containers).
    pub oom_kills: u64,
    /// Das letzte Ende war gewollt (`docker stop`/`kill`, Code 0 oder Signal beim Stoppen).
    pub expected_exit: bool,
    /// `no`, `always`, `unless-stopped`, `on-failure`.
    pub restart_policy: String,
    pub ports: Vec<Port>,
    pub mounts: Vec<Mount>,
    pub networks: Vec<NetAttach>,
    /// `bridge`, `host`, `container:<id>` …
    pub network_mode: String,
    /// Teilt das Netz des Hosts (oder eines anderen Containers) – Netzwerte fehlen dann.
    pub host_net: bool,
    /// Grenzen: Kerne (0 = keine), Bytes (0 = keine), Prozesse (0 = keine).
    pub cpu_limit: f64,
    pub mem_limit: u64,
    pub pids_limit: u64,
    /// Beschreibbare Schicht in Bytes (selten aktualisiert, 0 = unbekannt).
    pub size_rw: u64,
    // --- aktuelle Werte (nur bei laufenden Containern) ---
    /// Prozent, 100 = ein Kern.
    pub cpu: f64,
    /// Bytes (ohne Dateicache).
    pub mem: f64,
    /// Bytes inkl. Dateicache.
    pub mem_total: f64,
    /// Bit/s.
    pub net_rx: f64,
    pub net_tx: f64,
    /// Bytes/s.
    pub io_read: f64,
    pub io_write: f64,
    pub pids: u32,
}

impl Container {
    pub fn running(&self) -> bool {
        self.state == "running"
    }

    pub fn unhealthy(&self) -> bool {
        self.health == "unhealthy"
    }

    /// Speicher in Prozent der Grenze (`None` ohne Grenze).
    pub fn mem_pct(&self) -> Option<f64> {
        (self.mem_limit > 0).then(|| self.mem / self.mem_limit as f64 * 100.0)
    }
}

/// Zusammenfassung je App (Compose-Projekt).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct App {
    pub name: String,
    /// Containernamen.
    pub containers: Vec<String>,
    pub running: u32,
    pub total: u32,
    pub unhealthy: u32,
    pub restarts: u32,
    /// Ältester Start eines laufenden Containers (Unix-Sekunden).
    pub started: i64,
    /// Arbeitsverzeichnis des Projekts (`com.docker.compose.project.working_dir`).
    pub dir: String,
    pub cpu: f64,
    pub mem: f64,
    pub net_rx: f64,
    pub net_tx: f64,
    pub io_read: f64,
    pub io_write: f64,
}

impl App {
    /// `running`, `partial`, `stopped`.
    pub fn state(&self) -> &'static str {
        if self.running == 0 {
            "stopped"
        } else if self.running < self.total {
            "partial"
        } else {
            "running"
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Image {
    pub id: String,
    pub tags: Vec<String>,
    pub size: u64,
    pub created: i64,
    /// Container, die dieses Image benutzen.
    pub containers: u32,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Volume {
    pub name: String,
    pub driver: String,
    pub mountpoint: String,
    /// Bytes (aus `system/df`, -1 = unbekannt).
    pub size: i64,
    pub containers: u32,
    pub app: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Network {
    pub id: String,
    pub name: String,
    pub driver: String,
    pub scope: String,
    pub subnet: String,
    pub gateway: String,
    pub containers: u32,
    pub internal: bool,
}

/// Docker selbst (Momentaufnahme `docker.info`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DockerInfo {
    /// Docker-API erreichbar.
    pub available: bool,
    pub error: Option<String>,
    pub version: String,
    pub api_version: String,
    pub os: String,
    pub kernel: String,
    /// `1` oder `2`.
    pub cgroup_version: String,
    /// `systemd` oder `cgroupfs`.
    pub cgroup_driver: String,
    pub storage_driver: String,
    pub root_dir: String,
    pub ncpu: u32,
    pub mem_total: u64,
    pub containers: u32,
    pub running: u32,
    pub images: u32,
    /// Speicherplatz (aus `system/df`).
    pub images_size: u64,
    pub containers_size: u64,
    pub volumes_size: u64,
    pub build_cache_size: u64,
    /// Dauer eines Messtakts aller Container in Millisekunden.
    pub collect_ms: f64,
    /// Container, deren cgroup gefunden wurde.
    pub cgroups_found: u32,
}

// ------------------------------------------------------------------ API

/// `GET /api/v1/docker/containers/{name}` – Einzelheiten (nur lesen, Geheimnisse maskiert).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ContainerDetails {
    pub container: Container,
    pub full_id: String,
    pub image_id: String,
    pub cmd: Vec<String>,
    pub entrypoint: Vec<String>,
    pub working_dir: String,
    pub user: String,
    pub hostname: String,
    pub privileged: bool,
    pub cap_add: Vec<String>,
    /// (Name, Wert) – Werte mit `pass`, `secret`, `token`, `key` im Namen sind maskiert.
    pub env: Vec<(String, String)>,
    pub labels: BTreeMap<String, String>,
    pub healthcheck: Vec<String>,
    /// Letzte Ausgaben des Healthchecks.
    pub health_log: Vec<HealthLog>,
    pub cgroup: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HealthLog {
    /// Unix-Sekunden.
    pub start: i64,
    pub exit_code: i64,
    pub output: String,
}

/// Eine Zeile aus den Logs.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LogLine {
    /// Unix-Nanosekunden (aus dem Zeitstempel von Docker).
    pub ts: i64,
    /// Fehlerausgabe (stderr).
    pub err: bool,
    pub text: String,
}

/// `GET /api/v1/docker/containers/{name}/logs?tail=200&since=<ns>`
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LogPage {
    pub lines: Vec<LogLine>,
    /// Zeitstempel der letzten Zeile (für `since` beim Folgen).
    pub last: i64,
}

/// `GET /api/v1/docker/containers/{name}/top`
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProcessList {
    pub titles: Vec<String>,
    pub rows: Vec<Vec<String>>,
}
