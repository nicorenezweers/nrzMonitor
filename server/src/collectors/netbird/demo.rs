//! Beispieldaten: ein kleines NetBird-Netz mit Peers, die kommen und gehen.

use shared::{
    catalog::netbird::*,
    events::{Event, Severity},
    netbird::{Activity, Inventory, NetbirdStatus, Peer},
};

use crate::{core::Core, frame::Frame, registry::ChartDef, util};

/// (Name, IP, OS, Gruppen, Land, Stadt, ständig verbunden)
type DemoPeer = (&'static str, &'static str, &'static str, &'static [&'static str], &'static str, &'static str, bool);
const PEERS: &[DemoPeer] = &[
    ("umbrel", "100.92.14.3", "Linux 6.1 (umbrelOS)", &["Server", "Heim"], "CH", "Zürich", true),
    ("nas", "100.92.20.8", "Linux 5.15 (Debian 12)", &["Server", "Heim"], "CH", "Zürich", true),
    ("macbook-nico", "100.92.31.17", "Darwin 15.1", &["Geräte"], "CH", "Bern", false),
    ("iphone", "100.92.44.2", "iOS 18.1", &["Geräte", "Mobil"], "CH", "Bern", false),
    ("vps-fra", "100.92.7.91", "Linux 6.8 (Ubuntu 24.04)", &["Server", "Cloud"], "DE", "Frankfurt", true),
    ("raspberry", "100.92.50.4", "Linux 6.6 (Raspberry Pi OS)", &["Heim"], "CH", "Zürich", true),
    ("ipad", "100.92.61.9", "iPadOS 18.1", &["Geräte", "Mobil"], "CH", "Zürich", false),
    ("buero-pc", "100.92.70.12", "Windows 11", &["Geräte", "Büro"], "CH", "Luzern", false),
];

pub async fn run(core: Core) {
    let peers_chart = core.chart(ChartDef::tpl(&NETBIRD_PEERS));
    let inv_chart = core.chart(ChartDef::tpl(&NETBIRD_INVENTORY));
    let ev_chart = core.chart(ChartDef::tpl(&NETBIRD_EVENTS));
    let online: Vec<_> = PEERS.iter().map(|p| core.chart(ChartDef::tpl(&NETBIRD_PEER_ONLINE).instance(p.0).label("ip", p.1))).collect();
    let mut state: Vec<bool> = PEERS.iter().map(|p| p.6 || fastrand::bool()).collect();
    let start = util::now();
    let mut activity: Vec<Activity> = (0..25)
        .map(|i| {
            let (a, code) = [
                ("Peer angemeldet", "peer.login"),
                ("Peer zu Gruppe hinzugefügt", "peer.group.add"),
                ("Regel geändert", "policy.update"),
                ("Setup-Schlüssel benutzt", "setupkey.peer.add"),
                ("Benutzer angemeldet", "user.login"),
            ][i % 5];
            Activity {
                id: format!("{}", 1000 - i),
                ts: start - (i as i64) * 1900 - 300,
                activity: a.into(),
                code: code.into(),
                initiator: if i % 2 == 0 { "Nico".into() } else { "System".into() },
                target: PEERS[i % PEERS.len()].0.into(),
            }
        })
        .collect();
    // Ring für die Diagramme der letzten Minuten füllen.
    for back in (1..=28).rev() {
        let ts = (start - back * 30).div_euclid(30) * 30;
        let mut f = Frame::new(ts);
        let c = state.iter().filter(|s| **s).count() as f64;
        f.put(&peers_chart, &[c, PEERS.len() as f64 - c]);
        for (i, s) in state.iter().enumerate() {
            f.put(&online[i], &[if *s { 1.0 } else { 0.0 }]);
        }
        if ts % 60 == 0 {
            f.put(&ev_chart, &[f64::from(fastrand::u8(0..3))]);
        }
        core.submit(f);
    }
    let mut tick = 0u64;
    loop {
        let now = util::now();
        for (i, p) in PEERS.iter().enumerate() {
            if !p.6 && fastrand::f64() < 0.02 {
                state[i] = !state[i];
                let ev = if state[i] {
                    Event::new("netbird", "peer.online", Severity::Ok, p.0, format!("Peer {} verbunden", p.0))
                } else {
                    Event::new("netbird", "peer.offline", Severity::Warn, p.0, format!("Peer {} getrennt", p.0))
                };
                core.event(ev.message(format!("{} · {}", p.1, p.2)));
                activity.insert(0, Activity {
                    id: format!("{}", 2000 + tick),
                    ts: now,
                    activity: if state[i] { "Peer angemeldet".into() } else { "Peer abgemeldet".into() },
                    code: if state[i] { "peer.login".into() } else { "peer.logout".into() },
                    initiator: "System".into(),
                    target: p.0.into(),
                });
                activity.truncate(100);
            }
        }
        let ts = now.div_euclid(30) * 30;
        let mut f = Frame::new(ts);
        let c = state.iter().filter(|s| **s).count();
        f.put(&peers_chart, &[c as f64, (PEERS.len() - c) as f64]);
        for (i, s) in state.iter().enumerate() {
            f.put(&online[i], &[if *s { 1.0 } else { 0.0 }]);
        }
        if tick.is_multiple_of(2) {
            f.put(&ev_chart, &[activity.iter().filter(|a| a.ts > now - 60).count() as f64]);
        }
        if tick.is_multiple_of(10) {
            f.put(&inv_chart, &[3.0, 6.0, 5.0, 2.0, 1.0, 3.0, 1.0]);
        }
        core.submit(f);
        let peers: Vec<Peer> = PEERS
            .iter()
            .enumerate()
            .map(|(i, p)| Peer {
                id: format!("demo-{}", p.0),
                name: p.0.into(),
                hostname: p.0.into(),
                ip: p.1.into(),
                connection_ip: if p.4 == "CH" { "85.195.10.4".into() } else { "49.12.80.3".into() },
                dns_label: format!("{}.netbird.cloud", p.0),
                connected: state[i],
                last_seen: if state[i] { now } else { now - 3600 * (i as i64 + 1) },
                os: p.2.into(),
                version: if i == 7 { "0.30.1".into() } else { "0.31.0".into() },
                groups: p.3.iter().map(|g| (*g).to_string()).collect(),
                country: p.4.into(),
                city: p.5.into(),
                user: "nico".into(),
                ssh: i < 2,
                login_expired: false,
                approval_required: false,
            })
            .collect();
        let mut sorted = peers.clone();
        sorted.sort_by(|a, b| b.connected.cmp(&a.connected).then(a.name.cmp(&b.name)));
        core.snapshot("netbird.peers", &sorted);
        let status = NetbirdStatus {
            demo: true,
            ok: true,
            url: shared::netbird::DEFAULT_URL.into(),
            last_poll: util::now_ms(),
            peers: PEERS.len() as u32,
            connected: c as u32,
            inventory: Inventory { users: 3, groups: 6, policies: 5, routes: 2, networks: 1, setup_keys: 3, nameservers: 1 },
            users: vec!["Nico".into(), "Gast".into(), "Service".into()],
            groups: vec![("All".into(), 8), ("Server".into(), 3), ("Heim".into(), 3), ("Geräte".into(), 4), ("Mobil".into(), 2), ("Büro".into(), 1)],
            policies: vec![("Default".into(), false), ("Geräte → Server".into(), true), ("Admin SSH".into(), true), ("Büro → NAS".into(), true), ("Cloud ↔ Heim".into(), true)],
            routes: vec!["heimnetz 192.168.1.0/24".into(), "buero 10.10.0.0/24".into()],
            networks: vec!["Heimnetz".into()],
            setup_keys: vec![("Server".into(), true), ("Einmalig".into(), false), ("Mobil".into(), true)],
            nameservers: vec!["nrzNetwork".into()],
            activity: activity.clone(),
            ..NetbirdStatus::default()
        };
        core.snapshot("netbird.status", &status);
        tick += 1;
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    }
}
