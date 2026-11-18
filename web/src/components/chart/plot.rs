//! Zeichenfläche: Linien, Flächen und gestapelte Flächen als SVG in echten Pixeln (Breite gemessen),
//! mit runder Zeitachse, y-Achse nach Einheit, Schwellen, Ereignis-Marken, Fadenkreuz mit Punkt je
//! Reihe, Tooltip (mit Summe bei Stapeln), Zoomen durch Ziehen, Tastatur und Mausrad.
//!
//! Leistung: Die Geometrie entsteht nur, wenn sich Daten, Grösse oder Darstellung ändern
//! (`use_memo`); die Mausbewegung läuft höchstens einmal je Bildschirmbild und nur die kleine
//! Ebene darüber ([`Interact`]) zeichnet neu.

use std::{cell::Cell, rc::Rc};

use dioxus::{prelude::*, web::WebEventExt};
use shared::model::{ChartKind, Unit};
use wasm_bindgen::JsCast;

use super::{
    color,
    geom::{self, Frame, Geometry, PlotData, TickLabel, VIEW_W},
    nav,
};
use crate::{components::widgets::Tone, fmt, state::use_app};

/// Waagrechte Linie bei einem Wert, z. B. eine Alarmgrenze.
#[derive(Debug, Clone, PartialEq)]
pub struct Threshold {
    pub value: f64,
    pub tone: Tone,
    /// Kurzer Text, z. B. „Warnung“.
    pub label: String,
}

impl Threshold {
    pub fn new(value: f64, tone: Tone, label: impl Into<String>) -> Self {
        Self { value, tone, label: label.into() }
    }
}

/// Senkrechte Marke zu einem Zeitpunkt, z. B. ein Ereignis.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    /// Unix-Sekunden.
    pub ts: i64,
    pub tone: Tone,
    pub label: String,
}

/// CSS-Farbe eines Zustands (nur für Zustände, nie für Reihen).
pub fn tone_color(tone: Tone) -> &'static str {
    match tone {
        Tone::Neutral => "var(--muted-foreground)",
        Tone::Good => "var(--success)",
        Tone::Warn => "var(--warning)",
        Tone::Bad => "var(--destructive)",
    }
}

/// Abstand der Ortszeit zu UTC in Sekunden.
pub fn tz_offset() -> i64 {
    -(js_sys::Date::new_0().get_timezone_offset() * 60.0) as i64
}

fn tick_text(t: i64, step: i64, tz: i64) -> String {
    match geom::tick_label(t, step, tz) {
        TickLabel::Clock => fmt::clock(t),
        TickLabel::Hm => fmt::hm(t),
        TickLabel::Day => fmt::day(t),
    }
}

/// Wie dick Linien und wie kräftig Flächen gezeichnet werden.
fn style(kind: ChartKind, series: usize) -> (&'static str, &'static str) {
    match kind {
        ChartKind::Stacked => ("0.42", "1.25"),
        ChartKind::Area => ("0.13", if series <= 2 { "1.75" } else { "1.5" }),
        ChartKind::Line => ("0", if series == 1 { "2" } else { "1.75" }),
    }
}

