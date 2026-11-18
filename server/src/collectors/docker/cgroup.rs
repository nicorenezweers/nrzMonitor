//! Messwerte je Container direkt aus den cgroups (v2 und v1) und `/proc/<pid>/net/dev`.
//!
//! Die Pfade werden einmal je Container gesucht und vorgebaut; die Dateien bleiben offen und werden
//! jede Sekunde mit `pread` ab Anfang neu gelesen (cgroup- und proc-Dateien erzeugen ihren Inhalt
//! dabei neu). Ein Puffer für alle – so kostet ein Container wenige Mikrosekunden.

use std::{
    fs::File,
    os::unix::fs::FileExt,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    V2,
    V1,
}

/// Rohwerte eines Takts (Zähler noch nicht in Raten umgerechnet). `None` = nicht verfügbar.
#[derive(Debug, Default, Clone, Copy)]
pub struct Raw {
    pub cpu_user_us: Option<u64>,
    pub cpu_sys_us: Option<u64>,
    pub periods: Option<u64>,
    pub throttled: Option<u64>,
    pub mem_current: Option<u64>,
    pub mem_anon: Option<u64>,
    pub mem_file: Option<u64>,
    pub mem_kernel: Option<u64>,
    pub mem_inactive_file: Option<u64>,
    pub mem_swap: Option<u64>,
    pub io_read: Option<u64>,
    pub io_write: Option<u64>,
    pub pids: Option<u64>,
    pub oom_kills: Option<u64>,
    pub net_rx: Option<u64>,
    pub net_tx: Option<u64>,
}

/// Eine offen gehaltene Datei.
#[derive(Debug)]
struct Src {
    path: PathBuf,
    file: Option<File>,
    failed: bool,
}

impl Src {
    fn new(path: PathBuf) -> Self {
        Self { path, file: None, failed: false }
    }

    /// Ganze Datei in `buf` lesen. Fehlt sie, wird es später nicht jede Sekunde neu versucht
    /// (ausser die Datei war schon einmal lesbar – dann beim nächsten Mal neu öffnen).
    fn read(&mut self, buf: &mut Vec<u8>) -> bool {
        buf.clear();
        if self.failed {
            return false;
        }
        if self.file.is_none() {
            match File::open(&self.path) {
                Ok(f) => self.file = Some(f),
                Err(_) => {
                    self.failed = true;
                    return false;
                }
            }
        }
        let Some(f) = &self.file else { return false };
        if buf.capacity() < 4096 {
            buf.reserve(4096);
        }
        let mut off = 0u64;
        loop {
            if buf.len() == buf.capacity() {
                buf.reserve(4096);
            }
            let len = buf.len();
            let cap = buf.capacity();
            buf.resize(cap, 0);
            match f.read_at(&mut buf[len..], off) {
                Ok(0) => {
                    buf.truncate(len);
                    return len > 0;
                }
                Ok(n) => {
                    buf.truncate(len + n);
                    off += n as u64;
                }
                Err(_) => {
                    buf.clear();
                    self.file = None;
                    return false;
                }
            }
        }
    }
}

/// Vorgebaute Dateien eines Containers.
#[derive(Debug)]
pub struct Files {
    pub mode: Mode,
    /// cgroup-Ordner (v2) bzw. der des Speicher-Controllers (v1) – für die Anzeige.
    pub dir: PathBuf,
    cpu_stat: Src,
    /// v1: `cpuacct.usage` (ns) und `cpuacct.stat` (Ticks).
    cpuacct_usage: Option<Src>,
    cpuacct_stat: Option<Src>,
    mem_current: Src,
    mem_stat: Src,
    mem_swap: Option<Src>,
    mem_events: Src,
    io_stat: Src,
    pids: Src,
    net: Option<Src>,
}

/// cgroup-Wurzel des Hosts und ihre Art.
pub fn detect(sys: &Path) -> Option<(Mode, PathBuf)> {
    let root = sys.join("fs/cgroup");
    if root.join("cgroup.controllers").exists() {
        return Some((Mode::V2, root));
    }
    if root.join("memory").is_dir() || root.join("cpu,cpuacct").is_dir() || root.join("cpuacct").is_dir() {
        return Some((Mode::V1, root));
    }
    None
}

