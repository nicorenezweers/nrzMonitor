//! Prozesse (wie Netdata apps.plugin): alle 5 s `/proc/[pid]/{stat,io,cmdline,cgroup,fd}` lesen.
//! Anzahl, Zustände, CPU/Speicher/E/A/Threads/FDs je Programmgruppe (grösste einzeln, Rest `other`,
//! Dimensionen bleiben stabil) und die Prozessliste `system.processes`.

use std::{
    collections::HashMap,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::Instant,
};

use shared::{
    catalog::system::*,
    system::{ProcGroup, ProcInfo, ProcessList, snap},
};

use super::read;
use crate::{
    collectors::{Collector, Tick},
    core::Core,
    registry::{ChartDef, ChartHandle},
    util::round,
};

/// Wie oft (Sekunden) – passend zu `every(5)` der `proc.*`-Vorlagen.
const EVERY: u32 = 5;
/// Benannte Gruppen in den Diagrammen, dazu `other`.
const MAX_GROUPS: usize = 11;
/// Offene Dateien je Prozess nur jeden n-ten Durchgang zählen (Ordner lesen ist teuer) – verteilt über
/// die PIDs, damit jeder Durchgang etwa gleich lang dauert.
const FD_EVERY: u32 = 3;
const CMD_MAX: usize = 240;

/// Felder aus `/proc/[pid]/stat`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PidStat {
    pub comm: String,
    pub state: char,
    pub ppid: u32,
    pub utime: u64,
    pub stime: u64,
    pub threads: u32,
    pub starttime: u64,
    pub rss_pages: u64,
    /// Kernel-Thread (Elternteil kthreadd oder selbst PID 2).
    pub kernel: bool,
}

/// `/proc/[pid]/stat`: `pid (comm) S ppid …` – comm darf Leerzeichen und Klammern enthalten.
pub fn parse_pid_stat(pid: u32, text: &str) -> Option<PidStat> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    let comm = text.get(open + 1..close)?.to_string();
    let rest: Vec<&str> = text.get(close + 2..)?.split_ascii_whitespace().collect();
    // rest[0] = state (Feld 3), rest[n] = Feld n+3
    let f = |n: usize| rest.get(n - 3).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    let ppid = f(4) as u32;
    let flags = f(9);
    Some(PidStat {
        comm,
        state: rest.first()?.chars().next()?,
        ppid,
        utime: f(14),
        stime: f(15),
        threads: f(20) as u32,
        starttime: f(22),
        rss_pages: f(24),
        // PF_KTHREAD = 0x00200000
        kernel: pid == 2 || ppid == 2 || flags & 0x0020_0000 != 0,
    })
}

/// `/proc/[pid]/io` → (read_bytes, write_bytes).
pub fn parse_pid_io(text: &str) -> Option<(u64, u64)> {
    let mut r = None;
    let mut w = None;
    for (k, v) in read::kv(text) {
        match k {
            "read_bytes" => r = Some(v),
            "write_bytes" => w = Some(v),
            _ => {}
        }
    }
    Some((r?, w?))
}

/// Container-ID aus `/proc/[pid]/cgroup` (cgroup v1 und v2, Docker und containerd).
pub fn container_id(cgroup: &str) -> Option<String> {
    for line in cgroup.lines() {
        let path = line.rsplit(':').next().unwrap_or("");
        for part in path.split('/').rev() {
            let id = part
                .strip_prefix("docker-")
                .or_else(|| part.strip_prefix("cri-containerd-"))
                .or_else(|| part.strip_prefix("libpod-"))
                .map(|p| p.trim_end_matches(".scope"))
                .unwrap_or(part);
            if id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Some(id.to_string());
            }
        }
    }
    None
}

