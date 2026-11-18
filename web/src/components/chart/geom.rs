//! Rechenteil der Diagramme: Wertebereich, Achsenteilung, Zeitachse, SVG-Pfade, Statistik, CSV,
//! Heatmap und Zustandsband. Reines Rust ohne Browser – darum hier testen
//! (`cargo test --target <host>` im Ordner `web`).
//!
//! Gezeichnet wird in echten Pixeln: [`layout`] bekommt Breite und Höhe der Fläche (gemessen) und
//! das Zeitfenster. Ohne gemessene Breite gilt [`VIEW_W`].

use std::fmt::Write;

use shared::model::{ChartKind, Unit};

/// Breite der Zeichenfläche, solange die echte Breite noch nicht gemessen ist.
pub const VIEW_W: f64 = 1000.0;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlotSeries {
    pub id: String,
    pub name: String,
    /// Farbplatz (0–7 = feste Farben, darüber neutral).
    pub slot: usize,
    /// Unter der Nulllinie zeichnen.
    pub negate: bool,
    pub values: Vec<Option<f64>>,
}

/// Reihen auf einem gemeinsamen Raster: Punkt `i` liegt bei `start + i * step`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlotData {
    pub start: i64,
    pub step: u32,
    pub series: Vec<PlotSeries>,
}

impl PlotData {
    pub fn len(&self) -> usize {
        self.series.iter().map(|s| s.values.len()).max().unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.series.iter().all(|s| s.values.iter().all(Option::is_none))
    }

    pub fn time(&self, i: usize) -> i64 {
        self.start + i as i64 * i64::from(self.step)
    }

    pub fn end(&self) -> i64 {
        self.time(self.len().saturating_sub(1))
    }

    /// Index des Punkts zu einer Zeit (wenn sie im Raster liegt).
    pub fn index_of(&self, ts: i64) -> Option<usize> {
        let n = self.len();
        if n == 0 || ts < self.start {
            return None;
        }
        let i = ((ts - self.start) as f64 / f64::from(self.step.max(1))).round() as usize;
        (i < n).then_some(i)
    }

    /// Nächster Punkt zu einer Zeit, auf das Raster geklemmt (für Fadenkreuz und Tastatur).
    pub fn nearest(&self, ts: i64) -> Option<usize> {
        let n = self.len();
        if n == 0 {
            return None;
        }
        let i = ((ts - self.start) as f64 / f64::from(self.step.max(1))).round();
        Some(i.clamp(0.0, (n - 1) as f64) as usize)
    }

