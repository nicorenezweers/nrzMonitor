//! Diagramm-Karte: Titel, Zeichenfläche und Legende zu einem Diagramm aus dem Katalog – und der
//! grosse Dialog zum Analysieren (Statistik je Reihe, Tabelle, CSV).

use std::collections::HashSet;

use dioxus::{prelude::*, web::WebEventExt};
use shared::{
    model::{ChartKind, ChartMeta, Unit},
    query::ChartData,
};
use tw_merge::tw_merge;
use wasm_bindgen::JsCast;

use super::{
    color,
    data::use_feed,
    forms::{Heatmap, StateBand},
    geom::{self, PlotData, PlotSeries},
    marks::marks_for,
    plot::{Plot, Threshold},
};
use crate::{fmt, state::use_app};

/// Darstellung einer Karte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChartView {
    /// Nach Art des Diagramms; Zustände (0/1) als Band.
    #[default]
    Auto,
    /// Immer Linien/Flächen.
    Lines,
    /// Je Reihe eine Zeile, Wert als Helligkeit (z. B. viele Kerne).
    Heatmap,
    /// Zustandsband (an/aus).
    State,
}

/// Geöffneter Vergrösserungs-Dialog.
#[derive(Debug, Clone, PartialEq)]
pub struct Expanded {
    pub id: String,
    pub title: Option<String>,
    pub thresholds: Vec<Threshold>,
}

/// Diagramm, das gerade gross gezeigt wird (der Dialog steht einmal im Rahmen: [`ChartDialog`]).
pub static EXPANDED: GlobalSignal<Option<Expanded>> = Signal::global(|| None);

/// Diagramm gross öffnen.
pub fn expand(id: impl Into<String>) {
    *EXPANDED.write() = Some(Expanded { id: id.into(), title: None, thresholds: Vec::new() });
}

/// Diagramm aus dem Katalog nach ID. Folgt dem globalen Zeitraum und läuft live mit.
/// In Listen immer `key: "{id}"` setzen.
#[component]
pub fn ChartCard(
    id: String,
    /// Höhe der Zeichenfläche in Pixeln.
    #[props(default = 180)]
    height: u32,
    /// Titel statt des Katalogtitels.
    #[props(into, optional)]
    title: Option<String>,
    /// Legende weglassen (für kleine Kacheln).
    #[props(default)]
    compact: bool,
    #[props(into, optional)] class: Option<String>,
    /// Untertitel statt der Familie.
    #[props(into, optional)]
    subtitle: Option<String>,
    /// Legende mit Minimum, Mittel und Maximum im Zeitraum.
    #[props(default)]
    stats: bool,
    /// Waagrechte Grenzlinien (z. B. Alarmgrenzen).
    #[props(default)]
    thresholds: Vec<Threshold>,
    /// Ereignis-Marken zeigen.
    #[props(default = true)]
    events: bool,
    /// Darstellung (Linien, Heatmap, Zustandsband).
    #[props(default)]
    view: ChartView,
    /// Diese Dimensionen anfangs ausblenden (per Legende wieder einblendbar).
    #[props(default)]
    hide: Vec<String>,
    /// Knopf zum Vergrössern zeigen.
    #[props(default = true)]
    expandable: bool,
) -> Element {
    let app = use_app();
    let class = tw_merge!("group/card relative rounded-xl border bg-card p-4 shadow-sm", class.as_deref().unwrap_or(""));
    let Some(meta) = app.chart(&id) else {
        let loading = app.charts.read().is_empty();
        return rsx! {
            div { class: "{class}", "data-name": "ChartCard",
                p { class: "text-sm font-medium", "{title.clone().unwrap_or_else(|| id.clone())}" }
                div { class: "mt-3 grid place-items-center rounded-lg text-xs text-muted-foreground", style: "height:{height}px",
                    if loading {
                        div { class: "size-full animate-pulse rounded-lg bg-muted/30" }
                    } else {
                        "Keine Daten"
                    }
                }
            }
        };
    };
    rsx! {
        ChartBody {
            key: "{id}",
            meta,
            height,
            title,
            subtitle,
            compact,
            class,
            stats,
            thresholds,
            events,
            view,
            hide,
            expandable,
            big: false,
        }
    }
}

