//! CPU und Kernel: `/proc/stat` (gesamt und je Kern, ctxt, intr, forks, procs), Last, SoftIRQs, Takt,
//! Drosselung, PSI, RAPL, Laufzeit, offene Dateien, Entropie und die Eckdaten des Hosts (`system.info`).

use std::path::{Path, PathBuf};

use shared::{
    catalog::system::*,
    system::{HostInfo, snap},
};

use super::read;
use crate::{
    collectors::{Collector, Tick},
    core::Core,
    registry::{ChartDef, ChartHandle},
    util::{Counter, nan, round},
};

/// Jiffies einer `cpu`-Zeile.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Jiffies {
    pub user: u64,
    pub nice: u64,
    pub system: u64,
    pub idle: u64,
    pub iowait: u64,
    pub irq: u64,
    pub softirq: u64,
    pub steal: u64,
    pub guest: u64,
    pub guest_nice: u64,
}

impl Jiffies {
    fn from(v: &[u64]) -> Self {
        let g = |i: usize| v.get(i).copied().unwrap_or(0);
        Self {
            user: g(0),
            nice: g(1),
            system: g(2),
            idle: g(3),
            iowait: g(4),
            irq: g(5),
            softirq: g(6),
            steal: g(7),
            guest: g(8),
            guest_nice: g(9),
        }
    }

    /// Gesamt (Gast steckt schon in user/nice).
    fn total(&self) -> u64 {
        self.user + self.nice + self.system + self.idle + self.iowait + self.irq + self.softirq + self.steal
    }

    /// Prozente seit `prev` in der Reihenfolge von `system.cpu`; `None` bei Rücksetzer.
    pub fn percent(&self, prev: &Self) -> Option<[f64; 8]> {
        let d = |a: u64, b: u64| a.saturating_sub(b) as f64;
        let total = d(self.total(), prev.total());
        if total <= 0.0 || self.total() < prev.total() {
            return None;
        }
        let guest = d(self.guest, prev.guest) + d(self.guest_nice, prev.guest_nice);
        let user = (d(self.user, prev.user) - d(self.guest, prev.guest)).max(0.0);
        let nice = (d(self.nice, prev.nice) - d(self.guest_nice, prev.guest_nice)).max(0.0);
        let p = |v: f64| round(v / total * 100.0, 2);
        Some([
            p(user),
            p(nice),
            p(d(self.system, prev.system)),
            p(d(self.iowait, prev.iowait)),
            p(d(self.irq, prev.irq)),
            p(d(self.softirq, prev.softirq)),
            p(d(self.steal, prev.steal)),
            p(guest),
        ])
    }

    /// Beschäftigt in Prozent (ohne Leerlauf und E/A-Warten).
    pub fn busy(&self, prev: &Self) -> Option<f64> {
        let total = self.total().checked_sub(prev.total())? as f64;
        if total <= 0.0 {
            return None;
        }
        let idle = (self.idle + self.iowait).saturating_sub(prev.idle + prev.iowait) as f64;
        Some(round(((total - idle) / total * 100.0).clamp(0.0, 100.0), 2))
    }
}

/// Inhalt von `/proc/stat`.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Stat {
    pub total: Jiffies,
    /// (Kern-Nummer, Jiffies)
    pub cores: Vec<(usize, Jiffies)>,
    pub ctxt: u64,
    pub intr: u64,
    pub forks: u64,
    pub running: u64,
    pub blocked: u64,
    pub btime: i64,
}

pub fn parse_stat(text: &str, out: &mut Stat, scratch: &mut Vec<u64>) {
    out.cores.clear();
    for line in text.lines() {
        let Some(key) = line.split_ascii_whitespace().next() else { continue };
        let first = || line.split_ascii_whitespace().nth(1).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
        match key {
            "cpu" => {
                read::fields_u64(line, scratch);
                out.total = Jiffies::from(scratch);
            }
            k if k.starts_with("cpu") => {
                if let Ok(n) = k[3..].parse::<usize>() {
                    read::fields_u64(line, scratch);
                    out.cores.push((n, Jiffies::from(scratch)));
                }
            }
            "ctxt" => out.ctxt = first(),
            "intr" => out.intr = first(),
            "processes" => out.forks = first(),
            "procs_running" => out.running = first(),
            "procs_blocked" => out.blocked = first(),
            "btime" => out.btime = first() as i64,
            _ => {}
        }
    }
}

