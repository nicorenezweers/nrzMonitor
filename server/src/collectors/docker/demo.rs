//! Beispieldaten: ~25 Container in ~10 Apps mit glaubwürdigen Werten, Neustart-Schleife,
//! ungesundem Container und gelegentlichen Ereignissen.

use std::{collections::HashMap, time::Duration};

use shared::{
    docker::{Container, Image, Mount, NetAttach, Network, Port, Volume},
    events::{Event, Severity},
};

use super::{Entry, Shared, cgroup::Raw, inventory, stats};
use crate::{core::Core, registry::ChartDef, util, util::Walk};

/// (App, Dienst, Image, CPU %, Speicher MiB, Netz kbit/s, E/A KiB/s, Zustand, Health)
type Spec = (&'static str, &'static str, &'static str, f64, f64, f64, f64, &'static str, &'static str);

const SPECS: &[Spec] = &[
    ("nrz-git", "web", "nrz/git:1.22.3", 3.0, 210.0, 400.0, 60.0, "running", "healthy"),
    ("nrz-git", "db", "postgres:16-alpine", 1.5, 140.0, 120.0, 180.0, "running", "healthy"),
    ("nrz-git", "runner", "nrz/git-runner:0.2.11", 0.8, 90.0, 40.0, 20.0, "running", ""),
    ("nrz-dns", "server", "nrz/dns:2.4.0", 2.2, 64.0, 900.0, 15.0, "running", "healthy"),
    ("nrz-dns", "web", "nrz/dns-web:2.4.0", 0.3, 38.0, 20.0, 2.0, "running", ""),
    ("nrz-media", "server", "nrz/media:10.9.11", 18.0, 820.0, 12000.0, 2400.0, "running", "healthy"),
    ("nrz-media", "db", "postgres:15-alpine", 1.2, 180.0, 80.0, 120.0, "running", "healthy"),
    ("nrz-media", "redis", "redis:7.2-alpine", 0.6, 24.0, 60.0, 4.0, "running", ""),
    ("nrz-cloud", "app", "nrz/cloud:29.0.4", 6.0, 640.0, 2400.0, 700.0, "running", "healthy"),
    ("nrz-cloud", "db", "mariadb:11.4", 2.4, 410.0, 300.0, 520.0, "running", "healthy"),
    ("nrz-cloud", "redis", "redis:7.2-alpine", 0.7, 32.0, 120.0, 3.0, "running", ""),
    ("nrz-cloud", "cron", "nrz/cloud:29.0.4", 0.4, 120.0, 10.0, 30.0, "running", ""),
    ("nrz-photos", "server", "nrz/photos:1.115.0", 5.0, 520.0, 1600.0, 900.0, "running", "healthy"),
    ("nrz-photos", "ml", "nrz/photos-ml:1.115.0", 12.0, 1900.0, 30.0, 300.0, "running", "healthy"),
    ("nrz-photos", "db", "tensorchord/pgvecto-rs:pg14-v0.2.0", 1.8, 260.0, 200.0, 260.0, "running", "healthy"),
    ("nrz-photos", "redis", "redis:7.2-alpine", 0.5, 18.0, 70.0, 2.0, "running", ""),
    ("nrz-vault", "app", "nrz/vault:1.32.0", 0.6, 72.0, 50.0, 6.0, "running", "unhealthy"),
    ("nrz-home", "app", "nrz/home:2024.10.4", 4.0, 380.0, 0.0, 80.0, "running", "healthy"),
    ("nrz-home", "mqtt", "eclipse-mosquitto:2.0.20", 0.3, 12.0, 90.0, 1.0, "running", ""),
    ("nrz-proxy", "caddy", "caddy:2.8.4-alpine", 1.6, 48.0, 5200.0, 10.0, "running", ""),
    ("nrz-tunnel", "cloudflared", "cloudflare/cloudflared:2024.10.1", 0.9, 30.0, 800.0, 1.0, "restarting", ""),
    ("nrz-backup", "app", "nrz/backup:3.1.0", 0.0, 0.0, 0.0, 0.0, "exited", ""),
    ("nrz-monitor", "app", "nrz/monitor:1.0.0", 2.0, 96.0, 160.0, 120.0, "running", ""),
    ("nrz-books", "app", "nrz/books:0.9.2", 0.4, 110.0, 30.0, 12.0, "running", ""),
    ("", "watchtower", "containrrr/watchtower:1.7.1", 0.1, 14.0, 4.0, 0.0, "running", ""),
];

fn entries(now: i64) -> Vec<Entry> {
    let mut out = Vec::new();
    for (k, (app, svc, image, _, mem, _, _, state, health)) in SPECS.iter().enumerate() {
        let name = if app.is_empty() { (*svc).to_string() } else { format!("{app}_{svc}_1") };
        let full_id = format!("{:016x}{:048x}", 0x9e37_79b9_7f4a_7c15u64.wrapping_mul(k as u64 + 1), k);
        let host_net = matches!(*svc, "server" if *app == "nrz-dns") || *app == "nrz-home" && *svc == "app";
        let started = now - 3600 * (2 + (k as i64 * 7) % 90) - (k as i64 * 13) % 3600;
        let mut labels = std::collections::BTreeMap::new();
        if !app.is_empty() {
            labels.insert("com.docker.compose.project".to_string(), (*app).to_string());
            labels.insert("com.docker.compose.service".to_string(), (*svc).to_string());
            labels.insert("com.docker.compose.project.working_dir".to_string(), format!("/home/umbrel/umbrel/app-data/{app}"));
        }
        let ports = match *svc {
            "web" | "app" | "server" if !host_net => vec![Port { ip: "0.0.0.0".into(), private: 8080, public: 3000 + k as u16, proto: "tcp".into() }],
            "caddy" => vec![
                Port { ip: "0.0.0.0".into(), private: 80, public: 80, proto: "tcp".into() },
                Port { ip: "0.0.0.0".into(), private: 443, public: 443, proto: "tcp".into() },
            ],
            "db" => vec![Port { private: 5432, proto: "tcp".into(), ..Port::default() }],
            "redis" => vec![Port { private: 6379, proto: "tcp".into(), ..Port::default() }],
            _ => Vec::new(),
        };
        let mounts = vec![Mount {
            kind: if *svc == "db" { "volume".into() } else { "bind".into() },
            source: if *svc == "db" { format!("{app}_db-data") } else { format!("/home/umbrel/umbrel/app-data/{app}/data/{svc}") },
            destination: if *svc == "db" { "/var/lib/postgresql/data".into() } else { "/data".into() },
            rw: true,
        }];
        let networks = if host_net {
            vec![NetAttach { name: "host".into(), ..NetAttach::default() }]
        } else {
            vec![NetAttach { name: format!("{}_default", if app.is_empty() { "bridge" } else { app }), ip: format!("10.21.{}.{}", 1 + k / 8, 2 + k % 8), ..NetAttach::default() }]
        };
        let exited = *state == "exited";
        let c = Container {
            id: full_id.chars().take(12).collect(),
            name: name.clone(),
            image: (*image).to_string(),
            app: if app.is_empty() { name.clone() } else { (*app).to_string() },
            service: (*svc).to_string(),
            state: (*state).to_string(),
            health: (*health).to_string(),
            status: if exited { "Exited (0) 3 hours ago".into() } else { "Up".into() },
            created: started - 86_400 * 3,
            started,
            finished: if exited { now - 3 * 3600 } else { 0 },
            exit_code: 0,
            restarts: if *state == "restarting" { 14 } else { 0 },
            expected_exit: true,
            restart_policy: if exited { "no".into() } else { "unless-stopped".into() },
            ports,
            mounts,
            networks,
            network_mode: if host_net { "host".into() } else { "bridge".into() },
            host_net,
            cpu_limit: if *svc == "ml" { 2.0 } else { 0.0 },
            mem_limit: if *svc == "ml" { 2560 * 1024 * 1024 } else if *svc == "redis" { 256 * 1024 * 1024 } else { 0 },
            size_rw: ((*mem * 1024.0 * 37.0) as u64) % (300 * 1024 * 1024),
            ..Container::default()
        };
        out.push(Entry { c, full_id, image_id: format!("sha256:{image}"), pid: if exited { 0 } else { 1000 + k as i32 }, tty: false, labels });
    }
    out.sort_by(|a, b| a.c.name.cmp(&b.c.name));
    out
}

/// Beispielwerte als „cgroup-Zähler“ – der Takt-Sammler rechnet sie wie echte um.
struct DemoSource {
    walks: HashMap<String, [Walk; 6]>,
    counters: HashMap<String, [f64; 6]>,
    host_net: HashMap<String, bool>,
}

impl stats::Source for DemoSource {
    fn sync(&mut self, entries: &[Entry]) -> HashMap<String, String> {
        for e in entries {
            self.host_net.insert(e.full_id.clone(), e.c.host_net);
            if self.walks.contains_key(&e.full_id) {
                continue;
            }
            let Some((_, _, _, cpu, mem, net, io, _, _)) = SPECS.iter().find(|s| {
                let n = if s.0.is_empty() { s.1.to_string() } else { format!("{}_{}_1", s.0, s.1) };
                n == e.c.name
            }) else {
                continue;
            };
            let mib = 1024.0 * 1024.0;
            self.walks.insert(
                e.full_id.clone(),
                [
                    Walk::new(*cpu, 0.0, cpu * 6.0 + 5.0, cpu * 0.15 + 0.05),
                    Walk::new(mem * mib, mem * mib * 0.7, mem * mib * 1.25, mem * mib * 0.01),
                    Walk::new(net * 1000.0 / 8.0, 0.0, net * 1000.0, net * 30.0),
                    Walk::new(net * 400.0 / 8.0, 0.0, net * 600.0, net * 15.0),
                    Walk::new(io * 1024.0 * 0.6, 0.0, io * 1024.0 * 8.0, io * 60.0),
                    Walk::new(io * 1024.0 * 0.4, 0.0, io * 1024.0 * 8.0, io * 50.0),
                ],
            );
            self.counters.insert(e.full_id.clone(), [1e9, 0.0, 1e9, 1e9, 1e9, 1e9]);
        }
        entries.iter().map(|e| (e.full_id.clone(), format!("/sys/fs/cgroup/system.slice/docker-{}.scope", e.full_id))).collect()
    }

    fn read(&mut self, full_id: &str, slow: bool, dt: f64, out: &mut Raw) -> bool {
        let (Some(w), Some(c)) = (self.walks.get_mut(full_id), self.counters.get_mut(full_id)) else { return false };
        let cpu = w[0].next_spiky(0.02, w[0].value * 3.0 + 2.0);
        let mem = w[1].next();
        c[0] += cpu * 1e4 * dt;
        c[2] += w[2].next_spiky(0.03, w[2].value * 2.0) * dt;
        c[3] += w[3].next() * dt;
        c[4] += w[4].next_spiky(0.02, w[4].value * 4.0) * dt;
        c[5] += w[5].next_spiky(0.02, w[5].value * 4.0) * dt;
        let host = self.host_net.get(full_id).copied().unwrap_or(false);
        let mem = mem.round();
        *out = Raw {
            cpu_user_us: Some((c[0] * 0.8) as u64),
            cpu_sys_us: Some((c[0] * 0.2) as u64),
            periods: Some(0),
            throttled: Some(0),
            mem_current: Some((mem * 1.3) as u64),
            mem_anon: Some(mem as u64),
            mem_file: Some((mem * 0.25) as u64),
            mem_kernel: Some((mem * 0.05) as u64),
            mem_inactive_file: Some((mem * 0.3) as u64),
            mem_swap: Some(0),
            io_read: Some(c[4] as u64),
            io_write: Some(c[5] as u64),
            pids: slow.then_some(4 + (mem / 30e6) as u64),
            oom_kills: None,
            net_rx: (!host).then_some(c[2] as u64),
            net_tx: (!host).then_some(c[3] as u64),
        };
        true
    }
}

pub fn start(core: &Core, shared: Shared) {
    let now = util::now();
    {
        let mut st = shared.write();
        st.entries = entries(now);
        st.info = inventory::demo_info();
        st.info.containers = st.entries.len() as u32;
        st.info.running = st.entries.iter().filter(|e| e.c.running()).count() as u32;
        st.version = 1;
    }
    let info = shared.read().info.clone();
    core.snapshot("docker.info", &info);
    publish_lists(core, &shared);
    let source = DemoSource { walks: HashMap::new(), counters: HashMap::new(), host_net: HashMap::new() };
    crate::collectors::spawn(core, stats::Stats::new(core, shared.clone(), Box::new(source)));
    tokio::spawn(events(core.clone(), shared));
}

fn publish_lists(core: &Core, shared: &Shared) {
    let st = shared.read();
    let mut by_image: HashMap<String, u32> = HashMap::new();
    for e in &st.entries {
        *by_image.entry(e.c.image.clone()).or_default() += 1;
    }
    let mut images: Vec<Image> = by_image
        .iter()
        .enumerate()
        .map(|(k, (tag, n))| Image {
            id: format!("{:012x}", 0x5bd1_e995u64.wrapping_mul(k as u64 + 7) & 0xffff_ffff_ffff),
            tags: vec![tag.clone()],
            size: 40_000_000 + (tag.len() as u64 * 37_000_000) % 1_400_000_000,
            created: util::now() - 86_400 * (3 + k as i64 * 5),
            containers: *n,
        })
        .collect();
    images.push(Image { id: "a1b2c3d4e5f6".into(), tags: vec!["nrz/media:10.8.13".into()], size: 1_210_000_000, created: util::now() - 86_400 * 60, containers: 0 });
    images.sort_by_key(|x| std::cmp::Reverse(x.size));
    core.snapshot("docker.images", &images);
    let mut vols: Vec<Volume> = st
        .entries
        .iter()
        .filter(|e| e.c.service == "db")
        .enumerate()
        .map(|(k, e)| Volume {
            name: format!("{}_db-data", e.c.app),
            driver: "local".into(),
            mountpoint: format!("/var/lib/docker/volumes/{}_db-data/_data", e.c.app),
            size: 300_000_000 + k as i64 * 450_000_000,
            containers: 1,
            app: e.c.app.clone(),
        })
        .collect();
    vols.push(Volume { name: "3f9a0c1e7d…".into(), driver: "local".into(), mountpoint: String::new(), size: 12_000_000, containers: 0, app: String::new() });
    vols.sort_by_key(|x| std::cmp::Reverse(x.size));
    core.snapshot("docker.volumes", &vols);
    let mut nets: HashMap<String, u32> = HashMap::new();
    for e in &st.entries {
        for n in &e.c.networks {
            *nets.entry(n.name.clone()).or_default() += 1;
        }
    }
    let mut networks: Vec<Network> = nets
        .into_iter()
        .enumerate()
        .map(|(k, (name, n))| Network {
            id: format!("{:012x}", 0x2545_f491u64.wrapping_mul(k as u64 + 3) & 0xffff_ffff_ffff),
            driver: if name == "host" { "host".into() } else { "bridge".into() },
            scope: "local".into(),
            subnet: if name == "host" { String::new() } else { format!("10.21.{}.0/24", k + 1) },
            gateway: if name == "host" { String::new() } else { format!("10.21.{}.1", k + 1) },
            containers: n,
            internal: false,
            name,
        })
        .collect();
    networks.sort_by(|a, b| b.containers.cmp(&a.containers).then_with(|| a.name.cmp(&b.name)));
    core.snapshot("docker.networks", &networks);
}

/// Ereignisse und Zustandswechsel der Beispieldaten.
async fn events(core: Core, shared: Shared) {
    let images = core.chart(ChartDef::tpl(&shared::catalog::docker::DOCKER_IMAGES));
    let ev = |name: &str, app: &str, kind: &str, sev: Severity, title: String, msg: &str| {
        Event::new("docker", kind, sev, name, title).message(msg).label("container", name).label("app", app)
    };
    core.event(ev("nrz-books_app_1", "nrz-books", "container.start", Severity::Info, "nrz-books_app_1 gestartet".into(), ""));
    core.event(ev("nrz-vault_app_1", "nrz-vault", "container.unhealthy", Severity::Warn, "nrz-vault_app_1 ist ungesund".into(), "Healthcheck schlägt fehl."));
    let mut step = 0u64;
    loop {
        if step.is_multiple_of(300) {
            let mut f = crate::frame::Frame::new(util::now());
            f.put(&images, &[8.4e9, 6.2e8, 3.1e9, 1.2e9]);
            core.submit(f);
        }
        // Neustart-Schleife: cloudflared stirbt etwa jede Minute und startet neu.
        if step % 60 == 20 || step % 60 == 25 {
            let starting = step % 60 == 25;
            let mut st = shared.write();
            if let Some(e) = st.entries.iter_mut().find(|e| e.c.service == "cloudflared") {
                if starting {
                    e.c.state = "running".into();
                    e.c.started = util::now();
                    e.pid += 100;
                } else {
                    e.c.state = "restarting".into();
                    e.c.restarts += 1;
                    e.c.exit_code = 1;
                    e.c.expected_exit = false;
                    e.c.finished = util::now();
                }
                let name = e.c.name.clone();
                st.version += 1;
                drop(st);
                if starting {
                    core.event(ev(&name, "nrz-tunnel", "container.start", Severity::Info, format!("{name} gestartet"), ""));
                } else {
                    core.event(ev(&name, "nrz-tunnel", "container.die", Severity::Warn, format!("{name} unerwartet beendet"), "Code 1"));
                }
            }
        }
        if step == 150 {
            let mut st = shared.write();
            if let Some(e) = st.entries.iter_mut().find(|e| e.c.service == "ml") {
                e.c.oom_kills += 1;
                st.version += 1;
            }
            drop(st);
            core.event(ev("nrz-photos_ml_1", "nrz-photos", "container.oom", Severity::Crit, "nrz-photos_ml_1: Speicher erschöpft (OOM)".into(), "Der Kernel hat einen Prozess beendet."));
        }
        step += 1;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Beispiel-Logs für die API.
pub fn logs(name: &str, tail: usize, since_ns: i64) -> Vec<shared::docker::LogLine> {
    let now = util::now_ms() * 1_000_000;
    let msgs = [
        (false, "GET /api/health 200 1.2ms"),
        (false, "GET /api/v1/items?page=2 200 18.4ms"),
        (false, "POST /api/v1/login 200 92.0ms"),
        (true, "WARN slow query took 1240ms: SELECT * FROM items WHERE owner = $1"),
        (false, "cache hit ratio 0.94"),
        (false, "GET /assets/app.js 304 0.4ms"),
        (true, "ERROR connection reset by peer (upstream 10.21.1.3:5432)"),
        (false, "worker 3 finished job thumbnails in 2.1s"),
    ];
    let seed = name.len() as i64;
    // Eine Zeile alle 3 Sekunden, rückwärts von jetzt.
    let mut out: Vec<shared::docker::LogLine> = (0..tail.max(1) as i64)
        .map(|k| {
            let ts = (now / 3_000_000_000 - k) * 3_000_000_000;
            let (err, text) = msgs[((ts / 3_000_000_000 + seed).rem_euclid(msgs.len() as i64)) as usize];
            shared::docker::LogLine { ts, err, text: format!("{name}: {text}") }
        })
        .filter(|l| l.ts > since_ns)
        .collect();
    out.reverse();
    out
}