/// Reihen eines Diagramms als Zeichendaten (ausgeblendete weggelassen, Farbe = Platz im Katalog).
pub fn plot_data(meta: &ChartMeta, d: &ChartData, hidden: &HashSet<String>) -> PlotData {
    let series = meta
        .dims
        .iter()
        .enumerate()
        .filter(|(_, dim)| !hidden.contains(&dim.id))
        .filter_map(|(slot, dim)| {
            let i = d.dims.iter().position(|x| *x == dim.id)?;
            Some(PlotSeries { id: dim.id.clone(), name: dim.name.clone(), slot, negate: dim.negate, values: d.values.get(i)?.clone() })
        })
        .collect();
    PlotData { start: d.start, step: d.step, series }
}

#[component]
fn ChartBody(
    meta: ChartMeta,
    height: u32,
    title: Option<String>,
    subtitle: Option<String>,
    compact: bool,
    class: String,
    stats: bool,
    thresholds: Vec<Threshold>,
    events: bool,
    view: ChartView,
    hide: Vec<String>,
    expandable: bool,
    big: bool,
) -> Element {
    let app = use_app();
    let width = use_signal(|| 0.0f64);
    let feed = use_feed(&meta.id, Some(width));
    let raw = feed.data;
    let mut hidden = use_signal(|| hide.iter().cloned().collect::<HashSet<String>>());
    let meta = use_memo(use_reactive!(|meta| meta));
    let mut view_now = use_signal(|| view);

    let plot = use_memo(move || {
        let meta = meta.read();
        let hidden = hidden.read();
        raw.read().as_ref().map(|d| plot_data(&meta, d, &hidden)).unwrap_or_default()
    });
    // Sichtbares Fenster: live rechtsbündig am neuesten Punkt, sonst der feste Zeitraum.
    let window = use_memo(move || {
        let range = (feed.range)();
        let d = plot.read();
        if d.series.iter().all(|s| s.values.is_empty()) {
            return None;
        }
        if range.is_live() {
            let end = d.end().max(fmt::now() - 2 * i64::from(d.step.max(1)));
            Some((end - range.span(), end))
        } else {
            Some(range.resolve(fmt::now()))
        }
    });

    let m = meta.read();
    let heading = title.clone().unwrap_or_else(|| m.title.clone());
    let sub = subtitle.clone().unwrap_or_else(|| m.family.clone());
    let single = m.dims.len() == 1;
    let (unit, kind) = (m.unit, m.kind);
    let loading = raw.read().is_none();
    let stale = !loading && *feed.range.read() != (app.range)();
    let dims: Vec<(usize, String, String)> = m.dims.iter().enumerate().map(|(i, d)| (i, d.id.clone(), d.name.clone())).collect();
    let marks = if events { marks_for(&m) } else { Vec::new() };
    let current = view_now();
    let effective = match current {
        ChartView::Auto if unit == Unit::State => ChartView::State,
        ChartView::Auto => ChartView::Lines,
        v => v,
    };
    let can_heat = m.dims.len() >= 4 && kind == ChartKind::Line && unit != Unit::State;
    let id = m.id.clone();
    let th = thresholds.clone();
    let heading_aria = heading.clone();
    let show_stats = stats || big;

    rsx! {
        div {
            class: "{class}",
            "data-name": "ChartCard",
            "data-chart": "{m.id}",
            onresize: move |e| {
                if let Ok(s) = e.data().get_content_box_size() {
                    let v = (s.width / 40.0).round() * 40.0;
                    let mut width = width;
                    if v > 0.0 && v != *width.peek() {
                        width.set(v);
                    }
                }
            },
            div { class: "mb-3 flex items-start justify-between gap-3",
                div { class: "min-w-0",
                    if !big {
                        p { class: "truncate text-sm font-medium", "{heading}" }
                    }
                    if !sub.is_empty() {
                        p { class: "truncate text-xs text-muted-foreground", "{sub}" }
                    }
                }
                div { class: "flex shrink-0 items-center gap-1",
                    if single && effective == ChartView::Lines {
                        Current { plot, unit }
                    } else if !unit.label().is_empty() && !big {
                        span { class: "text-xs text-muted-foreground", "{unit.label()}" }
                    }
                    if can_heat && !compact {
                        IconButton {
                            label: if effective == ChartView::Heatmap { "Als Linien zeigen" } else { "Als Heatmap zeigen" },
                            onclick: move |_| view_now.set(if effective == ChartView::Heatmap { ChartView::Lines } else { ChartView::Heatmap }),
                            if effective == ChartView::Heatmap {
                                icons::ChartLine {}
                            } else {
                                icons::Grid3X3 {}
                            }
                        }
                    }
                    if expandable && !big {
                        IconButton {
                            label: "Vergrössern",
                            onclick: move |_| {
                                *EXPANDED.write() = Some(Expanded { id: id.clone(), title: title.clone(), thresholds: th.clone() });
                            },
                            icons::Maximize2 {}
                        }
                    }
                }
            }
            if loading {
                div { class: "animate-pulse rounded-lg bg-muted/30", style: "height:{height + 18}px" }
            } else {
                div { class: if stale { "opacity-60 transition-opacity duration-300" } else { "transition-opacity duration-300" },
                    match effective {
                        ChartView::Heatmap => rsx! { Heatmap { data: plot, unit, height, window: window() } },
                        ChartView::State => rsx! { StateBand { data: plot, window: window() } },
                        _ => rsx! {
                            Plot {
                                data: plot,
                                kind,
                                unit,
                                height,
                                window: window(),
                                thresholds: thresholds.clone(),
                                marks,
                                label: heading_aria,
                            }
                        },
                    }
                }
            }
            if !compact && (!single || show_stats) && effective == ChartView::Lines {
                Legend {
                    plot,
                    raw_dims: dims,
                    unit,
                    kind,
                    stats: show_stats,
                    big,
                    hidden: hidden(),
                    ontoggle: move |(id, solo): (String, bool)| {
                        let mut h = hidden.write();
                        if solo {
                            // Nur diese Reihe zeigen – oder wieder alle, wenn sie schon allein steht.
                            let all: Vec<String> = meta.peek().dims.iter().map(|d| d.id.clone()).collect();
                            let alone = all.iter().all(|d| *d == id || h.contains(d));
                            h.clear();
                            if !alone {
                                h.extend(all.into_iter().filter(|d| *d != id));
                            }
                        } else if !h.remove(&id) {
                            h.insert(id);
                        }
                    },
                }
            }
        }
    }
}