/// `/proc/softirqs`: (Art, Summe über alle Kerne).
pub fn parse_softirqs(text: &str, out: &mut Vec<(String, u64)>) {
    out.clear();
    for line in text.lines().skip(1) {
        let mut it = line.split_ascii_whitespace();
        let Some(name) = it.next() else { continue };
        let sum = it.filter_map(|v| v.parse::<u64>().ok()).sum();
        out.push((name.trim_end_matches(':').to_string(), sum));
    }
}

/// `avg10` der Zeilen `some` und `full` einer PSI-Datei.
pub fn parse_psi(text: &str) -> (Option<f64>, Option<f64>) {
    let mut some = None;
    let mut full = None;
    for line in text.lines() {
        let val = line
            .split_ascii_whitespace()
            .find_map(|f| f.strip_prefix("avg10="))
            .and_then(|v| v.parse::<f64>().ok());
        if line.starts_with("some") {
            some = val;
        } else if line.starts_with("full") {
            full = val;
        }
    }
    (some, full)
}

/// RAPL-Zone (`/sys/class/powercap/intel-rapl:0[:n]`).
struct Rapl {
    energy: PathBuf,
    max: u64,
    prev: Option<u64>,
    /// Index der Dimension; `None` = Gehäuse (Rest wird berechnet).
    dim: Option<usize>,
    /// Liegt innerhalb des Gehäuses (Kerne, Grafik) – wird vom Rest abgezogen.
    inner: bool,
}

struct Throttle {
    path: PathBuf,
    package: bool,
}

pub struct Cpu {
    proc: PathBuf,
    sys: PathBuf,
    root: PathBuf,
    buf: String,
    scratch: Vec<u64>,
    stat: Stat,
    prev: Option<Stat>,
    softirq_names: Vec<String>,
    softirq_now: Vec<(String, u64)>,
    softirq_prev: Vec<u64>,
    ctxt: Counter,
    intr: Counter,
    forks: Counter,
    rapl: Vec<Rapl>,
    rapl_dims: usize,
    rapl_ts: i64,
    throttle: Vec<Throttle>,
    throttle_core: Counter,
    throttle_pkg: Counter,
    throttle_ts: i64,
    freq_paths: Vec<PathBuf>,
    n_cores: usize,
    host_sent: i64,
    c_cpu: ChartHandle,
    c_load: ChartHandle,
    c_pressure: ChartHandle,
    c_procs: ChartHandle,
    c_forks: ChartHandle,
    c_ctxt: ChartHandle,
    c_intr: ChartHandle,
    c_uptime: ChartHandle,
    c_fds: ChartHandle,
    c_entropy: ChartHandle,
    c_cores: Option<ChartHandle>,
    c_freq: Option<ChartHandle>,
    c_softirqs: Option<ChartHandle>,
    c_power: Option<ChartHandle>,
    c_throttle: Option<ChartHandle>,
}

impl Cpu {
    pub fn new(core: &Core) -> Self {
        let mut me = Self {
            proc: core.cfg.proc.clone(),
            sys: core.cfg.sys.clone(),
            root: core.cfg.root.clone(),
            buf: String::with_capacity(16 * 1024),
            scratch: Vec::with_capacity(12),
            stat: Stat::default(),
            prev: None,
            softirq_names: Vec::new(),
            softirq_now: Vec::new(),
            softirq_prev: Vec::new(),
            ctxt: Counter::new(),
            intr: Counter::new(),
            forks: Counter::new(),
            rapl: Vec::new(),
            rapl_dims: 0,
            rapl_ts: 0,
            throttle: Vec::new(),
            throttle_core: Counter::new(),
            throttle_pkg: Counter::new(),
            throttle_ts: 0,
            freq_paths: Vec::new(),
            n_cores: 0,
            host_sent: 0,
            c_cpu: core.chart(ChartDef::tpl(&SYSTEM_CPU)),
            c_load: core.chart(ChartDef::tpl(&SYSTEM_LOAD)),
            c_pressure: core.chart(ChartDef::tpl(&SYSTEM_PRESSURE)),
            c_procs: core.chart(ChartDef::tpl(&SYSTEM_PROCESSES)),
            c_forks: core.chart(ChartDef::tpl(&SYSTEM_FORKS)),
            c_ctxt: core.chart(ChartDef::tpl(&SYSTEM_CTXT)),
            c_intr: core.chart(ChartDef::tpl(&SYSTEM_INTR)),
            c_uptime: core.chart(ChartDef::tpl(&SYSTEM_UPTIME)),
            c_fds: core.chart(ChartDef::tpl(&SYSTEM_FDS)),
            c_entropy: core.chart(ChartDef::tpl(&SYSTEM_ENTROPY)),
            c_cores: None,
            c_freq: None,
            c_softirqs: None,
            c_power: None,
            c_throttle: None,
        };
        me.setup_rapl(core);
        me.setup_throttle(core);
        me
    }

