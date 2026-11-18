//! Weitere Formen neben Linien: Heatmap (viele gleichartige Reihen, z. B. Kerne), Zustandsband
//! (an/aus über die Zeit) und Balkenliste (Rangliste).

use dioxus::prelude::*;
use shared::model::Unit;

use super::{
    color,
    geom::{self, PlotData},
};
use crate::{fmt, state::use_app};

const LEVELS: usize = 8;

/// Heatmap: je Reihe eine Zeile, die Helligkeit zeigt den Wert. Gut für viele Kerne oder Sensoren.
#[component]
pub fn Heatmap(
    data: ReadSignal<PlotData>,
    #[props(default)] unit: Unit,
    /// Höchstens so hoch (Pixel); Zeilen sind 6–34 px.
    #[props(default = 180)]
    height: u32,
    /// Sichtbares Zeitfenster (für die Ausrichtung am Fadenkreuz).
    #[props(default)]
    window: Option<(i64, i64)>,
    /// Oberer Wert der Farbskala (Standard: grösster Wert; bei Prozent 100).
    #[props(default)]
    max: Option<f64>,
) -> Element {
    let app = use_app();
    let mut width = use_signal(|| 0.0f64);
    let mut hover_cell = use_signal(|| None::<(usize, usize)>);
    let grid = use_memo(move || {
        let w = *width.read();
        let cols = if w > 0.0 { (w / 5.0) as usize } else { 120 };
        geom::heat_grid(&data.read(), cols.clamp(10, 240))
    });
    let g = grid.read();
    let d = data.read();
    let rows = g.rows.max(1);
    let row_h = (f64::from(height) / rows as f64).clamp(6.0, 34.0).floor();
    let h = row_h * rows as f64;
    let w = if *width.read() > 0.0 { *width.read() } else { 1000.0 };
    let top = max.unwrap_or(if unit == Unit::Percent { 100.0f64.max(g.max) } else { g.max });
    let paths = geom::heat_paths(&g, w, row_h, top, LEVELS);
    let names: Vec<String> = d.series.iter().map(|s| s.name.clone()).collect();
    let gutter = (names.iter().map(|n| n.chars().count()).max().unwrap_or(1) as f64 * 5.9 + 10.0).clamp(24.0, 110.0);
    let show_names = row_h >= 10.0;
    let (t0, t1) = window.unwrap_or((d.start, d.end()));
    let cross = (app.hover)().filter(|t| *t >= t0 && *t <= t1 && t1 > t0).map(|t| (t - t0) as f64 / (t1 - t0) as f64 * 100.0);
    let tip = hover_cell().and_then(|(r, c)| {
        let v = g.at(r, c)?;
        let ts = d.time((c * g.per_col).min(d.len().saturating_sub(1)));
        Some((names.get(r).cloned().unwrap_or_default(), ts, v, c as f64 / g.cols.max(1) as f64, r as f64 * row_h))
    });
    let (cols, per_col, step) = (g.cols, g.per_col, d.step);

    rsx! {
        div { class: "relative select-none", style: "padding-left:{gutter:.0}px", "data-name": "Heatmap",
            if show_names {
                div { class: "pointer-events-none absolute top-0 left-0 text-right text-[10px] leading-none text-muted-foreground", style: "width:{gutter - 6.0:.0}px",
                    for (i, n) in names.iter().enumerate() {
                        span { key: "{i}", class: "absolute right-0 truncate", style: "top:{i as f64 * row_h + row_h / 2.0:.1}px;transform:translateY(-50%);max-width:100%", "{n}" }
                    }
                }
            }
            div {
                class: "relative",
                style: "height:{h:.0}px",
                onresize: move |e| {
                    if let Ok(s) = e.data().get_content_box_size() {
                        let v = s.width.round();
                        if v > 0.0 && (v - *width.peek()).abs() >= 4.0 {
                            width.set(v);
                        }
                    }
                },
                onpointermove: move |e| {
                    let p = e.data().element_coordinates();
                    let w = *width.peek();
                    if w <= 0.0 || cols == 0 {
                        return;
                    }
                    let c = ((p.x / w) * cols as f64).floor().clamp(0.0, (cols - 1) as f64) as usize;
                    let r = (p.y / row_h).floor().clamp(0.0, (rows - 1) as f64) as usize;
                    if *hover_cell.peek() != Some((r, c)) {
                        hover_cell.set(Some((r, c)));
                    }
                },
                onpointerleave: move |_| hover_cell.set(None),
                svg { class: "absolute inset-0 size-full", view_box: "0 0 {w:.0} {h:.0}", preserve_aspect_ratio: "none", role: "img", "aria-label": "Heatmap",
                    rect { width: "{w:.0}", height: "{h:.0}", fill: "var(--muted)", fill_opacity: "0.25" }
                    for (l, p) in paths.iter().enumerate() {
                        if !p.is_empty() {
                            path { key: "{l}", d: "{p}", fill: "{color(0)}", fill_opacity: "{0.1 + 0.9 * (l as f64 + 1.0) / LEVELS as f64:.2}" }
                        }
                    }
                }
                if let Some(x) = cross {
                    div { class: "pointer-events-none absolute inset-y-0 w-px bg-foreground/50", style: "left:{x:.2}%" }
                }
                if let Some((name, ts, v, frac, y)) = tip {
                    div {
                        class: "pointer-events-none absolute z-30 rounded-lg border bg-popover/95 px-2.5 py-1.5 text-xs shadow-lg backdrop-blur-sm",
                        style: if frac > 0.6 { format!("right:calc({:.1}% + 10px);top:{:.0}px", (1.0 - frac) * 100.0, y) } else { format!("left:calc({:.1}% + 10px);top:{:.0}px", frac * 100.0, y) },
                        p { class: "text-[11px] text-muted-foreground tabular-nums",
                            if per_col > 1 { "ab " }
                            "{fmt::point(ts, step)}"
                        }
                        p { class: "flex items-center gap-2",
                            span { class: "text-muted-foreground", "{name}" }
                            span { class: "font-medium tabular-nums", "{unit.format(v)}" }
                        }
                    }
                }
            }
            div { class: "mt-2 flex items-center justify-end gap-2 text-[10px] text-muted-foreground tabular-nums",
                span { "{unit.format(0.0)}" }
                div { class: "flex h-2 w-24 overflow-hidden rounded-sm",
                    for l in 0..LEVELS {
                        span { key: "{l}", class: "flex-1", style: "background:{color(0)};opacity:{0.1 + 0.9 * (l as f64 + 1.0) / LEVELS as f64:.2}" }
                    }
                }
                span { "{unit.format(top)}" }
            }
        }
    }
}

