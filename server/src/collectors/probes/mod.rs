//! Erreichbarkeit: Ping, TCP und HTTP(S) zu Zielen aus den Einstellungen (`probes`).
//!
//! Standardziele: Gateway (aus `<proc>/net/route`), 1.1.1.1 und 9.9.9.9. Ping/TCP alle 10 s,
//! HTTP alle 30 s. Nach drei Fehlschlägen in Folge gilt ein Ziel als ausgefallen (Ereignis), die
//! erste geglückte Prüfung danach meldet die Rückkehr.

mod demo;
pub mod http;
pub mod icmp;

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::{Duration, Instant},
};

use shared::{
    catalog::probes::*,
    dns::TestResult,
    events::{Event, Severity},
    probes::{ProbeKind, ProbeSettings, ProbeState, ProbeTarget, ProbesStatus},
};
use tokio::task::JoinHandle;

use crate::{
    core::Core,
    frame::Frame,
    registry::{ChartDef, ChartHandle},
    util,
};

/// Fehlschläge in Folge, bis ein Ziel als ausgefallen gilt.
const DOWN_AFTER: u32 = 3;
const TIMEOUT: Duration = Duration::from_secs(5);

/// Gateway aus `<proc>/net/route` (Standardroute, Spalte „Gateway“ in Hex, little-endian).
pub fn gateway(proc: &std::path::Path) -> Option<Ipv4Addr> {
    let text = std::fs::read_to_string(proc.join("net/route")).ok()?;
    parse_route(&text)
}

fn parse_route(text: &str) -> Option<Ipv4Addr> {
    text.lines().skip(1).find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 4 || f[1] != "00000000" {
            return None;
        }
        let flags = u32::from_str_radix(f[3], 16).ok()?;
        if flags & 0x2 == 0 {
            return None;
        }
        let gw = u32::from_str_radix(f[2], 16).ok()?;
        (gw != 0).then(|| Ipv4Addr::from(gw.to_le_bytes()))
    })
}

/// Alle Ziele: Standardziele (falls an) und eigene. Eigene mit gleicher ID gehen vor.
pub fn targets(core: &Core, s: &ProbeSettings) -> Vec<(ProbeTarget, bool)> {
    let mut out: Vec<(ProbeTarget, bool)> = Vec::new();
    if s.defaults {
        if let Some(gw) = gateway(&core.cfg.proc) {
            out.push((ProbeTarget { id: "gateway".into(), name: "Gateway".into(), kind: ProbeKind::Ping, host: gw.to_string(), port: 0, enabled: true }, true));
        }
        for (id, name, host) in [("cloudflare-dns", "Cloudflare DNS", "1.1.1.1"), ("quad9", "Quad9", "9.9.9.9")] {
            out.push((ProbeTarget { id: id.into(), name: name.into(), kind: ProbeKind::Ping, host: host.into(), port: 0, enabled: true }, true));
        }
    }
    for t in &s.targets {
        if !t.enabled || t.id.is_empty() {
            continue;
        }
        out.retain(|(o, _)| o.id != t.id);
        out.push((t.clone(), false));
    }
    out
}

/// Anzeige des Ziels (`1.1.1.1`, `host:443`, `https://…`).
pub fn describe(t: &ProbeTarget) -> String {
    match t.kind {
        ProbeKind::Tcp => format!("{}:{}", t.host, t.port),
        _ => t.host.clone(),
    }
}

/// ID aus einem Namen: klein, nur `a-z 0-9 -`.
pub fn make_id(name: &str) -> String {
    let mut id: String = name
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'ä' => 'a',
            'ö' => 'o',
            'ü' => 'u',
            c if c.is_ascii_alphanumeric() => c,
            _ => '-',
        })
        .collect();
    while id.contains("--") {
        id = id.replace("--", "-");
    }
    let id = id.trim_matches('-').to_string();
    if id.is_empty() { format!("ziel-{}", fastrand::u16(..)) } else { id }
}