    fn setup_rapl(&mut self, core: &Core) {
        let base = self.sys.join("class/powercap");
        // Feste Reihenfolge der Dimensionen: Kerne, Grafik, RAM, Rest.
        const DIMS: [(&str, &str); 3] = [("core", "cores"), ("uncore", "uncore"), ("dram", "dram")];
        let mut found = [false; 3];
        let mut zones = Vec::new();
        for name in read::dir(&base) {
            let Some(rest) = name.strip_prefix("intel-rapl:") else { continue };
            let parts: Vec<usize> = rest.split(':').filter_map(|p| p.parse().ok()).collect();
            let dir = base.join(&name);
            let energy = dir.join("energy_uj");
            if read::int(&energy).is_none() {
                continue; // ohne Recht (nur root) oder ohne Zähler
            }
            let zone = read::text(dir.join("name")).unwrap_or_default();
            let max = read::int(dir.join("max_energy_range_uj")).unwrap_or(0).max(0) as u64;
            let dim = if parts.len() == 1 {
                None
            } else {
                match DIMS.iter().position(|(z, _)| zone == *z) {
                    Some(i) => {
                        found[i] = true;
                        Some(i)
                    }
                    None => continue,
                }
            };
            // DRAM ist manchmal eine eigene Hauptzone (`dram` auf Servern).
            let dim = if dim.is_none() && zone == "dram" {
                found[2] = true;
                Some(2)
            } else {
                dim
            };
            let inner = parts.len() > 1 && zone != "dram";
            zones.push(Rapl { energy, max, prev: None, dim, inner });
        }
        if zones.is_empty() {
            return;
        }
        // Dimensionen nur für vorhandene Zonen; Index umrechnen.
        let mut map = [usize::MAX; 3];
        let mut dims: Vec<(String, String)> = Vec::new();
        let names = ["Kerne", "Grafik", "RAM"];
        for i in 0..3 {
            if found[i] {
                map[i] = dims.len();
                dims.push((DIMS[i].1.to_string(), names[i].to_string()));
            }
        }
        dims.push(("rest".into(), "Rest (Gehäuse)".into()));
        for z in &mut zones {
            z.dim = z.dim.map(|d| map[d]);
        }
        self.rapl_dims = dims.len();
        self.rapl = zones;
        self.c_power = Some(core.chart(ChartDef::tpl(&CPU_POWER).dims(dims)));
    }

    fn setup_throttle(&mut self, core: &Core) {
        let base = self.sys.join("devices/system/cpu");
        let mut packages = Vec::new();
        for name in read::dir(&base) {
            if !name.starts_with("cpu") || name[3..].parse::<u32>().is_err() {
                continue;
            }
            let dir = base.join(&name);
            let core_p = dir.join("thermal_throttle/core_throttle_count");
            if read::int(&core_p).is_some() {
                self.throttle.push(Throttle { path: core_p, package: false });
            }
            let pkg = read::int(dir.join("topology/physical_package_id")).unwrap_or(0);
            let pkg_p = dir.join("thermal_throttle/package_throttle_count");
            if !packages.contains(&pkg) && read::int(&pkg_p).is_some() {
                packages.push(pkg);
                self.throttle.push(Throttle { path: pkg_p, package: true });
            }
        }
        if !self.throttle.is_empty() {
            self.c_throttle = Some(core.chart(ChartDef::tpl(&CPU_THROTTLE)));
        }
    }

