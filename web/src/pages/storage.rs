//! Seite „Speicher“: was nrzMonitor selbst belegt, je Stufe, und die Einstellungen dazu.

use dioxus::prelude::*;
use gloo_timers::future::TimeoutFuture;
use shared::{
    Unit,
    catalog::monitor::{MONITOR_DISK, MONITOR_INGEST, MONITOR_SERIES},
    storage::{StorageSettings as Settings, StorageStats, TIER_NAMES, TierStats},
};

use crate::{
    api,
    components::{
        chart::{ChartCard, section::ChartGrid},
        ui::{
            app_shell::{EmptyState, LoadingState, PageContainer, PageHeader, PageWidth, SettingsGroup, SettingsRow},
            button::{Button, ButtonSize, ButtonVariant},
            select::{Select, SelectContent, SelectGroup, SelectOption, SelectTrigger, SelectValue},
            table::{Table, TableBody, TableCell, TableHead, TableHeader, TableRow, TableWrapper},
            toast::{ToastType, show_toast},
        },
        widgets::{Meter, StatTile, StatusDot, Tone},
    },
    fmt,
    live::use_snap,
};

const DAY: u64 = 86_400;
const GIB: u64 = 1024 * 1024 * 1024;

/// Zeitspanne kurz und rund: `6 h`, `3 d`, `6 Mo`, `2 J`.
pub(crate) fn span_label(secs: u64) -> String {
    const MONTH: u64 = 30 * DAY;
    const YEAR: u64 = 365 * DAY;
    if secs >= YEAR && secs.is_multiple_of(YEAR) {
        format!("{} J", secs / YEAR)
    } else if secs >= 4 * MONTH && secs.is_multiple_of(MONTH) && secs < YEAR {
        format!("{} Mo", secs / MONTH)
    } else if secs >= DAY && secs.is_multiple_of(DAY) {
        format!("{} d", secs / DAY)
    } else if secs >= 3600 && secs.is_multiple_of(3600) {
        format!("{} h", secs / 3600)
    } else {
        Unit::Seconds.format(secs as f64)
    }
}

/// Statistik: Momentaufnahme `storage.stats`, sonst alle 30 s vom Endpunkt.
fn use_stats() -> (Memo<Option<StorageStats>>, Signal<Option<String>>) {
    let snap = use_snap::<StorageStats>("storage.stats");
    let mut fetched = use_signal(|| None::<StorageStats>);
    let mut error = use_signal(|| None::<String>);
    use_future(move || async move {
        loop {
            if snap.peek().is_none() {
                match api::get::<StorageStats>("/storage/stats").await {
                    Ok(s) => {
                        fetched.set(Some(s));
                        error.set(None);
                    }
                    Err(e) => error.set(Some(e)),
                }
            }
            TimeoutFuture::new(30_000).await;
        }
    });
    let stats = use_memo(move || snap.read().clone().or_else(|| fetched.read().clone()));
    (stats, error)
}

#[component]
pub fn Storage() -> Element {
    let (stats, error) = use_stats();
    rsx! {
        PageContainer { width: PageWidth::Full,
            PageHeader { title: "Speicher", description: "Was nrzMonitor selbst belegt und wie lange es aufbewahrt" }
            div { class: "flex min-w-0 flex-col gap-6",
                match (stats.read().as_ref(), error.read().as_ref()) {
                    (Some(s), _) => rsx! {
                        Tiles { stats: s.clone() }
                        TierTable { tiers: s.tiers.clone() }
                    },
                    (None, Some(e)) => rsx! {
                        EmptyState { title: "Keine Speicherzahlen", description: "Der Server liefert noch keine Angaben zum Speicher ({e}).", icon: rsx! { icons::Database {} } }
                    },
                    (None, None) => rsx! { LoadingState {} },
                }
                ChartGrid {
                    ChartCard { id: MONITOR_DISK.context.to_string() }
                    ChartCard { id: MONITOR_INGEST.context.to_string() }
                    ChartCard { id: MONITOR_SERIES.context.to_string() }
                }
                div { class: "max-w-3xl", StorageSettings {} }
            }
        }
    }
}

