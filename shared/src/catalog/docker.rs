//! Vorlagen für Container (Docker) und Apps (Compose-Projekte, auf umbrelOS = installierte Apps).

use super::{Tpl, d, dn};
use crate::model::{ChartKind::*, Unit::*};

// ------------------------------------------------------------------ je Container (Label `container`)

/// 100 % = ein ganzer Kern.
pub const DOCKER_CPU: Tpl = Tpl::new(
    "docker.cpu", "docker", "CPU", Percent, Stacked, &[d("user", "Benutzer"), d("system", "System")], 100,
).per("container");
pub const DOCKER_MEM: Tpl = Tpl::new(
    "docker.mem", "docker", "Arbeitsspeicher", Bytes, Stacked,
    &[d("anon", "Programm"), d("file", "Dateicache"), d("kernel", "Kernel"), d("swap", "Swap")], 110,
).per("container");
pub const DOCKER_NET: Tpl = Tpl::new(
    "docker.net", "docker", "Netzwerk", BitsPerSec, Area, &[d("received", "Empfangen"), dn("sent", "Gesendet")], 120,
).per("container");
pub const DOCKER_IO: Tpl = Tpl::new(
    "docker.io", "docker", "Datenträger", BytesPerSec, Area, &[d("read", "Lesen"), dn("write", "Schreiben")], 130,
).per("container");
pub const DOCKER_PIDS: Tpl = Tpl::new(
    "docker.pids", "docker", "Prozesse", Count, Line, &[d("pids", "Prozesse")], 140,
).per("container").every(5);
pub const DOCKER_THROTTLE: Tpl = Tpl::new(
    "docker.throttle", "docker", "CPU-Drosselung", Percent, Area, &[d("throttled", "Gedrosselt")], 150,
).per("container").every(5);

// ------------------------------------------------------------------ alle Container zusammen

pub const DOCKER_CONTAINERS: Tpl = Tpl::new(
    "docker.containers", "docker", "Container nach Zustand", Count, Stacked,
    &[
        d("running", "Laufend"), d("paused", "Pausiert"), d("restarting", "Startet neu"),
        d("exited", "Beendet"), d("unhealthy", "Ungesund"),
    ],
    10,
).every(5);
/// Dimensionen: je Container eine (grösste zuerst, Rest als `other`).
pub const DOCKER_CPU_ALL: Tpl = Tpl::new("docker.cpu_all", "docker", "CPU je Container", Percent, Stacked, &[], 20);
pub const DOCKER_MEM_ALL: Tpl = Tpl::new("docker.mem_all", "docker", "Speicher je Container", Bytes, Stacked, &[], 30);
pub const DOCKER_NET_RX_ALL: Tpl = Tpl::new("docker.net_rx_all", "docker", "Empfangen je Container", BitsPerSec, Stacked, &[], 40);
pub const DOCKER_NET_TX_ALL: Tpl = Tpl::new("docker.net_tx_all", "docker", "Gesendet je Container", BitsPerSec, Stacked, &[], 50);
pub const DOCKER_IMAGES: Tpl = Tpl::new(
    "docker.images", "docker", "Speicherplatz von Docker", Bytes, Stacked,
    &[d("images", "Images"), d("containers", "Container"), d("volumes", "Volumes"), d("build_cache", "Build-Cache")], 60,
).every(300);

// ------------------------------------------------------------------ je App (Label `app`)

pub const APP_CPU: Tpl = Tpl::new("app.cpu", "apps", "CPU", Percent, Area, &[d("cpu", "CPU")], 100).per("app");
pub const APP_MEM: Tpl = Tpl::new("app.mem", "apps", "Arbeitsspeicher", Bytes, Area, &[d("mem", "Belegt")], 110).per("app");
pub const APP_NET: Tpl = Tpl::new(
    "app.net", "apps", "Netzwerk", BitsPerSec, Area, &[d("received", "Empfangen"), dn("sent", "Gesendet")], 120,
).per("app");
pub const APP_IO: Tpl = Tpl::new(
    "app.io", "apps", "Datenträger", BytesPerSec, Area, &[d("read", "Lesen"), dn("write", "Schreiben")], 130,
).per("app");
/// Dimensionen: je App eine.
pub const APPS_CPU_ALL: Tpl = Tpl::new("apps.cpu_all", "apps", "CPU je App", Percent, Stacked, &[], 10);
pub const APPS_MEM_ALL: Tpl = Tpl::new("apps.mem_all", "apps", "Speicher je App", Bytes, Stacked, &[], 20);

pub const ALL: &[Tpl] = &[
    DOCKER_CPU, DOCKER_MEM, DOCKER_NET, DOCKER_IO, DOCKER_PIDS, DOCKER_THROTTLE,
    DOCKER_CONTAINERS, DOCKER_CPU_ALL, DOCKER_MEM_ALL, DOCKER_NET_RX_ALL, DOCKER_NET_TX_ALL, DOCKER_IMAGES,
    APP_CPU, APP_MEM, APP_NET, APP_IO, APPS_CPU_ALL, APPS_MEM_ALL,
];