/// Pfad aus `/proc/<pid>/cgroup` (für den Controller `ctrl`, v2: leer). Führende `/..`
/// (eigener cgroup-Namensraum) werden entfernt.
fn proc_cgroup(proc: &Path, pid: i32, ctrl: &str) -> Option<String> {
    if pid <= 0 {
        return None;
    }
    let text = std::fs::read_to_string(proc.join(pid.to_string()).join("cgroup")).ok()?;
    for line in text.lines() {
        let mut parts = line.splitn(3, ':');
        let (_, ctrls, path) = (parts.next()?, parts.next()?, parts.next()?);
        let hit = if ctrl.is_empty() { ctrls.is_empty() } else { ctrls.split(',').any(|c| c == ctrl) };
        if hit {
            let mut p = path;
            while let Some(r) = p.strip_prefix("/..") {
                p = r;
            }
            return Some(p.trim_start_matches('/').to_string());
        }
    }
    None
}

/// Ordner eines Containers unter `base` suchen: systemd-Treiber, cgroupfs, dann `/proc/<pid>/cgroup`.
fn find_dir(base: &Path, id: &str, proc: &Path, pid: i32, ctrl: &str) -> Option<PathBuf> {
    let candidates = [
        format!("system.slice/docker-{id}.scope"),
        format!("docker/{id}"),
        format!("docker.slice/docker-{id}.scope"),
        format!("machine.slice/docker-{id}.scope"),
    ];
    for c in &candidates {
        let p = base.join(c);
        if p.is_dir() {
            return Some(p);
        }
    }
    let rel = proc_cgroup(proc, pid, ctrl)?;
    if rel.is_empty() {
        return None;
    }
    let p = base.join(&rel);
    // Im eigenen cgroup-Namensraum ist der Pfad relativ zum eigenen Ordner (`/../buildx/<id>`):
    // dann eine Ebene tiefer suchen (`docker/buildx/<id>`, `system.slice/…`).
    let found = if p.is_dir() {
        p
    } else {
        std::fs::read_dir(base).ok()?.flatten().map(|e| e.path().join(&rel)).find(|p| p.is_dir())?
    };
    // Manche Programme (buildkitd, dind) ziehen in eine Unter-cgroup (`<id>/init`) – die Werte
    // gelten aber für den ganzen Container: zum Ordner mit der ID hochgehen.
    let mut dir = found.as_path();
    while !dir.file_name().is_some_and(|n| n.to_string_lossy().contains(id)) {
        match dir.parent() {
            Some(parent) if parent.starts_with(base) && parent != base => dir = parent,
            _ => return Some(found.clone()),
        }
    }
    Some(dir.to_path_buf())
}

impl Files {
    /// Dateien eines Containers vorbereiten. `id` = volle ID, `pid` = Hauptprozess (für Netz und Rückfall).
    pub fn new(mode: Mode, root: &Path, proc: &Path, id: &str, pid: i32, own_net: bool) -> Option<Self> {
        let net = (own_net && pid > 0).then(|| Src::new(proc.join(pid.to_string()).join("net/dev")));
        match mode {
            Mode::V2 => {
                let dir = find_dir(root, id, proc, pid, "")?;
                let f = |n: &str| Src::new(dir.join(n));
                Some(Self {
                    mode,
                    cpu_stat: f("cpu.stat"),
                    cpuacct_usage: None,
                    cpuacct_stat: None,
                    mem_current: f("memory.current"),
                    mem_stat: f("memory.stat"),
                    mem_swap: Some(f("memory.swap.current")),
                    mem_events: f("memory.events"),
                    io_stat: f("io.stat"),
                    pids: f("pids.current"),
                    net,
                    dir,
                })
            }
            Mode::V1 => {
                let ctrl = |names: &[&str]| -> Option<PathBuf> {
                    names.iter().find_map(|n| {
                        let base = root.join(n);
                        if !base.is_dir() {
                            return None;
                        }
                        let key = n.split(',').next_back().unwrap_or(n);
                        find_dir(&base, id, proc, pid, key)
                    })
                };
                let mem = ctrl(&["memory"])?;
                let cpuacct = ctrl(&["cpu,cpuacct", "cpuacct", "cpuacct,cpu"]);
                let cpu = ctrl(&["cpu,cpuacct", "cpu", "cpuacct,cpu"]).or_else(|| cpuacct.clone());
                let blkio = ctrl(&["blkio"]);
                let pids = ctrl(&["pids"]);
                let none = || PathBuf::from("/nonexistent");
                Some(Self {
                    mode,
                    cpu_stat: Src::new(cpu.map_or_else(none, |d| d.join("cpu.stat"))),
                    cpuacct_usage: cpuacct.as_ref().map(|d| Src::new(d.join("cpuacct.usage"))),
                    cpuacct_stat: cpuacct.as_ref().map(|d| Src::new(d.join("cpuacct.stat"))),
                    mem_current: Src::new(mem.join("memory.usage_in_bytes")),
                    mem_stat: Src::new(mem.join("memory.stat")),
                    mem_swap: Some(Src::new(mem.join("memory.memsw.usage_in_bytes"))),
                    mem_events: Src::new(mem.join("memory.oom_control")),
                    io_stat: Src::new(blkio.map_or_else(none, |d| {
                        let r = d.join("blkio.throttle.io_service_bytes_recursive");
                        if r.exists() { r } else { d.join("blkio.throttle.io_service_bytes") }
                    })),
                    pids: Src::new(pids.map_or_else(none, |d| d.join("pids.current"))),
                    net,
                    dir: mem,
                })
            }
        }
    }

