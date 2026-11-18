//! Kleine Anzeigen neben den Diagrammen: Kennzahl-Kachel, Füllstand, Zustandspunkt.

use dioxus::prelude::*;
use shared::model::Unit;
use tw_merge::tw_merge;

use shared::query::ChartData;

use crate::{
    components::chart::{data, geom, use_chart},
    live::use_tick,
    state::{TimeRange, use_app},
};

/// Neueste Werte eines Diagramms (Reihenfolge der Dimensionen) – `None`, solange keine da sind.
/// Bis der erste Wert über den Live-Kanal kommt (bei seltenen Diagrammen bis zu einigen Sekunden),
/// gilt der neueste gespeicherte Punkt.
pub fn use_latest(chart_id: &str) -> Memo<Option<Vec<Option<f64>>>> {
    let tick = use_tick(chart_id);
    let app = use_app();
    let mut stored = use_signal(|| None::<Vec<Option<f64>>>);
    let id = use_hook(|| chart_id.to_string());
    use_future(move || {
        let id = id.clone();
        async move {
            let every = app.chart(&id).map_or(1, |m| i64::from(m.interval.max(1)));
            let span = (every * 6).max(30);
            if let Some(d) = data::fetch(&id, TimeRange::live(span), (span / every + 1) as u32).await {
                let last: Vec<Option<f64>> = d.values.iter().map(|col| geom::latest(col)).collect();
                if last.iter().any(Option::is_some) {
                    stored.set(Some(last));
                }
            }
        }
    });
    use_memo(move || tick.read().as_ref().map(|(_, v)| (**v).clone()).or_else(|| stored.read().clone()))
}

/// Kennzahl: Beschriftung, grosser Wert, optional eine Zeile darunter.
#[component]
pub fn StatTile(
    #[props(into)] label: String,
    #[props(into)] value: String,
    #[props(into, optional)] hint: Option<String>,
    #[props(optional)] icon: Option<Element>,
    #[props(into, optional)] class: Option<String>,
    #[props(optional)] children: Option<Element>,
    /// Zustand als Punkt neben der Beschriftung (mit `hint` als Text).
    #[props(optional)]
    tone: Option<Tone>,
    /// Ziel beim Klick (Route als Text).
    #[props(into, optional)]
    href: Option<String>,
) -> Element {
    let base = if href.is_some() {
        "block rounded-xl border bg-card p-4 shadow-sm outline-none transition-colors hover:bg-accent/30 focus-visible:ring-[3px] focus-visible:ring-ring/50"
    } else {
        "rounded-xl border bg-card p-4 shadow-sm"
    };
    let class = tw_merge!(base, class.as_deref().unwrap_or(""));
    let dot = tone.map(|t| match t {
        Tone::Neutral => "bg-muted-foreground",
        Tone::Good => "bg-success",
        Tone::Warn => "bg-warning",
        Tone::Bad => "bg-destructive",
    });
    let body = rsx! {
        div { class: "flex items-center justify-between gap-2 text-xs text-muted-foreground [&_svg]:size-4",
            span { class: "flex min-w-0 items-center gap-1.5",
                if let Some(d) = dot {
                    span { class: "size-1.5 shrink-0 rounded-full {d}" }
                }
                span { class: "truncate", "{label}" }
            }
            if let Some(i) = icon {
                {i}
            }
        }
        p { class: "mt-1.5 truncate text-2xl font-semibold tracking-tight tabular-nums", "{value}" }
        if let Some(h) = hint {
            p { class: "mt-0.5 truncate text-xs text-muted-foreground", "{h}" }
        }
        if let Some(c) = children {
            div { class: "mt-3", {c} }
        }
    };
    match href {
        Some(to) => rsx! { Link { to, class: "{class}", "data-name": "StatTile", {body} } },
        None => rsx! { div { class: "{class}", "data-name": "StatTile", {body} } },
    }
}