    /// Summe aller Reihen am Punkt `i` (gespiegelte Reihen zählen negativ nicht mit).
    pub fn total_at(&self, i: usize) -> Option<f64> {
        let mut any = false;
        let mut sum = 0.0;
        for s in &self.series {
            if let Some(v) = s.values.get(i).copied().flatten() {
                any = true;
                sum += v;
            }
        }
        any.then_some(sum)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Shape {
    pub slot: usize,
    /// Linie (obere Kante).
    pub line: String,
    /// Fläche (leer = keine).
    pub area: String,
    /// y-Position der Linie je Punkt in Pixeln (`NaN` = kein Wert). Für die Punkte am Fadenkreuz.
    pub ys: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Geometry {
    pub y_min: f64,
    pub y_max: f64,
    /// Werte der waagrechten Hilfslinien.
    pub ticks: Vec<f64>,
    pub shapes: Vec<Shape>,
    pub height: f64,
    /// Breite der Fläche in Pixeln.
    pub width: f64,
    /// Sichtbares Zeitfenster (Unix-Sekunden).
    pub t0: i64,
    pub t1: i64,
}

impl Geometry {
    /// Wert → y in Pixeln.
    pub fn y(&self, v: f64) -> f64 {
        let span = (self.y_max - self.y_min).max(f64::MIN_POSITIVE);
        self.height - (v - self.y_min) / span * self.height
    }

    /// Zeit → x in Pixeln.
    pub fn x(&self, t: i64) -> f64 {
        let span = (self.t1 - self.t0).max(1) as f64;
        (t - self.t0) as f64 / span * self.width
    }

    /// Waagrechte Position (0..1) → Zeit.
    pub fn time_at(&self, frac: f64) -> i64 {
        self.t0 + (frac * (self.t1 - self.t0) as f64).round() as i64
    }
}

/// x-Position (0..[`VIEW_W`]) des Punkts `i` von `n`.
pub fn x_at(i: usize, n: usize) -> f64 {
    if n <= 1 { VIEW_W / 2.0 } else { i as f64 / (n - 1) as f64 * VIEW_W }
}

/// Punkt unter einer waagrechten Position (0..1).
pub fn index_at(frac: f64, n: usize) -> usize {
    if n <= 1 { 0 } else { (frac.clamp(0.0, 1.0) * (n - 1) as f64).round() as usize }
}

fn signed(v: f64, negate: bool) -> f64 {
    if negate { -v } else { v }
}

/// Wie gezeichnet wird.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub width: f64,
    pub height: f64,
    /// Zeitfenster; `None` = genau die Daten.
    pub window: Option<(i64, i64)>,
    /// Waagrechte Linien, die im Wertebereich liegen sollen (Schwellen) – nur, wenn sie nahe sind.
    pub extra: Option<f64>,
}

/// Geometrie mit der Breite [`VIEW_W`] über genau die Daten (ältere Schnittstelle).
pub fn geometry(data: &PlotData, kind: ChartKind, unit: Unit, height: f64) -> Geometry {
    layout(data, kind, unit, Frame { width: VIEW_W, height, window: None, extra: None })
}

/// Wertebereich (unten, oben) der Daten, wie er gezeichnet wird – vor dem Runden.
fn value_range(data: &PlotData, kind: ChartKind, unit: Unit) -> (f64, f64) {
    let n = data.len();
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    if kind == ChartKind::Stacked {
        for i in 0..n {
            let (mut pos, mut neg, mut any) = (0.0, 0.0, false);
            for s in &data.series {
                if let Some(v) = s.values.get(i).copied().flatten() {
                    any = true;
                    if s.negate { neg += v.max(0.0) } else { pos += v.max(0.0) }
                }
            }
            if any {
                hi = hi.max(pos);
                lo = lo.min(-neg);
            }
        }
    } else {
        for s in &data.series {
            for v in s.values.iter().flatten() {
                let v = signed(*v, s.negate);
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
    }
    if !lo.is_finite() || !hi.is_finite() {
        (lo, hi) = (0.0, 1.0);
    }
    // Flächen und gestapelte Diagramme stehen auf der Nulllinie; Linien nur, wenn sie ihr nahe sind.
    if kind != ChartKind::Line || (lo >= 0.0 && lo < hi * 0.6) {
        lo = lo.min(0.0);
    }
    if kind != ChartKind::Line || hi < 0.0 {
        hi = hi.max(0.0);
    }
    if unit == Unit::Percent && kind == ChartKind::Stacked && lo >= 0.0 && hi <= 100.0 && hi > 40.0 {
        hi = 100.0;
    }
    if unit == Unit::State {
        (lo, hi) = (0.0, hi.max(1.0));
    }
    if (hi - lo).abs() < 1e-9 {
        let pad = if hi.abs() < 1e-9 { 1.0 } else { hi.abs() * 0.1 };
        hi += pad;
        if kind == ChartKind::Line && lo != 0.0 {
            lo -= pad;
        }
    }
    (lo, hi)
}

/// Geometrie für eine Fläche von `frame.width` × `frame.height` Pixeln.
#[allow(clippy::needless_range_loop)] // mehrere Reihen am selben Index
pub fn layout(data: &PlotData, kind: ChartKind, unit: Unit, frame: Frame) -> Geometry {
    let n = data.len();
    let (mut lo, mut hi) = value_range(data, kind, unit);
    if let Some(x) = frame.extra.filter(|x| x.is_finite()) {
        // Schwelle einbeziehen, wenn sie den Bereich höchstens verdoppelt.
        if x > hi && x <= hi + (hi - lo).max(f64::MIN_POSITIVE) {
            hi = x;
        } else if x < lo && x >= lo - (hi - lo) {
            lo = x;
        }
    }
    let target = (frame.height / 42.0).round().clamp(2.0, 6.0) as usize;
    let (y_min, y_max, ticks) = nice_n(lo, hi, unit, target);
    let (t0, t1) = frame.window.filter(|(a, b)| b > a).unwrap_or_else(|| {
        let (a, b) = (data.start, data.end());
        if b > a { (a, b) } else { (a - 1, a + 1) }
    });
    let mut geo = Geometry {
        y_min,
        y_max,
        ticks,
        shapes: Vec::with_capacity(data.series.len()),
        height: frame.height,
        width: frame.width.max(1.0),
        t0,
        t1,
    };
    if n == 0 {
        return geo;
    }
    let xs: Vec<f64> = (0..n).map(|i| geo.x(data.time(i))).collect();
    let pt = |out: &mut String, first: bool, x: f64, y: f64| {
        let _ = write!(out, "{}{:.1} {:.1}", if first { "M" } else { "L" }, x, y);
    };

    match kind {
        ChartKind::Stacked => {
            let present: Vec<bool> = (0..n).map(|i| data.series.iter().any(|s| s.values.get(i).copied().flatten().is_some())).collect();
            let (mut pos, mut neg) = (vec![0.0f64; n], vec![0.0f64; n]);
            for s in &data.series {
                let acc = if s.negate { &mut neg } else { &mut pos };
                let sign = if s.negate { -1.0 } else { 1.0 };
                let bottom: Vec<f64> = acc.iter().map(|a| sign * a).collect();
                for (i, a) in acc.iter_mut().enumerate() {
                    *a += s.values.get(i).copied().flatten().unwrap_or(0.0).max(0.0);
                }
                let top: Vec<f64> = acc.iter().map(|a| sign * a).collect();
                let mut shape = Shape { slot: s.slot, ys: vec![f32::NAN; n], ..Shape::default() };
                for (from, to) in runs(&present) {
                    for i in from..to {
                        let y = geo.y(top[i]);
                        pt(&mut shape.line, i == from, xs[i], y);
                        pt(&mut shape.area, i == from, xs[i], y);
                        if s.values.get(i).copied().flatten().is_some() {
                            shape.ys[i] = y as f32;
                        }
                    }
                    if to - from == 1 {
                        shape.line.push_str("h0.01");
                    }
                    for i in (from..to).rev() {
                        let _ = write!(shape.area, "L{:.1} {:.1}", xs[i], geo.y(bottom[i]));
                    }
                    shape.area.push('Z');
                }
                geo.shapes.push(shape);
            }
        }
        ChartKind::Line | ChartKind::Area => {
            let zero = geo.y(0.0f64.clamp(y_min, y_max));
            for s in &data.series {
                let present: Vec<bool> = (0..n).map(|i| s.values.get(i).copied().flatten().is_some()).collect();
                let mut shape = Shape { slot: s.slot, ys: vec![f32::NAN; n], ..Shape::default() };
                for (from, to) in runs(&present) {
                    let mut segment = String::new();
                    for i in from..to {
                        let y = geo.y(signed(s.values[i].unwrap_or(0.0), s.negate));
                        pt(&mut segment, i == from, xs[i], y);
                        shape.ys[i] = y as f32;
                    }
                    shape.line.push_str(&segment);
                    if to - from == 1 {
                        shape.line.push_str("h0.01");
                    }
                    if kind == ChartKind::Area {
                        let _ = write!(shape.area, "{segment}L{:.1} {zero:.1}L{:.1} {zero:.1}Z", xs[to - 1], xs[from]);
                    }
                }
                geo.shapes.push(shape);
            }
        }
    }
    geo
}

/// Zusammenhängende Abschnitte `[from, to)` mit Werten.
fn runs(present: &[bool]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, p) in present.iter().enumerate() {
        match (p, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                out.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, present.len()));
    }
    out
}

/// Runder Wertebereich mit etwa vier Abschnitten. Byte-Einheiten teilen in Zweierpotenzen.
pub fn nice(lo: f64, hi: f64, unit: Unit) -> (f64, f64, Vec<f64>) {
    nice_n(lo, hi, unit, 4)
}

/// Runder Wertebereich mit etwa `target` Abschnitten.
pub fn nice_n(lo: f64, hi: f64, unit: Unit, target: usize) -> (f64, f64, Vec<f64>) {
    let span = (hi - lo).max(f64::MIN_POSITIVE);
    let raw = span / target.max(1) as f64;
    let binary = matches!(unit, Unit::Bytes | Unit::BytesPerSec) && raw >= 1.0;
    let step = if binary {
        let base = 1024f64.powf(raw.log(1024.0).floor());
        let m = raw / base;
        const STEPS: [f64; 18] = [1.0, 2.0, 4.0, 5.0, 8.0, 10.0, 16.0, 20.0, 32.0, 50.0, 64.0, 100.0, 128.0, 200.0, 256.0, 500.0, 512.0, 1024.0];
        STEPS.iter().copied().find(|s| *s >= m - 1e-9).unwrap_or(1024.0) * base
    } else if unit == Unit::State {
        1.0
    } else {
        let base = 10f64.powf(raw.log10().floor());
        let m = raw / base;
        [1.0, 2.0, 2.5, 5.0, 10.0].into_iter().find(|s| *s >= m - 1e-9).unwrap_or(10.0) * base
    };
    let y_min = (lo / step + 1e-9).floor() * step;
    let y_max = (hi / step - 1e-9).ceil() * step;
    let y_max = if y_max <= y_min { y_min + step } else { y_max };
    let count = ((y_max - y_min) / step).round() as usize;
    let ticks = (0..=count).map(|i| y_min + i as f64 * step).collect();
    (y_min, y_max, ticks)
}

/// Mögliche Abstände der Zeitachse in Sekunden.
const TIME_STEPS: &[i64] = &[
    1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600, 7200, 10_800, 21_600, 43_200, 86_400, 172_800, 604_800,
    1_209_600, 2_592_000,
];

/// Runde Zeitpunkte für die Zeitachse: so viele, dass je Beschriftung `label_px` Platz bleibt.
/// `tz` = Abstand der Ortszeit zu UTC in Sekunden (Tage beginnen um Mitternacht Ortszeit).
pub fn time_ticks(t0: i64, t1: i64, width: f64, label_px: f64, tz: i64) -> (Vec<i64>, i64) {
    let span = (t1 - t0).max(1);
    let max = (width / label_px.max(1.0)).floor().max(1.0) as i64;
    let step = TIME_STEPS.iter().copied().find(|s| span / s <= max).unwrap_or(*TIME_STEPS.last().unwrap_or(&86_400));
    let first = (t0 + tz).div_euclid(step) * step - tz;
    let first = if first < t0 { first + step } else { first };
    let mut out = Vec::new();
    let mut t = first;
    while t <= t1 && out.len() < 64 {
        out.push(t);
        t += step;
    }
    (out, step)
}

/// Wie ein Zeitpunkt der Achse beschriftet wird.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickLabel {
    /// `14:03:27`
    Clock,
    /// `14:03`
    Hm,
    /// `16.11.`
    Day,
}

/// Beschriftung eines Zeitpunkts der Achse: Mitternacht zeigt den Tag.
pub fn tick_label(t: i64, step: i64, tz: i64) -> TickLabel {
    if step >= 86_400 || (t + tz).rem_euclid(86_400) == 0 {
        TickLabel::Day
    } else if step < 60 {
        TickLabel::Clock
    } else {
        TickLabel::Hm
    }
}

/// Wie viele Punkte ein Diagramm anfordert: im Live-Fenster jeder Messwert (bis ~1000), sonst passend
/// zur Breite in wenigen Stufen (gleiche Stufe = gemeinsame Abfrage).
pub fn points_for(span: i64, interval: u32, width: f64) -> u32 {
    let own = i64::from(interval.max(1));
    let full = span / own + 1;
    if full <= 1000 {
        return full.max(2) as u32;
    }
    if width <= 480.0 {
        180
    } else if width <= 1100.0 {
        360
    } else {
        720
    }
}

/// Zeitfenster um `frac` (0..1) herum um `factor` vergrössern (> 1) oder verkleinern (< 1).
pub fn zoom_window(t0: i64, t1: i64, frac: f64, factor: f64, min_span: i64) -> (i64, i64) {
    let span = (t1 - t0) as f64;
    let new = (span * factor).max(min_span as f64);
    let center = t0 as f64 + span * frac.clamp(0.0, 1.0);
    let a = center - new * frac.clamp(0.0, 1.0);
    (a.round() as i64, (a + new).round() as i64)
}

/// Zeitfenster um einen Bruchteil seiner Länge verschieben (positiv = in die Zukunft), nie über `now`.
pub fn pan_window(t0: i64, t1: i64, frac: f64, now: i64) -> (i64, i64) {
    let span = t1 - t0;
    let shift = (span as f64 * frac).round() as i64;
    let b = (t1 + shift).min(now);
    (b - span, b)
}

/// Letzter vorhandener Wert einer Reihe.
pub fn latest(values: &[Option<f64>]) -> Option<f64> {
    values.iter().rev().take(5).find_map(|v| *v)
}

/// (Minimum, Mittel, Maximum) der vorhandenen Werte.
pub fn stats(values: &[Option<f64>]) -> Option<(f64, f64, f64)> {
    let mut it = values.iter().flatten().copied();
    let first = it.next()?;
    let (mut lo, mut hi, mut sum, mut n) = (first, first, first, 1usize);
    for v in it {
        lo = lo.min(v);
        hi = hi.max(v);
        sum += v;
        n += 1;
    }
    Some((lo, sum / n as f64, hi))
}

/// Kennzahlen einer Reihe im sichtbaren Zeitraum.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Summary {
    pub min: f64,
    pub avg: f64,
    pub max: f64,
    pub last: f64,
    /// Anteil der Punkte mit Wert (0..1).
    pub coverage: f64,
}

