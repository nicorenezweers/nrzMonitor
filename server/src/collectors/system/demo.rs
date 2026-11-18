//! Beispieldaten für alle Systemdiagramme und Momentaufnahmen (Entwicklung auf dem Mac, Vorschau,
//! Bildschirmfotos). Ein i5-6500T mit 16 GiB, NVMe, einer SATA-SSD und ~80 Containern.

use shared::{
    catalog::system::*,
    system::{
        DiskInfo, FsInfo, HostInfo, IfaceInfo, ProcGroup, ProcInfo, ProcessList, SensorInfo, SensorKind, snap,
    },
};

use crate::{
    collectors::{self, Collector, Tick},
    core::Core,
    registry::{ChartDef, ChartHandle},
    util::{self, Walk},
};

const CORES: usize = 4;
const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
const MEM_TOTAL: f64 = 16.0 * GIB;
/// (Name, Art, Empfang-Basis in Bit/s)
const IFACES: [(&str, &str, f64); 4] =
    [("enp1s0", "physical", 6.0e6), ("wt0", "vpn", 1.2e6), ("docker0", "docker", 2.0e6), ("br-umbrel", "bridge", 3.0e6)];
const DISKS: [(&str, &str, &str, f64); 2] = [
    ("nvme0n1", "nvme", "Samsung SSD 980 PRO 1TB", 1000.2e9),
    ("sda", "ssd", "CT2000MX500SSD1", 2000.4e9),
];
/// (Einhängepunkt, Gerät, Dateisystem, Grösse, Anfang belegt)
const MOUNTS: [(&str, &str, &str, f64, f64); 3] = [
    ("/", "/dev/nvme0n1p2", "ext4", 915.0 * GIB, 412.0 * GIB),
    ("/boot/efi", "/dev/nvme0n1p1", "vfat", 0.5 * GIB, 0.03 * GIB),
    ("/mnt/data", "/dev/sda1", "ext4", 1832.0 * GIB, 1690.0 * GIB),
];
const SOFTIRQS: [&str; 6] = ["NET_RX", "NET_TX", "TIMER", "SCHED", "RCU", "BLOCK"];
/// Programmgruppen: (Name, Anzahl Prozesse, Basis-CPU %, Speicher in MiB, Threads je Prozess, Benutzer, Container)
const GROUPS: [(&str, u32, f64, f64, u32, &str, &str); 12] = [
    ("bitcoind", 1, 9.0, 2400.0, 24, "1000", "bitcoin_bitcoind_1"),
    ("node", 6, 2.5, 310.0, 11, "1000", ""),
    ("postgres", 9, 0.6, 85.0, 1, "999", "immich_postgres_1"),
    ("dockerd", 1, 1.8, 180.0, 42, "root", ""),
    ("containerd-shim", 14, 0.05, 12.0, 10, "root", ""),
    ("python3", 4, 1.2, 140.0, 6, "1000", "home-assistant_server_1"),
    ("nginx", 5, 0.2, 9.0, 1, "www-data", "nginx-proxy-manager_app_1"),
    ("redis-server", 2, 0.4, 24.0, 5, "999", "immich_redis_1"),
    ("nrzmonitor", 1, 0.9, 64.0, 12, "root", "nrzmonitor_web_1"),
    ("systemd", 3, 0.05, 14.0, 1, "root", ""),
    ("kernel", 22, 0.04, 0.0, 1, "root", ""),
    ("sshd", 2, 0.0, 7.0, 1, "root", ""),
];
const GROUP_DIMS: usize = 11;

