//! Seite „Erkunden“: Diagramme aus dem ganzen Katalog suchen, auf einer Arbeitsfläche vergleichen
//! (gemerkt im Browser) und auswerten: Kennzahlen, Rangliste eines Kontexts, „Was hat sich geändert?“.

use std::collections::BTreeMap;

use dioxus::prelude::*;
use shared::{
    ChartMeta, Unit,
    query::Agg,
    storage::{CorrelateItem, CorrelateReq, CorrelateResp, SeriesStats, StatsReq, StatsResp, TopReq, TopResp},
};

use crate::{
    api,
    components::{
        chart::{ChartCard, color, section::ChartGrid},
        ui::{
            app_shell::{EmptyState, LoadingState, PageContainer, PageHeader, PageSection, PageWidth},
            badge::{Badge, BadgeVariant},
            button::{Button, ButtonSize, ButtonVariant},
            input::{Input, InputType},
            select::{Select, SelectContent, SelectGroup, SelectOption, SelectTrigger, SelectValue},
            table::{Table, TableBody, TableCell, TableHead, TableHeader, TableRow, TableWrapper},
        },
        widgets::{Sparkline, show},
    },
    fmt,
    state::{App, use_app},
};

/// Schlüssel im localStorage.
const KEY: &str = "nrzmon.explore.charts";
/// Vorschläge für den leeren Zustand (nur die, die es gibt).
const SUGGEST: &[&str] = &["system.cpu", "system.ram", "system.load", "system.net", "system.io"];
/// Höchstens so viele Treffer in der Suche.
const HITS: usize = 12;

/* ------------------------------ Arbeitsfläche merken ------------------------------ */