/// Kleiner Knopf mit Symbol in der Kopfzeile einer Karte (erscheint beim Darüberfahren).
#[component]
fn IconButton(#[props(into)] label: String, onclick: EventHandler<MouseEvent>, children: Element) -> Element {
    rsx! {
        button {
            r#type: "button",
            class: "inline-flex size-7 items-center justify-center rounded-md text-muted-foreground opacity-0 transition outline-none group-hover/card:opacity-100 group-focus-within/card:opacity-100 hover:bg-accent hover:text-foreground focus-visible:opacity-100 focus-visible:ring-[3px] focus-visible:ring-ring/50 [@media(hover:none)]:opacity-100 [&_svg]:size-3.5",
            title: "{label}",
            "aria-label": "{label}",
            onclick: move |e| onclick.call(e),
            {children}
        }
    }
}

/// Wert einer Reihe am Fadenkreuz (oder der neueste).
fn value_at(s: &PlotSeries, at: Option<usize>) -> Option<f64> {
    match at {
        Some(i) => s.values.get(i).copied().flatten(),
        None => geom::latest(&s.values),
    }
}

/// Aktueller Wert (oder der unter dem Mauszeiger) bei Diagrammen mit nur einer Reihe.
#[component]
fn Current(plot: ReadSignal<PlotData>, unit: Unit) -> Element {
    let app = use_app();
    let d = plot.read();
    let at = (app.hover)().and_then(|t| d.index_of(t));
    let value = d.series.first().and_then(|s| value_at(s, at));
    rsx! {
        span { class: "text-sm font-semibold tabular-nums", "{value.map_or_else(|| \"–\".to_string(), |v| unit.format(v))}" }
    }
}