pub fn start(core: &Core) {
    let cores = || (0..CORES).map(|i| (format!("cpu{i}"), format!("Kern {i}")));
    let temp_dims = std::iter::once(("package".to_string(), "Gehäuse".to_string()))
        .chain((0..CORES).map(|i| (format!("core{i}"), format!("Kern {i}"))));
    let mut group_dims: Vec<(String, String)> =
        GROUPS.iter().take(GROUP_DIMS).map(|g| (g.0.to_string(), g.0.to_string())).collect();
    group_dims.push(("other".into(), "Andere".into()));
    let gd = || group_dims.clone();
    let per = |tpl: &shared::catalog::Tpl, names: &[&str]| -> Vec<ChartHandle> {
        names.iter().map(|n| core.chart(ChartDef::tpl(tpl).instance(n))).collect()
    };
    let disks: Vec<&str> = DISKS.iter().map(|d| d.0).collect();
    let mounts: Vec<&str> = MOUNTS.iter().map(|m| m.0).collect();
    let ifaces: Vec<&str> = IFACES.iter().map(|i| i.0).collect();
    let sensors = demo_sensors();
    let chips = sensor_charts(core, &sensors);

    collectors::spawn(
        core,
        Demo {
            cpu: core.chart(ChartDef::tpl(&SYSTEM_CPU)),
            load: core.chart(ChartDef::tpl(&SYSTEM_LOAD)),
            ram: core.chart(ChartDef::tpl(&SYSTEM_RAM)),
            swap: core.chart(ChartDef::tpl(&SYSTEM_SWAP)),
            io: core.chart(ChartDef::tpl(&SYSTEM_IO)),
            net: core.chart(ChartDef::tpl(&SYSTEM_NET)),
            pressure: core.chart(ChartDef::tpl(&SYSTEM_PRESSURE)),
            processes: core.chart(ChartDef::tpl(&SYSTEM_PROCESSES)),
            forks: core.chart(ChartDef::tpl(&SYSTEM_FORKS)),
            ctxt: core.chart(ChartDef::tpl(&SYSTEM_CTXT)),
            intr: core.chart(ChartDef::tpl(&SYSTEM_INTR)),
            uptime: core.chart(ChartDef::tpl(&SYSTEM_UPTIME)),
            fds: core.chart(ChartDef::tpl(&SYSTEM_FDS)),
            entropy: core.chart(ChartDef::tpl(&SYSTEM_ENTROPY)),
            cores: core.chart(ChartDef::tpl(&CPU_CORES).dims(cores())),
            freq: core.chart(ChartDef::tpl(&CPU_FREQ).dims(cores())),
            temp: core.chart(ChartDef::tpl(&CPU_TEMP).dims(temp_dims)),
            power: core.chart(ChartDef::tpl(&CPU_POWER).dims([
                ("cores", "Kerne"),
                ("uncore", "Grafik"),
                ("dram", "RAM"),
                ("rest", "Rest (Gehäuse)"),
            ])),
            softirqs: core.chart(ChartDef::tpl(&CPU_SOFTIRQS).dims(SOFTIRQS.iter().map(|s| (*s, *s)))),
            throttle: core.chart(ChartDef::tpl(&CPU_THROTTLE)),
            available: core.chart(ChartDef::tpl(&MEM_AVAILABLE)),
            committed: core.chart(ChartDef::tpl(&MEM_COMMITTED)),
            kernel: core.chart(ChartDef::tpl(&MEM_KERNEL)),
            writeback: core.chart(ChartDef::tpl(&MEM_WRITEBACK)),
            paging: core.chart(ChartDef::tpl(&MEM_PAGING)),
            swapio: core.chart(ChartDef::tpl(&MEM_SWAPIO)),
            faults: core.chart(ChartDef::tpl(&MEM_FAULTS)),
            oom: core.chart(ChartDef::tpl(&MEM_OOM)),
            hugepages: core.chart(ChartDef::tpl(&MEM_HUGEPAGES)),
            disk_io: per(&DISK_IO, &disks),
            disk_ops: per(&DISK_OPS, &disks),
            disk_await: per(&DISK_AWAIT, &disks),
            disk_util: per(&DISK_UTIL, &disks),
            disk_queue: per(&DISK_QUEUE, &disks),
            disk_temp: per(&DISK_TEMP, &disks),
            space: per(&DISK_SPACE, &mounts),
            inodes: per(&DISK_INODES, &mounts),
            traffic: per(&NET_TRAFFIC, &ifaces),
            packets: per(&NET_PACKETS, &ifaces),
            errors: per(&NET_ERRORS, &ifaces),
            drops: per(&NET_DROPS, &ifaces),
            tcp_conn: core.chart(ChartDef::tpl(&NET_TCP_CONN)),
            tcp_opens: core.chart(ChartDef::tpl(&NET_TCP_OPENS)),
            tcp_errors: core.chart(ChartDef::tpl(&NET_TCP_ERRORS)),
            udp_packets: core.chart(ChartDef::tpl(&NET_UDP_PACKETS)),
            udp_errors: core.chart(ChartDef::tpl(&NET_UDP_ERRORS)),
            sockets: core.chart(ChartDef::tpl(&NET_SOCKETS)),
            conntrack: core.chart(ChartDef::tpl(&NET_CONNTRACK)),
            thermal: core.chart(ChartDef::tpl(&SENSORS_THERMAL).dims([
                ("acpitz", "acpitz"),
                ("x86_pkg_temp", "x86_pkg_temp"),
                ("pch_skylake", "pch_skylake"),
            ])),
            proc_count: core.chart(ChartDef::tpl(&PROC_COUNT)),
            proc_states: core.chart(ChartDef::tpl(&PROC_STATES)),
            proc_cpu: core.chart(ChartDef::tpl(&PROC_CPU).dims(gd())),
            proc_mem: core.chart(ChartDef::tpl(&PROC_MEM).dims(gd())),
            proc_read: core.chart(ChartDef::tpl(&PROC_IO_READ).dims(gd())),
            proc_write: core.chart(ChartDef::tpl(&PROC_IO_WRITE).dims(gd())),
            proc_threads: core.chart(ChartDef::tpl(&PROC_THREADS).dims(gd())),
            proc_fds: core.chart(ChartDef::tpl(&PROC_FDS).dims(gd())),
            chips,
            sensors,
            procs: demo_procs(),
            w_cores: (0..CORES).map(|_| Walk::new(18.0, 1.0, 100.0, 6.0)).collect(),
            w_ram: Walk::new(6.2 * GIB, 4.0 * GIB, 13.0 * GIB, 0.02 * GIB),
            w_disk: DISKS.iter().map(|_| (Walk::new(2.0e6, 0.0, 400.0e6, 1.5e6), Walk::new(5.0e6, 0.0, 300.0e6, 2.5e6))).collect(),
            w_net: IFACES.iter().map(|i| (Walk::new(i.2, 0.0, 900.0e6, i.2 * 0.4), Walk::new(i.2 * 0.4, 0.0, 400.0e6, i.2 * 0.2))).collect(),
            w_temp: Walk::new(46.0, 34.0, 92.0, 0.6),
            w_tcp: Walk::new(420.0, 200.0, 900.0, 12.0),
            w_ct: Walk::new(2600.0, 1200.0, 9000.0, 60.0),
            w_dirty: Walk::new(24.0e6, 0.0, 400.0e6, 6.0e6),
            loads: [0.6, 0.5, 0.4],
            used: MOUNTS.iter().map(|m| m.4).collect(),
            totals: vec![(0, 0); IFACES.len()],
            disk_rates: vec![(0.0, 0.0, 0.0); DISKS.len()],
            net_rates: vec![(0.0, 0.0); IFACES.len()],
            boot: util::now() - 23 * 86_400 - 4 * 3600,
            info_sent: false,
        },
    );
}