    /// Einen Takt lesen. `slow` = auch die selteneren Werte (Prozesse, OOM-Zähler).
    pub fn read(&mut self, buf: &mut Vec<u8>, slow: bool, out: &mut Raw) {
        *out = Raw::default();
        match self.mode {
            Mode::V2 => {
                if self.cpu_stat.read(buf) {
                    kv(buf, |k, v| match k {
                        b"user_usec" => out.cpu_user_us = Some(v),
                        b"system_usec" => out.cpu_sys_us = Some(v),
                        b"nr_periods" => out.periods = Some(v),
                        b"nr_throttled" => out.throttled = Some(v),
                        _ => {}
                    });
                }
                if self.mem_stat.read(buf) {
                    let mut kernel_parts = 0u64;
                    kv(buf, |k, v| match k {
                        b"anon" => out.mem_anon = Some(v),
                        b"file" => out.mem_file = Some(v),
                        b"kernel" => out.mem_kernel = Some(v),
                        b"inactive_file" => out.mem_inactive_file = Some(v),
                        b"kernel_stack" | b"slab" | b"sock" | b"percpu" | b"pagetables" => kernel_parts += v,
                        _ => {}
                    });
                    if out.mem_kernel.is_none() {
                        out.mem_kernel = Some(kernel_parts);
                    }
                }
                if self.mem_current.read(buf) {
                    out.mem_current = first_num(buf);
                }
                if let Some(s) = &mut self.mem_swap
                    && s.read(buf) {
                        out.mem_swap = first_num(buf);
                    }
                if self.io_stat.read(buf) {
                    let (mut r, mut w) = (0u64, 0u64);
                    for line in buf.split(|b| *b == b'\n') {
                        for field in line.split(|b| *b == b' ') {
                            if let Some(v) = field.strip_prefix(b"rbytes=") {
                                r += num(v);
                            } else if let Some(v) = field.strip_prefix(b"wbytes=") {
                                w += num(v);
                            }
                        }
                    }
                    out.io_read = Some(r);
                    out.io_write = Some(w);
                } else {
                    // Ohne io-Controller (keine Zugriffe bisher) gibt es die Datei manchmal nicht.
                    out.io_read = Some(0);
                    out.io_write = Some(0);
                }
                if slow {
                    if self.pids.read(buf) {
                        out.pids = first_num(buf);
                    }
                    if self.mem_events.read(buf) {
                        kv(buf, |k, v| {
                            if k == b"oom_kill" {
                                out.oom_kills = Some(v);
                            }
                        });
                    }
                }
            }
            Mode::V1 => {
                let total_ns = self.cpuacct_usage.as_mut().and_then(|s| if s.read(buf) { first_num(buf) } else { None });
                let (mut user_t, mut sys_t) = (0u64, 0u64);
                if let Some(s) = &mut self.cpuacct_stat
                    && s.read(buf) {
                        kv(buf, |k, v| match k {
                            b"user" => user_t = v,
                            b"system" => sys_t = v,
                            _ => {}
                        });
                    }
                if let Some(ns) = total_ns {
                    // Gesamtzeit genau (ns), Aufteilung nach dem Verhältnis der groben Ticks.
                    let us = ns / 1000;
                    let ticks = user_t + sys_t;
                    let user = if ticks > 0 { (us as u128 * u128::from(user_t) / u128::from(ticks)) as u64 } else { us };
                    out.cpu_user_us = Some(user);
                    out.cpu_sys_us = Some(us - user.min(us));
                }
                if self.cpu_stat.read(buf) {
                    kv(buf, |k, v| match k {
                        b"nr_periods" => out.periods = Some(v),
                        b"nr_throttled" => out.throttled = Some(v),
                        _ => {}
                    });
                }
                if self.mem_current.read(buf) {
                    out.mem_current = first_num(buf);
                }
                if self.mem_stat.read(buf) {
                    kv(buf, |k, v| match k {
                        b"total_rss" => out.mem_anon = Some(v),
                        b"total_cache" => out.mem_file = Some(v),
                        b"total_inactive_file" => out.mem_inactive_file = Some(v),
                        _ => {}
                    });
                    if let (Some(cur), Some(a), Some(f)) = (out.mem_current, out.mem_anon, out.mem_file) {
                        out.mem_kernel = Some(cur.saturating_sub(a + f));
                    }
                }
                if let Some(s) = &mut self.mem_swap
                    && s.read(buf) {
                        out.mem_swap = first_num(buf).zip(out.mem_current).map(|(sw, cur)| sw.saturating_sub(cur));
                    }
                if self.io_stat.read(buf) {
                    let (mut r, mut w) = (0u64, 0u64);
                    for line in buf.split(|b| *b == b'\n') {
                        let mut it = line.split(|b| *b == b' ');
                        let (_, op, v) = (it.next(), it.next(), it.next());
                        match (op, v) {
                            (Some(b"Read"), Some(v)) => r += num(v),
                            (Some(b"Write"), Some(v)) => w += num(v),
                            _ => {}
                        }
                    }
                    out.io_read = Some(r);
                    out.io_write = Some(w);
                }
                if slow {
                    if self.pids.read(buf) {
                        out.pids = first_num(buf);
                    }
                    if self.mem_events.read(buf) {
                        kv(buf, |k, v| {
                            if k == b"oom_kill" {
                                out.oom_kills = Some(v);
                            }
                        });
                    }
                }
            }
        }
        if let Some(n) = &mut self.net
            && n.read(buf) {
                let (rx, tx) = net_dev(buf);
                out.net_rx = Some(rx);
                out.net_tx = Some(tx);
            }
    }
}