/// Zustandsband: je Reihe eine Zeile, an = Erfolg, aus = Fehler, ohne Wert = grau.
/// Mit Anteil „an“ rechts – die Farbe steht nie allein.
#[component]
pub fn StateBand(
    data: ReadSignal<PlotData>,
    #[props(default)] window: Option<(i64, i64)>,
    /// Beschriftung für „an“ / „aus“.
    #[props(into, default = "Erreichbar".to_string())]
    on_label: String,
    #[props(into, default = "Ausfall".to_string())]
    off_label: String,
) -> Element {
    let app = use_app();
    let d = data.read();
    let (t0, t1) = window.unwrap_or((d.start, d.end().max(d.start + 1)));
    let span = (t1 - t0).max(1) as f64;
    let step = i64::from(d.step.max(1));
    let pct = |t: i64| ((t - t0) as f64 / span * 100.0).clamp(0.0, 100.0);
    let cross = (app.hover)().filter(|t| *t >= t0 && *t <= t1).map(pct);
    rsx! {
        div { class: "grid gap-2", "data-name": "StateBand",
            for s in d.series.iter() {
                {
                    let runs = geom::state_runs(&s.values);
                    let up = geom::uptime(&s.values);
                    let downs = runs.iter().filter(|r| r.up == Some(false)).count();
                    rsx! {
                        div { key: "{s.id}", class: "grid gap-1",
                            div { class: "flex items-center justify-between gap-2 text-xs",
                                span { class: "truncate text-muted-foreground", "{s.name}" }
                                span { class: "shrink-0 tabular-nums",
                                    if let Some(u) = up {
                                        span { class: "font-medium", "{shared::model::trim(u * 100.0, 2)} % {on_label.to_lowercase()}" }
                                        if downs > 0 {
                                            span { class: "text-muted-foreground", " · {downs}× {off_label}" }
                                        }
                                    } else {
                                        span { class: "text-muted-foreground", "keine Werte" }
                                    }
                                }
                            }
                            div { class: "relative h-5 overflow-hidden rounded-md bg-muted/40",
                                for r in runs.iter() {
                                    {
                                        let a = pct(d.start + r.from as i64 * step);
                                        let b = pct(d.start + r.to as i64 * step);
                                        let (bg, title) = match r.up {
                                            Some(true) => ("bg-success/70", on_label.clone()),
                                            Some(false) => ("bg-destructive", off_label.clone()),
                                            None => ("bg-transparent", "Keine Werte".to_string()),
                                        };
                                        rsx! {
                                            span {
                                                key: "{r.from}",
                                                class: "absolute inset-y-0 {bg}",
                                                style: "left:{a:.3}%;width:max({b - a:.3}%, 2px)",
                                                title: "{title}: {fmt::point(d.start + r.from as i64 * step, d.step)} – {fmt::point(d.start + r.to as i64 * step, d.step)}",
                                            }
                                        }
                                    }
                                }
                                if let Some(x) = cross {
                                    span { class: "pointer-events-none absolute inset-y-0 w-px bg-foreground/60", style: "left:{x:.2}%" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Eintrag einer [`BarList`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BarItem {
    pub label: String,
    pub value: f64,
    /// Wert als Text (sonst `unit.format(value)`).
    pub display: Option<String>,
    /// Leise Zeile rechts neben dem Namen (z. B. Image, Anteil).
    pub hint: Option<String>,
    /// Farbplatz der Reihe, falls der Balken zu einer Reihe gehört; sonst neutral.
    pub slot: Option<usize>,
    /// Ziel beim Klick (Route als Text, z. B. `/containers/web`).
    pub href: Option<String>,
}

impl BarItem {
    pub fn new(label: impl Into<String>, value: f64) -> Self {
        Self { label: label.into(), value, ..Self::default() }
    }
}

/// Rangliste mit Balken: Name, Wert und Anteil am grössten (oder an `max`).
#[component]
pub fn BarList(
    items: Vec<BarItem>,
    #[props(default)] unit: Unit,
    /// Bezugswert für die Balkenlänge (Standard: grösster Wert).
    #[props(default)]
    max: Option<f64>,
    /// Höchstens so viele Zeilen.
    #[props(default = 10)]
    limit: usize,
    #[props(into, default = "Keine Einträge".to_string())]
    empty: String,
) -> Element {
    if items.is_empty() {
        return rsx! { p { class: "py-6 text-center text-xs text-muted-foreground", "{empty}" } };
    }
    let top = max.unwrap_or_else(|| items.iter().map(|i| i.value).fold(0.0, f64::max)).max(f64::MIN_POSITIVE);
    rsx! {
        ul { class: "grid gap-2.5", "data-name": "BarList",
            for (i, it) in items.into_iter().take(limit).enumerate() {
                {
                    let pct = (it.value / top * 100.0).clamp(0.0, 100.0);
                    let value = it.display.clone().unwrap_or_else(|| unit.format(it.value));
                    let fill = it.slot.map_or_else(|| "background:var(--foreground);opacity:.55".to_string(), |s| format!("background:{}", color(s)));
                    let body = rsx! {
                        div { class: "flex items-baseline justify-between gap-3 text-xs",
                            span { class: "flex min-w-0 items-center gap-1.5",
                                if let Some(s) = it.slot {
                                    span { class: "size-2 shrink-0 rounded-[2px]", style: "background:{color(s)}" }
                                }
                                span { class: "truncate font-medium", "{it.label}" }
                                if let Some(h) = it.hint.as_ref() {
                                    span { class: "truncate text-muted-foreground", "{h}" }
                                }
                            }
                            span { class: "shrink-0 tabular-nums", "{value}" }
                        }
                        div { class: "mt-1 h-1.5 overflow-hidden rounded-full bg-muted",
                            div { class: "h-full rounded-full transition-[width] duration-500", style: "width:{pct:.1}%;{fill}" }
                        }
                    };
                    rsx! {
                        li { key: "{i}-{it.label}",
                            if let Some(href) = it.href.clone() {
                                Link { to: href, class: "-mx-1.5 block rounded-md px-1.5 py-0.5 outline-none hover:bg-accent/50 focus-visible:ring-[3px] focus-visible:ring-ring/50", {body} }
                            } else {
                                {body}
                            }
                        }
                    }
                }
            }
        }
    }
}