/// Diagramme je Chip aus den Fühlern bauen (Dimensionen in Reihenfolge der Fühler).
fn sensor_charts(core: &Core, sensors: &[SensorInfo]) -> Vec<(ChartHandle, Vec<usize>)> {
    let mut out = Vec::new();
    let mut keys: Vec<(String, SensorKind)> = Vec::new();
    for s in sensors {
        if s.kind != SensorKind::Thermal && !keys.contains(&(s.chip.clone(), s.kind)) {
            keys.push((s.chip.clone(), s.kind));
        }
    }
    for (chip, kind) in keys {
        let tpl = match kind {
            SensorKind::Fan => &SENSORS_FAN,
            SensorKind::Voltage => &SENSORS_VOLTAGE,
            SensorKind::Power => &SENSORS_POWER,
            SensorKind::Current => &SENSORS_CURRENT,
            _ => &SENSORS_TEMP,
        };
        let idx: Vec<usize> =
            sensors.iter().enumerate().filter(|(_, s)| s.chip == chip && s.kind == kind).map(|(i, _)| i).collect();
        let label = sensors[idx[0]].chip_label.clone();
        let def = ChartDef::tpl(tpl)
            .instance(&chip)
            .label("chip_label", label)
            .dims(idx.iter().map(|&i| (sensors[i].id.clone(), sensors[i].label.clone())));
        out.push((core.chart(def), idx));
    }
    out
}

fn demo_sensors() -> Vec<SensorInfo> {
    let mut v = Vec::new();
    let mut add = |chip: &str, chip_label: &str, kind: SensorKind, id: &str, label: &str, max: Option<f64>, crit: Option<f64>| {
        let tpl = match kind {
            SensorKind::Fan => &SENSORS_FAN,
            SensorKind::Voltage => &SENSORS_VOLTAGE,
            SensorKind::Thermal => &SENSORS_THERMAL,
            _ => &SENSORS_TEMP,
        };
        let chart = if kind == SensorKind::Thermal { tpl.chart_id(None) } else { tpl.chart_id(Some(chip)) };
        v.push(SensorInfo {
            chip: chip.into(),
            chip_label: chip_label.into(),
            id: id.into(),
            label: label.into(),
            kind,
            min: if kind == SensorKind::Fan { Some(600.0) } else { None },
            max,
            crit,
            chart,
            ..Default::default()
        });
    };
    let ct = "Intel Core (coretemp)";
    add("coretemp", ct, SensorKind::Temp, "temp1", "Package id 0", Some(84.0), Some(100.0));
    for i in 0..CORES {
        add("coretemp", ct, SensorKind::Temp, &format!("temp{}", i + 2), &format!("Core {i}"), Some(84.0), Some(100.0));
    }
    add("nvme", "NVMe nvme0", SensorKind::Temp, "temp1", "Composite", Some(81.8), Some(84.8));
    add("nvme", "NVMe nvme0", SensorKind::Temp, "temp2", "Sensor 1", Some(65261.8), Some(65261.8));
    add("drivetemp", "SATA sda (drivetemp)", SensorKind::Temp, "temp1", "Laufwerk", Some(70.0), Some(85.0));
    add("pch_skylake", "Chipsatz (PCH)", SensorKind::Temp, "temp1", "PCH", None, None);
    add("acpitz", "ACPI-Thermozone", SensorKind::Temp, "temp1", "temp1", None, Some(119.0));
    add("nct6793", "Nuvoton NCT6793D", SensorKind::Fan, "fan1", "CPU-Lüfter", None, None);
    add("nct6793", "Nuvoton NCT6793D", SensorKind::Fan, "fan2", "Gehäuselüfter", None, None);
    add("nct6793", "Nuvoton NCT6793D", SensorKind::Voltage, "in0", "Vcore", Some(1.49), None);
    add("nct6793", "Nuvoton NCT6793D", SensorKind::Voltage, "in1", "+12V", Some(12.6), None);
    add("nct6793", "Nuvoton NCT6793D", SensorKind::Voltage, "in2", "AVCC", Some(3.47), None);
    add("nct6793", "Nuvoton NCT6793D", SensorKind::Voltage, "in3", "+3.3V", Some(3.47), None);
    add("thermal", "Thermische Zonen", SensorKind::Thermal, "acpitz", "acpitz", None, Some(119.0));
    add("thermal", "Thermische Zonen", SensorKind::Thermal, "x86_pkg_temp", "x86_pkg_temp", None, None);
    add("thermal", "Thermische Zonen", SensorKind::Thermal, "pch_skylake", "pch_skylake", None, Some(115.0));
    // Index der Dimension im Diagramm (je Chip und Art fortlaufend).
    let mut seen: Vec<(String, SensorKind, usize)> = Vec::new();
    for s in &mut v {
        let key = if s.kind == SensorKind::Thermal { "thermal".to_string() } else { s.chip.clone() };
        match seen.iter_mut().find(|(c, k, _)| *c == key && *k == s.kind) {
            Some(e) => {
                e.2 += 1;
                s.dim = e.2;
            }
            None => {
                seen.push((key, s.kind, 0));
                s.dim = 0;
            }
        }
    }
    v
}