/// Programmgruppe eines Prozesses: Kernel-Threads zusammen, sonst der Kurzname ohne Nummern.
pub fn group_of(st: &PidStat) -> String {
    if st.kernel {
        return "kernel".into();
    }
    let c = st.comm.trim_matches(|ch| ch == '(' || ch == ')');
    // Interpreter- und Arbeiterprozesse vereinheitlichen.
    let c = c.split(['/', ':']).next().unwrap_or(c);
    let c = if c.starts_with("python") { "python" } else { c };
    let c = if c.starts_with("php-fpm") { "php-fpm" } else { c };
    let c = if c.starts_with("postgres") || c.starts_with("postmaster") { "postgres" } else { c };
    if c.is_empty() { "other".into() } else { c.to_string() }
}

/// Merkzettel je PID (über Durchgänge).
struct Known {
    starttime: u64,
    cpu: u64,
    io: Option<(u64, u64)>,
    cmd: String,
    container: String,
    user: String,
    group: String,
    fds: u32,
}

pub struct Procs {
    proc: PathBuf,
    root: PathBuf,
    buf: String,
    known: HashMap<u32, Known>,
    users: HashMap<u32, String>,
    containers: HashMap<String, String>,
    passes: u32,
    last: Option<Instant>,
    hz: f64,
    page: u64,
    boot: i64,
    cores: u32,
    mem_total: u64,
    dims: Vec<String>,
    c_count: ChartHandle,
    c_states: ChartHandle,
    charts: Option<[ChartHandle; 6]>,
}

impl Procs {
    pub fn new(core: &Core) -> Self {
        // SAFETY: sysconf ohne Nebenwirkungen.
        let (hz, page) = unsafe { (libc::sysconf(libc::_SC_CLK_TCK), libc::sysconf(libc::_SC_PAGESIZE)) };
        let stat = read::text(core.cfg.proc.join("stat")).unwrap_or_default();
        let boot = stat.lines().find_map(|l| l.strip_prefix("btime ")).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
        let cores = stat.lines().filter(|l| l.starts_with("cpu") && l.as_bytes().get(3).is_some_and(u8::is_ascii_digit)).count() as u32;
        let mem_total = read::text(core.cfg.proc.join("meminfo"))
            .and_then(|m| read::kv(&m).find(|(k, _)| *k == "MemTotal").map(|(_, v)| v * 1024))
            .unwrap_or(0);
        Self {
            proc: core.cfg.proc.clone(),
            root: core.cfg.root.clone(),
            buf: String::with_capacity(4096),
            known: HashMap::new(),
            users: HashMap::new(),
            containers: HashMap::new(),
            passes: 0,
            last: None,
            hz: if hz > 0 { hz as f64 } else { 100.0 },
            page: if page > 0 { page as u64 } else { 4096 },
            boot,
            cores: cores.max(1),
            mem_total,
            dims: Vec::new(),
            c_count: core.chart(ChartDef::tpl(&PROC_COUNT)),
            c_states: core.chart(ChartDef::tpl(&PROC_STATES)),
            charts: None,
        }
    }

    fn user(&mut self, uid: u32) -> String {
        if let Some(u) = self.users.get(&uid) {
            return u.clone();
        }
        let passwd = read::text(self.root.join("etc/passwd")).unwrap_or_default();
        for l in passwd.lines() {
            let f: Vec<&str> = l.split(':').collect();
            if let (Some(name), Some(id)) = (f.first(), f.get(2).and_then(|v| v.parse::<u32>().ok())) {
                self.users.entry(id).or_insert_with(|| (*name).to_string());
            }
        }
        self.users.entry(uid).or_insert_with(|| uid.to_string()).clone()
    }

