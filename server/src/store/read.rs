//! Lesen: Stufe wählen, Datenträger (Dateien + offene Abschnitte) und Ringpuffer auf ein Raster mischen.
//!
//! Je Punkt des Rasters werden Teilsummen gesammelt (Summe, Anzahl, Minimum, Maximum, letzter Wert).
//! So passen Rohwerte, Minuten- und Stundenwerte und der Ringpuffer zusammen, auch wenn ein Punkt
//! auf einer Grenze zwischen zwei Quellen liegt.

use std::collections::HashMap;

use duckdb::Connection;
use shared::{
    query::{Agg, TIER_RAM},
    storage::TIER_RES,
};

use super::{
    Shared,
    layout::{Catalog, TIERS, sql_str},
};

/// Teilsummen eines Rasterpunkts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Acc {
    pub sum: f64,
    pub cnt: f64,
    pub min: f64,
    pub max: f64,
    pub last_ts: i64,
    pub last: f64,
}

impl Acc {
    pub const EMPTY: Self = Self { sum: 0.0, cnt: 0.0, min: f64::INFINITY, max: f64::NEG_INFINITY, last_ts: i64::MIN, last: f64::NAN };

    pub fn add(&mut self, o: &Self) {
        self.sum += o.sum;
        self.cnt += o.cnt;
        self.min = self.min.min(o.min);
        self.max = self.max.max(o.max);
        if o.last_ts >= self.last_ts {
            self.last_ts = o.last_ts;
            self.last = o.last;
        }
    }

    fn raw(ts: i64, v: f64) -> Self {
        Self { sum: v, cnt: 1.0, min: v, max: v, last_ts: ts, last: v }
    }

    pub fn is_empty(&self) -> bool {
        self.cnt <= 0.0
    }

    pub fn value(&self, agg: Agg) -> Option<f64> {
        if self.is_empty() {
            return None;
        }
        let v = match agg {
            Agg::Avg => self.sum / self.cnt,
            Agg::Min => self.min,
            Agg::Max => self.max,
            Agg::Sum => self.sum,
            Agg::Last => self.last,
        };
        Some(tidy(v))
    }

    pub fn lo(&self) -> Option<f64> {
        (!self.is_empty()).then(|| tidy(self.min))
    }

    pub fn hi(&self) -> Option<f64> {
        (!self.is_empty()).then(|| tidy(self.max))
    }
}

/// Auf 6 gültige Stellen: entfernt die Reste von FLOAT (12.34 statt 12.340000152587891).
pub fn tidy(v: f64) -> f64 {
    if v == 0.0 || !v.is_finite() {
        return v;
    }
    let digits = 5 - v.abs().log10().floor() as i32;
    if digits <= 0 {
        return v.round();
    }
    let f = 10f64.powi(digits.min(15));
    (v * f).round() / f
}

/// Höchstens so viele Zeilen je Reihe aus einer Stufe lesen (24 h Sekundenwerte → Minuten).
const MAX_ROWS_PER_SERIES: i64 = 20_000;

/// Raster einer Abfrage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    pub tier: u8,
    pub start: i64,
    pub step: u32,
    pub n: usize,
}