struct DemoProc {
    info: ProcInfo,
    walk: Walk,
    fds: u32,
}

fn demo_procs() -> Vec<DemoProc> {
    let now = util::now();
    let mut out = Vec::new();
    let mut pid = 300u32;
    for (gi, g) in GROUPS.iter().enumerate() {
        for k in 0..g.1 {
            pid += 7 + (gi as u32 * 13 + k * 31) % 90;
            let kernel = g.0 == "kernel";
            let name = match g.0 {
                "kernel" => ["kworker/0:1", "ksoftirqd/1", "rcu_sched", "kthreadd", "jbd2/nvme0n1p2", "nvme-wq"][k as usize % 6].to_string(),
                "systemd" => ["systemd", "systemd-journal", "systemd-udevd"][k as usize % 3].to_string(),
                _ => g.0.to_string(),
            };
            let cmd = match g.0 {
                "kernel" => String::new(),
                "node" => format!("node /app/server/index.js --port {}", 3000 + k),
                "postgres" => format!("postgres: immich immich 172.18.0.{} idle", 10 + k),
                "containerd-shim" => format!("/usr/bin/containerd-shim-runc-v2 -namespace moby -id {:016x}{:048x} -address /run/containerd/containerd.sock", 0xabc0_u64 + u64::from(k) * 7919, k),
                "bitcoind" => "bitcoind -datadir=/data/.bitcoin -dbcache=2048 -txindex=1".to_string(),
                "python3" => "python3 -m homeassistant --config /config".to_string(),
                "nginx" => if k == 0 { "nginx: master process nginx -g daemon off;".to_string() } else { "nginx: worker process".to_string() },
                "dockerd" => "/usr/bin/dockerd -H fd:// --containerd=/run/containerd/containerd.sock".to_string(),
                "nrzmonitor" => "/app/nrzmonitor".to_string(),
                _ => format!("/usr/sbin/{name}"),
            };
            let base = g.2 * (0.6 + (k as f64 * 0.37) % 0.8);
            out.push(DemoProc {
                info: ProcInfo {
                    pid,
                    ppid: if kernel { 2 } else { 1 },
                    name,
                    user: g.5.to_string(),
                    mem: (g.3 * (0.7 + (k as f64 * 0.21) % 0.6) * 1024.0 * 1024.0) as u64,
                    state: if kernel { "I".into() } else { "S".into() },
                    started: now - 86_400 * 23 + i64::from(pid) * 37,
                    threads: g.4,
                    cmd,
                    container: g.6.to_string(),
                    group: g.0.to_string(),
                    ..Default::default()
                },
                walk: Walk::new(base, 0.0, (base * 8.0 + 1.0).min(100.0 * g.4.min(4) as f64), (base * 0.25).max(0.01)),
                fds: if kernel { 0 } else { 8 + g.4 * 3 },
            });
        }
    }
    out
}

