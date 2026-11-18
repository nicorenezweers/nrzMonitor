//! Beispieldaten: Gateway, 1.1.1.1, 9.9.9.9 und eine HTTPS-Seite.

use shared::{
    events::{Event, Severity},
    probes::{ProbeKind, ProbeState, ProbeTarget, ProbesStatus},
};

use super::{Charts, Outcome, describe, http::HttpResult};
use crate::{core::Core, frame::Frame, util::{self, Walk}};

pub async fn run(core: Core) {
    let defs = [
        (ProbeTarget { id: "gateway".into(), name: "Gateway".into(), kind: ProbeKind::Ping, host: "192.168.1.1".into(), port: 0, enabled: true }, true, 0.6),
        (ProbeTarget { id: "cloudflare-dns".into(), name: "Cloudflare DNS".into(), kind: ProbeKind::Ping, host: "1.1.1.1".into(), port: 0, enabled: true }, true, 9.0),
        (ProbeTarget { id: "quad9".into(), name: "Quad9".into(), kind: ProbeKind::Ping, host: "9.9.9.9".into(), port: 0, enabled: true }, true, 14.0),
        (ProbeTarget { id: "dns-zweers".into(), name: "nrzNetwork".into(), kind: ProbeKind::Http, host: "https://dns.zweers.ch/".into(), port: 0, enabled: true }, false, 85.0),
    ];
    let charts: Vec<Charts> = defs.iter().map(|(t, _, _)| Charts::new(&core, t)).collect();
    let mut walks: Vec<Walk> = defs.iter().map(|(_, _, b)| Walk::new(*b, b * 0.6, b * 6.0, b * 0.08)).collect();
    let start = util::now();
    let mut states: Vec<ProbeState> = defs
        .iter()
        .map(|(t, builtin, _)| ProbeState {
            id: t.id.clone(),
            name: t.name.clone(),
            kind: t.kind,
            target: describe(t),
            builtin: *builtin,
            up: true,
            since: start - 86_400 * 2,
            uptime_pct: 100.0,
            ..ProbeState::default()
        })
        .collect();
    let mut down_left = 0u32;
    let mut tick: i64 = -89;
    loop {
        // Vorlauf füllt die letzten 15 Minuten, danach im Takt.
        let now = if tick < 0 { start + tick * 10 } else { util::now() };
        let ts = now.div_euclid(10) * 10;
        let mut f = Frame::new(ts);
        if tick >= 0 && down_left == 0 && fastrand::f64() < 0.004 {
            down_left = 4;
            core.event(Event::new("probe", "probe.down", Severity::Crit, "Quad9", "Quad9 nicht erreichbar").message("Ping · 9.9.9.9: Keine Antwort"));
            states[2].up = false;
            states[2].since = now;
        }
        for (i, (t, _, _)) in defs.iter().enumerate() {
            if t.kind == ProbeKind::Http && ts.rem_euclid(30) != 0 {
                continue;
            }
            let down = i == 2 && down_left > 0;
            let base = walks[i].value * 2.0;
            let lat = walks[i].next_spiky(0.02, base);
            let o = if down {
                Outcome { loss: 100.0, error: Some("Keine Antwort".into()), ..Outcome::default() }
            } else if t.kind == ProbeKind::Http {
                let h = HttpResult {
                    status: 200,
                    dns_ms: util::round(lat * 0.08, 2),
                    connect_ms: util::round(lat * 0.18, 2),
                    tls_ms: util::round(lat * 0.32, 2),
                    wait_ms: util::round(lat * 0.36, 2),
                    transfer_ms: util::round(lat * 0.06, 2),
                    total_ms: util::round(lat, 2),
                };
                Outcome { ok: true, latency_ms: Some(util::round(lat, 2)), status: Some(200), http: Some(h), ..Outcome::default() }
            } else {
                let loss = if fastrand::f64() < 0.03 { 33.33 } else { 0.0 };
                Outcome { ok: true, latency_ms: Some(util::round(lat, 2)), loss, ..Outcome::default() }
            };
            charts[i].put(&mut f, &o);
            let st = &mut states[i];
            st.latency_ms = o.latency_ms;
            st.loss = o.loss;
            st.status = o.status;
            st.error = o.error.clone();
            st.last_check = now;
            st.uptime_pct = if i == 2 { 99.71 } else { 100.0 };
        }
        if down_left > 0 && tick >= 0 {
            down_left -= 1;
            if down_left == 0 {
                core.event(Event::new("probe", "probe.up", Severity::Ok, "Quad9", "Quad9 wieder erreichbar").message("Ping · 9.9.9.9"));
                states[2].up = true;
                states[2].since = now;
            }
        }
        core.submit(f);
        core.snapshot("probes.status", &ProbesStatus { demo: true, targets: states.clone(), icmp_error: None });
        tick += 1;
        if tick > 0 {
            let ms = util::now_ms();
            let next = (ms.div_euclid(10_000) + 1) * 10_000;
            tokio::time::sleep(std::time::Duration::from_millis((next - ms) as u64)).await;
        }
    }
}
