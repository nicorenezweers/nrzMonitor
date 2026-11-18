//! Auswertungen auf dem Archiv: Kennzahlen je Reihe, Rangliste eines Kontexts, „Was hat sich
//! geändert?“ (wie Netdatas Metric Correlations) und CSV-Export.

use std::{collections::HashMap, fmt::Write as _};

use shared::{
    ChartMeta,
    query::Agg,
    storage::{CorrelateItem, CorrelateReq, CorrelateResp, SeriesStats, StatsReq, StatsResp, TopItem, TopReq, TopResp},
};

use super::{Acc, Store, read::tidy, resolve};

/// Punkte für Kennzahlen: fein genug für Perzentile, klein genug für viele Reihen.
const STATS_POINTS: u32 = 1500;

fn quantile(sorted: &[f64], q: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let pos = q * (sorted.len() - 1) as f64;
    let (i, frac) = (pos.floor() as usize, pos.fract());
    let a = sorted[i];
    let b = sorted[(i + 1).min(sorted.len() - 1)];
    Some(tidy(a + (b - a) * frac))
}

/// Gesamtwert einer Reihe über alle Rasterpunkte.
fn total(accs: &[Acc]) -> Acc {
    let mut t = Acc::EMPTY;
    for a in accs {
        if !a.is_empty() {
            t.add(a);
        }
    }
    t
}

/// (Anzahl, Minimum, Mittel, Maximum, letzter, [p50, p95, p99], Standardabweichung)
type Summary = (u64, Option<f64>, Option<f64>, Option<f64>, Option<f64>, [Option<f64>; 3], Option<f64>);

fn stats_of(accs: &[Acc]) -> Summary {
    let t = total(accs);
    let mut pts: Vec<(f64, f64)> = accs.iter().filter(|a| !a.is_empty()).map(|a| (a.sum / a.cnt, a.cnt)).collect();
    let stddev = if t.cnt > 0.0 && !pts.is_empty() {
        let mean = t.sum / t.cnt;
        let var = pts.iter().map(|(v, w)| w * (v - mean).powi(2)).sum::<f64>() / t.cnt;
        Some(tidy(var.sqrt()))
    } else {
        None
    };
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let sorted: Vec<f64> = pts.iter().map(|p| p.0).collect();
    (
        t.cnt as u64,
        t.lo(),
        t.value(Agg::Avg),
        t.hi(),
        t.value(Agg::Last),
        [quantile(&sorted, 0.5), quantile(&sorted, 0.95), quantile(&sorted, 0.99)],
        stddev,
    )
}

/// Kolmogorow-Smirnow-Abstand zweier Stichproben (0 = gleich verteilt, 1 = völlig getrennt).
fn ks(mut a: Vec<f64>, mut b: Vec<f64>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    a.sort_by(f64::total_cmp);
    b.sort_by(f64::total_cmp);
    let (mut i, mut j, mut d) = (0, 0, 0f64);
    while i < a.len() && j < b.len() {
        let x = a[i].min(b[j]);
        while i < a.len() && a[i] <= x {
            i += 1;
        }
        while j < b.len() && b[j] <= x {
            j += 1;
        }
        d = d.max((i as f64 / a.len() as f64 - j as f64 / b.len() as f64).abs());
    }
    d
}

/// Veränderung einer Reihe: Verteilung (KS) gewichtet mit der Grösse des Unterschieds,
/// damit winzige Verschiebungen (0 → 0.01) nicht ganz oben landen.
pub fn change_score(base: &[f64], win: &[f64]) -> f64 {
    if base.is_empty() || win.is_empty() {
        return 0.0;
    }
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
    let (mb, mw) = (mean(base), mean(win));
    let max = |v: &[f64]| v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let scale = base.iter().chain(win).map(|v| v.abs()).fold(0.0, f64::max);
    if scale == 0.0 {
        return 0.0;
    }
    let diff = (mw - mb).abs() + 0.5 * (max(win) - max(base)).abs();
    let weight = (diff / (0.05 * scale)).min(1.0);
    ks(base.to_vec(), win.to_vec()) * weight
}

fn instance(meta: &ChartMeta) -> String {
    if meta.id == meta.context { meta.id.clone() } else { meta.family.clone() }
}