    /// Container-Name zur ID: aus der Docker-Momentaufnahme (Bereich 3), sonst aus der Konfiguration
    /// unter `<root>/var/lib/docker`, sonst die kurze ID.
    fn container(&mut self, core: &Core, id: &str) -> String {
        if let Some(n) = self.containers.get(id) {
            return n.clone();
        }
        let from_snap = core.snaps.get("docker.containers").and_then(|s| {
            s.data.as_array()?.iter().find_map(|c| {
                let cid = c.get("id")?.as_str()?;
                (id.starts_with(cid) || cid.starts_with(id)).then(|| c.get("name")?.as_str().map(str::to_string))?
            })
        });
        let name = from_snap
            .or_else(|| {
                let cfg = read::text(self.root.join(format!("var/lib/docker/containers/{id}/config.v2.json")))?;
                let v: serde_json::Value = serde_json::from_str(&cfg).ok()?;
                Some(v.get("Name")?.as_str()?.trim_start_matches('/').to_string())
            })
            .unwrap_or_else(|| id[..12].to_string());
        // Kurze IDs nicht dauerhaft merken – vielleicht kennt Docker den Namen später.
        if name.len() != 12 || !id.starts_with(&name) {
            self.containers.insert(id.to_string(), name.clone());
        }
        name
    }

    fn count_fds(&self, dir: &Path) -> Option<u32> {
        let fd = dir.join("fd");
        // Ab Linux 6.2 steht die Anzahl als Grösse des Ordners da – ohne ihn zu lesen.
        let size = std::fs::metadata(&fd).ok()?.len();
        if size > 0 {
            return Some(size as u32);
        }
        Some(std::fs::read_dir(&fd).ok()?.count() as u32)
    }