    /// Kerne gefunden oder Anzahl geändert: Diagramme je Kern (neu) anmelden.
    fn ensure_cores(&mut self, core: &Core) {
        let n = self.stat.cores.iter().map(|(i, _)| i + 1).max().unwrap_or(0);
        if n <= self.n_cores {
            return;
        }
        self.n_cores = n;
        let dims = || (0..n).map(|i| (format!("cpu{i}"), format!("Kern {i}")));
        self.c_cores = Some(core.chart(ChartDef::tpl(&CPU_CORES).dims(dims())));
        self.freq_paths = (0..n).map(|i| self.sys.join(format!("devices/system/cpu/cpu{i}/cpufreq/scaling_cur_freq"))).collect();
        // Takt nur, wenn es eine Quelle gibt (cpufreq oder „cpu MHz“; fehlt in vielen VMs).
        let has_freq = self.freq_paths.first().is_some_and(|p| p.exists())
            || read::text(self.proc.join("cpuinfo")).is_some_and(|c| c.contains("cpu MHz"));
        if has_freq {
            self.c_freq = Some(core.chart(ChartDef::tpl(&CPU_FREQ).dims(dims())));
        }
    }

    fn softirqs(&mut self, t: &mut Tick<'_>) {
        if !read::into(&mut self.buf, &self.proc.join("softirqs")) {
            return;
        }
        parse_softirqs(&self.buf, &mut self.softirq_now);
        let names_changed = self.softirq_names.len() != self.softirq_now.len()
            || self.softirq_names.iter().zip(&self.softirq_now).any(|(a, b)| *a != b.0);
        if names_changed {
            self.softirq_names = self.softirq_now.iter().map(|(n, _)| n.clone()).collect();
            let dims = self.softirq_names.iter().map(|n| (n.clone(), n.clone()));
            self.c_softirqs = Some(t.core.chart(ChartDef::tpl(&CPU_SOFTIRQS).dims(dims)));
            self.softirq_prev = self.softirq_now.iter().map(|(_, v)| *v).collect();
            return;
        }
        let vals: Vec<f64> = self
            .softirq_now
            .iter()
            .zip(self.softirq_prev.iter_mut())
            .map(|((_, now), prev)| {
                let v = if *now >= *prev { ((*now - *prev) as f64 / t.dt).round() } else { f64::NAN };
                *prev = *now;
                v
            })
            .collect();
        if let Some(c) = &self.c_softirqs {
            t.put(c, &vals);
        }
    }

    fn freq(&mut self, t: &mut Tick<'_>) {
        let Some(chart) = &self.c_freq else { return };
        let mut vals: Vec<f64> = self
            .freq_paths
            .iter()
            .map(|p| read::num::<f64>(&mut self.buf, p).map_or(f64::NAN, |khz| (khz / 1000.0).round()))
            .collect();
        if vals.iter().all(|v| v.is_nan()) {
            // Ohne cpufreq (VMs): „cpu MHz“ aus /proc/cpuinfo.
            if read::into(&mut self.buf, &self.proc.join("cpuinfo")) {
                let mut i = 0;
                for line in self.buf.lines() {
                    if line.starts_with("processor") {
                        i = line.split(':').nth(1).and_then(|v| v.trim().parse().ok()).unwrap_or(i);
                    } else if line.starts_with("cpu MHz")
                        && let (Some(v), Some(slot)) =
                            (line.split(':').nth(1).and_then(|v| v.trim().parse::<f64>().ok()), vals.get_mut(i))
                        {
                            *slot = v.round();
                        }
                }
            }
        }
        if vals.iter().any(|v| !v.is_nan()) {
            t.put(chart, &vals);
        }
    }