impl Plan {
    pub fn end(&self) -> i64 {
        self.start + self.n as i64 * i64::from(self.step)
    }
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// Ab hier kommen die Werte aus dem Ringpuffer (dort liegt alles, auch was noch nicht geschrieben ist).
/// Vor dem Start dieses Prozesses steht nichts im Ring – das kommt dann vom Datenträger.
pub fn ring_from(sh: &Shared, now: i64) -> i64 {
    (now - i64::from(sh.ring.window().min(600)) + 30).max(sh.started + 1)
}

/// Stufe und Raster wählen. `max_points` begrenzt die Punktzahl (Export).
pub fn plan(sh: &Shared, now: i64, after: i64, before: i64, points: u32, own: u32) -> Plan {
    let own = own.max(1);
    let span = (before - after).max(1);
    let wanted = (span as f64 / f64::from(points.max(1))).ceil().max(1.0) as u32;
    let tier = if after >= ring_from(sh, now) {
        TIER_RAM
    } else {
        let retention = sh.settings.read().retention;
        let mut t = (0..TIERS).rev().find(|t| TIER_RES[*t] <= wanted).unwrap_or(0);
        // Gröbere Stufe, wenn die feinere den Beginn nicht mehr hat oder zu viele Zeilen je Reihe läse.
        while t + 1 < TIERS && (after < now - retention[t] as i64 || span / i64::from(TIER_RES[t]) > MAX_ROWS_PER_SERIES) {
            t += 1;
        }
        t as u8
    };
    let res = if tier == TIER_RAM { 1 } else { TIER_RES[tier as usize] };
    let unit = res / gcd(res, own) * own;
    let step = wanted.max(own).div_ceil(unit) * unit;
    let start = after.div_euclid(i64::from(step)) * i64::from(step);
    let n = ((before - start) / i64::from(step) + 1).clamp(1, 200_000) as usize;
    Plan { tier, start, step, n }
}

/// Werte der Reihen (alle mit demselben Takt `own`) auf das Raster `plan` legen.
pub fn read(sh: &Shared, conn: &Connection, series: &[u32], plan: &Plan, now: i64) -> anyhow::Result<HashMap<u32, Vec<Acc>>> {
    let mut out: HashMap<u32, Vec<Acc>> = series.iter().map(|s| (*s, vec![Acc::EMPTY; plan.n])).collect();
    if series.is_empty() {
        return Ok(out);
    }
    let end = plan.end();
    let rfrom = ring_from(sh, now);
    let ring_lo = if plan.tier == TIER_RAM { plan.start } else { rfrom.max(plan.start) };
    if plan.tier != TIER_RAM && plan.start < rfrom {
        let disk_end = end.min(rfrom);
        let mut tries = 0;
        loop {
            let cat = sh.catalog.read().clone();
            match read_disk(sh, conn, &cat, series, plan, disk_end, &mut out) {
                Ok(()) => break,
                // Ein Abschnitt wurde gerade abgeschlossen oder gelöscht: mit neuem Verzeichnis wiederholen.
                Err(e) if tries == 0 => {
                    tracing::debug!("Abfrage wird wiederholt: {e}");
                    tries += 1;
                    for accs in out.values_mut() {
                        accs.fill(Acc::EMPTY);
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }
    add_ring(sh, series, plan, ring_lo, &mut out);
    Ok(out)
}

/// Nur aus dem Ringpuffer (ganzer Zeitraum liegt im Arbeitsspeicher).
pub fn read_ring_only(sh: &Shared, series: &[u32], plan: &Plan) -> HashMap<u32, Vec<Acc>> {
    let mut out: HashMap<u32, Vec<Acc>> = series.iter().map(|s| (*s, vec![Acc::EMPTY; plan.n])).collect();
    add_ring(sh, series, plan, plan.start, &mut out);
    out
}

/// Ringpuffer: Rohwerte ab `from` im eigenen Takt der Reihe holen und einsortieren.
fn add_ring(sh: &Shared, series: &[u32], plan: &Plan, from: i64, out: &mut HashMap<u32, Vec<Acc>>) {
    let end = plan.end();
    for s in series {
        let own = i64::from(sh.registry.series_interval(*s).max(1));
        let first = (from + own - 1).div_euclid(own) * own;
        if first >= end {
            continue;
        }
        let m = ((end - first + own - 1) / own) as usize;
        let (vals, _, _) = sh.ring.read(*s, first, own as u32, m, |v| v[v.len() - 1]);
        let Some(accs) = out.get_mut(s) else { continue };
        for (i, v) in vals.into_iter().enumerate() {
            let Some(v) = v else { continue };
            let ts = first + i as i64 * own;
            let b = ((ts - plan.start) / i64::from(plan.step)) as usize;
            if b < accs.len() {
                accs[b].add(&Acc::raw(ts, v));
            }
        }
    }
}

fn read_disk(
    sh: &Shared,
    conn: &Connection,
    cat: &Catalog,
    series: &[u32],
    plan: &Plan,
    disk_end: i64,
    out: &mut HashMap<u32, Vec<Acc>>,
) -> anyhow::Result<()> {
    let list = series.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
    let mut parts = Vec::new();
    let mut add = |tier: usize, from: i64, to: i64| {
        if from >= to {
            return;
        }
        let cols = if tier == 0 { "series, ts, v, v AS lo, v AS hi, 1 AS n" } else { "series, ts, v, lo, hi, n" };
        let filter = format!("series IN ({list}) AND ts >= {} AND ts < {}", from.max(0), to.max(0));
        let (files, tables) = cat.sources(tier, from, to, &sh.dir);
        if !files.is_empty() {
            let files: Vec<String> = files.iter().map(|p| sql_str(p)).collect();
            parts.push(format!("SELECT {cols} FROM read_parquet([{}]) WHERE {filter}", files.join(", ")));
        }
        for t in tables {
            parts.push(format!("SELECT {cols} FROM {t} WHERE {filter}"));
        }
    };
    if plan.tier == 0 {
        add(0, plan.start, disk_end);
    } else {
        // Verdichtet bis `closed`, danach aus den Rohwerten der offenen Stunden.
        add(plan.tier as usize, plan.start, disk_end.min(cat.closed));
        add(0, plan.start.max(cat.closed), disk_end);
    }
    if parts.is_empty() {
        return Ok(());
    }
    let sql = format!(
        "SELECT series, ((ts::BIGINT - {start}) // {step})::BIGINT AS b, sum(v::DOUBLE * n), sum(n)::DOUBLE,
                min(lo)::DOUBLE, max(hi)::DOUBLE, max(ts)::BIGINT, arg_max(v, ts)::DOUBLE
         FROM ({union}) GROUP BY 1, 2",
        start = plan.start,
        step = plan.step,
        union = parts.join(" UNION ALL "),
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let s: u32 = r.get(0)?;
        let b: i64 = r.get(1)?;
        let acc = Acc { sum: r.get(2)?, cnt: r.get(3)?, min: r.get(4)?, max: r.get(5)?, last_ts: r.get(6)?, last: r.get(7)? };
        if let Some(slot) = out.get_mut(&s).and_then(|accs| usize::try_from(b).ok().and_then(|b| accs.get_mut(b))) {
            slot.add(&acc);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tidy_removes_float_noise() {
        assert_eq!(tidy(f64::from(12.34f32)), 12.34);
        assert_eq!(tidy(f64::from(0.0123f32)), 0.0123);
        assert_eq!(tidy(f64::from(3_221_225_472f32)), 3_221_225_472.0);
        assert_eq!(tidy(0.0), 0.0);
    }

    #[test]
    fn acc_combines() {
        let mut a = Acc::EMPTY;
        a.add(&Acc::raw(10, 2.0));
        a.add(&Acc { sum: 12.0, cnt: 3.0, min: 1.0, max: 9.0, last_ts: 5, last: 1.0 });
        assert_eq!(a.value(Agg::Avg), Some(3.5));
        assert_eq!(a.value(Agg::Min), Some(1.0));
        assert_eq!(a.value(Agg::Max), Some(9.0));
        assert_eq!(a.value(Agg::Last), Some(2.0));
        assert_eq!(a.value(Agg::Sum), Some(14.0));
        assert_eq!(Acc::EMPTY.value(Agg::Avg), None);
    }
}