struct Demo {
    cpu: ChartHandle,
    load: ChartHandle,
    ram: ChartHandle,
    swap: ChartHandle,
    io: ChartHandle,
    net: ChartHandle,
    pressure: ChartHandle,
    processes: ChartHandle,
    forks: ChartHandle,
    ctxt: ChartHandle,
    intr: ChartHandle,
    uptime: ChartHandle,
    fds: ChartHandle,
    entropy: ChartHandle,
    cores: ChartHandle,
    freq: ChartHandle,
    temp: ChartHandle,
    power: ChartHandle,
    softirqs: ChartHandle,
    throttle: ChartHandle,
    available: ChartHandle,
    committed: ChartHandle,
    kernel: ChartHandle,
    writeback: ChartHandle,
    paging: ChartHandle,
    swapio: ChartHandle,
    faults: ChartHandle,
    oom: ChartHandle,
    hugepages: ChartHandle,
    disk_io: Vec<ChartHandle>,
    disk_ops: Vec<ChartHandle>,
    disk_await: Vec<ChartHandle>,
    disk_util: Vec<ChartHandle>,
    disk_queue: Vec<ChartHandle>,
    disk_temp: Vec<ChartHandle>,
    space: Vec<ChartHandle>,
    inodes: Vec<ChartHandle>,
    traffic: Vec<ChartHandle>,
    packets: Vec<ChartHandle>,
    errors: Vec<ChartHandle>,
    drops: Vec<ChartHandle>,
    tcp_conn: ChartHandle,
    tcp_opens: ChartHandle,
    tcp_errors: ChartHandle,
    udp_packets: ChartHandle,
    udp_errors: ChartHandle,
    sockets: ChartHandle,
    conntrack: ChartHandle,
    thermal: ChartHandle,
    proc_count: ChartHandle,
    proc_states: ChartHandle,
    proc_cpu: ChartHandle,
    proc_mem: ChartHandle,
    proc_read: ChartHandle,
    proc_write: ChartHandle,
    proc_threads: ChartHandle,
    proc_fds: ChartHandle,
    /// Diagramm je Chip und Art mit den Indizes der Fühler in `sensors`.
    chips: Vec<(ChartHandle, Vec<usize>)>,
    sensors: Vec<SensorInfo>,
    procs: Vec<DemoProc>,
    w_cores: Vec<Walk>,
    w_ram: Walk,
    w_disk: Vec<(Walk, Walk)>,
    w_net: Vec<(Walk, Walk)>,
    w_temp: Walk,
    w_tcp: Walk,
    w_ct: Walk,
    w_dirty: Walk,
    loads: [f64; 3],
    used: Vec<f64>,
    totals: Vec<(u64, u64)>,
    /// Zuletzt gemeldete Werte für die Momentaufnahmen.
    disk_rates: Vec<(f64, f64, f64)>,
    net_rates: Vec<(f64, f64)>,
    boot: i64,
    info_sent: bool,
}

fn r2(v: f64) -> f64 {
    util::round(v, 2)
}

fn jitter(base: f64, rel: f64) -> f64 {
    base * (1.0 + (fastrand::f64() - 0.5) * 2.0 * rel)
}

impl Demo {
    fn cpu(&mut self, t: &mut Tick<'_>) -> (Vec<f64>, f64) {
        let per_core: Vec<f64> = self.w_cores.iter_mut().map(|w| r2(w.next_spiky(0.02, 45.0))).collect();
        let total = per_core.iter().sum::<f64>() / CORES as f64;
        t.put(&self.cores, &per_core);
        t.put(
            &self.cpu,
            &[r2(total * 0.62), r2(total * 0.01), r2(total * 0.24), r2(total * 0.06), r2(total * 0.01), r2(total * 0.04), 0.0, 0.0],
        );
        let target = total / 100.0 * CORES as f64;
        for (load, k) in self.loads.iter_mut().zip([0.08, 0.02, 0.007]) {
            *load += (target - *load) * k;
        }
        t.put(&self.load, &[r2(self.loads[0]), r2(self.loads[1]), r2(self.loads[2])]);
        t.put(&self.pressure, &[r2(total * 0.04), r2(total * 0.01), 0.0, r2(total * 0.03), r2(total * 0.008)]);
        let running = (target + 1.0 + fastrand::f64() * 2.0).round();
        t.put(&self.processes, &[running, if fastrand::f64() < 0.1 { 1.0 } else { 0.0 }]);
        t.put(&self.forks, &[jitter(18.0 + total * 0.4, 0.4).round()]);
        t.put(&self.ctxt, &[jitter(9_000.0 + total * 300.0, 0.15).round()]);
        t.put(&self.intr, &[jitter(5_500.0 + total * 120.0, 0.15).round()]);
        let soft: Vec<f64> = [900.0, 40.0, 1_100.0, 700.0, 600.0, 90.0].iter().map(|b| jitter(b * (0.6 + total / 50.0), 0.2).round()).collect();
        t.put(&self.softirqs, &soft);
        if t.due(2) {
            let pkg = 3.0 + total * 0.22;
            t.put(&self.power, &[util::round(pkg * 0.62, 1), util::round(0.3 + fastrand::f64() * 0.2, 1), util::round(1.1 + total * 0.01, 1), util::round(pkg * 0.18, 1)]);
        }
        if t.due(5) {
            let freq: Vec<f64> = per_core.iter().map(|c| (800.0 + c * 22.0 + fastrand::f64() * 200.0).min(3100.0).round()).collect();
            t.put(&self.freq, &freq);
            t.put(&self.throttle, &[0.0, 0.0]);
        }
        if t.due(10) {
            t.put(&self.uptime, &[(util::now() - self.boot) as f64]);
            t.put(&self.fds, &[jitter(14_800.0, 0.02).round()]);
            t.put(&self.entropy, &[256.0]);
        }
        (per_core, total)
    }