    fn pass(&mut self, t: &mut Tick<'_>) {
        let now = Instant::now();
        let dt = self.last.map_or(0.0, |l| now.duration_since(l).as_secs_f64());
        self.last = Some(now);
        self.passes += 1;
        let Ok(rd) = std::fs::read_dir(&self.proc) else { return };
        let mut procs: Vec<ProcInfo> = Vec::with_capacity(self.known.len() + 16);
        let mut fds: Vec<u32> = Vec::with_capacity(self.known.len() + 16);
        let mut seen: Vec<u32> = Vec::with_capacity(self.known.len() + 16);
        let (mut threads, mut states) = (0u32, [0u32; 5]);
        for e in rd.flatten() {
            let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
            let dir = e.path();
            if !read::into(&mut self.buf, &dir.join("stat")) {
                continue;
            }
            let Some(st) = parse_pid_stat(pid, &self.buf) else { continue };
            seen.push(pid);
            threads += st.threads;
            states[match st.state {
                'R' => 0,
                'S' | 'D' | 'W' | 'P' => 1,
                'I' => 2,
                'T' | 't' => 3,
                'Z' | 'X' => 4,
                _ => 1,
            }] += 1;
            let cpu_ticks = st.utime + st.stime;
            let io = if st.kernel || !read::into(&mut self.buf, &dir.join("io")) { None } else { parse_pid_io(&self.buf) };
            // Neu oder PID wiederverwendet: einmalige Angaben lesen.
            let fresh = self.known.get(&pid).is_none_or(|k| k.starttime != st.starttime);
            if fresh {
                let cmd = if st.kernel {
                    String::new()
                } else {
                    let raw = std::fs::read(dir.join("cmdline")).unwrap_or_default();
                    let mut s: String = String::from_utf8_lossy(&raw).replace('\0', " ").trim().to_string();
                    if s.len() > CMD_MAX {
                        let mut cut = CMD_MAX;
                        while !s.is_char_boundary(cut) {
                            cut -= 1;
                        }
                        s.truncate(cut);
                        s.push('…');
                    }
                    s
                };
                let container = if st.kernel || !read::into(&mut self.buf, &dir.join("cgroup")) {
                    String::new()
                } else {
                    container_id(&self.buf).map(|id| self.container(t.core, &id)).unwrap_or_default()
                };
                let uid = std::fs::metadata(&dir).map(|m| m.uid()).unwrap_or(0);
                let user = self.user(uid);
                self.known.insert(pid, Known {
                    starttime: st.starttime,
                    cpu: cpu_ticks,
                    io,
                    cmd,
                    container,
                    user,
                    group: group_of(&st),
                    fds: 0,
                });
            }
            let fd_due = fresh || (pid + self.passes).is_multiple_of(FD_EVERY);
            let fd_count = if fd_due && !st.kernel { self.count_fds(&dir) } else { None };
            let Some(k) = self.known.get_mut(&pid) else { continue };
            let cpu = if fresh || dt <= 0.0 {
                0.0
            } else {
                round(cpu_ticks.saturating_sub(k.cpu) as f64 / self.hz / dt * 100.0, 2)
            };
            let (rd_b, wr_b) = match (io, k.io) {
                (Some(n), Some(p)) if !fresh && dt > 0.0 => {
                    ((n.0.saturating_sub(p.0) as f64 / dt).round(), (n.1.saturating_sub(p.1) as f64 / dt).round())
                }
                _ => (0.0, 0.0),
            };
            k.cpu = cpu_ticks;
            k.io = io;
            if let Some(f) = fd_count {
                k.fds = f;
            }
            fds.push(k.fds);
            procs.push(ProcInfo {
                pid,
                ppid: st.ppid,
                name: st.comm,
                user: k.user.clone(),
                cpu,
                mem: st.rss_pages * self.page,
                state: st.state.to_string(),
                started: self.boot + (st.starttime as f64 / self.hz) as i64,
                threads: st.threads,
                cmd: k.cmd.clone(),
                container: k.container.clone(),
                group: k.group.clone(),
                read: rd_b,
                write: wr_b,
            });
        }
        let read_ms = now.elapsed().as_secs_f64() * 1000.0;
        seen.sort_unstable();
        self.known.retain(|pid, _| seen.binary_search(pid).is_ok());

        let total = procs.len() as u32;
        t.put(&self.c_count, &[f64::from(total), f64::from(threads)]);
        t.put(&self.c_states, &states.map(f64::from));

        // Summen je Gruppe.
        let mut groups: Vec<ProcGroup> = Vec::new();
        let mut index: HashMap<&str, usize> = HashMap::new();
        for (p, f) in procs.iter().zip(&fds) {
            let i = *index.entry(p.group.as_str()).or_insert_with(|| {
                groups.push(ProcGroup { name: p.group.clone(), ..Default::default() });
                groups.len() - 1
            });
            let g = &mut groups[i];
            g.count += 1;
            g.cpu += p.cpu;
            g.mem += p.mem;
            g.threads += p.threads;
            g.fds += f;
            g.read += p.read;
            g.write += p.write;
        }
        for g in &mut groups {
            g.cpu = round(g.cpu, 2);
        }
        // Rang: CPU-Anteil plus Speicheranteil (beides 0–100).
        let score = |g: &ProcGroup| g.cpu / f64::from(self.cores) + g.mem as f64 / self.mem_total.max(1) as f64 * 100.0;
        groups.sort_by(|a, b| score(b).total_cmp(&score(a)));

        // Dimensionen: ab dem zweiten Durchgang (dann gibt es CPU-Raten) die grössten festlegen, später
        // nur anhängen, solange Platz ist.
        if self.passes >= 2 {
            let before = self.dims.len();
            for g in &groups {
                if self.dims.len() >= MAX_GROUPS {
                    break;
                }
                if g.name != "other" && !self.dims.contains(&g.name) && score(g) >= 0.5 {
                    self.dims.push(g.name.clone());
                }
            }
            if self.charts.is_none() || self.dims.len() != before {
                let dims: Vec<(String, String)> = self
                    .dims
                    .iter()
                    .map(|d| (d.clone(), if d == "kernel" { "Kernel".into() } else { d.clone() }))
                    .chain(std::iter::once(("other".to_string(), "Andere".to_string())))
                    .collect();
                let c = |tpl| t.core.chart(ChartDef::tpl(tpl).dims(dims.clone()));
                self.charts = Some([c(&PROC_CPU), c(&PROC_MEM), c(&PROC_IO_READ), c(&PROC_IO_WRITE), c(&PROC_THREADS), c(&PROC_FDS)]);
            }
            if let Some(charts) = &self.charts {
                let n = self.dims.len() + 1;
                let mut vals = vec![vec![0.0; n]; 6];
                for g in &groups {
                    let i = self.dims.iter().position(|d| *d == g.name).unwrap_or(n - 1);
                    for (k, v) in [g.cpu, g.mem as f64, g.read, g.write, f64::from(g.threads), f64::from(g.fds)].into_iter().enumerate() {
                        vals[k][i] += v;
                    }
                }
                for (k, chart) in charts.iter().enumerate() {
                    let v: Vec<f64> = vals[k].iter().map(|x| if k == 0 { round(*x, 2) } else { x.round() }).collect();
                    t.put(chart, &v);
                }
            }
        }

        procs.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then(b.mem.cmp(&a.mem)));
        let list = ProcessList {
            ts: t.ts,
            total,
            threads,
            running: states[0],
            cores: self.cores,
            mem_total: self.mem_total,
            procs,
            groups,
        };
        let snap_start = Instant::now();
        t.core.snapshot(snap::PROCESSES, &list);
        tracing::debug!(
            "Prozesse: lesen {read_ms:.1} ms, Momentaufnahme {:.1} ms",
            snap_start.elapsed().as_secs_f64() * 1000.0
        );
    }
}