#[component]
fn Tiles(stats: StorageStats) -> Element {
    let s = stats;
    let b = |v: u64| Unit::Bytes.format(v as f64);
    let (used_hint, pct) = if s.max_bytes > 0 {
        let pct = s.total_bytes as f64 / s.max_bytes as f64 * 100.0;
        (format!("von {} · {} %", b(s.max_bytes), shared::model::trim(pct, 1)), Some(pct))
    } else {
        ("keine Obergrenze".to_string(), None)
    };
    let raw = s.tiers.iter().find(|t| t.tier == 0);
    let bpv = raw.filter(|t| t.rows > 0).map_or_else(|| "–".to_string(), |t| format!("{:.2} B", t.bytes_per_value));
    let free = if s.disk_free > 0 { format!("{} frei", b(s.disk_free)) } else { "Datei der Datenbank".to_string() };
    let ring_min = s.ring_window / 60;
    let dropped = if s.dropped > 0 { format!("{} verworfen", Unit::Count.format(s.dropped as f64)) } else { "nichts verworfen".to_string() };
    rsx! {
        div { class: "grid grid-cols-2 gap-4 lg:grid-cols-4",
            StatTile { label: "Belegt", value: b(s.total_bytes), hint: used_hint, icon: rsx! { icons::HardDrive {} },
                if let Some(p) = pct {
                    Meter { value: p }
                }
            }
            StatTile { label: "Werte je Sekunde", value: Unit::PerSec.format(s.samples_per_sec), hint: "Mittel über 1 min", icon: rsx! { icons::Activity {} } }
            StatTile { label: "Reihen", value: Unit::Count.format(s.series as f64), hint: format!("{} aktive Diagramme", s.charts), icon: rsx! { icons::ChartLine {} } }
            StatTile { label: "Bytes je Wert", value: bpv, hint: "Sekunden · Ziel ≤ 1 B", icon: rsx! { icons::Gauge {} } }
            StatTile { label: "DuckDB", value: b(s.db_bytes), hint: free, icon: rsx! { icons::Database {} } }
            StatTile { label: "Ringpuffer (RAM)", value: b(s.ring_bytes), hint: format!("{ring_min} min voll aufgelöst"), icon: rsx! { icons::MemoryStick {} } }
            StatTile { label: "Ereignisse", value: Unit::Count.format(s.events as f64), hint: "gespeichert", icon: rsx! { icons::History {} } }
            StatTile { label: "Werte seit Start", value: Unit::Count.format(s.samples_total as f64), hint: dropped, icon: rsx! { icons::Sigma {} } }
        }
    }
}

/// Ein Wert einer Stufe für Tabelle und Karte.
struct TierRow {
    name: String,
    resolution: String,
    rows: String,
    bytes: String,
    files: String,
    hot: String,
    span: String,
    since: String,
    bpv: String,
    retention: String,
}

fn tier_row(t: &TierStats) -> TierRow {
    let name = TIER_NAMES.get(usize::from(t.tier)).copied().unwrap_or("?").to_string();
    let (span, since) = if t.oldest > 0 && t.newest >= t.oldest {
        (Unit::Seconds.format((t.newest - t.oldest) as f64), format!("seit {}", fmt::point(t.oldest, 60)))
    } else {
        ("–".to_string(), "leer".to_string())
    };
    TierRow {
        name,
        resolution: span_label(u64::from(t.resolution)),
        rows: Unit::Count.format(t.rows as f64),
        bytes: Unit::Bytes.format(t.bytes as f64),
        files: t.files.to_string(),
        hot: Unit::Count.format(t.hot_rows as f64),
        span,
        since,
        bpv: if t.rows > 0 { format!("{:.2} B", t.bytes_per_value) } else { "–".to_string() },
        retention: span_label(t.retention),
    }
}