    fn memory(&mut self, t: &mut Tick<'_>) {
        let used = self.w_ram.next().round();
        let (buffers, cached) = (0.35 * GIB, 4.1 * GIB);
        let free = (MEM_TOTAL - used - buffers - cached).max(0.0);
        t.put(&self.ram, &[used, buffers, cached, free]);
        t.put(&self.swap, &[(0.42 * GIB).round(), (3.58 * GIB).round()]);
        t.put(&self.available, &[(MEM_TOTAL - used - 0.4 * GIB).round()]);
        t.put(&self.writeback, &[self.w_dirty.next_spiky(0.02, 120.0e6).round(), if fastrand::f64() < 0.1 { 2.0e6 } else { 0.0 }]);
        let (rd, wr) = self.disk_rates.iter().fold((0.0, 0.0), |a, d| (a.0 + d.0, a.1 + d.1));
        t.put(&self.paging, &[rd.round(), wr.round()]);
        t.put(&self.swapio, &[if fastrand::f64() < 0.05 { 16_384.0 } else { 0.0 }, 0.0]);
        t.put(&self.faults, &[jitter(4_200.0, 0.4).round(), if fastrand::f64() < 0.08 { 3.0 } else { 0.0 }]);
        t.put(&self.oom, &[0.0]);
        if t.due(5) {
            t.put(&self.committed, &[(used * 1.9).round(), (MEM_TOTAL * 0.5 + 4.0 * GIB).round()]);
            t.put(&self.kernel, &[(0.62 * GIB).round(), (0.21 * GIB).round(), 28.0e6, 64.0e6, 52.0e6]);
        }
        if t.due(10) {
            t.put(&self.hugepages, &[0.0, 0.0]);
        }
    }

    fn disks(&mut self, t: &mut Tick<'_>, total_cpu: f64) {
        let (mut rs, mut ws) = (0.0, 0.0);
        for i in 0..DISKS.len() {
            let scale = if i == 0 { 1.0 } else { 0.25 };
            let read = (self.w_disk[i].0.next_spiky(0.03, 120.0e6) * scale).round();
            let write = (self.w_disk[i].1.next_spiky(0.04, 80.0e6) * scale).round();
            let (ro, wo) = ((read / 48_000.0).round(), (write / 24_000.0).round());
            let util = r2(((read + write) / 5.0e6).min(100.0));
            t.put(&self.disk_io[i], &[read, write]);
            t.put(&self.disk_ops[i], &[ro, wo]);
            let base = if i == 0 { 0.08 } else { 0.6 };
            t.put(&self.disk_await[i], &[r2(jitter(base, 0.5)), r2(jitter(base * 2.5, 0.5))]);
            t.put(&self.disk_util[i], &[util]);
            t.put(&self.disk_queue[i], &[r2(util / 60.0)]);
            if t.due(10) {
                let temp = if i == 0 { 38.0 + total_cpu * 0.1 } else { 31.0 + fastrand::f64() };
                t.put(&self.disk_temp[i], &[util::round(temp, 1)]);
            }
            self.disk_rates[i] = (read, write, util);
            rs += read;
            ws += write;
        }
        t.put(&self.io, &[rs, ws]);
        if t.due(10) {
            for (i, m) in MOUNTS.iter().enumerate() {
                if i != 1 {
                    self.used[i] = (self.used[i] + ws * 2.0e-4).min(m.3 * 0.97);
                }
                let reserved = if m.2 == "ext4" { (m.3 * 0.05).round() } else { 0.0 };
                let used = self.used[i].round();
                t.put(&self.space[i], &[used, (m.3 - used - reserved).max(0.0).round(), reserved]);
                let inodes = (m.3 / 16_384.0).round();
                let iu = (used / 180_000.0).round();
                t.put(&self.inodes[i], &[iu, inodes - iu]);
            }
        }
    }

    fn network(&mut self, t: &mut Tick<'_>) {
        let (mut rx_sum, mut tx_sum) = (0.0, 0.0);
        for (i, iface) in IFACES.iter().enumerate() {
            let rx = self.w_net[i].0.next_spiky(0.03, 150.0e6).round();
            let tx = self.w_net[i].1.next_spiky(0.03, 60.0e6).round();
            t.put(&self.traffic[i], &[rx, tx]);
            t.put(&self.packets[i], &[(rx / 8.0 / 900.0).round(), (tx / 8.0 / 700.0).round()]);
            t.put(&self.errors[i], &[0.0, 0.0]);
            t.put(&self.drops[i], &[if i == 0 && fastrand::f64() < 0.05 { 1.0 } else { 0.0 }, 0.0]);
            self.totals[i].0 += (rx / 8.0) as u64;
            self.totals[i].1 += (tx / 8.0) as u64;
            self.net_rates[i] = (rx, tx);
            if iface.1 == "physical" {
                rx_sum += rx;
                tx_sum += tx;
            }
        }
        t.put(&self.net, &[rx_sum, tx_sum]);
        let tcp = self.w_tcp.next().round();
        t.put(&self.tcp_conn, &[tcp]);
        t.put(&self.tcp_opens, &[jitter(6.0, 0.6).round(), jitter(9.0, 0.6).round()]);
        t.put(&self.tcp_errors, &[jitter(1.5, 1.0).round(), 0.0, jitter(2.0, 1.0).round(), if fastrand::f64() < 0.2 { 1.0 } else { 0.0 }, 0.0]);
        t.put(&self.udp_packets, &[jitter(140.0, 0.4).round(), jitter(150.0, 0.4).round()]);
        t.put(&self.udp_errors, &[0.0, if fastrand::f64() < 0.3 { 1.0 } else { 0.0 }, 0.0, 0.0]);
        if t.due(5) {
            t.put(&self.sockets, &[(tcp * 1.3).round(), 48.0, 0.0, jitter(60.0, 0.4).round()]);
            t.put(&self.conntrack, &[self.w_ct.next().round(), 262_144.0]);
        }
    }