/// Legende: Farbmarke, Name und Wert (aktuell oder unter dem Mauszeiger), auf Wunsch mit
/// Minimum/Mittel/Maximum. Klick blendet eine Reihe aus, Klick mit Umschalt/Strg/⌘ zeigt nur sie.
#[component]
fn Legend(
    plot: ReadSignal<PlotData>,
    raw_dims: Vec<(usize, String, String)>,
    unit: Unit,
    kind: ChartKind,
    stats: bool,
    big: bool,
    hidden: HashSet<String>,
    ontoggle: EventHandler<(String, bool)>,
) -> Element {
    let app = use_app();
    let d = plot.read();
    let at = (app.hover)().and_then(|t| d.index_of(t));
    let key = if kind == ChartKind::Line { "h-0.5 w-3 rounded-full" } else { "size-2.5 rounded-[3px]" };
    let hint = "Klick: aus-/einblenden · Umschalt-Klick: nur diese Reihe";
    if stats {
        let total_row = (kind == ChartKind::Stacked && d.series.len() > 1).then(|| {
            let n = d.len();
            let sums: Vec<Option<f64>> = (0..n).map(|i| d.total_at(i)).collect();
            (geom::summary(&sums), at.and_then(|i| d.total_at(i)).or_else(|| geom::latest(&sums)))
        });
        return rsx! {
            div { class: if big { "mt-4 max-h-[40vh] overflow-auto rounded-lg border" } else { "mt-3 max-h-48 overflow-auto rounded-lg border" },
                table { class: "w-full text-xs whitespace-nowrap",
                    thead { class: "sticky top-0 bg-card text-muted-foreground",
                        tr { class: "border-b",
                            th { class: "px-2.5 py-1.5 text-left font-medium", "Reihe" }
                            th { class: "px-2.5 py-1.5 text-right font-medium", if at.is_some() { "Fadenkreuz" } else { "Aktuell" } }
                            th { class: "px-2.5 py-1.5 text-right font-medium", "Min" }
                            th { class: "px-2.5 py-1.5 text-right font-medium", "Mittel" }
                            th { class: "px-2.5 py-1.5 text-right font-medium", "Max" }
                        }
                    }
                    tbody { class: "divide-y",
                        for (slot, id, name) in raw_dims {
                            {
                                let off = hidden.contains(&id);
                                let s = d.series.iter().find(|s| s.id == id);
                                let sum = s.and_then(|s| geom::summary(&s.values));
                                let value = s.and_then(|s| value_at(s, at));
                                let id_click = id.clone();
                                let cell = |v: Option<f64>| v.map_or_else(|| "–".to_string(), |v| unit.format(v));
                                rsx! {
                                    tr { key: "{id}", class: if off { "opacity-45" } else { "" },
                                        td { class: "px-2.5 py-1",
                                            button {
                                                r#type: "button",
                                                class: "flex items-center gap-2 rounded text-left outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50",
                                                title: hint,
                                                onclick: move |e| {
                                                    let m = e.modifiers();
                                                    ontoggle.call((id_click.clone(), m.shift() || m.ctrl() || m.meta()));
                                                },
                                                span { class: "{key} shrink-0", style: "background:{color(slot)}" }
                                                span { class: "text-muted-foreground", "{name}" }
                                            }
                                        }
                                        td { class: "px-2.5 py-1 text-right font-medium tabular-nums", "{cell(value)}" }
                                        td { class: "px-2.5 py-1 text-right text-muted-foreground tabular-nums", "{cell(sum.map(|s| s.min))}" }
                                        td { class: "px-2.5 py-1 text-right text-muted-foreground tabular-nums", "{cell(sum.map(|s| s.avg))}" }
                                        td { class: "px-2.5 py-1 text-right text-muted-foreground tabular-nums", "{cell(sum.map(|s| s.max))}" }
                                    }
                                }
                            }
                        }
                        if let Some((sum, now)) = total_row {
                            {
                                let cell = |v: Option<f64>| v.map_or_else(|| "–".to_string(), |v| unit.format(v));
                                rsx! {
                                    tr { class: "bg-muted/30",
                                        td { class: "px-2.5 py-1 font-medium", "Summe" }
                                        td { class: "px-2.5 py-1 text-right font-semibold tabular-nums", "{cell(now)}" }
                                        td { class: "px-2.5 py-1 text-right text-muted-foreground tabular-nums", "{cell(sum.map(|s| s.min))}" }
                                        td { class: "px-2.5 py-1 text-right text-muted-foreground tabular-nums", "{cell(sum.map(|s| s.avg))}" }
                                        td { class: "px-2.5 py-1 text-right text-muted-foreground tabular-nums", "{cell(sum.map(|s| s.max))}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        };
    }
    rsx! {
        div { class: "mt-3 flex max-h-[4.5rem] flex-wrap gap-x-4 gap-y-1 overflow-y-auto text-xs",
            for (slot, id, name) in raw_dims {
                {
                    let off = hidden.contains(&id);
                    let value = d.series.iter().find(|s| s.id == id).and_then(|s| value_at(s, at));
                    let id_click = id.clone();
                    rsx! {
                        button {
                            key: "{id}",
                            r#type: "button",
                            class: "flex items-center gap-1.5 rounded outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50",
                            class: if off { "opacity-40" },
                            title: hint,
                            "aria-pressed": "{!off}",
                            onclick: move |e| {
                                let m = e.modifiers();
                                ontoggle.call((id_click.clone(), m.shift() || m.ctrl() || m.meta()));
                            },
                            span { class: "{key} shrink-0", style: "background:{color(slot)}" }
                            span { class: "text-muted-foreground", "{name}" }
                            if let Some(v) = value {
                                span { class: "font-medium tabular-nums", "{unit.format(v)}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Zeit für Tabelle und CSV: `2026-11-15 14:03:27` (Ortszeit).
fn iso(ts: i64) -> String {
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ts as f64 * 1000.0));
    format!(
        "{}-{:02}-{:02} {:02}:{:02}:{:02}",
        d.get_full_year(),
        d.get_month() + 1,
        d.get_date(),
        d.get_hours(),
        d.get_minutes(),
        d.get_seconds()
    )
}

fn download(name: &str, text: &str) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return };
    let parts = js_sys::Array::of1(&wasm_bindgen::JsValue::from_str(text));
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type("text/csv;charset=utf-8");
    let Ok(blob) = web_sys::Blob::new_with_str_sequence_and_options(&parts, &opts) else { return };
    let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else { return };
    if let Ok(a) = doc.create_element("a").and_then(|e| e.dyn_into::<web_sys::HtmlAnchorElement>().map_err(Into::into)) {
        a.set_href(&url);
        a.set_download(name);
        a.click();
    }
    let _ = web_sys::Url::revoke_object_url(&url);
}

/// Grosse Ansicht eines Diagramms: Zeichenfläche, Statistik je Reihe, Tabelle und CSV.
/// Einmal im Rahmen einhängen; geöffnet wird mit [`expand`] oder dem Knopf auf der Karte.
#[component]
pub fn ChartDialog() -> Element {
    let app = use_app();
    let Some(open) = EXPANDED() else { return rsx! {} };
    let Some(meta) = app.chart(&open.id) else { return rsx! {} };
    rsx! { DialogBody { key: "{open.id}", meta, title: open.title, thresholds: open.thresholds } }
}

#[component]
fn DialogBody(meta: ChartMeta, title: Option<String>, thresholds: Vec<Threshold>) -> Element {
    let mut table = use_signal(|| false);
    let mut copied = use_signal(|| false);
    let feed = use_feed(&meta.id, None);
    let close = move || *EXPANDED.write() = None;
    let heading = title.clone().unwrap_or_else(|| meta.title.clone());
    let meta_csv = meta.clone();
    let csv = move || {
        let d = feed.data.peek();
        d.as_ref().map(|d| geom::to_csv(&plot_data(&meta_csv, d, &HashSet::new()), iso)).unwrap_or_default()
    };
    let csv_dl = csv.clone();
    let file = format!("{}.csv", meta.id);
    let crumbs = [shared::catalog::section_title(&meta.section).to_string(), meta.family.clone()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");

    rsx! {
        div {
            class: "fixed inset-0 z-50 flex items-end justify-center bg-black/60 p-0 animate-nrz-fade sm:items-center sm:p-6",
            onclick: move |_| close(),
            div {
                class: "flex max-h-[100dvh] w-full max-w-6xl flex-col overflow-hidden rounded-t-xl border bg-card shadow-2xl outline-none animate-nrz-enter sm:max-h-[92dvh] sm:rounded-xl",
                role: "dialog",
                "aria-modal": "true",
                "aria-label": "{heading}",
                tabindex: "-1",
                onclick: move |e| e.stop_propagation(),
                onmounted: move |e| {
                    let _ = e.data().as_web_event().dyn_into::<web_sys::HtmlElement>().map(|el| el.focus());
                },
                onkeydown: move |e| {
                    if e.key() == Key::Escape {
                        e.stop_propagation();
                        close();
                    }
                },
                div { class: "flex flex-wrap items-center gap-2 border-b px-4 py-3 sm:px-5",
                    div { class: "min-w-0 flex-1 basis-full sm:basis-auto",
                        p { class: "truncate text-xs text-muted-foreground", "{crumbs}" }
                        h2 { class: "truncate text-base font-semibold", "{heading}" }
                    }
                    div { class: "inline-flex rounded-md border bg-background p-0.5 text-xs",
                        button {
                            r#type: "button",
                            class: "rounded px-2 py-1 outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50",
                            class: if !table() { "bg-accent font-medium" } else { "text-muted-foreground hover:text-foreground" },
                            onclick: move |_| table.set(false),
                            "Diagramm"
                        }
                        button {
                            r#type: "button",
                            class: "rounded px-2 py-1 outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50",
                            class: if table() { "bg-accent font-medium" } else { "text-muted-foreground hover:text-foreground" },
                            onclick: move |_| table.set(true),
                            "Tabelle"
                        }
                    }
                    button {
                        r#type: "button",
                        class: "inline-flex h-8 items-center gap-1.5 rounded-md border px-2.5 text-xs outline-none hover:bg-accent focus-visible:ring-[3px] focus-visible:ring-ring/50 [&_svg]:size-3.5",
                        title: "Werte als CSV in die Zwischenablage",
                        onclick: move |_| {
                            let text = csv();
                            if let Some(w) = web_sys::window() {
                                let _ = w.navigator().clipboard().write_text(&text);
                            }
                            copied.set(true);
                            spawn(async move {
                                gloo_timers::future::TimeoutFuture::new(1800).await;
                                copied.set(false);
                            });
                        },
                        if copied() {
                            icons::Check {}
                            "Kopiert"
                        } else {
                            icons::Copy {}
                            span { class: "hidden sm:inline", "CSV kopieren" }
                        }
                    }
                    button {
                        r#type: "button",
                        class: "inline-flex h-8 items-center gap-1.5 rounded-md border px-2.5 text-xs outline-none hover:bg-accent focus-visible:ring-[3px] focus-visible:ring-ring/50 [&_svg]:size-3.5",
                        title: "Werte als CSV-Datei herunterladen",
                        onclick: move |_| download(&file, &csv_dl()),
                        icons::Download {}
                        span { class: "hidden sm:inline", "Herunterladen" }
                    }
                    button {
                        r#type: "button",
                        class: "inline-flex size-8 items-center justify-center rounded-md text-muted-foreground outline-none hover:bg-accent hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 [&_svg]:size-4",
                        title: "Schliessen (Esc)",
                        "aria-label": "Schliessen",
                        onclick: move |_| close(),
                        icons::X {}
                    }
                }
                div { class: "min-h-0 flex-1 overflow-y-auto p-4 sm:p-5",
                    if table() {
                        DataTable { meta: meta.clone(), data: feed.data }
                    } else {
                        ChartBody {
                            meta: meta.clone(),
                            height: 340,
                            title: None,
                            subtitle: Some(String::new()),
                            compact: false,
                            class: "".to_string(),
                            stats: true,
                            thresholds: thresholds.clone(),
                            events: true,
                            view: ChartView::Auto,
                            hide: Vec::new(),
                            expandable: false,
                            big: true,
                        }
                        p { class: "mt-3 text-xs text-muted-foreground",
                            "Ziehen zoomt · Doppelklick oder Esc im Diagramm geht zurück · Umschalt + Mausrad zoomt · Pfeiltasten bewegen das Fadenkreuz"
                        }
                    }
                }
            }
        }
    }
}

/// Werte als Tabelle (neueste zuerst, höchstens 1000 Zeilen).
#[component]
fn DataTable(meta: ChartMeta, data: Signal<Option<ChartData>>) -> Element {
    let guard = data.read();
    let Some(d) = guard.as_ref() else {
        return rsx! { p { class: "text-sm text-muted-foreground", "Lade …" } };
    };
    let p = plot_data(&meta, d, &HashSet::new());
    let n = p.len();
    let rows: Vec<usize> = (0..n).rev().take(1000).collect();
    let unit = meta.unit;
    rsx! {
        div { class: "max-h-[70dvh] overflow-auto rounded-lg border",
            table { class: "w-full text-xs",
                thead { class: "sticky top-0 bg-card text-muted-foreground",
                    tr { class: "border-b",
                        th { class: "px-2.5 py-1.5 text-left font-medium whitespace-nowrap", "Zeit" }
                        for s in p.series.iter() {
                            th { key: "{s.id}", class: "px-2.5 py-1.5 text-right font-medium whitespace-nowrap",
                                span { class: "inline-flex items-center gap-1.5",
                                    span { class: "size-2 rounded-[2px]", style: "background:{color(s.slot)}" }
                                    "{s.name}"
                                }
                            }
                        }
                    }
                }
                tbody { class: "divide-y",
                    for i in rows {
                        tr { key: "{i}",
                            td { class: "px-2.5 py-1 whitespace-nowrap text-muted-foreground tabular-nums", "{fmt::point(p.time(i), p.step)}" }
                            for s in p.series.iter() {
                                td { class: "px-2.5 py-1 text-right tabular-nums",
                                    "{s.values.get(i).copied().flatten().map_or_else(|| \"–\".to_string(), |v| unit.format(v))}"
                                }
                            }
                        }
                    }
                }
            }
        }
        if n > 1000 {
            p { class: "mt-2 text-xs text-muted-foreground", "Gezeigt: die neuesten 1000 von {n} Punkten. Die CSV enthält alle." }
        }
    }
}