impl Store {
    pub async fn analyze_stats(&self, req: &StatsReq) -> anyhow::Result<StatsResp> {
        let (after, before) = resolve(self.now(), req.after, req.before)?;
        let metas: Vec<ChartMeta> = req.charts.iter().filter_map(|id| self.sh.registry.get(id)).collect();
        let series: Vec<u32> = metas.iter().flat_map(|m| m.dims.iter().map(|d| d.series)).collect();
        let (plan, data) = self.grid(series, after, before, STATS_POINTS).await?;
        let mut out = Vec::new();
        for m in &metas {
            for d in &m.dims {
                let accs = data.get(&d.series).map_or(&[][..], Vec::as_slice);
                let (count, min, avg, max, last, [p50, p95, p99], stddev) = stats_of(accs);
                out.push(SeriesStats {
                    chart: m.id.clone(),
                    dim: d.id.clone(),
                    name: d.name.clone(),
                    unit: m.unit,
                    count,
                    min,
                    avg,
                    max,
                    p50,
                    p95,
                    p99,
                    last,
                    stddev,
                });
            }
        }
        Ok(StatsResp { after, before, tier: plan.tier, series: out })
    }

    pub async fn analyze_top(&self, req: &TopReq) -> anyhow::Result<TopResp> {
        let (after, before) = resolve(self.now(), req.after, req.before)?;
        let metas = self.sh.registry.by_context(&req.context);
        let pick = |m: &ChartMeta| -> Vec<u32> {
            m.dims.iter().filter(|d| req.dim.as_deref().is_none_or(|x| x.is_empty() || d.id == x)).map(|d| d.series).collect()
        };
        let series: Vec<u32> = metas.iter().flat_map(pick).collect();
        let (_, data) = self.grid(series, after, before, 60).await?;
        let mut items: Vec<TopItem> = metas
            .iter()
            .filter_map(|m| {
                let vals: Vec<f64> =
                    pick(m).iter().filter_map(|s| data.get(s)).filter_map(|accs| total(accs).value(req.agg)).collect();
                (!vals.is_empty()).then(|| TopItem { chart: m.id.clone(), instance: instance(m), value: tidy(vals.iter().sum()) })
            })
            .collect();
        items.sort_by(|a, b| b.value.total_cmp(&a.value));
        items.truncate(req.limit.unwrap_or(20).clamp(1, 500) as usize);
        Ok(TopResp { context: req.context.clone(), unit: metas.first().map(|m| m.unit).unwrap_or_default(), after, before, items })
    }

