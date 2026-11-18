//! Aus dem Abfrageprotokoll die Diagramme im 10-Sekunden-Raster bilden.
//!
//! Anfragen treffen leicht verspätet ein (nrzNetwork schreibt gebündelt, wir holen mit Überlappung).
//! Ein Abschnitt wird darum erst abgegeben, wenn er sicher vollständig ist – und jeder genau einmal.

use std::collections::{BTreeMap, HashMap, HashSet};

use shared::{catalog::dns::*, dns::QueryEntry};

use crate::{
    core::Core,
    frame::Frame,
    registry::{ChartDef, ChartHandle},
    util,
};

const STEP: i64 = shared::catalog::dns::STEP as i64;
/// Höchstens so viele einzelne Dimensionen bei Clients/Typen, Rest als `other`.
const MAX_DIMS: usize = 11;
/// Ältere Abschnitte lassen sich nicht mehr abgeben (Ring und Speicher nehmen ≤ 15 min zurück).
const MAX_AGE: i64 = 14 * 60;

#[derive(Default)]
struct Bucket {
    kinds: [u32; 7],
    rcodes: [u32; 5],
    qtypes: HashMap<String, u32>,
    clients: HashMap<String, u32>,
    ms: Vec<u32>,
}

/// Dynamische Dimensionen (stabile Reihenfolge, neue hinten, Rest `other`).
struct Dyn {
    names: Vec<String>,
    handle: ChartHandle,
    tpl: &'static shared::catalog::Tpl,
}

impl Dyn {
    fn new(core: &Core, tpl: &'static shared::catalog::Tpl, names: Vec<String>) -> Self {
        let handle = core.chart(def(tpl, &names));
        Self { names, handle, tpl }
    }

    /// Platz eines Namens; neue bekommen einen, solange frei. `None` = `other`.
    fn slot(&mut self, core: &Core, name: &str) -> Option<usize> {
        if let Some(i) = self.names.iter().position(|n| n == name) {
            return Some(i);
        }
        if self.names.len() < MAX_DIMS && !name.is_empty() {
            self.names.push(name.to_string());
            self.handle = core.chart(def(self.tpl, &self.names));
            return Some(self.names.len() - 1);
        }
        None
    }

    fn values(&mut self, core: &Core, counts: &HashMap<String, u32>) -> Vec<f64> {
        let mut order: Vec<(&String, &u32)> = counts.iter().collect();
        order.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let mut out = vec![0.0; self.names.len() + 1];
        let mut other = 0u32;
        for (name, n) in order {
            match self.slot(core, name) {
                Some(i) => {
                    if out.len() <= i {
                        out.resize(i + 1, 0.0);
                    }
                    out[i] += f64::from(*n);
                }
                None => other += n,
            }
        }
        out.resize(self.names.len() + 1, 0.0);
        let last = out.len() - 1;
        out[last] = f64::from(other);
        out.iter().map(|v| util::round(v / STEP as f64, 1)).collect()
    }
}

fn def(tpl: &shared::catalog::Tpl, names: &[String]) -> ChartDef {
    let mut dims: Vec<(String, String)> = names.iter().map(|n| (n.clone(), n.clone())).collect();
    dims.push(("other".into(), "Andere".into()));
    ChartDef::tpl(tpl).dims(dims)
}

pub struct Agg {
    core: Core,
    queries: ChartHandle,
    rcodes: ChartHandle,
    latency: ChartHandle,
    blocked: ChartHandle,
    clients_active: ChartHandle,
    qtypes: Dyn,
    clients: Dyn,
    buckets: BTreeMap<i64, Bucket>,
    minutes: BTreeMap<i64, HashSet<String>>,
    /// Abschnitte mit Beginn vor diesem Zeitpunkt sind abgegeben (Unix-Sekunden).
    pub emitted_until: i64,
}

impl Agg {
    /// `qtypes`/`clients`: bekannte Namen (häufigste zuerst) für die ersten Dimensionen.
    pub fn new(core: &Core, emitted_until: i64, qtypes: Vec<String>, clients: Vec<String>) -> Self {
        let take = |v: Vec<String>| v.into_iter().filter(|s| !s.is_empty()).take(MAX_DIMS).collect::<Vec<_>>();
        Self {
            core: core.clone(),
            queries: core.chart(ChartDef::tpl(&DNS_QUERIES)),
            rcodes: core.chart(ChartDef::tpl(&DNS_RCODES)),
            latency: core.chart(ChartDef::tpl(&DNS_LATENCY)),
            blocked: core.chart(ChartDef::tpl(&DNS_BLOCKED_PCT)),
            clients_active: core.chart(ChartDef::tpl(&DNS_CLIENTS)),
            qtypes: Dyn::new(core, &DNS_QTYPES, take(qtypes)),
            clients: Dyn::new(core, &DNS_CLIENT_QUERIES, take(clients)),
            buckets: BTreeMap::new(),
            minutes: BTreeMap::new(),
            emitted_until: emitted_until.div_euclid(STEP) * STEP,
        }
    }

