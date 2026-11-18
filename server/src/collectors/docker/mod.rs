//! Sammler des Bereichs `docker`: Bestand über die Docker-API (Ereignisstrom), Messwerte jede
//! Sekunde direkt aus den cgroups. Bei `core.cfg.demo` glaubwürdige Beispieldaten.

use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, OnceLock},
};

use parking_lot::RwLock;
use shared::docker::{Container, DockerInfo};

use crate::core::Core;

pub mod api;
mod cgroup;
pub mod demo;
pub mod inventory;
mod stats;

/// Ein Container mit allem, was Sammler und API brauchen.
#[derive(Debug, Clone, Default)]
pub struct Entry {
    pub c: Container,
    pub full_id: String,
    pub image_id: String,
    /// Hauptprozess (0 = läuft nicht).
    pub pid: i32,
    pub tty: bool,
    pub labels: BTreeMap<String, String>,
}

/// Gemeinsamer Stand von Bestand (Ereignisstrom) und Takt-Sammler.
#[derive(Debug, Default)]
pub struct State {
    /// Zählt bei jeder Änderung des Bestands hoch.
    pub version: u64,
    /// Nach Namen sortiert.
    pub entries: Vec<Entry>,
    pub info: DockerInfo,
    pub volume_sizes: HashMap<String, i64>,
    pub df_running: bool,
    /// Volle ID → cgroup-Ordner (für die Detailansicht).
    pub cgroups: HashMap<String, String>,
}

pub type Shared = Arc<RwLock<State>>;

static SHARED: OnceLock<Shared> = OnceLock::new();

/// Stand des Docker-Sammlers (für die API).
pub fn shared() -> Option<&'static Shared> {
    SHARED.get()
}

pub fn start(core: &Core) {
    let shared: Shared = Arc::default();
    let _ = SHARED.set(shared.clone());
    if core.cfg.demo {
        demo::start(core, shared);
        return;
    }
    raise_nofile();
    tokio::spawn(inventory::run(core.clone(), shared.clone()));
    let source = stats::CgroupSource::new(&core.cfg.sys, &core.cfg.proc);
    crate::collectors::spawn(core, stats::Stats::new(core, shared, Box::new(source)));
}

/// Die cgroup-Dateien bleiben offen (≈ 8 je laufendem Container): weiche Grenze für offene
/// Dateien auf die harte anheben (höchstens 65536), falls sie knapp eingestellt ist.
fn raise_nofile() {
    let mut lim = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    // SAFETY: gültiger Zeiger auf eine rlimit-Struktur.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut lim) } != 0 {
        return;
    }
    let want = lim.rlim_max.min(65_536);
    if lim.rlim_cur < want {
        lim.rlim_cur = want;
        // SAFETY: wie oben; Fehler sind harmlos (Grenze bleibt).
        unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raw const lim) };
    }
}