pub fn start(core: &Core) {
    let core = core.clone();
    tokio::spawn(async move {
        let mut rx = core.settings.subscribe();
        let mut running: Option<(ProbeSettings, JoinHandle<()>)> = None;
        loop {
            let s = core.settings.get::<ProbeSettings>("probes");
            if running.as_ref().map(|(o, _)| o) != Some(&s) {
                if let Some((_, h)) = running.take() {
                    h.abort();
                }
                let c = core.clone();
                let demo = core.cfg.demo && s.targets.is_empty();
                let list = targets(&core, &s);
                let h = if demo { tokio::spawn(demo::run(c)) } else { tokio::spawn(async move { Runner::new(c, list).run().await }) };
                running = Some((s, h));
            }
            if rx.changed().await.is_err() {
                return;
            }
        }
    });
}

/// Ergebnis einer Prüfung.
#[derive(Debug, Clone, Default)]
pub struct Outcome {
    pub ok: bool,
    pub latency_ms: Option<f64>,
    pub loss: f64,
    pub status: Option<u16>,
    pub http: Option<http::HttpResult>,
    pub error: Option<String>,
}

async fn resolve(host: &str, port: u16) -> Result<SocketAddr, String> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, port));
    }
    let mut addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port)).await.map_err(|e| format!("DNS: {e}"))?.collect();
    addrs.sort_by_key(|a| !a.is_ipv4());
    addrs.into_iter().next().ok_or_else(|| "DNS: keine Adresse".to_string())
}

async fn tcp(host: &str, port: u16, refused_ok: bool) -> Outcome {
    let addr = match resolve(host, port).await {
        Ok(a) => a,
        Err(e) => return Outcome { loss: 100.0, error: Some(e), ..Outcome::default() },
    };
    let t = Instant::now();
    match tokio::time::timeout(TIMEOUT, tokio::net::TcpStream::connect(addr)).await {
        Ok(Ok(_)) => Outcome { ok: true, latency_ms: Some(util::round(t.elapsed().as_secs_f64() * 1000.0, 2)), ..Outcome::default() },
        // Abgewiesen heisst: der Rechner antwortet (für den Ersatz-Ping genügt das).
        Ok(Err(e)) if refused_ok && e.kind() == std::io::ErrorKind::ConnectionRefused => {
            Outcome { ok: true, latency_ms: Some(util::round(t.elapsed().as_secs_f64() * 1000.0, 2)), ..Outcome::default() }
        }
        Ok(Err(e)) => Outcome { loss: 100.0, error: Some(format!("Verbinden: {e}")), ..Outcome::default() },
        Err(_) => Outcome { loss: 100.0, error: Some("Zeitüberschreitung".into()), ..Outcome::default() },
    }
}

async fn ping(host: &str, icmp_ok: bool) -> Outcome {
    if !icmp_ok {
        return tcp(host, 443, true).await;
    }
    let ip = match resolve(host, 0).await {
        Ok(SocketAddr::V4(a)) => *a.ip(),
        Ok(SocketAddr::V6(_)) => return tcp(host, 443, true).await,
        Err(e) => return Outcome { loss: 100.0, error: Some(e), ..Outcome::default() },
    };
    match tokio::task::spawn_blocking(move || icmp::ping(ip, 3, Duration::from_secs(2))).await {
        Ok(Ok(r)) => {
            let loss = util::round(f64::from(r.sent - r.received) * 100.0 / f64::from(r.sent.max(1)), 2);
            Outcome {
                ok: r.received > 0,
                latency_ms: r.avg_ms.map(|v| util::round(v, 2)),
                loss,
                error: (r.received == 0).then(|| "Keine Antwort".to_string()),
                ..Outcome::default()
            }
        }
        Ok(Err(e)) => Outcome { loss: 100.0, error: Some(format!("ICMP: {e}")), ..Outcome::default() },
        Err(e) => Outcome { loss: 100.0, error: Some(e.to_string()), ..Outcome::default() },
    }
}

/// Ein Ziel einmal prüfen.
pub async fn probe(t: &ProbeTarget, icmp_ok: bool) -> Outcome {
    match t.kind {
        ProbeKind::Ping => ping(&t.host, icmp_ok).await,
        ProbeKind::Tcp => tcp(&t.host, t.port, false).await,
        ProbeKind::Http => match http::check(&t.host, Duration::from_secs(10)).await {
            Ok(r) => {
                let ok = r.status < 400;
                Outcome {
                    ok,
                    latency_ms: Some(util::round(r.total_ms, 2)),
                    loss: if ok { 0.0 } else { 100.0 },
                    status: Some(r.status),
                    error: (!ok).then(|| format!("HTTP {}", r.status)),
                    http: Some(r),
                }
            }
            Err(e) => Outcome { loss: 100.0, error: Some(e), ..Outcome::default() },
        },
    }
}