pub fn summary(values: &[Option<f64>]) -> Option<Summary> {
    let (min, avg, max) = stats(values)?;
    let have = values.iter().filter(|v| v.is_some()).count();
    Some(Summary { min, avg, max, last: latest(values).unwrap_or(f64::NAN), coverage: have as f64 / values.len().max(1) as f64 })
}

/// Daten als CSV (Trennzeichen `;`, Dezimalpunkt): erste Spalte die Zeit, dann je Reihe eine.
pub fn to_csv(data: &PlotData, time: impl Fn(i64) -> String) -> String {
    let mut out = String::from("Zeit");
    for s in &data.series {
        out.push(';');
        out.push_str(&csv_cell(&s.name));
    }
    out.push('\n');
    for i in 0..data.len() {
        out.push_str(&time(data.time(i)));
        for s in &data.series {
            out.push(';');
            if let Some(v) = s.values.get(i).copied().flatten() {
                let _ = write!(out, "{v}");
            }
        }
        out.push('\n');
    }
    out
}

fn csv_cell(s: &str) -> String {
    if s.contains([';', '"', '\n']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

/// Raster der Heatmap: je Reihe eine Zeile, `cols` Spalten (Mittel der Punkte darin).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HeatGrid {
    pub rows: usize,
    pub cols: usize,
    /// Zeilenweise, `None` = kein Wert.
    pub cells: Vec<Option<f64>>,
    pub max: f64,
    /// Punkte je Spalte.
    pub per_col: usize,
}

impl HeatGrid {
    pub fn at(&self, row: usize, col: usize) -> Option<f64> {
        self.cells.get(row * self.cols + col).copied().flatten()
    }
}

pub fn heat_grid(data: &PlotData, max_cols: usize) -> HeatGrid {
    let n = data.len();
    let rows = data.series.len();
    if n == 0 || rows == 0 {
        return HeatGrid::default();
    }
    let per_col = n.div_ceil(max_cols.max(1)).max(1);
    let cols = n.div_ceil(per_col);
    let mut cells = Vec::with_capacity(rows * cols);
    let mut max = 0.0f64;
    for s in &data.series {
        for c in 0..cols {
            let from = c * per_col;
            let to = (from + per_col).min(n);
            let (mut sum, mut k) = (0.0, 0usize);
            for v in s.values.get(from..to.min(s.values.len())).unwrap_or(&[]).iter().flatten() {
                sum += v;
                k += 1;
            }
            let v = (k > 0).then(|| sum / k as f64);
            if let Some(v) = v {
                max = max.max(v);
            }
            cells.push(v);
        }
    }
    HeatGrid { rows, cols, cells, max, per_col }
}

/// Stufe 0..levels einer Zelle (0 = sehr wenig, aber vorhanden).
pub fn heat_level(v: f64, max: f64, levels: usize) -> usize {
    if max <= 0.0 || levels == 0 {
        return 0;
    }
    (((v / max).clamp(0.0, 1.0) * levels as f64).ceil() as usize).clamp(1, levels) - 1
}

/// Je Stufe ein Pfad aus Rechtecken: Zellen `cell_w` × `row_h` Pixel, kleine Fuge dazwischen.
pub fn heat_paths(grid: &HeatGrid, width: f64, row_h: f64, max: f64, levels: usize) -> Vec<String> {
    let mut out = vec![String::new(); levels];
    if grid.cols == 0 {
        return out;
    }
    let cell_w = width / grid.cols as f64;
    let gap_x = if cell_w > 4.0 { 1.0 } else { 0.0 };
    let gap_y = if row_h > 6.0 { 1.0 } else { 0.0 };
    for r in 0..grid.rows {
        for c in 0..grid.cols {
            if let Some(v) = grid.at(r, c) {
                let l = heat_level(v, max, levels);
                let _ = write!(
                    out[l],
                    "M{:.1} {:.1}h{:.1}v{:.1}h{:.1}Z",
                    c as f64 * cell_w,
                    r as f64 * row_h,
                    (cell_w - gap_x).max(0.5),
                    (row_h - gap_y).max(0.5),
                    -(cell_w - gap_x).max(0.5)
                );
            }
        }
    }
    out
}

/// Abschnitt eines Zustandsbands: `[from, to)` mit gleichem Zustand (`None` = kein Wert).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateRun {
    pub from: usize,
    pub to: usize,
    pub up: Option<bool>,
}