/// Füllstand als Ring (0–100 %), Wert in der Mitte. Farbe zeigt den Zustand (ab `warn`/`crit`).
#[component]
pub fn Ring(
    /// 0–100.
    value: f64,
    #[props(default = 56)] size: u32,
    #[props(default = 80.0)] warn: f64,
    #[props(default = 92.0)] crit: f64,
    /// Text in der Mitte (Standard: Prozent).
    #[props(into, optional)]
    label: Option<String>,
) -> Element {
    let v = if value.is_finite() { value.clamp(0.0, 100.0) } else { 0.0 };
    let stroke = if v >= crit {
        "var(--destructive)"
    } else if v >= warn {
        "var(--warning)"
    } else {
        "var(--foreground)"
    };
    let r = 15.5;
    let c = 2.0 * std::f64::consts::PI * r;
    let dash = c * v / 100.0;
    let text = label.unwrap_or_else(|| format!("{v:.0} %"));
    rsx! {
        div { class: "relative inline-grid shrink-0 place-items-center", style: "width:{size}px;height:{size}px", role: "meter", "aria-valuenow": "{v:.0}", "aria-valuemin": "0", "aria-valuemax": "100",
            svg { view_box: "0 0 36 36", class: "absolute inset-0 size-full -rotate-90",
                circle { cx: "18", cy: "18", r: "{r}", fill: "none", stroke: "var(--muted)", stroke_width: "3.2" }
                circle {
                    cx: "18",
                    cy: "18",
                    r: "{r}",
                    fill: "none",
                    stroke: "{stroke}",
                    stroke_opacity: if v < warn { "0.75" } else { "1" },
                    stroke_width: "3.2",
                    stroke_linecap: "round",
                    stroke_dasharray: "{dash:.2} {c:.2}",
                    style: "transition: stroke-dasharray 500ms ease",
                }
            }
            span { class: "relative text-[11px] font-semibold tabular-nums", "{text}" }
        }
    }
}

/// Füllstand 0–100 %. Die Farbe zeigt den Zustand: ab `warn` Warnung, ab `crit` Fehler.
#[component]
pub fn Meter(
    /// 0–100.
    value: f64,
    #[props(default = 80.0)] warn: f64,
    #[props(default = 92.0)] crit: f64,
    #[props(into, optional)] class: Option<String>,
) -> Element {
    let v = value.clamp(0.0, 100.0);
    let tone = if v >= crit {
        "bg-destructive"
    } else if v >= warn {
        "bg-warning"
    } else {
        "bg-foreground/70"
    };
    let class = tw_merge!("h-1.5 w-full overflow-hidden rounded-full bg-muted", class.as_deref().unwrap_or(""));
    rsx! {
        div { class: "{class}", role: "meter", "aria-valuenow": "{v:.0}", "aria-valuemin": "0", "aria-valuemax": "100",
            div { class: "h-full rounded-full transition-[width] duration-500 {tone}", style: "width:{v:.1}%" }
        }
    }
}

/// Zustand als Punkt mit Text (nie nur Farbe).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tone {
    #[default]
    Neutral,
    Good,
    Warn,
    Bad,
}

#[component]
pub fn StatusDot(tone: Tone, #[props(into)] label: String) -> Element {
    let color = match tone {
        Tone::Neutral => "bg-muted-foreground",
        Tone::Good => "bg-success",
        Tone::Warn => "bg-warning",
        Tone::Bad => "bg-destructive",
    };
    rsx! {
        span { class: "inline-flex items-center gap-1.5 text-xs",
            span { class: "size-2 shrink-0 rounded-full {color}" }
            span { "{label}" }
        }
    }
}