/// Zeilen `schlüssel wert` durchgehen.
fn kv(buf: &[u8], mut f: impl FnMut(&[u8], u64)) {
    for line in buf.split(|b| *b == b'\n') {
        let mut it = line.splitn(2, |b| *b == b' ');
        if let (Some(k), Some(v)) = (it.next(), it.next()) {
            f(k, num(v));
        }
    }
}

fn num(b: &[u8]) -> u64 {
    let mut n = 0u64;
    for &c in b {
        if c.is_ascii_digit() {
            n = n.wrapping_mul(10).wrapping_add(u64::from(c - b'0'));
        } else {
            break;
        }
    }
    n
}

fn first_num(buf: &[u8]) -> Option<u64> {
    let t = buf.split(|b| b.is_ascii_whitespace()).next()?;
    (!t.is_empty() && t[0].is_ascii_digit()).then(|| num(t))
}

/// Summe empfangen/gesendet aller Schnittstellen ausser `lo` (Bytes).
pub fn net_dev(buf: &[u8]) -> (u64, u64) {
    let (mut rx, mut tx) = (0u64, 0u64);
    for line in buf.split(|b| *b == b'\n').skip(2) {
        let Some(colon) = line.iter().position(|b| *b == b':') else { continue };
        let name = line[..colon].trim_ascii();
        if name == b"lo" {
            continue;
        }
        let mut fields = line[colon + 1..].split(|b| *b == b' ').filter(|f| !f.is_empty());
        let r = fields.next().map_or(0, num);
        let t = fields.nth(7).map_or(0, num);
        rx += r;
        tx += t;
    }
    (rx, tx)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, text: &str) {
        std::fs::create_dir_all(dir).expect("mkdir");
        std::fs::write(dir.join(name), text).expect("write");
    }

    #[test]
    fn v2_systemd_and_cgroupfs() {
        let tmp = std::env::temp_dir().join(format!("nrzmon-cg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let sys = tmp.join("sys");
        let root = sys.join("fs/cgroup");
        write(&root, "cgroup.controllers", "cpu memory io pids");
        let a = root.join("system.slice/docker-aaa.scope");
        write(&a, "cpu.stat", "usage_usec 3000\nuser_usec 2000\nsystem_usec 1000\nnr_periods 10\nnr_throttled 2\nthrottled_usec 5\n");
        write(&a, "memory.current", "1000\n");
        write(&a, "memory.stat", "anon 600\nfile 300\nkernel 100\ninactive_file 200\n");
        write(&a, "memory.swap.current", "7\n");
        write(&a, "memory.events", "low 0\nhigh 0\nmax 0\noom 1\noom_kill 1\n");
        write(&a, "io.stat", "8:0 rbytes=100 wbytes=50 rios=1 wios=1\n8:16 rbytes=10 wbytes=5 rios=1 wios=1\n");
        write(&a, "pids.current", "12\n");
        let b = root.join("docker/bbb");
        write(&b, "memory.stat", "anon 1\nfile 2\nkernel_stack 3\nslab 4\n");
        let proc = tmp.join("proc");
        write(&proc.join("42/net"), "dev", "Inter-|   Receive\n face |bytes\n    lo: 999 1 0 0 0 0 0 0 999 1 0 0 0 0 0 0\n  eth0: 1500 10 0 0 0 0 0 0 700 5 0 0 0 0 0 0\n");

        let (mode, r) = detect(&sys).expect("cgroup");
        assert_eq!(mode, Mode::V2);
        let mut f = Files::new(mode, &r, &proc, "aaa", 42, true).expect("systemd-Pfad");
        let (mut buf, mut raw) = (Vec::new(), Raw::default());
        f.read(&mut buf, true, &mut raw);
        assert_eq!(raw.cpu_user_us, Some(2000));
        assert_eq!(raw.cpu_sys_us, Some(1000));
        assert_eq!(raw.throttled, Some(2));
        assert_eq!(raw.mem_anon, Some(600));
        assert_eq!(raw.mem_kernel, Some(100));
        assert_eq!(raw.mem_swap, Some(7));
        assert_eq!(raw.io_read, Some(110));
        assert_eq!(raw.io_write, Some(55));
        assert_eq!(raw.pids, Some(12));
        assert_eq!(raw.oom_kills, Some(1));
        assert_eq!((raw.net_rx, raw.net_tx), (Some(1500), Some(700)));
        // Zweites Lesen derselben offenen Dateien.
        write(&a, "memory.current", "2000\n");
        f.read(&mut buf, false, &mut raw);
        assert_eq!(raw.mem_current, Some(2000));
        assert_eq!(raw.pids, None);

        let mut g = Files::new(mode, &r, &proc, "bbb", 0, false).expect("cgroupfs-Pfad");
        g.read(&mut buf, false, &mut raw);
        assert_eq!(raw.mem_kernel, Some(7), "Kernel aus Einzelteilen");
        assert_eq!(raw.net_rx, None);
        assert!(Files::new(mode, &r, &proc, "ccc", 0, false).is_none());
        // Eigener cgroup-Namensraum: `/proc/<pid>/cgroup` relativ zum eigenen Ordner.
        write(&root.join("docker/buildx/ddd"), "memory.current", "5\n");
        write(&root.join("docker/buildx/ddd/init"), "memory.current", "1\n");
        write(&proc.join("43"), "cgroup", "0::/../buildx/ddd/init\n");
        let mut h = Files::new(mode, &r, &proc, "ddd", 43, false).expect("relativer Pfad");
        h.read(&mut buf, false, &mut raw);
        assert_eq!(raw.mem_current, Some(5));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn v1_paths() {
        let tmp = std::env::temp_dir().join(format!("nrzmon-cg1-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let sys = tmp.join("sys");
        let root = sys.join("fs/cgroup");
        let cpu = root.join("cpu,cpuacct/docker/xyz");
        write(&cpu, "cpuacct.usage", "4000000\n");
        write(&cpu, "cpuacct.stat", "user 30\nsystem 10\n");
        write(&cpu, "cpu.stat", "nr_periods 4\nnr_throttled 1\nthrottled_time 9\n");
        let mem = root.join("memory/docker/xyz");
        write(&mem, "memory.usage_in_bytes", "1000\n");
        write(&mem, "memory.stat", "total_rss 500\ntotal_cache 300\ntotal_inactive_file 100\n");
        let blk = root.join("blkio/docker/xyz");
        write(&blk, "blkio.throttle.io_service_bytes", "8:0 Read 40\n8:0 Write 20\n8:0 Total 60\nTotal 60\n");
        let (mode, r) = detect(&sys).expect("v1");
        assert_eq!(mode, Mode::V1);
        let mut f = Files::new(mode, &r, &tmp, "xyz", 0, false).expect("v1-Pfade");
        let (mut buf, mut raw) = (Vec::new(), Raw::default());
        f.read(&mut buf, true, &mut raw);
        assert_eq!(raw.cpu_user_us, Some(3000));
        assert_eq!(raw.cpu_sys_us, Some(1000));
        assert_eq!(raw.mem_kernel, Some(200));
        assert_eq!((raw.io_read, raw.io_write), (Some(40), Some(20)));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
