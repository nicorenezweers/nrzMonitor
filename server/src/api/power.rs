//! Strom: Zusammenfassung von Leistung, Energie und Kosten, Einstellungen (Preise, Grundlast).
//!
//! Energie = Summe der gespeicherten Leistungswerte × Takt – exakt über alle Speicherstufen,
//! Zeiten ohne Werte (Gerät aus) zählen nicht. Kosten werden bei jeder Abfrage mit dem Preis
//! gerechnet, der zur jeweiligen Stunde galt; geänderte Preise wirken darum auch rückwirkend.

use std::sync::OnceLock;

use axum::{
    Json, Router,
    extract::{Query, State},
    routing::get,
};
use parking_lot::Mutex;
use serde::Deserialize;
use shared::{
    catalog::power::POWER_DRAW,
    power::{EnergyBucket, EnergyPeriod, PowerSettings, PowerSummary},
};

use super::{ApiErr, ApiResult};
use crate::{collectors::power, core::Core, util};

const HOUR: i64 = 3600;
const DAY: i64 = 86_400;
/// So weit zurück wird gerechnet (Aufbewahrung der Stundenstufe).
const HISTORY_DAYS: i64 = 2 * 366;
/// Stunden je Abfrage an den Speicher.
const CHUNK_HOURS: i64 = 4800;
/// Wie lange die Stundenwerte zwischengespeichert bleiben.
const CACHE_SECS: i64 = 30;

pub fn routes() -> Router<Core> {
    Router::new().route("/power/summary", get(summary)).route("/power/settings", get(get_settings).put(put_settings))
}

async fn get_settings(State(core): State<Core>) -> Json<PowerSettings> {
    let mut s: PowerSettings = core.settings.get("power");
    s.normalize();
    Json(s)
}

async fn put_settings(State(core): State<Core>, Json(mut s): Json<PowerSettings>) -> ApiResult<PowerSettings> {
    s.normalize();
    if s.prices.len() > 500 {
        return Err(ApiErr::bad("Höchstens 500 Preise."));
    }
    core.settings.set("power", &s)?;
    Ok(Json(s))
}

/// Eine Stunde: Beginn, Energie (Wh), Sekunden mit Werten.
#[derive(Debug, Clone, Copy)]
struct Hour {
    start: i64,
    wh: f64,
    secs: f64,
}

type Cache = Mutex<Option<(i64, i64, Vec<Hour>)>>;

fn cache() -> &'static Cache {
    static C: OnceLock<Cache> = OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}

/// Energie je Stunde von `from` bis `to` (beide auf Stunden gerundet).
async fn hourly(core: &Core, from: i64, to: i64) -> anyhow::Result<Vec<Hour>> {
    let now = util::now();
    if let Some((at, f, hours)) = cache().lock().as_ref()
        && now - at < CACHE_SECS
        && *f == from
    {
        return Ok(hours.clone());
    }
    let Some(meta) = core.registry.get(POWER_DRAW.context) else { return Ok(Vec::new()) };
    let series: Vec<u32> = meta.dims.iter().map(|d| d.series).collect();
    let interval = f64::from(meta.interval.max(1));
    let mut out = Vec::new();
    let mut a = from;
    while a < to {
        let n = ((to - a + HOUR - 1) / HOUR).clamp(1, CHUNK_HOURS);
        let b = a + n * HOUR;
        let (plan, data) = core.store.grid(series.clone(), a, b, n as u32).await?;
        let step = i64::from(plan.step.max(1));
        for i in 0..plan.n {
            let start = plan.start + i as i64 * step;
            if start < a || start >= b {
                continue;
            }
            let (mut sum, mut cnt) = (0.0, 0.0f64);
            for (k, s) in series.iter().enumerate() {
                if let Some(acc) = data.get(s).and_then(|v| v.get(i))
                    && acc.cnt > 0.0
                    && acc.sum.is_finite()
                {
                    sum += acc.sum;
                    if k == 0 {
                        cnt = acc.cnt;
                    }
                }
            }
            if cnt > 0.0 {
                out.push(Hour { start, wh: sum * interval / 3600.0, secs: cnt * interval });
            }
        }
        a = b;
    }
    *cache().lock() = Some((now, from, out.clone()));
    Ok(out)
}

