//! Typen des Bereichs `system` (Momentaufnahmen, Listen).
//! Besitzer und Inhalt: siehe `ARCHITECTURE.md`.

use serde::{Deserialize, Serialize};

/// Momentaufnahme `system.info`: Eckdaten des Hosts (ändern sich selten).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HostInfo {
    pub hostname: String,
    /// z. B. `Debian GNU/Linux 12 (bookworm)`.
    pub os: String,
    pub kernel: String,
    /// `x86_64`, `aarch64`.
    pub arch: String,
    pub cpu_model: String,
    /// Logische Kerne.
    pub cpu_cores: u32,
    /// Bytes.
    pub mem_total: u64,
    pub swap_total: u64,
    /// Start des Hosts (Unix-Sekunden).
    pub boot_time: i64,
    /// `kvm`, `docker` … oder leer.
    pub virtualization: String,
}

/// Momentaufnahme `system.disks`: ein Eintrag je echtem Blockgerät (ohne loop/ram/Partitionen).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DiskInfo {
    /// Kernel-Name, z. B. `nvme0n1`, `sda` (= Instanz der `disk.*`-Diagramme).
    pub name: String,
    pub model: String,
    pub serial: String,
    /// `nvme`, `ssd`, `hdd`, `virtual`, `usb`, `mmc`.
    pub kind: String,
    /// Bytes.
    pub size: u64,
    pub temp: Option<f64>,
    /// Bytes/s.
    pub read: f64,
    pub write: f64,
    /// Prozent beschäftigt.
    pub util: f64,
    /// Partitionen (Namen).
    pub partitions: Vec<String>,
}

/// Momentaufnahme `system.filesystems`: eingehängte echte Dateisysteme.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FsInfo {
    /// Einhängepunkt auf dem Host, z. B. `/`, `/mnt/data` (= Instanz von `disk.space`/`disk.inodes`).
    pub mount: String,
    /// Gerät, z. B. `/dev/nvme0n1p2`.
    pub device: String,
    pub fstype: String,
    /// Bytes.
    pub size: u64,
    pub used: u64,
    pub avail: u64,
    pub inodes: u64,
    pub inodes_used: u64,
    pub readonly: bool,
}

impl FsInfo {
    /// Belegung in Prozent wie `df` (belegt / (belegt + verfügbar)).
    #[must_use]
    pub fn used_pct(&self) -> f64 {
        let base = self.used + self.avail;
        if base == 0 { 0.0 } else { self.used as f64 / base as f64 * 100.0 }
    }
}

/// Momentaufnahme `system.interfaces`: Netzwerk-Schnittstellen (ohne einzelne veth*).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct IfaceInfo {
    /// z. B. `enp1s0`, `wt0`, `docker0` (= Instanz der `net.*`-Diagramme).
    pub name: String,
    /// `physical`, `wireless`, `bridge`, `vpn`, `docker`, `virtual`.
    pub kind: String,
    pub up: bool,
    /// `up`, `down`, `unknown` …
    pub state: String,
    pub mac: String,
    pub mtu: u32,
    /// Mbit/s, 0 = unbekannt.
    pub speed: u32,
    /// Adressen mit Präfix, z. B. `192.168.1.10/24`.
    pub addrs: Vec<String>,
    /// Bit/s.
    pub rx: f64,
    pub tx: f64,
    /// Bytes seit Start der Schnittstelle.
    pub rx_total: u64,
    pub tx_total: u64,
    /// Bei Bridges: Anzahl angeschlossener Ports (veth …).
    pub ports: u32,
}

/// Art eines Fühlers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SensorKind {
    #[default]
    Temp,
    Fan,
    Voltage,
    Power,
    Current,
    /// Thermische Zone (`/sys/class/thermal`).
    Thermal,
    /// Akku-Ladung in Prozent.
    Battery,
}

/// Momentaufnahme `system.sensors`: ein Eintrag je Fühler.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SensorInfo {
    /// Instanz des Chips, z. B. `coretemp`, `nvme`, `acpitz` (bei mehreren gleichen mit Gerät: `nvme-nvme1`).
    pub chip: String,
    /// Lesbarer Name des Chips, z. B. `Intel Core (coretemp)`.
    pub chip_label: String,
    /// Technischer Name des Fühlers, z. B. `temp1` (= Dimension im Diagramm).
    pub id: String,
    /// Lesbarer Name, z. B. `Package id 0`, `Core 2`, `Composite`.
    pub label: String,
    pub kind: SensorKind,
    pub value: Option<f64>,
    pub min: Option<f64>,
    /// Warngrenze (`max`/`high`).
    pub max: Option<f64>,
    /// Kritische Grenze (`crit`).
    pub crit: Option<f64>,
    /// Diagramm mit dem Verlauf (z. B. `sensors.temp.coretemp`) und Index der Dimension darin.
    pub chart: String,
    pub dim: usize,
}

impl SensorInfo {
    /// Massgebliche Obergrenze: zuerst `crit`, sonst `max`.
    #[must_use]
    pub fn limit(&self) -> Option<f64> {
        self.crit.or(self.max)
    }
}

/// Momentaufnahme `system.processes`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProcessList {
    /// Unix-Sekunden der Messung.
    pub ts: i64,
    pub total: u32,
    pub threads: u32,
    pub running: u32,
    /// Logische Kerne (CPU % je Prozess: 100 = ein Kern).
    pub cores: u32,
    /// Bytes Arbeitsspeicher gesamt (für Anteile).
    pub mem_total: u64,
    pub procs: Vec<ProcInfo>,
    /// Summen je Programmgruppe, grösste zuerst.
    pub groups: Vec<ProcGroup>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProcInfo {
    pub pid: u32,
    pub ppid: u32,
    /// Kurzname (`comm`).
    pub name: String,
    pub user: String,
    /// Prozent (100 = ein Kern).
    pub cpu: f64,
    /// Belegter Arbeitsspeicher (RSS) in Bytes.
    pub mem: u64,
    /// `R`, `S`, `D`, `I`, `T`, `Z`.
    pub state: String,
    /// Start (Unix-Sekunden).
    pub started: i64,
    pub threads: u32,
    /// Befehlszeile (gekürzt), bei Kernel-Threads leer.
    pub cmd: String,
    /// Container-Name (leer = Host).
    pub container: String,
    /// Programmgruppe (wie in den `proc.*`-Diagrammen).
    pub group: String,
    /// E/A in Bytes/s (falls lesbar).
    pub read: f64,
    pub write: f64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProcGroup {
    pub name: String,
    pub count: u32,
    pub cpu: f64,
    pub mem: u64,
    pub threads: u32,
    pub fds: u32,
    pub read: f64,
    pub write: f64,
}

/// Schlüssel der Momentaufnahmen dieses Bereichs.
pub mod snap {
    pub const INFO: &str = "system.info";
    pub const DISKS: &str = "system.disks";
    pub const FILESYSTEMS: &str = "system.filesystems";
    pub const INTERFACES: &str = "system.interfaces";
    pub const SENSORS: &str = "system.sensors";
    pub const PROCESSES: &str = "system.processes";
}