    fn sensors(&mut self, t: &mut Tick<'_>, per_core: &[f64], total: f64) {
        if !t.due(2) {
            return;
        }
        let base = self.w_temp.next() + total * 0.25;
        let mut core_temps = vec![util::round(base + 2.0, 1)];
        core_temps.extend(per_core.iter().map(|c| util::round(base + c * 0.08, 1)));
        t.put(&self.temp, &core_temps);
        let disk_temp = |i: usize| self.disk_rates.get(i).map_or(0.0, |d| d.2);
        for s in &mut self.sensors {
            let v = match (s.chip.as_str(), s.kind, s.id.as_str()) {
                ("coretemp", _, id) => {
                    let n: usize = id.trim_start_matches("temp").parse().unwrap_or(1);
                    core_temps.get(n - 1).copied().unwrap_or(base)
                }
                ("nvme", _, "temp1") => 39.0 + disk_temp(0) * 0.1 + total * 0.05,
                ("nvme", _, _) => 44.0 + disk_temp(0) * 0.15,
                ("drivetemp", _, _) => 31.0 + disk_temp(1) * 0.05,
                ("pch_skylake", _, _) | (_, SensorKind::Thermal, "pch_skylake") => 49.0 + total * 0.05,
                ("acpitz", _, _) | (_, SensorKind::Thermal, "acpitz") => 27.8,
                (_, SensorKind::Thermal, _) => base + 2.0,
                (_, SensorKind::Fan, "fan1") => 900.0 + total * 18.0 + fastrand::f64() * 30.0,
                (_, SensorKind::Fan, _) => 620.0 + fastrand::f64() * 15.0,
                (_, SensorKind::Voltage, "in0") => 0.82 + total * 0.004 + fastrand::f64() * 0.01,
                (_, SensorKind::Voltage, "in1") => 12.1 + fastrand::f64() * 0.05,
                (_, SensorKind::Voltage, _) => 3.33 + fastrand::f64() * 0.02,
                _ => 0.0,
            };
            let decimals = match s.kind {
                SensorKind::Voltage => 3,
                SensorKind::Fan => 0,
                _ => 1,
            };
            s.value = Some(util::round(v, decimals));
        }
        let voltage_due = t.due(5);
        for (chart, idx) in &self.chips {
            if self.sensors[idx[0]].kind == SensorKind::Voltage && !voltage_due {
                continue;
            }
            let vals: Vec<f64> = idx.iter().map(|&i| self.sensors[i].value.unwrap_or(f64::NAN)).collect();
            t.put(chart, &vals);
        }
        let thermal: Vec<f64> =
            self.sensors.iter().filter(|s| s.kind == SensorKind::Thermal).map(|s| s.value.unwrap_or(f64::NAN)).collect();
        t.put(&self.thermal, &thermal);
        t.core.snapshot(snap::SENSORS, &self.sensors);
    }

    fn processes(&mut self, t: &mut Tick<'_>) {
        if !t.due(5) {
            return;
        }
        let mut groups: Vec<ProcGroup> =
            GROUPS.iter().take(GROUP_DIMS).map(|g| ProcGroup { name: g.0.into(), ..Default::default() }).collect();
        groups.push(ProcGroup { name: "other".into(), ..Default::default() });
        let mut threads = 0u32;
        let mut running = 0u32;
        let mut zombies = 0u32;
        for p in &mut self.procs {
            p.info.cpu = r2(p.walk.next_spiky(0.005, p.walk.value * 2.0 + 1.0));
            p.info.state = if p.info.group == "kernel" {
                "I".into()
            } else if p.info.cpu > 15.0 && fastrand::f64() < 0.5 {
                running += 1;
                "R".into()
            } else {
                "S".into()
            };
            let io = p.info.cpu * 40_000.0;
            p.info.read = (io * fastrand::f64()).round();
            p.info.write = (io * 1.5 * fastrand::f64()).round();
            threads += p.info.threads;
            let gi = groups.iter().position(|g| g.name == p.info.group).unwrap_or(GROUP_DIMS);
            let g = &mut groups[gi];
            g.count += 1;
            g.cpu += p.info.cpu;
            g.mem += p.info.mem;
            g.threads += p.info.threads;
            g.fds += p.fds;
            g.read += p.info.read;
            g.write += p.info.write;
        }
        if fastrand::f64() < 0.1 {
            zombies = 1;
        }
        let col = |f: &dyn Fn(&ProcGroup) -> f64| -> Vec<f64> { groups.iter().map(f).collect() };
        t.put(&self.proc_cpu, &col(&|g| r2(g.cpu)));
        t.put(&self.proc_mem, &col(&|g| g.mem as f64));
        t.put(&self.proc_read, &col(&|g| g.read.round()));
        t.put(&self.proc_write, &col(&|g| g.write.round()));
        t.put(&self.proc_threads, &col(&|g| f64::from(g.threads)));
        t.put(&self.proc_fds, &col(&|g| f64::from(g.fds)));
        let total = self.procs.len() as u32;
        let idle = self.procs.iter().filter(|p| p.info.state == "I").count() as u32;
        t.put(&self.proc_count, &[f64::from(total), f64::from(threads)]);
        t.put(&self.proc_states, &[f64::from(running.max(1)), f64::from(total - idle - running), f64::from(idle), 0.0, f64::from(zombies)]);
        for g in &mut groups {
            g.cpu = r2(g.cpu);
        }
        groups.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
        let list = ProcessList {
            ts: t.ts,
            total,
            threads,
            running: running.max(1),
            cores: CORES as u32,
            mem_total: MEM_TOTAL as u64,
            procs: self.procs.iter().map(|p| p.info.clone()).collect(),
            groups,
        };
        t.core.snapshot(snap::PROCESSES, &list);
    }