#[component]
fn TierTable(tiers: Vec<TierStats>) -> Element {
    if tiers.is_empty() {
        return rsx! {};
    }
    let rows: Vec<TierRow> = tiers.iter().map(tier_row).collect();
    rsx! {
        section { class: "grid gap-3",
            h2 { class: "text-sm font-semibold", "Stufen" }
            // Desktop: Tabelle
            div { class: "hidden md:block",
                TableWrapper { class: "max-h-none rounded-xl bg-card shadow-sm",
                    Table { class: "max-w-none",
                        TableHeader {
                            TableRow {
                                TableHead { "Stufe" }
                                TableHead { class: "text-right", "Zeilen" }
                                TableHead { class: "text-right", "Belegt" }
                                TableHead { class: "text-right", "Dateien" }
                                TableHead { class: "text-right", "Offen" }
                                TableHead { "Zeitspanne" }
                                TableHead { class: "text-right", "Bytes je Wert" }
                                TableHead { class: "text-right", "Aufbewahrung" }
                            }
                        }
                        TableBody {
                            for r in rows.iter() {
                                TableRow { key: "{r.name}",
                                    TableCell { class: "py-3",
                                        p { class: "font-medium", "{r.name}" }
                                        p { class: "text-xs text-muted-foreground", "Raster {r.resolution}" }
                                    }
                                    TableCell { class: "py-3 text-right tabular-nums", "{r.rows}" }
                                    TableCell { class: "py-3 text-right tabular-nums", "{r.bytes}" }
                                    TableCell { class: "py-3 text-right tabular-nums", "{r.files}" }
                                    TableCell { class: "py-3 text-right tabular-nums", "{r.hot}" }
                                    TableCell { class: "py-3",
                                        p { class: "tabular-nums", "{r.span}" }
                                        p { class: "text-xs text-muted-foreground tabular-nums", "{r.since}" }
                                    }
                                    TableCell { class: "py-3 text-right tabular-nums", "{r.bpv}" }
                                    TableCell { class: "py-3 text-right tabular-nums", "{r.retention}" }
                                }
                            }
                        }
                    }
                }
            }
            // Handy: Karten
            div { class: "grid gap-3 md:hidden",
                for r in rows.iter() {
                    div { key: "{r.name}", class: "rounded-xl border bg-card p-4 shadow-sm",
                        div { class: "flex items-baseline justify-between gap-2",
                            p { class: "font-medium", "{r.name}" }
                            span { class: "text-xs text-muted-foreground", "Raster {r.resolution} · {r.retention} aufbewahren" }
                        }
                        dl { class: "mt-3 grid grid-cols-2 gap-x-4 gap-y-2 text-sm",
                            KeyVal { k: "Belegt", v: r.bytes.clone() }
                            KeyVal { k: "Zeilen", v: r.rows.clone() }
                            KeyVal { k: "Bytes je Wert", v: r.bpv.clone() }
                            KeyVal { k: "Dateien", v: r.files.clone() }
                            KeyVal { k: "Offene Zeilen", v: r.hot.clone() }
                            KeyVal { k: "Zeitspanne", v: r.span.clone() }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn KeyVal(k: &'static str, v: String) -> Element {
    rsx! {
        div { class: "min-w-0",
            dt { class: "text-xs text-muted-foreground", "{k}" }
            dd { class: "truncate tabular-nums", "{v}" }
        }
    }
}

/* ------------------------------ Einstellungen ------------------------------ */

const RET_RAW: &[u64] = &[6 * 3600, 12 * 3600, DAY, 3 * DAY, 7 * DAY, 14 * DAY];
const RET_MIN: &[u64] = &[7 * DAY, 14 * DAY, 30 * DAY, 90 * DAY, 180 * DAY, 365 * DAY];
const RET_HOUR: &[u64] = &[180 * DAY, 365 * DAY, 2 * 365 * DAY, 5 * 365 * DAY, 10 * 365 * DAY];
const RET_EVENTS: &[u64] = &[7 * DAY, 30 * DAY, 90 * DAY, 365 * DAY];
const LIMITS: &[u64] = &[GIB / 2, GIB, 2 * GIB, 5 * GIB, 10 * GIB, 20 * GIB, 0];

fn limit_label(v: u64) -> String {
    if v == 0 { "keine".to_string() } else { Unit::Bytes.format(v as f64) }
}

/// Auswahl aus festen Werten; ein abweichender aktueller Wert erscheint zusätzlich.
fn choices(base: &[u64], current: u64, label: fn(u64) -> String) -> Vec<(String, u64)> {
    let mut list: Vec<u64> = base.to_vec();
    if !list.contains(&current) {
        list.push(current);
        // „keine“ (0) bleibt am Ende.
        list.sort_by_key(|v| if *v == 0 { u64::MAX } else { *v });
    }
    list.into_iter().map(|v| (label(v), v)).collect()
}

/// Auswahlfeld für einen Zahlenwert (Beschriftung = Wert des Select-Bausteins).
#[component]
fn Choice(options: Vec<(String, u64)>, value: u64, on_change: EventHandler<u64>) -> Element {
    let current = options.iter().find(|(_, v)| *v == value).map(|(l, _)| l.clone());
    let opts = options.clone();
    rsx! {
        Select {
            key: "{value}",
            class: "w-32",
            default_value: current,
            on_change: move |l: Option<String>| {
                if let Some(v) = l.and_then(|l| opts.iter().find(|(x, _)| *x == l).map(|(_, v)| *v)) {
                    on_change.call(v);
                }
            },
            SelectTrigger { class: "w-32", SelectValue {} }
            SelectContent { class: "w-32 right-0 left-auto",
                SelectGroup { aria_label: "Auswahl",
                    for (l, _) in options.iter() {
                        SelectOption { key: "{l}", value: l.clone(), "{l}" }
                    }
                }
            }
        }
    }
}

/// Abschnitt der Einstellungen (auch auf der Einstellungsseite): Aufbewahrung und Speichergrenze.
#[component]
pub fn StorageSettings() -> Element {
    // (gespeichert, bearbeitet)
    let mut saved = use_signal(|| None::<Settings>);
    let mut draft = use_signal(Settings::default);
    let mut load_error = use_signal(|| None::<String>);
    let mut busy = use_signal(|| false);
    let mut note = use_signal(|| None::<(bool, String)>);

    use_future(move || async move {
        match api::get::<Settings>("/storage/settings").await {
            Ok(s) => {
                draft.set(s.clone());
                saved.set(Some(s));
            }
            Err(e) => load_error.set(Some(e)),
        }
    });

    let d = draft.read().clone();
    let dirty = saved.read().as_ref() != Some(&d);
    let loaded = saved.read().is_some();
    let save = move |_| {
        let body = draft.read().clone();
        busy.set(true);
        spawn(async move {
            match api::put::<Settings, Settings>("/storage/settings", &body).await {
                Ok(s) => {
                    draft.set(s.clone());
                    saved.set(Some(s));
                    load_error.set(None);
                    note.set(Some((true, "Gespeichert".to_string())));
                    show_toast(ToastType::Success, "Speicher-Einstellungen gespeichert");
                }
                Err(e) => {
                    note.set(Some((false, format!("Nicht gespeichert: {e}"))));
                    show_toast(ToastType::Error, format!("Nicht gespeichert: {e}"));
                }
            }
            busy.set(false);
        });
    };

    let footer = match load_error.read().as_ref() {
        Some(e) => format!("Einstellungen nicht geladen ({e}) – angezeigt sind die Standardwerte."),
        None => "Wird die Obergrenze erreicht, fällt zuerst das Älteste der Sekundenwerte weg. Änderungen wirken ohne Neustart.".to_string(),
    };

    rsx! {
        SettingsGroup {
            title: "Speicher",
            footer,
            actions: rsx! {
                if let Some((ok, text)) = note.read().as_ref() {
                    StatusDot { tone: if *ok { Tone::Good } else { Tone::Bad }, label: text.clone() }
                }
                Button {
                    size: ButtonSize::Sm,
                    variant: if dirty { ButtonVariant::Default } else { ButtonVariant::Outline },
                    disabled: *busy.read() || (!dirty && loaded),
                    onclick: save,
                    if *busy.read() { "Speichere …" } else { "Speichern" }
                }
            },
            SettingsRow { label: "Sekundenwerte", hint: "Jeder Messwert in voller Auflösung",
                Choice { options: choices(RET_RAW, d.retention[0], span_label), value: d.retention[0], on_change: move |v| { draft.write().retention[0] = v; note.set(None); } }
            }
            SettingsRow { label: "Minutenwerte", hint: "Mittel, Minimum und Maximum je Minute",
                Choice { options: choices(RET_MIN, d.retention[1], span_label), value: d.retention[1], on_change: move |v| { draft.write().retention[1] = v; note.set(None); } }
            }
            SettingsRow { label: "Stundenwerte", hint: "Mittel, Minimum und Maximum je Stunde",
                Choice { options: choices(RET_HOUR, d.retention[2], span_label), value: d.retention[2], on_change: move |v| { draft.write().retention[2] = v; note.set(None); } }
            }
            SettingsRow { label: "Obergrenze", hint: "Speicherplatz für alles zusammen",
                Choice { options: choices(LIMITS, d.max_bytes, limit_label), value: d.max_bytes, on_change: move |v| { draft.write().max_bytes = v; note.set(None); } }
            }
            SettingsRow { label: "Ereignisse", hint: "Wie lange Ereignisse bleiben",
                Choice { options: choices(RET_EVENTS, d.event_retention, span_label), value: d.event_retention, on_change: move |v| { draft.write().event_retention = v; note.set(None); } }
            }
        }
    }
}