/// Kleine Verlaufslinie ohne Achsen (für Kacheln und Tabellenzeilen).
#[component]
pub fn Sparkline(
    values: Vec<Option<f64>>,
    #[props(default = 32)] height: u32,
    #[props(default)] slot: usize,
    /// Zarte Fläche unter der Linie.
    #[props(default)]
    area: bool,
    /// Feste Untergrenze (Standard: kleinster Wert, bei Fläche 0).
    #[props(default)]
    min: Option<f64>,
    /// Feste Obergrenze (Standard: grösster Wert).
    #[props(default)]
    max: Option<f64>,
) -> Element {
    let n = values.len();
    let Some((lo, _, hi)) = geom::stats(&values) else {
        return rsx! { div { style: "height:{height}px" } };
    };
    let lo = min.unwrap_or(if area { lo.min(0.0) } else { lo });
    let hi = max.unwrap_or(hi).max(lo + f64::MIN_POSITIVE);
    let h = f64::from(height);
    let span = (hi - lo).max(f64::MIN_POSITIVE);
    let mut d = String::new();
    let mut fill = String::new();
    let mut run_start: Option<f64> = None;
    let mut last_x = 0.0;
    let base = h - 1.0;
    for (i, v) in values.iter().enumerate() {
        match v {
            Some(v) => {
                let x = geom::x_at(i, n);
                let y = h - 2.0 - ((v - lo) / span).clamp(0.0, 1.0) * (h - 4.0);
                let cmd = if run_start.is_none() { "M" } else { "L" };
                d.push_str(&format!("{cmd}{x:.1} {y:.1}"));
                if area {
                    fill.push_str(&format!("{cmd}{x:.1} {y:.1}"));
                }
                if run_start.is_none() {
                    run_start = Some(x);
                }
                last_x = x;
            }
            None => {
                if let (true, Some(x0)) = (area, run_start.take()) {
                    fill.push_str(&format!("L{last_x:.1} {base:.1}L{x0:.1} {base:.1}Z"));
                }
                run_start = None;
            }
        }
    }
    if let (true, Some(x0)) = (area, run_start) {
        fill.push_str(&format!("L{last_x:.1} {base:.1}L{x0:.1} {base:.1}Z"));
    }
    let c = crate::components::chart::color(slot);
    rsx! {
        svg { class: "nrz-plot block w-full", style: "height:{height}px", view_box: "0 0 {geom::VIEW_W} {height}", preserve_aspect_ratio: "none", "aria-hidden": "true",
            if area {
                path { d: "{fill}", fill: "{c}", fill_opacity: "0.14", stroke: "none" }
            }
            path { d: "{d}", fill: "none", stroke: "{c}", stroke_width: "1.5", stroke_linejoin: "round", stroke_linecap: "round" }
        }
    }
}

/// Eine Reihe aus einem Diagramm berechnen (für Mini-Verläufe und Kennzahlen): `f(daten, i)` je Punkt.
/// Folgt dem Zeitraum der Seite und läuft live mit.
pub fn use_series(chart_id: &str, f: fn(&ChartData, usize) -> Option<f64>) -> Memo<Vec<Option<f64>>> {
    let data = use_chart(chart_id);
    use_memo(move || data.read().as_ref().map(|d| (0..d.len()).map(|i| f(d, i)).collect()).unwrap_or_default())
}

/// Wert einer Dimension (nach ID) am Punkt `i`.
pub fn dim(d: &ChartData, id: &str, i: usize) -> Option<f64> {
    let k = d.dims.iter().position(|x| x == id)?;
    d.values.get(k)?.get(i).copied().flatten()
}

/// Summe aller Dimensionen am Punkt `i` (`None`, wenn keine einen Wert hat).
pub fn sum(d: &ChartData, i: usize) -> Option<f64> {
    let mut any = false;
    let mut s = 0.0;
    for col in &d.values {
        if let Some(v) = col.get(i).copied().flatten() {
            any = true;
            s += v;
        }
    }
    any.then_some(s)
}

/// Grösster Wert aller Dimensionen am Punkt `i`.
pub fn max_of(d: &ChartData, i: usize) -> Option<f64> {
    d.values.iter().filter_map(|c| c.get(i).copied().flatten()).fold(None, |a, v| Some(a.map_or(v, |a: f64| a.max(v))))
}

/// Wert mit Einheit oder ein Strich, wenn er fehlt.
pub fn show(unit: Unit, v: Option<f64>) -> String {
    v.map_or_else(|| "–".to_string(), |v| unit.format(v))
}