    fn power(&mut self, t: &mut Tick<'_>) {
        let Some(chart) = &self.c_power else { return };
        let dt = if self.rapl_ts == 0 { 0.0 } else { (t.ts - self.rapl_ts) as f64 };
        self.rapl_ts = t.ts;
        let mut vals = vec![0.0; self.rapl_dims];
        let (mut pkg, mut inner) = (0.0, 0.0);
        let mut ok = dt > 0.0;
        for z in &mut self.rapl {
            let Some(now) = read::num::<u64>(&mut self.buf, &z.energy) else {
                ok = false;
                continue;
            };
            let delta = match z.prev.replace(now) {
                Some(p) if now >= p => now - p,
                Some(p) if z.max > 0 => z.max - p + now, // Überlauf
                _ => {
                    ok = false;
                    continue;
                }
            };
            let watts = delta as f64 / 1e6 / dt.max(1e-9);
            match z.dim {
                Some(d) => vals[d] += watts,
                None => pkg += watts,
            }
            if z.inner {
                inner += watts;
            }
        }
        if !ok {
            return;
        }
        // Rest = Gehäuse minus Kerne und Grafik (RAM liegt ausserhalb).
        let last = self.rapl_dims - 1;
        vals[last] = (pkg - inner).max(0.0);
        for v in &mut vals {
            *v = round(*v, 1);
        }
        t.put(chart, &vals);
    }

    fn throttle(&mut self, t: &mut Tick<'_>) {
        let Some(chart) = &self.c_throttle else { return };
        let dt = if self.throttle_ts == 0 { 0.0 } else { (t.ts - self.throttle_ts) as f64 };
        self.throttle_ts = t.ts;
        let (mut core_sum, mut pkg_sum) = (0u64, 0u64);
        for th in &self.throttle {
            let v = read::num::<u64>(&mut self.buf, &th.path).unwrap_or(0);
            if th.package { pkg_sum += v } else { core_sum += v }
        }
        let c = self.throttle_core.rate(core_sum, dt).map(|v| round(v, 2));
        let p = self.throttle_pkg.rate(pkg_sum, dt).map(|v| round(v, 2));
        if c.is_some() || p.is_some() {
            t.put(chart, &[nan(c), nan(p)]);
        }
    }

    fn pressure(&mut self, t: &mut Tick<'_>) {
        let mut get = |name: &str| {
            if read::into(&mut self.buf, &self.proc.join("pressure").join(name)) { parse_psi(&self.buf) } else { (None, None) }
        };
        let (cpu, _) = get("cpu");
        let (ms, mf) = get("memory");
        let (is, iff) = get("io");
        if cpu.is_some() || ms.is_some() || is.is_some() {
            t.put(&self.c_pressure, &[nan(cpu), nan(ms), nan(mf), nan(is), nan(iff)]);
        }
    }

    fn slow(&mut self, t: &mut Tick<'_>) {
        if let Some(up) = read::text(self.proc.join("uptime"))
            && let Some(v) = up.split_ascii_whitespace().next().and_then(|v| v.parse::<f64>().ok()) {
                t.put(&self.c_uptime, &[v.round()]);
            }
        if let Some(fr) = read::text(self.proc.join("sys/fs/file-nr")) {
            let v: Vec<f64> = fr.split_ascii_whitespace().filter_map(|v| v.parse().ok()).collect();
            if v.len() >= 2 {
                t.put(&self.c_fds, &[v[0] - v[1]]);
            }
        }
        if let Some(e) = read::int(self.proc.join("sys/kernel/random/entropy_avail")) {
            t.put(&self.c_entropy, &[e as f64]);
        }
    }

    fn host_info(&mut self, core: &Core) {
        let info = host_info(&self.proc, &self.sys, &self.root, &core.hostname, self.stat.btime, self.n_cores);
        core.snapshot(snap::INFO, &info);
    }
}