pub struct Charts {
    latency: ChartHandle,
    loss: ChartHandle,
    up: ChartHandle,
    http: Option<ChartHandle>,
}

impl Charts {
    pub fn new(core: &Core, t: &ProbeTarget) -> Self {
        let every = if t.kind == ProbeKind::Http { 30 } else { 10 };
        let c = |tpl: &shared::catalog::Tpl| {
            core.chart(ChartDef::tpl(tpl).instance(&t.id).family(&t.name).label("kind", t.kind.label()).label("host", describe(t)).every(every))
        };
        Self {
            latency: c(&PROBE_LATENCY),
            loss: c(&PROBE_LOSS),
            up: c(&PROBE_UP),
            http: (t.kind == ProbeKind::Http).then(|| c(&PROBE_HTTP)),
        }
    }

    pub fn put(&self, f: &mut Frame, o: &Outcome) {
        f.put(&self.latency, &[util::nan(o.latency_ms)]);
        f.put(&self.loss, &[o.loss]);
        f.put(&self.up, &[if o.ok { 1.0 } else { 0.0 }]);
        if let (Some(h), Some(r)) = (&self.http, &o.http) {
            let r2 = |v: f64| util::round(v, 2);
            f.put(h, &[r2(r.dns_ms), r2(r.connect_ms), r2(r.tls_ms), r2(r.wait_ms), r2(r.transfer_ms)]);
        }
    }
}

struct Target {
    def: ProbeTarget,
    builtin: bool,
    charts: Charts,
    state: ProbeState,
    fails: u32,
    checks: u64,
    oks: u64,
}

pub struct Runner {
    core: Core,
    targets: Vec<Target>,
    icmp_error: Option<String>,
}

impl Runner {
    pub fn new(core: Core, list: Vec<(ProbeTarget, bool)>) -> Self {
        let targets = list
            .into_iter()
            .map(|(def, builtin)| Target {
                charts: Charts::new(&core, &def),
                state: ProbeState {
                    id: def.id.clone(),
                    name: def.name.clone(),
                    kind: def.kind,
                    target: describe(&def),
                    builtin,
                    pending: true,
                    ..ProbeState::default()
                },
                def,
                builtin,
                fails: 0,
                checks: 0,
                oks: 0,
            })
            .collect();
        Self { core, targets, icmp_error: icmp::available().err() }
    }

    pub async fn run(mut self) {
        let mut tick: u64 = 0;
        loop {
            self.round(util::now(), tick).await;
            tick += 1;
            let now = util::now_ms();
            let next = (now.div_euclid(10_000) + 1) * 10_000;
            tokio::time::sleep(Duration::from_millis((next - now) as u64)).await;
        }
    }

    pub async fn round(&mut self, now: i64, tick: u64) {
        let icmp_ok = self.icmp_error.is_none();
        let due: Vec<usize> = (0..self.targets.len()).filter(|i| self.targets[*i].def.kind != ProbeKind::Http || tick.is_multiple_of(3)).collect();
        let futures = due.iter().map(|i| {
            let t = self.targets[*i].def.clone();
            async move { probe(&t, icmp_ok).await }
        });
        let results = futures_util::future::join_all(futures).await;
        let ts = now.div_euclid(10) * 10;
        let mut f = Frame::new(ts);
        for (i, o) in due.into_iter().zip(results) {
            let core = self.core.clone();
            let t = &mut self.targets[i];
            t.charts.put(&mut f, &o);
            t.checks += 1;
            let st = &mut t.state;
            st.last_check = now;
            st.latency_ms = o.latency_ms;
            st.loss = o.loss;
            st.status = o.status;
            st.error = o.error.clone();
            if o.ok {
                t.oks += 1;
                let was_down = !st.pending && !st.up;
                if st.pending || was_down {
                    st.since = now;
                }
                if was_down {
                    core.event(
                        Event::new("probe", "probe.up", Severity::Ok, &t.def.name, format!("{} wieder erreichbar", t.def.name))
                            .message(format!("{} · {}", t.def.kind.label(), describe(&t.def)))
                            .label("target", t.def.id.clone()),
                    );
                }
                st.up = true;
                st.pending = false;
                t.fails = 0;
            } else {
                t.fails += 1;
                if t.fails >= DOWN_AFTER && (st.up || st.pending) {
                    if !st.pending {
                        core.event(
                            Event::new("probe", "probe.down", Severity::Crit, &t.def.name, format!("{} nicht erreichbar", t.def.name))
                                .message(format!("{} · {}: {}", t.def.kind.label(), describe(&t.def), o.error.clone().unwrap_or_default()))
                                .label("target", t.def.id.clone()),
                        );
                    }
                    st.up = false;
                    st.pending = false;
                    st.since = now;
                }
            }
            st.uptime_pct = util::round(t.oks as f64 * 100.0 / t.checks.max(1) as f64, 2);
        }
        self.core.submit(f);
        let status = ProbesStatus {
            demo: false,
            targets: self.targets.iter().map(|t| ProbeState { builtin: t.builtin, ..t.state.clone() }).collect(),
            icmp_error: self.icmp_error.clone(),
        };
        self.core.snapshot("probes.status", &status);
    }
}