/// Zeichenfläche für eigene Daten. Breite = Breite des Elternelements.
#[component]
pub fn Plot(
    data: ReadSignal<PlotData>,
    #[props(default)] kind: ChartKind,
    #[props(default)] unit: Unit,
    /// Höhe der Zeichenfläche in Pixeln.
    #[props(default = 180)]
    height: u32,
    /// Achsenbeschriftungen zeigen.
    #[props(default = true)]
    axes: bool,
    /// Fadenkreuz, Tooltip und Zoomen.
    #[props(default = true)]
    interactive: bool,
    /// Sichtbares Zeitfenster (Unix-Sekunden); ohne = genau die Daten.
    #[props(default)]
    window: Option<(i64, i64)>,
    /// Waagrechte Grenzlinien.
    #[props(default)]
    thresholds: Vec<Threshold>,
    /// Senkrechte Marken (Ereignisse).
    #[props(default)]
    marks: Vec<Mark>,
    /// Summe im Tooltip (Standard: bei gestapelten Flächen).
    #[props(default)]
    total: Option<bool>,
    /// Beschreibung für Bildschirmleser.
    #[props(into, default)]
    label: String,
) -> Element {
    let mut width = use_signal(|| 0.0f64);
    let extra = thresholds.iter().map(|t| t.value).fold(None, |acc: Option<f64>, v| Some(acc.map_or(v, |a| a.max(v))));
    let h = f64::from(height);

    let geo = use_memo(use_reactive!(|kind, unit, height, window, extra| {
        let w = *width.read();
        let w = if w > 0.0 { w } else { VIEW_W };
        geom::layout(&data.read(), kind, unit, Frame { width: w, height: f64::from(height), window, extra })
    }));

    let g = geo.read();
    let w = g.width;
    let tz = tz_offset();
    let n = data.read().len();
    let empty = n > 0 && data.read().is_empty();
    let series = g.shapes.len();
    let (area_opacity, line_width) = style(kind, series);
    let zero_y = (g.y_min < 0.0 && g.y_max > 0.0).then(|| g.y(0.0).round() + 0.5);
    let (ticks, tick_step) = if axes { geom::time_ticks(g.t0, g.t1, w, 76.0, tz) } else { (Vec::new(), 1) };
    let y_labels: Vec<(f64, String)> = if axes { g.ticks.iter().map(|t| (g.y(*t), unit.format(t.abs()))).collect() } else { Vec::new() };
    let gutter = if axes { y_labels.iter().map(|(_, l)| l.chars().count()).max().unwrap_or(1) as f64 * 5.9 + 8.0 } else { 0.0 };
    let gutter = gutter.clamp(0.0, 76.0);
    let thr: Vec<(f64, &Threshold)> =
        thresholds.iter().filter(|t| t.value >= g.y_min && t.value <= g.y_max).map(|t| (g.y(t.value).round() + 0.5, t)).collect();
    let mk: Vec<(f64, &Mark)> = marks.iter().filter(|m| m.ts >= g.t0 && m.ts <= g.t1).map(|m| (g.x(m.ts).round() + 0.5, m)).collect();
    let total = total.unwrap_or(kind == ChartKind::Stacked);
    let aria = if label.is_empty() { "Diagramm".to_string() } else { label.clone() };

    rsx! {
        div { class: "relative select-none", style: "padding-left:{gutter:.0}px", "data-name": "Plot",
            if axes {
                div {
                    class: "pointer-events-none absolute top-0 left-0 text-right text-[10px] leading-none text-muted-foreground tabular-nums",
                    style: "height:{height}px;width:{gutter - 6.0:.0}px",
                    for (y, l) in y_labels {
                        span { class: "absolute right-0 -translate-y-1/2 whitespace-nowrap", style: "top:{y:.1}px", "{l}" }
                    }
                }
            }
            div {
                class: "relative",
                style: "height:{height}px",
                onresize: move |e| {
                    if let Ok(s) = e.data().get_content_box_size() {
                        let v = s.width.round();
                        if v > 0.0 && (v - *width.peek()).abs() >= 1.0 {
                            width.set(v);
                        }
                    }
                },
                svg {
                    class: "nrz-plot absolute inset-0 size-full overflow-visible",
                    view_box: "0 0 {w:.0} {height}",
                    preserve_aspect_ratio: "none",
                    role: "img",
                    "aria-label": "{aria}",
                    // Hilfslinien: waagrecht je Wert, senkrecht je Zeitmarke – haarfein.
                    g { shape_rendering: "crispEdges",
                        for t in g.ticks.iter() {
                            line {
                                x1: "0",
                                x2: "{w:.0}",
                                y1: "{g.y(*t).round() + 0.5:.1}",
                                y2: "{g.y(*t).round() + 0.5:.1}",
                                stroke: "var(--chart-grid)",
                                stroke_width: "1",
                            }
                        }
                        for t in ticks.iter() {
                            line {
                                x1: "{g.x(*t).round() + 0.5:.1}",
                                x2: "{g.x(*t).round() + 0.5:.1}",
                                y1: "0",
                                y2: "{h}",
                                stroke: "var(--chart-grid)",
                                stroke_opacity: "0.55",
                                stroke_width: "1",
                            }
                        }
                        if let Some(y) = zero_y {
                            line { x1: "0", x2: "{w:.0}", y1: "{y:.1}", y2: "{y:.1}", stroke: "var(--chart-axis)", stroke_width: "1" }
                        }
                    }
                    for s in g.shapes.iter().filter(|s| !s.area.is_empty()) {
                        path { d: "{s.area}", fill: "{color(s.slot)}", fill_opacity: "{area_opacity}", stroke: "none" }
                    }
                    for s in g.shapes.iter() {
                        path {
                            d: "{s.line}",
                            fill: "none",
                            stroke: "{color(s.slot)}",
                            stroke_width: "{line_width}",
                            stroke_linejoin: "round",
                            stroke_linecap: "round",
                        }
                    }
                    for (y, t) in thr.iter() {
                        line {
                            x1: "0",
                            x2: "{w:.0}",
                            y1: "{y:.1}",
                            y2: "{y:.1}",
                            stroke: "{tone_color(t.tone)}",
                            stroke_width: "1",
                            stroke_dasharray: "5 4",
                            stroke_opacity: "0.85",
                        }
                    }
                    for (x, m) in mk.iter() {
                        line {
                            x1: "{x:.1}",
                            x2: "{x:.1}",
                            y1: "0",
                            y2: "{h}",
                            stroke: "{tone_color(m.tone)}",
                            stroke_width: "1",
                            stroke_dasharray: "2 3",
                            stroke_opacity: "0.7",
                        }
                    }
                }
                for (y, t) in thr.iter() {
                    span {
                        class: "pointer-events-none absolute right-1 inline-flex -translate-y-full items-center gap-1 pb-0.5 text-[10px] leading-none text-muted-foreground",
                        style: "top:{y:.1}px",
                        span { class: "size-1.5 rounded-full", style: "background:{tone_color(t.tone)}" }
                        "{t.label} {unit.format(t.value)}"
                    }
                }
                if empty {
                    div { class: "pointer-events-none absolute inset-0 grid place-items-center text-xs text-muted-foreground",
                        "Keine Werte im Zeitraum"
                    }
                }
                if interactive && n > 0 {
                    Interact { data, geo, unit, kind, total, height }
                }
                // Marken über der Bedienfläche, damit ihr Titel beim Darüberfahren erscheint.
                for (i, (x, m)) in mk.iter().enumerate() {
                    span {
                        key: "{i}",
                        class: "absolute -top-1 size-2 -translate-x-1/2 rounded-full ring-2 ring-card",
                        style: "left:{x:.1}px;background:{tone_color(m.tone)}",
                        title: "{fmt::full(m.ts)} · {m.label}",
                    }
                }
            }
            if axes && !ticks.is_empty() {
                div { class: "pointer-events-none relative mt-1.5 h-3 text-[10px] leading-none text-muted-foreground tabular-nums",
                    for t in ticks.iter().copied() {
                        {
                            let x = g.x(t);
                            let shift = if x < 24.0 { "" } else if x > w - 24.0 { "-translate-x-full" } else { "-translate-x-1/2" };
                            rsx! {
                                span { key: "{t}", class: "absolute whitespace-nowrap {shift}", style: "left:{x:.1}px", "{tick_text(t, tick_step, tz)}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Waagrechte Position des Zeigers in der Fläche (0..1). Dioxus verteilt Ereignisse an der Wurzel –
/// darum zählt der Abstand zum Ziel (`offsetX`), nicht `currentTarget`.
fn frac_of(web: &web_sys::MouseEvent) -> Option<f64> {
    let target = web.target()?.dyn_into::<web_sys::Element>().ok()?;
    let w = f64::from(target.client_width());
    (w > 0.0).then(|| (f64::from(web.offset_x()) / w).clamp(0.0, 1.0))
}

fn pointer_frac(e: &Event<PointerData>) -> Option<f64> {
    frac_of(&e.data().as_web_event())
}

/// Nächstes Bildschirmbild abwarten.
async fn next_frame() {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(w) = web_sys::window() {
            let _ = w.request_animation_frame(&resolve);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

/// Bedienebene: Fadenkreuz, Punkte, Tooltip, Auswahlrahmen und alle Eingaben.
#[component]
fn Interact(data: ReadSignal<PlotData>, geo: Memo<Geometry>, unit: Unit, kind: ChartKind, total: bool, height: u32) -> Element {
    let app = use_app();
    let mut hover = app.hover;
    let mut inside = use_signal(|| false);
    let mut drag = use_signal(|| None::<(f64, f64)>);
    // Mausbewegung: höchstens einmal je Bild an alle Diagramme weitergeben.
    let pending = use_hook(|| Rc::new(Cell::new(None::<Option<i64>>)));
    let scheduled = use_hook(|| Rc::new(Cell::new(false)));
    let wheel_gen = use_hook(|| Rc::new(Cell::new(0u32)));
    let wheel_acc = use_hook(|| Rc::new(Cell::new((0.5f64, 1.0f64))));

    let push_hover = {
        let (pending, scheduled) = (pending.clone(), scheduled.clone());
        move |t: Option<i64>| {
            pending.set(Some(t));
            if scheduled.get() {
                return;
            }
            scheduled.set(true);
            let (pending, scheduled) = (pending.clone(), scheduled.clone());
            spawn(async move {
                next_frame().await;
                scheduled.set(false);
                if let Some(t) = pending.take().filter(|t| *hover.peek() != *t) {
                    hover.set(t);
                }
            });
        }
    };
    // Zeit unter dem Zeiger, auf das Raster der Daten gerastet.
    let snap = move |frac: f64| -> Option<i64> {
        let t = geo.peek().time_at(frac);
        let d = data.peek();
        d.nearest(t).map(|i| d.time(i))
    };

    let g = geo.read();
    let hover_t = hover();
    let d = data.read();
    let idx = hover_t.and_then(|t| if t >= g.t0 && t <= g.t1 { d.index_of(t) } else { None });
    let cross_x = hover_t.filter(|t| *t >= g.t0 && *t <= g.t1).map(|t| g.x(t));
    let dots: Vec<(f64, f64, usize)> = match idx {
        Some(i) => {
            let x = g.x(d.time(i));
            g.shapes.iter().filter_map(|s| s.ys.get(i).filter(|y| !y.is_nan()).map(|y| (x, f64::from(*y), s.slot))).collect()
        }
        None => Vec::new(),
    };
    let width = g.width;

    let on_move = push_hover.clone();
    let on_key = push_hover.clone();
    let on_leave = push_hover.clone();
    let on_focus = push_hover.clone();

    rsx! {
        if let Some(x) = cross_x {
            div { class: "pointer-events-none absolute inset-y-0 w-px bg-foreground/40", style: "left:{x:.1}px" }
        }
        for (x, y, slot) in dots {
            span {
                class: "pointer-events-none absolute size-[9px] -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-card",
                style: "left:{x:.1}px;top:{y:.1}px;background:{color(slot)}",
            }
        }
        if let Some((a, b)) = drag() {
            div {
                class: "pointer-events-none absolute inset-y-0 border-x border-foreground/30 bg-foreground/10",
                style: "left:{a.min(b) * 100.0:.2}%;width:{(a - b).abs() * 100.0:.2}%",
            }
        }
        if inside() && drag().is_none() {
            if let Some(i) = idx {
                Tooltip { data, i, unit, kind, total, frac: (g.x(d.time(i)) / width).clamp(0.0, 1.0) }
            }
        }
        div {
            class: "absolute inset-0 cursor-crosshair touch-pan-y rounded-sm outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50",
            tabindex: "0",
            "aria-label": "Diagramm: Pfeiltasten bewegen das Fadenkreuz, Plus und Minus zoomen, Escape geht zurück",
            onpointermove: move |e| {
                let Some(f) = pointer_frac(&e) else { return };
                on_move(snap(f));
                let start = drag.peek().as_ref().map(|(a, _)| *a);
                if let Some(a) = start {
                    drag.set(Some((a, f)));
                }
            },
            onpointerenter: move |_| inside.set(true),
            onpointerleave: move |_| {
                inside.set(false);
                drag.set(None);
                on_leave(None);
            },
            onpointerdown: move |e| {
                // Ziehen zum Zoomen nur mit der Maus; am Handy bewegt der Finger das Fadenkreuz.
                if e.data().pointer_type() != "mouse" || e.data().trigger_button() != Some(dioxus::html::input_data::MouseButton::Primary) {
                    return;
                }
                if let Some(f) = pointer_frac(&e) {
                    let web = e.data().as_web_event();
                    if let Some(t) = web.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
                        let _ = t.set_pointer_capture(web.pointer_id());
                    }
                    drag.set(Some((f, f)));
                }
            },
            onpointerup: move |_| {
                if let Some((a, b)) = drag.take() {
                    let (lo, hi) = (a.min(b), a.max(b));
                    if hi - lo > 0.01 {
                        let g = geo.peek();
                        nav::zoom(app, g.time_at(lo), g.time_at(hi));
                    }
                }
            },
            ondoubleclick: move |_| nav::back(app),
            onwheel: {
                let (wheel_gen, wheel_acc) = (wheel_gen.clone(), wheel_acc.clone());
                move |e: Event<WheelData>| {
                    let m = e.modifiers();
                    if !(m.shift() || m.ctrl() || m.meta()) {
                        return;
                    }
                    e.prevent_default();
                    let dy = e.data().delta().strip_units().y;
                    let web = e.data().as_web_event();
                    let frac = frac_of(&web).unwrap_or(0.5);
                    // Mehrere Raddrehungen sammeln und gemeinsam anwenden.
                    let (_, f) = wheel_acc.get();
                    wheel_acc.set((frac, f * if dy > 0.0 { 1.25 } else { 0.8 }));
                    let gen_now = wheel_gen.get().wrapping_add(1);
                    wheel_gen.set(gen_now);
                    let (wheel_gen, wheel_acc) = (wheel_gen.clone(), wheel_acc.clone());
                    spawn(async move {
                        gloo_timers::future::TimeoutFuture::new(220).await;
                        if wheel_gen.get() == gen_now {
                            let (frac, factor) = wheel_acc.get();
                            wheel_acc.set((0.5, 1.0));
                            if (factor - 1.0).abs() > 1e-3 {
                                nav::zoom_by(app, frac, factor);
                            }
                        }
                    });
                }
            },
            onfocus: move |_| {
                let d = data.peek();
                on_focus(d.len().checked_sub(1).map(|i| d.time(i)));
            },
            onblur: move |_| hover.set(None),
            onkeydown: move |e| {
                let d = data.peek();
                let n = d.len();
                if n == 0 {
                    return;
                }
                let cur = hover.peek().and_then(|t| d.nearest(t)).unwrap_or(n - 1);
                let jump = if e.modifiers().shift() { 10 } else { 1 };
                match e.key() {
                    Key::ArrowLeft => {
                        e.prevent_default();
                        on_key(Some(d.time(cur.saturating_sub(jump))));
                    }
                    Key::ArrowRight => {
                        e.prevent_default();
                        on_key(Some(d.time((cur + jump).min(n - 1))));
                    }
                    Key::Home => on_key(Some(d.time(0))),
                    Key::End => on_key(Some(d.time(n - 1))),
                    Key::Escape => nav::back(app),
                    Key::Character(c) if c == "+" || c == "=" => {
                        let g = geo.peek();
                        nav::zoom_by(app, (g.x(d.time(cur)) / g.width).clamp(0.0, 1.0), 0.5);
                    }
                    Key::Character(c) if c == "-" => nav::zoom_by(app, 0.5, 2.0),
                    _ => {}
                }
            },
        }
    }
}

/// Werte aller Reihen am Punkt `i`, bei Stapeln mit Summe.
#[component]
fn Tooltip(data: ReadSignal<PlotData>, i: usize, unit: Unit, kind: ChartKind, total: bool, frac: f64) -> Element {
    const MAX_ROWS: usize = 12;
    let d = data.read();
    let mut rows: Vec<(usize, &str, f64)> =
        d.series.iter().filter_map(|s| s.values.get(i).copied().flatten().map(|v| (s.slot, s.name.as_str(), v))).collect();
    if rows.is_empty() {
        return rsx! {};
    }
    let sum = if total && rows.len() > 1 { d.total_at(i) } else { None };
    let many = rows.len() > MAX_ROWS;
    if many {
        rows.sort_by(|a, b| b.2.abs().total_cmp(&a.2.abs()));
    }
    let hidden = rows.len().saturating_sub(MAX_ROWS);
    let side = if frac > 0.55 { format!("right:calc({:.2}% + 14px)", (1.0 - frac) * 100.0) } else { format!("left:calc({:.2}% + 14px)", frac * 100.0) };
    let key = if kind == ChartKind::Line { "h-0.5 w-3 rounded-full" } else { "size-2.5 rounded-[3px]" };
    rsx! {
        div {
            class: "pointer-events-none absolute top-1 z-30 min-w-40 max-w-80 rounded-lg border bg-popover/95 px-2.5 py-2 text-xs text-popover-foreground shadow-lg backdrop-blur-sm",
            style: "{side}",
            p { class: "mb-1.5 text-[11px] text-muted-foreground tabular-nums", "{fmt::point(d.time(i), d.step)}" }
            div { class: "grid grid-cols-[auto_1fr_auto] items-center gap-x-2 gap-y-1",
                for (slot, name, v) in rows.into_iter().take(MAX_ROWS) {
                    span { class: "{key}", style: "background:{color(slot)}" }
                    span { class: "truncate text-muted-foreground", "{name}" }
                    span { class: "text-right font-medium tabular-nums", "{unit.format(v)}" }
                }
                if let Some(s) = sum {
                    span { class: "col-span-3 my-0.5 h-px bg-border" }
                    span {}
                    span { class: "text-muted-foreground", "Summe" }
                    span { class: "text-right font-semibold tabular-nums", "{unit.format(s)}" }
                }
            }
            if hidden > 0 {
                p { class: "mt-1 text-[11px] text-muted-foreground", "+ {hidden} weitere" }
            }
        }
    }
}