/// Eckdaten des Hosts.
pub fn host_info(proc: &Path, sys: &Path, root: &Path, hostname: &str, btime: i64, cores: usize) -> HostInfo {
    let os = read::text(root.join("etc/os-release"))
        .and_then(|t| {
            t.lines()
                .find_map(|l| l.strip_prefix("PRETTY_NAME="))
                .map(|v| v.trim_matches('"').to_string())
        })
        .unwrap_or_default();
    let cpuinfo = read::text(proc.join("cpuinfo")).unwrap_or_default();
    let field = |k: &str| {
        cpuinfo.lines().find(|l| l.starts_with(k)).and_then(|l| l.split(':').nth(1)).map(|v| v.trim().to_string())
    };
    let cpu_model = field("model name")
        .or_else(|| field("Model"))
        .or_else(|| field("Hardware"))
        .or_else(|| read::text(sys.join("firmware/devicetree/base/model")).map(|m| m.trim_end_matches('\0').to_string()))
        .or_else(|| {
            // ARM ohne Namen: Hersteller aus „CPU implementer“.
            let vendor = match field("CPU implementer")?.as_str() {
                "0x41" => "ARM",
                "0x61" => "Apple",
                "0x51" => "Qualcomm",
                "0x48" => "HiSilicon",
                "0xc0" => "Ampere",
                _ => "ARM-kompatibel",
            };
            Some(format!("{vendor} ({})", std::env::consts::ARCH))
        })
        .unwrap_or_default();
    let meminfo = read::text(proc.join("meminfo")).unwrap_or_default();
    let mem = |k: &str| read::kv(&meminfo).find(|(n, _)| *n == k).map_or(0, |(_, v)| v * 1024);
    let product = read::text(sys.join("class/dmi/id/product_name")).unwrap_or_default().to_lowercase();
    let vendor = read::text(sys.join("class/dmi/id/sys_vendor")).unwrap_or_default().to_lowercase();
    let virtualization = if product.contains("kvm") || product.contains("qemu") || vendor.contains("qemu") {
        "kvm"
    } else if vendor.contains("vmware") {
        "vmware"
    } else if product.contains("virtualbox") {
        "virtualbox"
    } else if vendor.contains("microsoft") && product.contains("virtual") {
        "hyperv"
    } else if vendor.contains("apple") && product.contains("virtual") || cpuinfo.contains(" hypervisor") {
        "vm"
    } else {
        ""
    };
    HostInfo {
        hostname: hostname.to_string(),
        os,
        kernel: read::text(proc.join("sys/kernel/osrelease")).unwrap_or_default(),
        arch: std::env::consts::ARCH.to_string(),
        cpu_model,
        cpu_cores: cores as u32,
        mem_total: mem("MemTotal"),
        swap_total: mem("SwapTotal"),
        boot_time: btime,
        virtualization: virtualization.to_string(),
    }
}