/// „Ziel testen“: eine einzelne Prüfung.
pub async fn test(t: &ProbeTarget) -> TestResult {
    let o = probe(t, icmp::available().is_ok()).await;
    let mut details = Vec::new();
    if let Some(r) = &o.http {
        details.push(format!(
            "HTTP {} · DNS {:.0} ms, Verbinden {:.0} ms, TLS {:.0} ms, Warten {:.0} ms, Übertragen {:.0} ms",
            r.status, r.dns_ms, r.connect_ms, r.tls_ms, r.wait_ms, r.transfer_ms
        ));
    }
    if o.ok {
        TestResult { ok: true, message: format!("Erreichbar ({:.1} ms)", o.latency_ms.unwrap_or(0.0)), details }
    } else {
        TestResult { ok: false, message: o.error.unwrap_or_else(|| "Nicht erreichbar".into()), details }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_gateway() {
        let text = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\n\
                    eth0\t0001A8C0\t00000000\t0001\t0\t0\t0\t00FFFFFF\n\
                    eth0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\n";
        assert_eq!(parse_route(text), Some(Ipv4Addr::new(192, 168, 1, 1)));
        assert_eq!(parse_route("Iface\n"), None);
    }

    #[test]
    fn ids() {
        assert_eq!(make_id("Mein NAS (Büro)"), "mein-nas-buro");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tcp_down_and_up_events() {
        let dir = std::env::temp_dir().join(format!("nrzmon-probe-{}-{}", std::process::id(), fastrand::u32(..)));
        let cfg = crate::config::Config {
            bind: vec!["127.0.0.1:0".parse().unwrap()],
            data_dir: dir.clone(),
            proc: "/nonexistent".into(),
            sys: "/nonexistent".into(),
            root: "/".into(),
            docker_sock: "/nonexistent".into(),
            interval: 1,
            live_window: 900,
            demo: false,
            password: None,
            web_dir: None,
            db_memory: "128MB".into(),
            db_threads: 1,
        };
        let core = Core::new(cfg).unwrap();
        let mut events = core.subscribe_events();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let t = ProbeTarget { id: "lokal".into(), name: "Lokal".into(), kind: ProbeKind::Tcp, host: "127.0.0.1".into(), port, enabled: true };
        let mut r = Runner::new(core.clone(), vec![(t, false)]);
        let now = util::now();
        r.round(now, 0).await;
        assert!(r.targets[0].state.up);
        drop(listener);
        for i in 1..=3 {
            r.round(now + i * 10, i as u64).await;
        }
        assert!(!r.targets[0].state.up);
        let e = events.recv().await.unwrap();
        assert_eq!(e.kind, "probe.down");
        let _listener = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
        r.round(now + 40, 4).await;
        assert_eq!(events.recv().await.unwrap().kind, "probe.up");
        let snap = core.snaps.get("probes.status").unwrap();
        assert_eq!(snap.data["targets"][0]["uptime_pct"], 40.0);
        assert!(core.registry.handle("probe.latency.lokal").is_some());
        let _ = std::fs::remove_dir_all(dir);
    }
}
