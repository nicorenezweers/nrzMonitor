//! Beispieldaten: zwei Zonen mit Verkehr, Firewall und zwei Tunneln.

use std::collections::HashMap;

use shared::{
    catalog::cloudflare::*,
    cloudflare::{CfStatus, CfTunnel, CfZone},
    events::{Event, Severity},
};

use super::{LAG, Minute, Rolling, ZoneCharts};
use crate::{core::Core, frame::Frame, registry::ChartDef, util::{self, Walk}};

const COUNTRIES: &[(&str, f64)] = &[("CH", 0.46), ("DE", 0.18), ("US", 0.12), ("FR", 0.06), ("AT", 0.05), ("NL", 0.04), ("GB", 0.03), ("SG", 0.02), ("BR", 0.02), ("CN", 0.02)];

struct Zone {
    info: CfZone,
    charts: ZoneCharts,
    rolling: Rolling,
    load: Walk,
}

fn minute(load: &mut Walk, scale: f64) -> Minute {
    let req = (load.next_spiky(0.02, 400.0) * scale).max(1.0);
    let cached = req * (0.55 + fastrand::f64() * 0.2);
    let bytes = req * (18_000.0 + fastrand::f64() * 8000.0);
    let mut countries = HashMap::new();
    for (c, w) in COUNTRIES {
        countries.insert((*c).to_string(), (req * w * (0.8 + fastrand::f64() * 0.4)).round());
    }
    let s4 = req * 0.04 * fastrand::f64();
    let s5 = if fastrand::f64() < 0.05 { req * 0.02 } else { 0.0 };
    Minute {
        requests: req.round(),
        cached: cached.round(),
        bytes: bytes.round(),
        cached_bytes: (bytes * cached / req).round(),
        threats: Some((req * 0.006 * fastrand::f64()).round()),
        visitors: Some((req * 0.08).round()),
        pageviews: Some((req * 0.21).round()),
        status: [(req * 0.86).round(), (req * 0.07).round(), s4.round(), s5.round()],
        countries,
        firewall: [(req * 0.01 * fastrand::f64()).round(), (req * 0.004).round(), (req * 0.002).round(), 0.0],
    }
}

pub async fn run(core: Core) {
    let mut zones: Vec<Zone> = [("zweers.ch", "Free Website", 160.0), ("nrz-apps.ch", "Pro Website", 60.0)]
        .into_iter()
        .map(|(name, plan, base)| Zone {
            info: CfZone {
                id: format!("demo-{name}"),
                name: name.into(),
                plan: plan.into(),
                status: "active".into(),
                dataset: if plan.starts_with("Free") { "httpRequestsAdaptiveGroups" } else { "httpRequests1mGroups" }.into(),
                firewall_dataset: "firewallEventsAdaptive".into(),
                ..CfZone::default()
            },
            charts: ZoneCharts::new(&core, name),
            rolling: Rolling::default(),
            load: Walk::new(base, base * 0.2, base * 6.0, base * 0.15),
        })
        .collect();
    let tunnels = [("umbrel-home", vec!["ZRH", "FRA"]), ("buero", vec!["ZRH"])];
    let tcharts: Vec<_> = tunnels
        .iter()
        .map(|(n, _)| (core.chart(ChartDef::tpl(&CF_TUNNEL_CONNS).instance(n)), core.chart(ChartDef::tpl(&CF_TUNNEL_UP).instance(n))))
        .collect();
    let start = util::now();
    let mut last = (start - LAG - 13 * 60).div_euclid(60) * 60;
    // Vorlauf für die 24-h-Summen (ohne Diagramme).
    for z in &mut zones {
        let mut w = z.load.clone();
        for i in 0..1440 {
            let ts = last - (1440 - i) * 60;
            z.rolling.push(ts, minute(&mut w, 1.0));
        }
    }
    let mut buero_up = true;
    loop {
        let now = util::now();
        let until = (now - LAG).div_euclid(60) * 60;
        while last + 60 <= until {
            last += 60;
            let mut f = Frame::new(last);
            for z in &mut zones {
                let m = minute(&mut z.load, 1.0);
                z.charts.put(&core, &mut f, &m, true);
                z.rolling.push(last, m);
                z.info.last_minute = last;
            }
            core.submit(f);
        }
        // Tunnel: gelegentlich fällt „buero“ kurz aus.
        if fastrand::f64() < 0.03 {
            buero_up = !buero_up;
            let (sev, title) = if buero_up { (Severity::Ok, "Tunnel buero: verbunden") } else { (Severity::Crit, "Tunnel buero: getrennt") };
            core.event(Event::new("cloudflare", if buero_up { "tunnel.up" } else { "tunnel.down" }, sev, "buero", title));
        }
        let mut f = Frame::new(now.div_euclid(60) * 60);
        let mut list = Vec::new();
        for (i, (name, colos)) in tunnels.iter().enumerate() {
            let up = i == 0 || buero_up;
            let conns = if up { (colos.len() * 2) as u32 } else { 0 };
            f.put(&tcharts[i].0, &[f64::from(conns)]);
            f.put(&tcharts[i].1, &[if up { 1.0 } else { 0.0 }]);
            list.push(CfTunnel {
                id: format!("demo-{name}"),
                name: (*name).into(),
                status: if up { "healthy" } else { "down" }.into(),
                connections: conns,
                colos: if up { colos.iter().map(|c| (*c).to_string()).collect() } else { vec![] },
                status_since: start - 86_400 * 3,
                created_at: "2025-03-01T10:00:00Z".into(),
            });
        }
        core.submit(f);
        let status = CfStatus {
            demo: true,
            ok: true,
            last_poll: util::now_ms(),
            account_id: "demo".into(),
            zones: zones
                .iter()
                .map(|z| {
                    let mut info = z.info.clone();
                    z.rolling.fill(&mut info);
                    info
                })
                .collect(),
            tunnels: list,
            hints: vec!["zweers.ch: Free-Tarif – ohne Bedrohungen und Seitenaufrufe (httpRequestsAdaptiveGroups).".into()],
            ..CfStatus::default()
        };
        core.snapshot("cloudflare.status", &status);
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    }
}