    fn snapshots(&mut self, t: &mut Tick<'_>) {
        if !self.info_sent || t.due(60) {
            self.info_sent = true;
            let info = HostInfo {
                hostname: t.core.hostname.clone(),
                os: "umbrelOS 1.4 (Debian GNU/Linux 12)".into(),
                kernel: "6.1.0-27-amd64".into(),
                arch: "x86_64".into(),
                cpu_model: "Intel(R) Core(TM) i5-6500T CPU @ 2.50GHz".into(),
                cpu_cores: CORES as u32,
                mem_total: MEM_TOTAL as u64,
                swap_total: (4.0 * GIB) as u64,
                boot_time: self.boot,
                virtualization: String::new(),
            };
            t.core.snapshot(snap::INFO, &info);
        }
        if !t.due(5) {
            return;
        }
        let temps: Vec<Option<f64>> = vec![
            self.sensors.iter().find(|s| s.chip == "nvme").and_then(|s| s.value),
            self.sensors.iter().find(|s| s.chip == "drivetemp").and_then(|s| s.value),
        ];
        let disks: Vec<DiskInfo> = DISKS
            .iter()
            .enumerate()
            .map(|(i, d)| DiskInfo {
                name: d.0.into(),
                model: d.2.into(),
                serial: format!("S5GXNX0T{}04{}", 7 - i, 21 + i),
                kind: d.1.into(),
                size: d.3 as u64,
                temp: temps[i],
                read: self.disk_rates[i].0,
                write: self.disk_rates[i].1,
                util: self.disk_rates[i].2,
                partitions: if i == 0 { vec!["nvme0n1p1".into(), "nvme0n1p2".into()] } else { vec!["sda1".into()] },
            })
            .collect();
        t.core.snapshot(snap::DISKS, &disks);
        let fs: Vec<FsInfo> = MOUNTS
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let reserved = if m.2 == "ext4" { m.3 * 0.05 } else { 0.0 };
                let inodes = (m.3 / 16_384.0) as u64;
                FsInfo {
                    mount: m.0.into(),
                    device: m.1.into(),
                    fstype: m.2.into(),
                    size: m.3 as u64,
                    used: self.used[i] as u64,
                    avail: (m.3 - self.used[i] - reserved).max(0.0) as u64,
                    inodes,
                    inodes_used: (self.used[i] / 180_000.0) as u64,
                    readonly: false,
                }
            })
            .collect();
        t.core.snapshot(snap::FILESYSTEMS, &fs);
        let addrs: [&[&str]; 4] = [
            &["192.168.1.42/24", "fe80::a2b3:ccff:fe12:3456/64"],
            &["100.78.107.162/16"],
            &["172.17.0.1/16"],
            &["10.21.0.1/16"],
        ];
        let ifaces: Vec<IfaceInfo> = IFACES
            .iter()
            .enumerate()
            .map(|(i, f)| IfaceInfo {
                name: f.0.into(),
                kind: f.1.into(),
                up: true,
                state: if f.1 == "vpn" { "unknown".into() } else { "up".into() },
                mac: if f.1 == "vpn" { String::new() } else { format!("a0:b3:cc:12:34:{:02x}", 0x56 + i) },
                mtu: if f.1 == "vpn" { 1280 } else { 1500 },
                speed: if f.1 == "physical" { 1000 } else { 0 },
                addrs: addrs[i].iter().map(|s| (*s).to_string()).collect(),
                rx: self.net_rates[i].0,
                tx: self.net_rates[i].1,
                rx_total: self.totals[i].0 + 812_000_000_000 / (i as u64 + 1),
                tx_total: self.totals[i].1 + 214_000_000_000 / (i as u64 + 1),
                ports: match f.1 {
                    "docker" => 3,
                    "bridge" => 74,
                    _ => 0,
                },
            })
            .collect();
        t.core.snapshot(snap::INTERFACES, &ifaces);
    }
}

impl Collector for Demo {
    fn name(&self) -> &'static str {
        "system"
    }

    fn collect(&mut self, t: &mut Tick<'_>) -> anyhow::Result<()> {
        let (per_core, total) = self.cpu(t);
        self.memory(t);
        self.disks(t, total);
        self.network(t);
        self.sensors(t, &per_core, total);
        self.processes(t);
        self.snapshots(t);
        Ok(())
    }
}