    pub fn add(&mut self, entries: &[QueryEntry]) {
        for e in entries {
            let secs = e.ts.div_euclid(1000);
            let b = secs.div_euclid(STEP) * STEP;
            if b < self.emitted_until {
                continue;
            }
            let bucket = self.buckets.entry(b).or_default();
            if let Some(i) = KIND_IDX.iter().position(|k| *k == e.kind) {
                bucket.kinds[i] += 1;
            }
            let rc = match e.rcode.as_str() {
                "NOERROR" => 0,
                "NXDOMAIN" => 1,
                "SERVFAIL" => 2,
                "REFUSED" => 3,
                _ => 4,
            };
            bucket.rcodes[rc] += 1;
            *bucket.qtypes.entry(e.qtype.clone()).or_default() += 1;
            *bucket.clients.entry(e.client.clone()).or_default() += 1;
            if !matches!(e.kind.as_str(), "blocked" | "dropped" | "failed") {
                bucket.ms.push(e.ms);
            }
            self.minutes.entry(secs.div_euclid(60) * 60).or_default().insert(e.client.clone());
        }
    }

    /// Alle Abschnitte abgeben, die vor `final_until` enden (auch leere – sonst entstehen Lücken).
    pub fn flush(&mut self, final_until: i64, now: i64) -> Vec<Frame> {
        let mut frames = Vec::new();
        let limit = final_until.div_euclid(STEP) * STEP;
        let oldest = (now - MAX_AGE).div_euclid(STEP) * STEP;
        if self.emitted_until < oldest {
            self.emitted_until = oldest;
        }
        let mut b = self.emitted_until;
        while b + STEP <= limit {
            let bucket = self.buckets.remove(&b).unwrap_or_default();
            frames.push(self.frame(b, &bucket));
            b += STEP;
        }
        self.emitted_until = self.emitted_until.max(b);
        self.buckets.retain(|k, _| *k >= self.emitted_until);
        // Aktive Clients je Minute, wenn die Minute vollständig ist.
        let done: Vec<i64> = self.minutes.keys().copied().filter(|m| m + 60 <= limit).collect();
        for m in done {
            if let Some(set) = self.minutes.remove(&m)
                && m >= oldest
            {
                let mut f = Frame::new(m);
                f.put(&self.clients_active, &[set.len() as f64]);
                frames.push(f);
            }
        }
        frames
    }

    fn frame(&mut self, ts: i64, b: &Bucket) -> Frame {
        let mut f = Frame::new(ts);
        let rate = |n: u32| util::round(f64::from(n) / STEP as f64, 1);
        f.put(&self.queries, &b.kinds.map(rate));
        f.put(&self.rcodes, &b.rcodes.map(rate));
        let total: u32 = b.kinds.iter().sum();
        let blocked = b.kinds[2];
        f.put(&self.blocked, &[if total == 0 { 0.0 } else { util::round(f64::from(blocked) * 100.0 / f64::from(total), 2) }]);
        let mut ms = b.ms.clone();
        if ms.is_empty() {
            f.put(&self.latency, &[f64::NAN, f64::NAN, f64::NAN]);
        } else {
            ms.sort_unstable();
            let avg = ms.iter().map(|v| f64::from(*v)).sum::<f64>() / ms.len() as f64;
            let p95 = f64::from(ms[((ms.len() as f64 * 0.95).ceil() as usize).saturating_sub(1).min(ms.len() - 1)]);
            let max = f64::from(*ms.last().unwrap_or(&0));
            f.put(&self.latency, &[util::round(avg, 2), p95, max]);
        }
        let q = self.qtypes.values(&self.core, &b.qtypes);
        f.put(&self.qtypes.handle, &q);
        let c = self.clients.values(&self.core, &b.clients);
        f.put(&self.clients.handle, &c);
        f
    }
}

/// Reihenfolge wie die Dimensionen von `dns.queries`.
const KIND_IDX: [&str; 7] = ["forwarded", "cached", "blocked", "authoritative", "local", "failed", "dropped"];

#[cfg(test)]
mod tests {
    #[test]
    fn kind_order_matches_catalog() {
        let dims: Vec<&str> = shared::catalog::dns::DNS_QUERIES.dims.iter().map(|d| d.id).collect();
        assert_eq!(dims, super::KIND_IDX);
    }
}
