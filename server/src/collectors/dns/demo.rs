//! Beispieldaten: ein glaubwürdiges Heimnetz mit Clients, Domains, Sperren und Weiterleitungen.

use shared::dns::QueryEntry;

use super::{BlockList, BlockingStatus, ServerStatus, UpstreamHealth};
use crate::util;

/// (IP, Gewicht) – wer wie viel fragt.
const CLIENTS: &[(&str, f64)] = &[
    ("192.168.1.20", 0.22),
    ("192.168.1.31", 0.16),
    ("192.168.1.42", 0.12),
    ("192.168.1.15", 0.10),
    ("192.168.1.50", 0.08),
    ("192.168.1.77", 0.07),
    ("192.168.1.10", 0.06),
    ("172.17.0.1", 0.06),
    ("192.168.1.63", 0.05),
    ("100.92.14.3", 0.04),
    ("192.168.1.88", 0.03),
    ("fd00::1c", 0.01),
];

const DOMAINS: &[(&str, f64)] = &[
    ("api.github.com", 0.07),
    ("www.google.com", 0.08),
    ("i.ytimg.com", 0.05),
    ("www.youtube.com", 0.05),
    ("dns.zweers.ch", 0.04),
    ("time.apple.com", 0.04),
    ("gateway.icloud.com", 0.05),
    ("registry-1.docker.io", 0.04),
    ("api.netbird.io", 0.03),
    ("signal.netbird.io", 0.02),
    ("update.umbrel.com", 0.02),
    ("ntp.ubuntu.com", 0.02),
    ("www.srf.ch", 0.03),
    ("www.20min.ch", 0.03),
    ("cdn.jsdelivr.net", 0.02),
    ("fonts.gstatic.com", 0.03),
    ("open.spotify.com", 0.03),
    ("api.cloudflare.com", 0.02),
    ("mail.proton.me", 0.02),
    ("raspberrypi.local", 0.02),
    ("nas.home.arpa", 0.03),
    ("1.1.168.192.in-addr.arpa", 0.02),
    ("connectivitycheck.gstatic.com", 0.03),
    ("push.apple.com", 0.03),
];

const BLOCKED: &[(&str, f64)] = &[
    ("doubleclick.net", 0.25),
    ("googleads.g.doubleclick.net", 0.15),
    ("app-measurement.com", 0.12),
    ("graph.facebook.com", 0.08),
    ("telemetry.microsoft.com", 0.1),
    ("ads.youtube.com", 0.08),
    ("metrics.icloud.com", 0.08),
    ("tracking.example-ads.com", 0.06),
    ("analytics.tiktok.com", 0.08),
];

const UPSTREAMS: &[(&str, &str, f64)] =
    &[("tls://1.1.1.1:853", "Cloudflare", 14.0), ("tls://9.9.9.9:853", "Quad9", 19.0), ("udp://192.168.1.1:53", "Router", 2.5)];

pub struct Gen {
    next_id: i64,
    started: i64,
    total: u64,
    ok: [u64; 3],
    failed: [u64; 3],
}

fn pick<T>(list: &[(T, f64)]) -> &T {
    let sum: f64 = list.iter().map(|(_, w)| w).sum();
    let mut r = fastrand::f64() * sum;
    for (v, w) in list {
        if r < *w {
            return v;
        }
        r -= w;
    }
    &list[list.len() - 1].0
}

impl Gen {
    pub fn new() -> Self {
        Self { next_id: 1, started: util::now() - 3 * 86_400, total: 0, ok: [0; 3], failed: [0; 3] }
    }

    /// Anfragen je Sekunde zur Tageszeit (tagsüber mehr, nachts wenig).
    fn rate(ts: i64) -> f64 {
        let hour = ((ts + 3600).rem_euclid(86_400)) as f64 / 3600.0;
        let day = (-((hour - 14.0) / 5.0).powi(2)).exp();
        let evening = (-((hour - 20.5) / 2.0).powi(2)).exp();
        0.7 + 1.4 * day + 1.1 * evening
    }

    /// Alle Anfragen der Sekunden `from..=to`.
    pub fn generate(&mut self, from: i64, to: i64) -> Vec<QueryEntry> {
        let mut out = Vec::new();
        for s in from..=to {
            let r = Self::rate(s) * (0.7 + fastrand::f64() * 0.6);
            let n = r.floor() as u32 + u32::from(fastrand::f64() < r.fract());
            // Gelegentlich ein Schwall (App-Start, Update).
            let n = if fastrand::f64() < 0.002 { n + fastrand::u32(10..40) } else { n };
            for _ in 0..n {
                out.push(self.entry(s * 1000 + i64::from(fastrand::u32(0..1000))));
            }
        }
        out
    }