// ------------------------------------------------------------------ Kalender (Ortszeit)

/// Tage seit 1970-01-01 → (Jahr, Monat, Tag).
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// (Jahr, Monat, Tag) → Tage seit 1970-01-01.
fn days_from(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Beginn eines Monats (Ortszeit) als Unix-Sekunden; `back` Monate vor dem von `ts`.
fn month_start(ts: i64, off: i64, back: i64) -> i64 {
    let (y, m, _) = civil((ts + off).div_euclid(DAY));
    let idx = y * 12 + (m - 1) - back;
    days_from(idx.div_euclid(12), idx.rem_euclid(12) + 1, 1) * DAY - off
}

fn year_start(ts: i64, off: i64) -> i64 {
    let (y, _, _) = civil((ts + off).div_euclid(DAY));
    days_from(y, 1, 1) * DAY - off
}

// ------------------------------------------------------------------ Zusammenfassung

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SummaryQuery {
    /// Abstand der Ortszeit zu UTC in Minuten (Mitteleuropa: 60 bzw. 120).
    tz: i64,
}

fn sum(hours: &[Hour], s: &PowerSettings, from: i64, to: i64) -> (f64, f64, f64) {
    let now = util::now();
    let (mut kwh, mut cost, mut secs) = (0.0, 0.0, 0.0);
    for h in hours.iter().filter(|h| h.start >= from && h.start < to) {
        let k = h.wh / 1000.0;
        kwh += k;
        // Wechselt der Preis in dieser Stunde, zählt jeder Preis für seinen Anteil der Stunde.
        let end = (h.start + HOUR).min(now.max(h.start + 1));
        cost += k * s.price_between(h.start, end) / 100.0;
        secs += h.secs;
    }
    (kwh, cost, secs)
}

fn bucket(hours: &[Hour], s: &PowerSettings, from: i64, to: i64, now: i64) -> EnergyBucket {
    let (kwh, cost, secs) = sum(hours, s, from, to);
    let length = (to.min(now) - from).max(1) as f64;
    EnergyBucket {
        start: from,
        kwh: util::round(kwh, 4),
        cost: util::round(cost, 4),
        avg_watts: if secs > 0.0 { util::round(kwh * 1000.0 * 3600.0 / secs, 1) } else { 0.0 },
        coverage: util::round((secs / length).min(1.0), 3),
    }
}

async fn summary(State(core): State<Core>, Query(q): Query<SummaryQuery>) -> ApiResult<PowerSummary> {
    let mut s: PowerSettings = core.settings.get("power");
    s.normalize();
    let now = util::now();
    let off = q.tz.clamp(-14 * 60, 14 * 60) * 60;
    let from = (now - HISTORY_DAYS * DAY).div_euclid(HOUR) * HOUR;
    let to = now.div_euclid(HOUR) * HOUR + HOUR;
    let hours = hourly(&core, from, to).await?;

    let today = (now + off).div_euclid(DAY) * DAY - off;
    let month = month_start(now, off, 0);
    let last_month = month_start(now, off, 1);
    let year = year_start(now, off);
    let first = hours.first().map(|h| h.start);
    let end = now + 1;
    let defs: [(&str, &str, i64, i64); 7] = [
        ("today", "Heute", today, end),
        ("yesterday", "Gestern", today - DAY, today),
        ("7d", "Letzte 7 Tage", now - 7 * DAY, end),
        ("month", "Dieser Monat", month, end),
        ("last_month", "Letzter Monat", last_month, month),
        ("year", "Dieses Jahr", year, end),
        ("total", "Gesamt", first.unwrap_or(now), end),
    ];
    let periods = defs
        .iter()
        .map(|(key, label, a, b)| {
            // Die laufende Stunde beginnt vor `now - 7 d` – sie zählt mit, wenn sie überlappt.
            let a_hour = a.div_euclid(HOUR) * HOUR;
            let (kwh, cost, _) = sum(&hours, &s, a_hour, *b);
            EnergyPeriod { key: (*key).into(), label: (*label).into(), from: *a, to: (*b).min(now), kwh: util::round(kwh, 4), cost: util::round(cost, 4) }
        })
        .collect();

    let hour_now = now.div_euclid(HOUR) * HOUR;
    let hours_out = (0..48).rev().map(|k| hour_now - k * HOUR).map(|a| bucket(&hours, &s, a, a + HOUR, now)).collect();
    let days = (0..31).rev().map(|k| today - k * DAY).map(|a| bucket(&hours, &s, a, a + DAY, now)).collect();
    let months = (0..12)
        .rev()
        .map(|k| {
            let a = month_start(now, off, k);
            let b = if k == 0 { end } else { month_start(now, off, k - 1) };
            bucket(&hours, &s, a, b, now)
        })
        .collect();

    let last24 = bucket(&hours, &s, now - DAY, end, now);
    let avg24 = (last24.coverage > 0.0).then_some(last24.avg_watts);
    let price = s.price_at(now);
    let month_kwh = avg24.unwrap_or(0.0) * 24.0 * 30.44 / 1000.0;
    let year_kwh = avg24.unwrap_or(0.0) * 24.0 * 365.0 / 1000.0;

    let (cpu, sensors) = power::measured(&core, &s, now);
    let mut sources = Vec::new();
    if cpu.is_some() {
        let names: Vec<String> = core.registry.get("cpu.power").map(|m| m.dims.iter().map(|d| d.name.clone()).collect()).unwrap_or_default();
        sources.push(format!("RAPL: {}", names.join(", ")));
    }
    if sensors.is_some() {
        let chips: Vec<String> = core.registry.by_context("sensors.power").iter().map(|c| c.family.clone()).collect();
        sources.push(format!("Leistungsfühler: {}", chips.join(", ")));
    }

    Ok(Json(PowerSummary {
        now,
        currency: s.currency.clone(),
        price_now: price,
        measured: cpu.is_some() || sensors.is_some(),
        sources,
        watts_now: power::latest_sum(&core, POWER_DRAW.context, now).map(|w| util::round(w, 1)),
        avg_watts_24h: avg24,
        periods,
        hours: hours_out,
        days,
        months,
        projection_month_kwh: util::round(month_kwh, 3),
        projection_month_cost: util::round(month_kwh * price / 100.0, 2),
        projection_year_kwh: util::round(year_kwh, 2),
        projection_year_cost: util::round(year_kwh * price / 100.0, 2),
        data_since: first,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(days_from(2026, 11, 15), civil_inv(2026, 11, 15));
        // 15.11.2026 12:00 UTC, Ortszeit +1 h → Monatsbeginn 01.11.2026 00:00 Ortszeit = 31.10. 23:00 UTC
        let ts = days_from(2026, 11, 15) * DAY + 12 * HOUR;
        assert_eq!(month_start(ts, HOUR, 0), days_from(2026, 11, 1) * DAY - HOUR);
        assert_eq!(month_start(ts, HOUR, 11), days_from(2025, 12, 1) * DAY - HOUR);
        assert_eq!(year_start(ts, HOUR), days_from(2026, 1, 1) * DAY - HOUR);
    }

    fn civil_inv(y: i64, m: i64, d: i64) -> i64 {
        (0..30_000).find(|x| civil(*x) == (y, m, d)).unwrap_or(-1)
    }

    #[test]
    fn costs_follow_price_history() {
        let s = PowerSettings {
            prices: vec![shared::power::PricePoint { from: 0, cents: 20.0 }, shared::power::PricePoint { from: 2 * HOUR, cents: 40.0 }],
            ..PowerSettings::default()
        };
        let hours = vec![Hour { start: 0, wh: 500.0, secs: 3600.0 }, Hour { start: HOUR, wh: 500.0, secs: 3600.0 }, Hour { start: 2 * HOUR, wh: 500.0, secs: 1800.0 }];
        let (kwh, cost, _) = sum(&hours, &s, 0, 3 * HOUR);
        assert!((kwh - 1.5).abs() < 1e-9);
        assert!((cost - (0.5 * 0.2 + 0.5 * 0.2 + 0.5 * 0.4)).abs() < 1e-9);
        let b = bucket(&hours, &s, 2 * HOUR, 3 * HOUR, 10 * HOUR);
        assert!((b.avg_watts - 1000.0).abs() < 1e-6, "500 Wh in einer halben Stunde = 1000 W");
        assert!((b.coverage - 0.5).abs() < 1e-9);
    }
}