impl Collector for Cpu {
    fn name(&self) -> &'static str {
        "cpu"
    }

    fn collect(&mut self, t: &mut Tick<'_>) -> anyhow::Result<()> {
        anyhow::ensure!(read::into(&mut self.buf, &self.proc.join("stat")), "/proc/stat nicht lesbar");
        parse_stat(&self.buf, &mut self.stat, &mut self.scratch);
        self.ensure_cores(t.core);
        if let Some(prev) = &self.prev {
            if let Some(p) = self.stat.total.percent(&prev.total) {
                t.put(&self.c_cpu, &p);
            }
            if let Some(c) = &self.c_cores {
                let mut vals = vec![f64::NAN; self.n_cores];
                for (i, j) in &self.stat.cores {
                    if let Some((_, pj)) = prev.cores.iter().find(|(pi, _)| pi == i) {
                        vals[*i] = nan(j.busy(pj));
                    }
                }
                t.put(c, &vals);
            }
        }
        let r = |v: Option<f64>| nan(v.map(|v| v.round()));
        t.put(&self.c_ctxt, &[r(self.ctxt.rate(self.stat.ctxt, t.dt))]);
        t.put(&self.c_intr, &[r(self.intr.rate(self.stat.intr, t.dt))]);
        t.put(&self.c_forks, &[r(self.forks.rate(self.stat.forks, t.dt))]);
        t.put(&self.c_procs, &[self.stat.running as f64, self.stat.blocked as f64]);
        self.prev = Some(self.stat.clone());

        if read::into(&mut self.buf, &self.proc.join("loadavg")) {
            let v: Vec<f64> = self.buf.split_ascii_whitespace().take(3).filter_map(|v| v.parse().ok()).collect();
            if v.len() == 3 {
                t.put(&self.c_load, &[v[0], v[1], v[2]]);
            }
        }
        self.softirqs(t);
        self.pressure(t);
        if t.due(2) {
            self.power(t);
        }
        if t.due(5) {
            self.freq(t);
            self.throttle(t);
        }
        if t.due(10) {
            self.slow(t);
        }
        if self.host_sent == 0 || t.ts - self.host_sent >= 300 {
            self.host_sent = t.ts;
            self.host_info(t.core);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAT_A: &str = "cpu  100 10 50 800 20 5 5 0 0 0
cpu0 50 5 25 400 10 3 2 0 0 0
cpu1 50 5 25 400 10 2 3 0 0 0
intr 12345 1 2 3
ctxt 1000
btime 1700000000
processes 500
procs_running 3
procs_blocked 1
softirq 10 1 2
";
    const STAT_B: &str = "cpu  160 10 70 880 30 5 5 0 20 0
cpu0 90 5 35 420 15 3 2 0 20 0
cpu1 70 5 35 460 15 2 3 0 0 0
intr 12445 1 2 3
ctxt 1600
btime 1700000000
processes 510
procs_running 2
procs_blocked 0
";

    #[test]
    fn stat_and_percent() {
        let mut a = Stat::default();
        let mut b = Stat::default();
        let mut s = Vec::new();
        parse_stat(STAT_A, &mut a, &mut s);
        parse_stat(STAT_B, &mut b, &mut s);
        assert_eq!(a.cores.len(), 2);
        assert_eq!((a.ctxt, a.intr, a.forks, a.running, a.blocked, a.btime), (1000, 12345, 500, 3, 1, 1_700_000_000));
        // Zuwachs gesamt: user 60, system 20, idle 80, iowait 10 → 170; Gast 20 steckt in user.
        let p = b.total.percent(&a.total).unwrap();
        assert_eq!(p[0], round(40.0 / 170.0 * 100.0, 2));
        assert_eq!(p[2], round(20.0 / 170.0 * 100.0, 2));
        assert_eq!(p[3], round(10.0 / 170.0 * 100.0, 2));
        assert_eq!(p[7], round(20.0 / 170.0 * 100.0, 2));
        // Kern 0: gesamt +75, Leerlauf+Warten +25 → 66.67 %.
        assert_eq!(b.cores[0].1.busy(&a.cores[0].1), Some(66.67));
        assert!(a.total.percent(&b.total).is_none());
    }

    #[test]
    fn softirqs_and_psi() {
        let mut v = Vec::new();
        parse_softirqs("                    CPU0       CPU1\n          HI:          1          2\n       TIMER:        100        200\n", &mut v);
        assert_eq!(v, [("HI".to_string(), 3), ("TIMER".to_string(), 300)]);
        let (s, f) = parse_psi("some avg10=1.50 avg60=0.20 avg300=0.00 total=123\nfull avg10=0.25 avg60=0.00 avg300=0.00 total=5\n");
        assert_eq!((s, f), (Some(1.5), Some(0.25)));
        assert_eq!(parse_psi("some avg10=0.00 avg60=0.00 avg300=0.00 total=0\n"), (Some(0.0), None));
    }

    #[test]
    fn host_info_from_files() {
        let dir = std::env::temp_dir().join(format!("nrzmon-host-{}", std::process::id()));
        let (proc, sys, root) = (dir.join("proc"), dir.join("sys"), dir.join("root"));
        std::fs::create_dir_all(proc.join("sys/kernel")).unwrap();
        std::fs::create_dir_all(sys.join("class/dmi/id")).unwrap();
        std::fs::create_dir_all(root.join("etc")).unwrap();
        std::fs::write(root.join("etc/os-release"), "NAME=x\nPRETTY_NAME=\"umbrelOS 1.4\"\n").unwrap();
        std::fs::write(proc.join("cpuinfo"), "processor\t: 0\nmodel name\t: Intel(R) Core(TM) i5-6500T\n").unwrap();
        std::fs::write(proc.join("meminfo"), "MemTotal:       16000 kB\nSwapTotal:  4 kB\n").unwrap();
        std::fs::write(proc.join("sys/kernel/osrelease"), "6.1.0\n").unwrap();
        std::fs::write(sys.join("class/dmi/id/product_name"), "Standard PC (Q35 + ICH9, 2009)\n").unwrap();
        std::fs::write(sys.join("class/dmi/id/sys_vendor"), "QEMU\n").unwrap();
        let h = host_info(&proc, &sys, &root, "umbrel", 5, 4);
        assert_eq!(h.os, "umbrelOS 1.4");
        assert_eq!(h.cpu_model, "Intel(R) Core(TM) i5-6500T");
        assert_eq!((h.mem_total, h.swap_total, h.cpu_cores), (16_000 * 1024, 4096, 4));
        assert_eq!(h.virtualization, "kvm");
        let _ = std::fs::remove_dir_all(dir);
    }
}