    fn entry(&mut self, ts: i64) -> QueryEntry {
        let id = self.next_id;
        self.next_id += 1;
        self.total += 1;
        let client = pick(CLIENTS).to_string();
        let roll = fastrand::f64();
        let qtype = if roll < 0.55 {
            "A"
        } else if roll < 0.85 {
            "AAAA"
        } else if roll < 0.94 {
            "HTTPS"
        } else if roll < 0.97 {
            "PTR"
        } else if roll < 0.985 {
            "TXT"
        } else {
            "SRV"
        };
        let blocked = fastrand::f64() < 0.14;
        let (name, kind, rcode, answer, ms, upstream) = if blocked {
            (pick(BLOCKED).to_string(), "blocked", "NOERROR", if qtype == "AAAA" { "::" } else { "0.0.0.0" }.to_string(), 0, String::new())
        } else {
            let name = pick(DOMAINS).to_string();
            let local = name.ends_with(".local") || name.ends_with(".home.arpa") || name.ends_with(".in-addr.arpa");
            let r = fastrand::f64();
            if local {
                let kind = if name.ends_with(".home.arpa") { "authoritative" } else { "local" };
                (name, kind, "NOERROR", "192.168.1.5".to_string(), fastrand::u32(0..2), String::new())
            } else if r < 0.58 {
                (name, "cached", "NOERROR", fake_answer(qtype), fastrand::u32(0..2), String::new())
            } else if r < 0.985 {
                let u = fastrand::usize(0..UPSTREAMS.len());
                let (label, _, base) = UPSTREAMS[u];
                self.ok[u] += 1;
                let ms = (base * (0.5 + fastrand::f64() * 1.5) + if fastrand::f64() < 0.03 { 120.0 } else { 0.0 }) as u32;
                let nx = fastrand::f64() < 0.05;
                (name, "forwarded", if nx { "NXDOMAIN" } else { "NOERROR" }, if nx { String::new() } else { fake_answer(qtype) }, ms, label.to_string())
            } else {
                let u = fastrand::usize(0..UPSTREAMS.len());
                self.failed[u] += 1;
                (name, "failed", "SERVFAIL", String::new(), 2000, String::new())
            }
        };
        QueryEntry {
            ts,
            id,
            client,
            proto: if fastrand::f64() < 0.93 { "udp" } else if fastrand::bool() { "tcp" } else { "https" }.into(),
            name,
            qtype: qtype.into(),
            rcode: rcode.into(),
            kind: kind.into(),
            answer,
            ms,
            upstream,
        }
    }

    pub fn server_status(&self) -> ServerStatus {
        let now = util::now();
        ServerStatus {
            version: "1.4.2".into(),
            uptime_secs: (now - self.started) as u64,
            dns_listen: vec!["0.0.0.0:53/udp".into(), "0.0.0.0:53/tcp".into()],
            listen_errors: vec![],
            queries_per_second: util::round(Self::rate(now), 2),
            cache_entries: 4200 + (self.total % 900),
            zones: 2,
            blocked_domains: 184_312,
            blocking_enabled: true,
            blocking_paused_until: None,
            upstreams: UPSTREAMS
                .iter()
                .enumerate()
                .map(|(i, (label, name, base))| UpstreamHealth {
                    upstream: (*label).into(),
                    name: (*name).into(),
                    ok: 50_000 + self.ok[i],
                    failed: 40 + self.failed[i],
                    avg_ms: util::round(base * (0.9 + fastrand::f64() * 0.2), 2),
                    last_error: (i == 1).then(|| "Zeitüberschreitung".into()),
                    last_error_at: (i == 1).then(|| now - 5400),
                })
                .collect(),
        }
    }

    pub fn blocking_status(&self) -> BlockingStatus {
        let now = util::now();
        let list = |name: &str, url: &str, n: u64, err: Option<&str>| BlockList {
            name: name.into(),
            url: url.into(),
            enabled: true,
            domain_count: n,
            last_updated_at: Some(now - 7200),
            last_error: err.map(Into::into),
        };
        BlockingStatus {
            enabled: true,
            paused_until: None,
            total_domains: 184_312,
            lists: vec![
                list("StevenBlack Unified", "https://raw.githubusercontent.com/StevenBlack/hosts/master/hosts", 98_204, None),
                list("OISD small", "https://small.oisd.nl/domainswild", 61_883, None),
                list("HaGeZi Pro", "https://raw.githubusercontent.com/hagezi/dns-blocklists/main/wildcard/pro.txt", 41_120, None),
                list("Eigene Tracker", "https://example.org/liste.txt", 0, Some("HTTP 404")),
            ],
            custom_blocked: 12,
            allowed: 4,
        }
    }
}

fn fake_answer(qtype: &str) -> String {
    match qtype {
        "A" => format!("{}.{}.{}.{}", fastrand::u8(1..223), fastrand::u8(0..255), fastrand::u8(0..255), fastrand::u8(1..254)),
        "AAAA" => format!("2a00:1450:400a:{:x}::200e", fastrand::u16(0x800..0x8ff)),
        "HTTPS" => "1 . alpn=h2,h3".into(),
        "PTR" => "router.home.arpa".into(),
        _ => String::new(),
    }
}