    pub async fn analyze_correlate(&self, req: &CorrelateReq) -> anyhow::Result<CorrelateResp> {
        let started = std::time::Instant::now();
        let (after, before) = resolve(self.now(), req.after, req.before)?;
        let span = before - after;
        let base_after = if req.baseline_after > 0 && req.baseline_after < after { req.baseline_after } else { after - 4 * span };
        let metas: Vec<ChartMeta> = self
            .sh
            .registry
            .list(false)
            .into_iter()
            .filter(|m| req.section.as_deref().is_none_or(|s| s.is_empty() || m.section == s))
            .collect();
        let series: Vec<u32> = metas.iter().flat_map(|m| m.dims.iter().map(|d| d.series)).collect();
        let total_span = before - base_after;
        // Etwa 60 Punkte im Fenster, Vergleichszeitraum entsprechend.
        let points = ((total_span as f64 / span as f64) * 60.0).clamp(60.0, 2000.0) as u32;
        let (plan, data) = self.grid(series, base_after, before, points).await?;
        let split = ((after - plan.start).max(0) / i64::from(plan.step)) as usize;
        let spark_n = req.points.unwrap_or(60).min(500) as usize;
        let mut examined = 0u64;
        let mut items = Vec::new();
        for m in &metas {
            for d in &m.dims {
                let Some(accs) = data.get(&d.series) else { continue };
                let split = split.min(accs.len());
                let avgs = |part: &[Acc]| -> Vec<f64> { part.iter().filter_map(|a| a.value(Agg::Avg)).collect() };
                let (base, win) = (avgs(&accs[..split]), avgs(&accs[split..]));
                if base.len() < 3 || win.is_empty() {
                    continue;
                }
                examined += 1;
                let score = change_score(&base, &win);
                if score <= 0.0 {
                    continue;
                }
                let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
                items.push((score, m, d, mean(&base), mean(&win), accs));
            }
        }
        items.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| (b.4 - b.3).abs().total_cmp(&(a.4 - a.3).abs())));
        items.truncate(req.limit.unwrap_or(30).clamp(1, 500) as usize);
        let items = items
            .into_iter()
            .map(|(score, m, d, before_avg, window_avg, accs)| {
                let (spark, spark_split) = resample(accs, split, spark_n);
                CorrelateItem {
                    chart: m.id.clone(),
                    dim: d.id.clone(),
                    title: m.title.clone(),
                    name: d.name.clone(),
                    unit: m.unit,
                    score: (score * 1000.0).round() / 1000.0,
                    before_avg: tidy(before_avg),
                    window_avg: tidy(window_avg),
                    spark,
                    spark_split,
                }
            })
            .collect();
        Ok(CorrelateResp { after, before, baseline_after: base_after, examined, took_ms: started.elapsed().as_millis() as u64, items })
    }

    /// CSV: erste Spalte Zeit (ISO 8601, UTC), dann je Diagramm und Dimension eine Spalte.
    pub async fn export_csv(&self, charts: &[String], after: i64, before: i64, points: u32) -> anyhow::Result<String> {
        let (after, before) = resolve(self.now(), after, before)?;
        let metas: Vec<ChartMeta> = charts.iter().filter_map(|id| self.sh.registry.get(id)).collect();
        anyhow::ensure!(!metas.is_empty(), "Keine bekannten Diagramme");
        let points = if points == 0 { (before - after).clamp(2, 100_000) as u32 } else { points.clamp(2, 100_000) };
        let mut cols: Vec<(String, HashMap<i64, f64>)> = Vec::new();
        for m in &metas {
            let series: Vec<u32> = m.dims.iter().map(|d| d.series).collect();
            let (plan, data) = self.grid(series, after, before, points).await?;
            for d in &m.dims {
                let mut map = HashMap::new();
                if let Some(accs) = data.get(&d.series) {
                    for (i, a) in accs.iter().enumerate() {
                        if let Some(v) = a.value(Agg::Avg) {
                            map.insert(plan.start + i as i64 * i64::from(plan.step), v);
                        }
                    }
                }
                cols.push((format!("{}.{}", m.id, d.id), map));
            }
        }
        let mut times: Vec<i64> = cols.iter().flat_map(|(_, m)| m.keys().copied()).filter(|t| *t >= after - 3600 && *t <= before).collect();
        times.sort_unstable();
        times.dedup();
        let mut out = String::from("time");
        for (name, _) in &cols {
            let _ = write!(out, ",{}", name.replace([',', '"', '\n'], "_"));
        }
        out.push('\n');
        for t in times {
            out.push_str(&iso(t));
            for (_, m) in &cols {
                out.push(',');
                if let Some(v) = m.get(&t) {
                    let _ = write!(out, "{v}");
                }
            }
            out.push('\n');
        }
        Ok(out)
    }
}

/// Raster auf `n` Punkte zusammenfassen; liefert auch den ersten Punkt im Fenster.
fn resample(accs: &[Acc], split: usize, n: usize) -> (Vec<Option<f64>>, u32) {
    if n == 0 || accs.is_empty() {
        return (Vec::new(), 0);
    }
    let per = accs.len().div_ceil(n).max(1);
    let spark = accs
        .chunks(per)
        .map(|c| {
            let mut t = Acc::EMPTY;
            c.iter().for_each(|a| t.add(a));
            t.value(Agg::Avg)
        })
        .collect();
    (spark, split.div_ceil(per) as u32)
}

/// Unix-Sekunden → `2026-11-15T12:34:56Z`.
pub fn iso(ts: i64) -> String {
    let days = ts.div_euclid(86_400);
    let secs = ts.rem_euclid(86_400);
    // Tage seit 1970 → Datum (Howard Hinnant, civil_from_days).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", secs / 3600, secs / 60 % 60, secs % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso(1_763_210_096), "2025-11-15T12:34:56Z");
        assert_eq!(quantile(&[1.0, 2.0, 3.0, 4.0, 5.0], 0.5), Some(3.0));
        assert_eq!(ks(vec![1.0, 2.0, 3.0], vec![1.0, 2.0, 3.0]), 0.0);
        assert_eq!(ks(vec![1.0, 2.0], vec![5.0, 6.0]), 1.0);
        let flat = vec![10.0; 50];
        let shifted = vec![20.0; 10];
        assert!(change_score(&flat, &shifted) > 0.9);
        assert_eq!(change_score(&flat, &flat[..10]), 0.0);
        let noise: Vec<f64> = (0..50).map(|i| 50.0 + (i % 5) as f64).collect();
        assert!(change_score(&noise, &noise[..10]) < 0.3);
    }
}