impl Collector for Procs {
    fn name(&self) -> &'static str {
        "processes"
    }

    fn collect(&mut self, t: &mut Tick<'_>) -> anyhow::Result<()> {
        if t.due(EVERY) {
            let started = Instant::now();
            self.pass(t);
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            // Eigene Messung des Durchgangs (die Taktdauer des Sammlers schwankt zwischen 0 und diesem Wert).
            t.core.record_duration("processes.pass", started.elapsed());
            tracing::debug!("Prozessdurchlauf: {ms:.1} ms, {} Prozesse", self.known.len());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_stat() {
        let text = "1234 (my (odd) prog) S 1 1234 1234 0 -1 4194560 500 0 0 0 150 50 0 0 20 0 7 0 98765 123456789 2560 18446744073709551615 1 1 0 0 0 0 0 0 0 0 0 0 17 2 0 0 0 0 0";
        let s = parse_pid_stat(1234, text).unwrap();
        assert_eq!(s.comm, "my (odd) prog");
        assert_eq!((s.state, s.ppid, s.utime, s.stime, s.threads, s.starttime, s.rss_pages), ('S', 1, 150, 50, 7, 98765, 2560));
        assert!(!s.kernel);
        let k = parse_pid_stat(55, "55 (kworker/0:1-events) I 2 0 0 0 -1 69238880 0 0 0 0 0 3 0 0 20 0 1 0 100 0 0 18446744073709551615 0 0 0 0 0 0 0 2147483647 0 0 0 0 17 0 0 0 0 0 0").unwrap();
        assert!(k.kernel);
        assert_eq!(group_of(&k), "kernel");
        assert_eq!(group_of(&PidStat { comm: "python3.11".into(), ..Default::default() }), "python");
        assert_eq!(group_of(&PidStat { comm: "nginx".into(), ..Default::default() }), "nginx");
    }

    #[test]
    fn io_and_cgroup() {
        let io = "rchar: 100\nwchar: 200\nsyscr: 1\nsyscw: 2\nread_bytes: 4096\nwrite_bytes: 8192\ncancelled_write_bytes: 0\n";
        assert_eq!(parse_pid_io(io), Some((4096, 8192)));
        let id = "a".repeat(64);
        assert_eq!(container_id(&format!("0::/system.slice/docker-{id}.scope\n")), Some(id.clone()));
        assert_eq!(container_id(&format!("12:cpu,cpuacct:/docker/{id}\n0::/\n")), Some(id.clone()));
        assert_eq!(container_id("0::/user.slice/user-1000.slice/session-2.scope\n"), None);
    }

    #[test]
    fn pass_over_own_proc() {
        // Auf Linux-Rechnern einmal über das echte /proc laufen (auf dem Mac gibt es keins).
        if !Path::new("/proc/self/stat").exists() {
            return;
        }
        let me = std::process::id();
        let text = std::fs::read_to_string(format!("/proc/{me}/stat")).unwrap();
        assert!(parse_pid_stat(me, &text).is_some());
    }
}