/// Zustandsband: zusammenhängende Abschnitte „an“ (> 0.5), „aus“, „unbekannt“.
pub fn state_runs(values: &[Option<f64>]) -> Vec<StateRun> {
    let mut out: Vec<StateRun> = Vec::new();
    for (i, v) in values.iter().enumerate() {
        let up = v.map(|v| v > 0.5);
        match out.last_mut() {
            Some(r) if r.up == up => r.to = i + 1,
            _ => out.push(StateRun { from: i, to: i + 1, up }),
        }
    }
    out
}

/// Anteil „an“ unter den Punkten mit Wert (0..1).
pub fn uptime(values: &[Option<f64>]) -> Option<f64> {
    let have: Vec<f64> = values.iter().flatten().copied().collect();
    (!have.is_empty()).then(|| have.iter().filter(|v| **v > 0.5).count() as f64 / have.len() as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series(values: &[Option<f64>], negate: bool) -> PlotSeries {
        PlotSeries { id: "a".into(), name: "A".into(), slot: 0, negate, values: values.to_vec() }
    }

    #[test]
    fn nice_ranges() {
        let (lo, hi, ticks) = nice(0.0, 87.0, Unit::Percent);
        assert_eq!((lo, hi), (0.0, 100.0));
        assert_eq!(ticks, vec![0.0, 25.0, 50.0, 75.0, 100.0]);
        let (lo, hi, ticks) = nice(0.0, 3.0 * 1024.0 * 1024.0 * 1024.0, Unit::Bytes);
        assert_eq!(lo, 0.0);
        assert_eq!(hi, 3.0 * 1024.0 * 1024.0 * 1024.0);
        assert_eq!(ticks.len(), 4);
        let (lo, hi, _) = nice(41.2, 47.9, Unit::Celsius);
        assert_eq!((lo, hi), (40.0, 48.0));
        let (_, _, ticks) = nice_n(0.0, 1.0, Unit::State, 4);
        assert_eq!(ticks, vec![0.0, 1.0]);
    }

    #[test]
    fn lines_break_at_gaps() {
        let data = PlotData { start: 0, step: 1, series: vec![series(&[Some(1.0), Some(2.0), None, Some(3.0)], false)] };
        let geo = geometry(&data, ChartKind::Line, Unit::Count, 100.0);
        assert_eq!(geo.shapes[0].line.matches('M').count(), 2);
        assert!(geo.shapes[0].area.is_empty());
        assert_eq!(geo.y_min, 0.0);
        assert!(geo.shapes[0].ys[2].is_nan());
        assert!(!geo.shapes[0].ys[3].is_nan());
    }

    #[test]
    fn stacked_sums_and_mirrors() {
        let data = PlotData {
            start: 0,
            step: 1,
            series: vec![series(&[Some(10.0), Some(20.0)], false), series(&[Some(30.0), Some(40.0)], false), series(&[Some(5.0), Some(5.0)], true)],
        };
        let geo = geometry(&data, ChartKind::Stacked, Unit::Count, 100.0);
        assert!(geo.y_max >= 60.0);
        assert!(geo.y_min <= -5.0);
        assert_eq!(geo.shapes.len(), 3);
        assert!(geo.shapes.iter().all(|s| s.area.ends_with('Z')));
        // Die zweite Reihe liegt oben auf der ersten.
        assert!(geo.shapes[1].ys[1] < geo.shapes[0].ys[1]);
        assert_eq!(data.total_at(1), Some(65.0));
    }

    #[test]
    fn layout_uses_time_window() {
        let data = PlotData { start: 100, step: 10, series: vec![series(&[Some(0.5), Some(1.0), Some(1.5)], false)] };
        let geo = layout(&data, ChartKind::Line, Unit::Count, Frame { width: 200.0, height: 100.0, window: Some((80, 120)), extra: None });
        assert_eq!(geo.x(100), 100.0);
        assert_eq!(geo.x(120), 200.0);
        assert!(geo.shapes[0].line.starts_with("M100.0"));
        assert_eq!(geo.time_at(0.5), 100);
        // Schwelle nahe dem Bereich wird einbezogen, weit weg nicht.
        let near = layout(&data, ChartKind::Line, Unit::Count, Frame { width: 200.0, height: 100.0, window: None, extra: Some(1.8) });
        assert!(near.y_max >= 1.8);
        let far = layout(&data, ChartKind::Line, Unit::Count, Frame { width: 200.0, height: 100.0, window: None, extra: Some(50.0) });
        assert!(far.y_max < 50.0);
    }

    #[test]
    fn indices() {
        let data = PlotData { start: 100, step: 10, series: vec![series(&[Some(1.0); 5], false)] };
        assert_eq!(data.index_of(120), Some(2));
        assert_eq!(data.index_of(99), None);
        assert_eq!(data.index_of(143), Some(4), "nahe am letzten Punkt zählt noch");
        assert_eq!(data.index_of(146), None);
        assert_eq!(data.nearest(500), Some(4));
        assert_eq!(data.nearest(0), Some(0));
        assert_eq!(index_at(0.5, 5), 2);
        assert_eq!(data.end(), 140);
    }

    #[test]
    fn time_axis() {
        // 15 Minuten auf 600 px mit 70 px je Beschriftung → 2-Minuten-Schritte, rund.
        let (ticks, step) = time_ticks(1_000_030, 1_000_930, 600.0, 70.0, 0);
        assert_eq!(step, 120);
        assert!(ticks.iter().all(|t| t % 120 == 0));
        assert!(ticks.len() <= 8);
        // Tage beginnen um Mitternacht Ortszeit (UTC+1).
        let (ticks, step) = time_ticks(0, 7 * 86_400, 400.0, 70.0, 3600);
        assert_eq!(step, 172_800);
        assert!(ticks.iter().all(|t| (t + 3600).rem_euclid(86_400) == 0));
        assert_eq!(tick_label(86_400 - 3600, 3600, 3600), TickLabel::Day);
        assert_eq!(tick_label(30, 10, 0), TickLabel::Clock);
        assert_eq!(tick_label(120, 60, 0), TickLabel::Hm);
    }

    #[test]
    fn points_and_windows() {
        assert_eq!(points_for(900, 1, 600.0), 901);
        assert_eq!(points_for(86_400, 1, 600.0), 360);
        assert_eq!(points_for(86_400, 1, 1400.0), 720);
        assert_eq!(zoom_window(0, 100, 0.5, 0.5, 2), (25, 75));
        assert_eq!(zoom_window(0, 100, 0.0, 2.0, 2), (0, 200));
        assert_eq!(pan_window(0, 100, 0.5, 120), (20, 120));
        assert_eq!(pan_window(0, 100, -0.5, 120), (-50, 50));
    }

    #[test]
    fn summaries_and_csv() {
        let s = summary(&[Some(1.0), None, Some(3.0)]).unwrap();
        assert_eq!((s.min, s.avg, s.max, s.last), (1.0, 2.0, 3.0, 3.0));
        assert!((s.coverage - 2.0 / 3.0).abs() < 1e-9);
        let data = PlotData {
            start: 10,
            step: 5,
            series: vec![PlotSeries { name: "a;b".into(), values: vec![Some(1.5), None], ..PlotSeries::default() }],
        };
        assert_eq!(to_csv(&data, |t| t.to_string()), "Zeit;\"a;b\"\n10;1.5\n15;\n");
    }

    #[test]
    fn heatmap_and_states() {
        let data = PlotData { start: 0, step: 1, series: vec![series(&[Some(0.0), Some(10.0), Some(20.0), None], false), series(&[None; 4], false)] };
        let g = heat_grid(&data, 2);
        assert_eq!((g.rows, g.cols, g.per_col), (2, 2, 2));
        assert_eq!(g.at(0, 0), Some(5.0));
        assert_eq!(g.at(0, 1), Some(20.0));
        assert_eq!(g.at(1, 0), None);
        assert_eq!(heat_level(20.0, 20.0, 5), 4);
        assert_eq!(heat_level(0.1, 20.0, 5), 0);
        let paths = heat_paths(&g, 100.0, 10.0, g.max, 5);
        assert_eq!(paths.iter().filter(|p| !p.is_empty()).count(), 2);
        let runs = state_runs(&[Some(1.0), Some(1.0), Some(0.0), None]);
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0], StateRun { from: 0, to: 2, up: Some(true) });
        assert_eq!(uptime(&[Some(1.0), Some(0.0), None]), Some(0.5));
    }
}