thread_local! {
    static NAV_USED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

/// Start: `?charts=a,b` in der Adresse, sonst der gemerkte Stand.
fn load_workspace() -> Vec<String> {
    let mut search = web_sys::window().and_then(|w| w.location().search().ok()).unwrap_or_default();
    // Der Router schreibt die Adresse beim Start um – dann steht der Parameter nur noch im
    // Navigationseintrag des Browsers (einmal je Seitenaufruf auswerten).
    if search.is_empty() && !NAV_USED.with(|u| u.replace(true)) {
        search = js_sys::Function::new_no_args(
            "const e = performance.getEntriesByType('navigation')[0]; if (!e) return ''; \
             const u = new URL(e.name); return u.pathname.endsWith('/explore') ? u.search : '';",
        )
        .call0(&wasm_bindgen::JsValue::NULL)
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default();
    }
    for pair in search.trim_start_matches('?').split('&') {
        if let Some(v) = pair.strip_prefix("charts=") {
            let v = js_sys::decode_uri_component(v).map(String::from).unwrap_or_else(|_| v.to_string());
            return dedup(v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect());
        }
    }
    local_storage()
        .and_then(|s| s.get_item(KEY).ok().flatten())
        .and_then(|j| serde_json::from_str::<Vec<String>>(&j).ok())
        .map(dedup)
        .unwrap_or_default()
}

fn dedup(list: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(list.len());
    for id in list {
        if !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

fn add(mut ws: Signal<Vec<String>>, id: &str) {
    if !ws.read().iter().any(|x| x == id) {
        ws.write().push(id.to_string());
    }
}

/* ------------------------------ Suche ------------------------------ */

fn haystack(c: &ChartMeta) -> String {
    let mut s = format!("{} {} {} {} {}", c.id, c.title, c.context, c.section, c.family);
    for (k, v) in &c.labels {
        s.push_str(&format!(" {k}={v} {v}"));
    }
    for d in &c.dims {
        s.push(' ');
        s.push_str(&d.name);
    }
    s.to_lowercase()
}

/// Treffer zu einer Suche, die besten zuerst.
fn search(charts: &[ChartMeta], q: &str) -> Vec<ChartMeta> {
    let tokens: Vec<String> = q.split_whitespace().map(str::to_lowercase).collect();
    if tokens.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<(u32, &ChartMeta)> = charts
        .iter()
        .filter_map(|c| {
            let hay = haystack(c);
            if !tokens.iter().all(|t| hay.contains(t.as_str())) {
                return None;
            }
            let (id, title) = (c.id.to_lowercase(), c.title.to_lowercase());
            let first = &tokens[0];
            let mut score: u32 = 0;
            if id == *first {
                score += 100;
            }
            if id.starts_with(first.as_str()) || title.starts_with(first.as_str()) {
                score += 20;
            }
            if title.contains(first.as_str()) {
                score += 10;
            }
            if c.retired {
                score = score.saturating_sub(30);
            }
            Some((score, c))
        })
        .collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.priority.cmp(&b.1.priority)));
    hits.into_iter().take(HITS).map(|(_, c)| c.clone()).collect()
}

/* ------------------------------ Seite ------------------------------ */

#[component]
pub fn Explore() -> Element {
    let app = use_app();
    let mut ws = use_signal(load_workspace);
    use_effect(move || {
        let list = ws.read().clone();
        if let Some(s) = local_storage() {
            let _ = s.set_item(KEY, &serde_json::to_string(&list).unwrap_or_default());
        }
    });

    let csv = use_memo(move || {
        let list = ws.read();
        let range = app.range.read();
        format!(
            "/api/v1{}",
            api::qs(
                "/export.csv",
                &[("charts", list.join(",")), ("after", range.after.to_string()), ("before", range.before.to_string()), ("points", "0".into())]
            )
        )
    });
    let empty = ws.read().is_empty();

    rsx! {
        PageContainer { width: PageWidth::Full,
            PageHeader {
                title: "Erkunden",
                description: "Beliebige Metriken vergleichen und auswerten",
                actions: rsx! {
                    if !empty {
                        a {
                            class: "inline-flex h-8 items-center gap-1.5 rounded-md border bg-background px-3 text-sm font-medium shadow-xs hover:bg-accent hover:text-accent-foreground [&_svg]:size-4",
                            href: "{csv}",
                            download: "nrzmonitor.csv",
                            icons::Download {}
                            "CSV"
                        }
                        Button { variant: ButtonVariant::Ghost, size: ButtonSize::Sm, onclick: move |_| ws.write().clear(),
                            icons::X {}
                            "Leeren"
                        }
                    }
                },
            }
            div { class: "flex min-w-0 flex-col gap-6",
                SearchBox { ws }
                if empty {
                    Suggestions { ws }
                } else {
                    Workspace { ws }
                    StatsSection { ws }
                }
                TopSection { ws }
                ChangeSection { ws }
            }
        }
    }
}

#[component]
fn SearchBox(ws: Signal<Vec<String>>) -> Element {
    let app = use_app();
    let mut q = use_signal(String::new);
    let hits = use_memo(move || search(&app.charts.read(), &q.read()));
    let total = app.charts.read().len();
    rsx! {
        div { class: "rounded-xl border bg-card p-3 shadow-sm",
            div { class: "relative",
                icons::Search { class: "pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" }
                Input {
                    r#type: InputType::Search,
                    class: "pl-9",
                    placeholder: "Diagramm suchen: Titel, ID, Container, Gerät … ({total} im Katalog)",
                    value: q(),
                    oninput: move |e: FormEvent| q.set(e.value()),
                }
            }
            if !q.read().trim().is_empty() {
                if hits.read().is_empty() {
                    p { class: "px-1 pt-3 text-sm text-muted-foreground", "Kein Diagramm gefunden." }
                } else {
                    ul { class: "mt-2 grid gap-0.5",
                        for c in hits.read().iter().cloned() {
                            SearchHit { key: "{c.id}", chart: c, ws, on_pick: move |_| q.set(String::new()) }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn SearchHit(chart: ChartMeta, ws: Signal<Vec<String>>, on_pick: EventHandler<()>) -> Element {
    let on = ws.read().contains(&chart.id);
    let id = chart.id.clone();
    let labels: Vec<String> = chart.labels.iter().map(|(k, v)| format!("{k}={v}")).collect();
    rsx! {
        li {
            button {
                r#type: "button",
                class: "flex w-full items-center gap-3 rounded-md px-2 py-2 text-left hover:bg-accent hover:text-accent-foreground",
                onclick: move |_| {
                    add(ws, &id);
                    on_pick.call(());
                },
                div { class: "min-w-0 flex-1",
                    p { class: "truncate text-sm font-medium", "{chart.title}" }
                    p { class: "truncate font-mono text-xs text-muted-foreground",
                        "{chart.id}"
                        if !labels.is_empty() {
                            span { class: "hidden sm:inline", " · {labels.join(\" \")}" }
                        }
                    }
                }
                Badge { variant: BadgeVariant::Muted, class: "hidden sm:inline-flex", "{chart.section}" }
                if chart.retired {
                    Badge { variant: BadgeVariant::Outline, "stillgelegt" }
                }
                if on {
                    span { class: "flex shrink-0 items-center gap-1 text-xs text-muted-foreground", icons::Check { class: "size-4" } "auf der Fläche" }
                } else {
                    icons::Plus { class: "size-4 shrink-0 text-muted-foreground" }
                }
            }
        }
    }
}

#[component]
fn Suggestions(ws: Signal<Vec<String>>) -> Element {
    let app = use_app();
    let present: Vec<ChartMeta> = SUGGEST.iter().filter_map(|id| app.chart(id)).collect();
    let all: Vec<String> = present.iter().map(|c| c.id.clone()).collect();
    rsx! {
        EmptyState {
            title: "Die Arbeitsfläche ist leer",
            description: "Diagramme oben suchen und anklicken – sie bleiben hier gemerkt. Oder mit einem Vorschlag beginnen:",
            icon: rsx! { icons::Telescope {} },
            for c in present.iter().cloned() {
                Button { key: "{c.id}", variant: ButtonVariant::Outline, size: ButtonSize::Sm, onclick: move |_| add(ws, &c.id),
                    icons::Plus {}
                    "{c.title}"
                }
            }
            if present.len() > 1 {
                Button { variant: ButtonVariant::Default, size: ButtonSize::Sm, onclick: move |_| ws.set(all.clone()), "Alle hinzufügen" }
            }
        }
    }
}

#[component]
fn Workspace(ws: Signal<Vec<String>>) -> Element {
    let list = ws.read().clone();
    let n = list.len();
    let mv = move |i: usize, j: usize| {
        let mut ws = ws;
        let mut w = ws.write();
        if i < w.len() && j < w.len() {
            w.swap(i, j);
        }
    };
    rsx! {
        ChartGrid {
            for (i, id) in list.into_iter().enumerate() {
                div { key: "{id}", class: "flex min-w-0 flex-col gap-1",
                    div { class: "flex items-center justify-between gap-2 pl-1",
                        span { class: "truncate font-mono text-xs text-muted-foreground", "{id}" }
                        div { class: "flex shrink-0 items-center",
                            Button { variant: ButtonVariant::Ghost, size: ButtonSize::IconXs, title: "Nach vorn", aria_label: "Nach vorn", disabled: i == 0,
                                onclick: move |_| mv(i, i.wrapping_sub(1)),
                                icons::ArrowLeft {}
                            }
                            Button { variant: ButtonVariant::Ghost, size: ButtonSize::IconXs, title: "Nach hinten", aria_label: "Nach hinten", disabled: i + 1 == n,
                                onclick: move |_| mv(i, i + 1),
                                icons::ArrowRight {}
                            }
                            Button { variant: ButtonVariant::Ghost, size: ButtonSize::IconXs, title: "Entfernen", aria_label: "Entfernen",
                                onclick: {
                                    let id = id.clone();
                                    move |_| ws.write().retain(|x| *x != id)
                                },
                                icons::X {}
                            }
                        }
                    }
                    ChartCard { key: "{id}", id: id.clone() }
                }
            }
        }
    }
}

/* ------------------------------ Kennzahlen ------------------------------ */

/// Farbplatz einer Dimension wie in der Diagramm-Karte.
fn slot(app: App, chart: &str, dim: &str) -> usize {
    app.chart(chart).and_then(|c| c.dim_index(dim)).unwrap_or(usize::MAX)
}

#[component]
fn StatsSection(ws: Signal<Vec<String>>) -> Element {
    let app = use_app();
    let stats = use_resource(move || {
        let charts = ws.read().clone();
        let range = *app.range.read();
        async move { api::post::<_, StatsResp>("/analyze/stats", &StatsReq { charts, after: range.after, before: range.before }).await }
    });
    // Altes Bild stehen lassen, solange neu gerechnet wird.
    let mut last = use_signal(|| None::<Result<StatsResp, String>>);
    use_effect(move || {
        if let Some(r) = stats.read().as_ref() {
            last.set(Some(r.clone()));
        }
    });
    let loading = stats.read().is_none();
    rsx! {
        PageSection { title: "Kennzahlen", description: "Je Reihe der Arbeitsfläche im gewählten Zeitraum",
            match last.read().as_ref() {
                None => rsx! { LoadingState {} },
                Some(Err(e)) => rsx! { Unavailable { text: e.clone() } },
                Some(Ok(r)) if r.series.is_empty() => rsx! { p { class: "text-sm text-muted-foreground", "Keine Werte im Zeitraum." } },
                Some(Ok(r)) => rsx! {
                    div { class: if loading { "opacity-50 transition-opacity" } else { "transition-opacity" },
                        StatsTable { series: r.series.clone() }
                    }
                },
            }
        }
    }
}

#[component]
fn StatsTable(series: Vec<SeriesStats>) -> Element {
    let app = use_app();
    let cols = ["Min", "Mittel", "Max", "p50", "p95", "p99", "Letzter", "Streuung"];
    rsx! {
        TableWrapper { class: "-mx-1 max-h-[32rem] border-0",
            Table { class: "max-w-none",
                TableHeader {
                    TableRow {
                        TableHead { class: "sticky left-0 z-10 bg-card", "Reihe" }
                        for c in cols {
                            TableHead { key: "{c}", class: "text-right", "{c}" }
                        }
                    }
                }
                TableBody {
                    for s in series.iter() {
                        {
                            let title = app.chart(&s.chart).map_or_else(|| s.chart.clone(), |c| c.title);
                            let u = s.unit;
                            let mark = color(slot(app, &s.chart, &s.dim));
                            let vals = [s.min, s.avg, s.max, s.p50, s.p95, s.p99, s.last, s.stddev];
                            rsx! {
                                TableRow { key: "{s.chart}/{s.dim}",
                                    TableCell { class: "sticky left-0 z-10 max-w-56 bg-card px-2 py-2",
                                        div { class: "flex items-center gap-2",
                                            span { class: "size-2.5 shrink-0 rounded-sm", style: "background-color:{mark}" }
                                            div { class: "min-w-0",
                                                p { class: "truncate font-medium", "{s.name}" }
                                                p { class: "truncate text-xs text-muted-foreground", "{title}" }
                                            }
                                        }
                                    }
                                    for (k, v) in vals.into_iter().enumerate() {
                                        TableCell { key: "{k}", class: "px-2 py-2 text-right whitespace-nowrap tabular-nums", "{show(u, v)}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Endpunkt fehlt oder meldet einen Fehler: ruhig anzeigen.
#[component]
fn Unavailable(text: String) -> Element {
    rsx! {
        div { class: "flex items-center gap-2 text-sm text-muted-foreground",
            icons::CircleSlash { class: "size-4 shrink-0" }
            span { "Auswertung nicht verfügbar ({text})." }
        }
    }
}

/* ------------------------------ Rangliste ------------------------------ */

const AGGS: &[(&str, Agg)] = &[("Mittel", Agg::Avg), ("Maximum", Agg::Max), ("Minimum", Agg::Min), ("Letzter Wert", Agg::Last)];

/// Kontexte mit mehreren Instanzen: (Kontext, Titel, Anzahl), die grössten zuerst.
fn contexts(charts: &[ChartMeta]) -> Vec<(String, String, usize)> {
    let mut map: BTreeMap<&str, (String, usize)> = BTreeMap::new();
    for c in charts.iter().filter(|c| !c.retired) {
        let e = map.entry(c.context.as_str()).or_insert_with(|| (c.title.clone(), 0));
        e.1 += 1;
    }
    let mut out: Vec<(String, String, usize)> =
        map.into_iter().filter(|(_, (_, n))| *n > 1).map(|(k, (t, n))| (k.to_string(), t, n)).collect();
    out.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
    out
}

#[component]
fn TopSection(ws: Signal<Vec<String>>) -> Element {
    let app = use_app();
    let ctxs = use_memo(move || contexts(&app.charts.read()));
    let mut ctx = use_signal(|| None::<String>);
    let mut agg = use_signal(|| AGGS[0]);
    // Standard: der Kontext mit den meisten Instanzen.
    let current = use_memo(move || ctx.read().clone().or_else(|| ctxs.read().first().map(|c| c.0.clone())));

    let top = use_resource(move || {
        let context = current.read().clone();
        let range = *app.range.read();
        let a = agg.read().1;
        async move {
            let context = context?;
            Some(
                api::post::<_, TopResp>(
                    "/analyze/top",
                    &TopReq { context, after: range.after, before: range.before, agg: a, dim: None, limit: Some(20) },
                )
                .await,
            )
        }
    });

    let selects = rsx! {
        if let Some(cur) = current.read().clone() {
            Select {
                key: "{ctxs.read().len()}",
                class: "w-full sm:w-56",
                default_value: cur,
                on_change: move |v: Option<String>| ctx.set(v),
                SelectTrigger { SelectValue {} }
                SelectContent { class: "w-72 max-w-[calc(100vw-2rem)] sm:right-0 sm:left-auto",
                    SelectGroup { aria_label: "Kontext",
                        for (k, t, n) in ctxs.read().iter().cloned() {
                            SelectOption { key: "{k}", value: k.clone(),
                                span { class: "min-w-0 flex-1 truncate", "{t}" }
                                span { class: "font-mono text-xs text-muted-foreground", "{k} · {n}" }
                            }
                        }
                    }
                }
            }
            Select {
                class: "w-full sm:w-36",
                default_value: agg.read().0.to_string(),
                on_change: move |v: Option<String>| {
                    if let Some(a) = AGGS.iter().find(|(l, _)| Some(*l) == v.as_deref()) {
                        agg.set(*a);
                    }
                },
                SelectTrigger { SelectValue {} }
                SelectContent { class: "w-36 sm:right-0 sm:left-auto",
                    SelectGroup { aria_label: "Kennzahl",
                        for (l, _) in AGGS.iter() {
                            SelectOption { key: "{l}", value: l.to_string(), "{l}" }
                        }
                    }
                }
            }
        }
    };

    rsx! {
        PageSection {
            title: "Rangliste",
            description: "Instanzen eines Kontexts im Zeitraum geordnet – Klick legt das Diagramm auf die Fläche",
            actions: rsx! { div { class: "grid w-full grid-cols-2 gap-2 sm:flex", {selects} } },
            class: "[&>div:first-child>div:last-child]:w-full sm:[&>div:first-child>div:last-child]:w-auto",
            match top.read().as_ref() {
                None => rsx! { LoadingState {} },
                Some(None) => rsx! { p { class: "text-sm text-muted-foreground", "Kein Kontext mit mehreren Instanzen im Katalog." } },
                Some(Some(Err(e))) => rsx! { Unavailable { text: e.clone() } },
                Some(Some(Ok(r))) if r.items.is_empty() => rsx! { p { class: "text-sm text-muted-foreground", "Keine Werte im Zeitraum." } },
                Some(Some(Ok(r))) => rsx! { TopList { resp: r.clone(), ws } },
            }
        }
    }
}

#[component]
fn TopList(resp: TopResp, ws: Signal<Vec<String>>) -> Element {
    let max = resp.items.iter().map(|i| i.value.abs()).fold(0.0_f64, f64::max).max(f64::MIN_POSITIVE);
    let unit = resp.unit;
    rsx! {
        ol { class: "grid gap-x-8 gap-y-1 xl:grid-cols-2",
            for (n, it) in resp.items.iter().enumerate() {
                {
                    let pct = (it.value.abs() / max * 100.0).clamp(0.0, 100.0);
                    let chart = it.chart.clone();
                    let on = ws.read().contains(&it.chart);
                    rsx! {
                        li { key: "{it.chart}",
                            button {
                                r#type: "button",
                                class: "grid w-full grid-cols-[1.5rem_minmax(0,1fr)_auto] items-center gap-x-3 gap-y-1 rounded-md px-2 py-1.5 text-left hover:bg-accent",
                                title: if on { "Schon auf der Arbeitsfläche" } else { "Zur Arbeitsfläche hinzufügen" },
                                onclick: move |_| add(ws, &chart),
                                span { class: "text-xs tabular-nums text-muted-foreground", "{n + 1}" }
                                span { class: "truncate text-sm", "{it.instance}" }
                                span { class: "text-sm font-medium tabular-nums", "{unit.format(it.value)}" }
                                span {}
                                div { class: "col-span-2 h-1.5 overflow-hidden rounded-full bg-muted",
                                    div { class: "h-full rounded-full", style: "width:{pct:.1}%;background:{color(0)}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/* ------------------------------ Was hat sich geändert? ------------------------------ */

#[component]
fn ChangeSection(ws: Signal<Vec<String>>) -> Element {
    let app = use_app();
    let mut result = use_signal(|| None::<Result<CorrelateResp, String>>);
    let mut busy = use_signal(|| false);

    let range = *app.range.read();
    let (a, b) = range.resolve(fmt::now());
    let window = if range.is_live() {
        format!("die letzten {}", super::storage::span_label(range.span() as u64))
    } else {
        format!("{} – {}", fmt::point(a, 60), fmt::point(b, 60))
    };

    let run = move |_| {
        let range = *app.range.peek();
        let (after, before) = range.resolve(fmt::now());
        busy.set(true);
        spawn(async move {
            let req = CorrelateReq { after, before, baseline_after: 0, section: None, limit: Some(30), points: Some(60) };
            result.set(Some(api::post::<_, CorrelateResp>("/analyze/correlate", &req).await));
            busy.set(false);
        });
    };

    rsx! {
        PageSection {
            title: "Was hat sich geändert?",
            description: "Vergleicht {window} mit dem Zeitraum davor (viermal so lang). Tipp: im Diagramm einen Bereich ziehen, um hineinzuzoomen.",
            actions: rsx! {
                Button { size: ButtonSize::Sm, disabled: busy(), onclick: run,
                    if busy() {
                        icons::LoaderCircle { class: "animate-spin" }
                        "Rechne …"
                    } else {
                        icons::ScanSearch {}
                        "Analysieren"
                    }
                }
            },
            match result.read().as_ref() {
                None => rsx! { p { class: "text-sm text-muted-foreground", "Noch nicht ausgewertet. „Analysieren“ sucht die Reihen, die sich im Zeitraum am stärksten verändert haben." } },
                Some(Err(e)) => rsx! { Unavailable { text: e.clone() } },
                Some(Ok(r)) if r.items.is_empty() => rsx! { p { class: "text-sm text-muted-foreground", "Keine auffällige Veränderung ({r.examined} Reihen untersucht)." } },
                Some(Ok(r)) => rsx! {
                    p { class: "mb-3 text-xs text-muted-foreground",
                        "{r.examined} Reihen untersucht in {r.took_ms} ms · Fenster {fmt::point(r.after, 60)} – {fmt::point(r.before, 60)}, verglichen ab {fmt::point(r.baseline_after, 60)}"
                    }
                    ul { class: "divide-y",
                        for it in r.items.iter().cloned() {
                            ChangeRow { key: "{it.chart}/{it.dim}", item: it, ws }
                        }
                    }
                },
            }
        }
    }
}

/// Relative Veränderung als Text: `+42 %`, `−8 %`, `neu`.
fn delta(before: f64, after: f64) -> String {
    if before.abs() < f64::EPSILON {
        return if after.abs() < f64::EPSILON { "±0 %".to_string() } else { "neu".to_string() };
    }
    let p = (after - before) / before.abs() * 100.0;
    let sign = if p >= 0.0 { "+" } else { "−" };
    let v = p.abs();
    if v >= 1000.0 { format!("{sign}{:.0}×", v / 100.0 + 1.0) } else { format!("{sign}{} %", shared::model::trim(v, if v < 10.0 { 1 } else { 0 })) }
}

#[component]
fn ChangeRow(item: CorrelateItem, ws: Signal<Vec<String>>) -> Element {
    let it = item;
    let u: Unit = it.unit;
    let n = it.spark.len();
    let split = if n > 1 { (f64::from(it.spark_split) / (n - 1) as f64 * 100.0).clamp(0.0, 100.0) } else { 100.0 };
    let up = it.window_avg >= it.before_avg;
    let on = ws.read().contains(&it.chart);
    let chart = it.chart.clone();
    let score = (it.score * 100.0).clamp(0.0, 100.0);
    rsx! {
        li {
            button {
                r#type: "button",
                class: "grid w-full grid-cols-[minmax(0,1fr)_auto] items-center gap-x-4 gap-y-2 px-1 py-3 text-left hover:bg-accent/50 md:grid-cols-[minmax(0,1.4fr)_7rem_minmax(0,1fr)_minmax(8rem,14rem)]",
                title: if on { "Schon auf der Arbeitsfläche" } else { "Zur Arbeitsfläche hinzufügen" },
                onclick: move |_| add(ws, &chart),
                div { class: "min-w-0",
                    p { class: "truncate text-sm font-medium", "{it.title}" }
                    p { class: "truncate text-xs text-muted-foreground",
                        "{it.name} · "
                        span { class: "font-mono", "{it.chart}" }
                    }
                }
                div { class: "text-right md:text-left",
                    p { class: "flex items-center justify-end gap-1 text-sm font-semibold tabular-nums md:justify-start",
                        if up { icons::TrendingUp { class: "size-4 text-muted-foreground" } } else { icons::TrendingDown { class: "size-4 text-muted-foreground" } }
                        "{delta(it.before_avg, it.window_avg)}"
                    }
                    div { class: "mt-1 h-1 w-full overflow-hidden rounded-full bg-muted", title: "Stärke der Veränderung {score:.0} %",
                        div { class: "h-full rounded-full bg-foreground/60", style: "width:{score:.0}%" }
                    }
                }
                p { class: "text-xs whitespace-nowrap text-muted-foreground tabular-nums",
                    "Ø davor "
                    span { class: "text-foreground", "{u.format(it.before_avg)}" }
                    br {}
                    "Ø im Fenster "
                    span { class: "text-foreground", "{u.format(it.window_avg)}" }
                }
                div { class: "relative h-9 w-28 min-w-0 md:w-auto",
                    // Fenster markiert
                    div { class: "absolute inset-y-0 right-0 rounded-sm bg-muted", style: "left:{split:.1}%" }
                    div { class: "relative", Sparkline { values: it.spark.clone(), height: 36, slot: 0 } }
                }
            }
        }
    }
}
