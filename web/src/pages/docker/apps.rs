//! Seiten „Apps“ (Karten je Compose-Projekt) und Detail einer App.

use dioxus::prelude::*;
use shared::{
    catalog::docker::*,
    docker::{App, Container as Ctr},
    events::Event,
};

use super::{use_snapshot, ContainerTable, Segmented, bits, bps, bytes, detail::EventList, pct};
use crate::{
    components::{
        chart::{ChartCard, section::ChartGrid, use_chart},
        lazy::Lazy,
        ui::{
            app_shell::{EmptyState, LoadingState, PageContainer, PageHeader, PageSection, PageWidth},
            input::Input,
        },
        widgets::{Sparkline, StatTile, StatusDot, Tone},
    },
    live::use_events,
    route::Route,
};

fn app_state(a: &App) -> (&'static str, Tone) {
    match a.state() {
        "running" if a.unhealthy > 0 => ("Ungesund", Tone::Warn),
        "running" => ("Läuft", Tone::Good),
        "partial" => ("Teilweise", Tone::Warn),
        _ => ("Gestoppt", Tone::Neutral),
    }
}

#[component]
pub fn Apps() -> Element {
    let apps = use_snapshot::<Vec<App>>("docker.apps");
    let mut query = use_signal(String::new);
    let order = use_signal(|| "cpu".to_string());
    let Some(all) = apps.read().clone() else {
        return rsx! {
            PageContainer { width: PageWidth::Full,
                PageHeader { title: "Apps", description: "Container zusammengefasst je App (Compose-Projekt)" }
                LoadingState {}
            }
        };
    };
    let running = all.iter().filter(|a| a.state() == "running").count();
    let partial = all.iter().filter(|a| a.state() == "partial").count();
    let stopped = all.iter().filter(|a| a.state() == "stopped").count();
    let q = query().to_lowercase();
    let mut list: Vec<App> = all.iter().filter(|a| q.is_empty() || a.name.to_lowercase().contains(&q)).cloned().collect();
    match order().as_str() {
        "mem" => list.sort_by(|a, b| b.mem.total_cmp(&a.mem)),
        "name" => list.sort_by(|a, b| a.name.cmp(&b.name)),
        _ => list.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then_with(|| a.name.cmp(&b.name))),
    }
    // Probleme immer zuerst.
    list.sort_by_key(|a| !matches!(app_state(a).1, Tone::Warn | Tone::Bad));
    let total_cpu: f64 = all.iter().map(|a| a.cpu).sum();
    let total_mem: f64 = all.iter().map(|a| a.mem).sum();
    rsx! {
        PageContainer { width: PageWidth::Full,
            PageHeader { title: "Apps", description: "Container zusammengefasst je App (Compose-Projekt)" }
            div { class: "grid grid-cols-[minmax(0,1fr)] gap-6",
                div { class: "grid grid-cols-2 gap-4 md:grid-cols-4",
                    StatTile { label: "Apps", value: "{all.len()}", hint: "{running} laufen vollständig", icon: rsx! { icons::LayoutGrid {} } }
                    StatTile { label: "Teilweise / gestoppt", value: "{partial} / {stopped}", hint: "nicht alle Container laufen", icon: rsx! { icons::CircleDashed {} } }
                    StatTile { label: "CPU aller Apps", value: pct((total_cpu * 10.0).round() / 10.0), hint: "100 % = ein Kern", icon: rsx! { icons::Cpu {} } }
                    StatTile { label: "Speicher aller Apps", value: bytes(total_mem), hint: "ohne Dateicache", icon: rsx! { icons::MemoryStick {} } }
                }
                ChartGrid {
                    ChartCard { id: APPS_CPU_ALL.context, height: 200 }
                    ChartCard { id: APPS_MEM_ALL.context, height: 200 }
                }
                div { class: "flex flex-wrap items-center gap-2",
                    div { class: "w-full sm:w-72",
                        Input { placeholder: "App suchen", value: query(), oninput: move |e: FormEvent| query.set(e.value()) }
                    }
                    Segmented {
                        options: vec![("cpu".into(), "CPU".into()), ("mem".into(), "Speicher".into()), ("name".into(), "Name".into())],
                        value: order,
                    }
                }
                if list.is_empty() {
                    EmptyState { title: "Keine Apps", description: "Keine App passt zur Suche.", icon: rsx! { icons::SearchX {} } }
                }
                div { class: "grid gap-4 sm:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4",
                    for a in list {
                        AppCard { key: "{a.name}", app: a }
                    }
                }
            }
        }
    }
}

#[component]
fn AppCard(app: App) -> Element {
    let (label, tone) = app_state(&app);
    let to = Route::AppDetail { name: app.name.clone() };
    let up = if app.started > 0 && app.running > 0 { shared::model::duration((crate::fmt::now() - app.started).max(0) as f64) } else { "–".into() };
    let stopped = app.running == 0;
    rsx! {
        Link { to, class: "group rounded-xl border bg-card p-4 shadow-sm transition-colors hover:bg-accent/40",
            div { class: "flex items-start justify-between gap-2",
                div { class: "min-w-0",
                    p { class: "truncate font-semibold", "{app.name}" }
                    p { class: "text-xs text-muted-foreground",
                        "{app.running}/{app.total} Container"
                        if !stopped { " · seit {up}" }
                    }
                }
                StatusDot { tone, label }
            }
            if stopped {
                div { class: "mt-4 grid h-20 place-items-center rounded-lg bg-muted/40 text-xs text-muted-foreground", "Keine laufenden Container" }
            } else {
                Lazy { min_height: 36, class: "mt-3",
                    AppSpark { name: app.name.clone() }
                }
                dl { class: "mt-3 grid grid-cols-3 gap-2 text-xs",
                    div {
                        dt { class: "text-muted-foreground", "CPU" }
                        dd { class: "text-sm font-medium tabular-nums", "{pct(app.cpu)}" }
                    }
                    div {
                        dt { class: "text-muted-foreground", "Speicher" }
                        dd { class: "text-sm font-medium tabular-nums", "{bytes(app.mem)}" }
                    }
                    div {
                        dt { class: "text-muted-foreground", "Netz ↓" }
                        dd { class: "text-sm font-medium tabular-nums truncate", "{bits(app.net_rx)}" }
                    }
                }
            }
            if app.restarts > 0 {
                p { class: "mt-2 text-xs text-muted-foreground", "{app.restarts} Neustarts" }
            }
        }
    }
}

/// CPU-Verlauf einer App als Mini-Linie (Zeitraum der Seite).
#[component]
fn AppSpark(name: String) -> Element {
    let data = use_chart(&APP_CPU.chart_id(Some(&name)));
    let values = data.read().as_ref().and_then(|d| d.values.first().cloned()).unwrap_or_default();
    rsx! { Sparkline { values, height: 36 } }
}

#[component]
pub fn AppDetail(name: String) -> Element {
    rsx! { AppView { key: "{name}", name } }
}

#[component]
fn AppView(name: String) -> Element {
    let apps = use_snapshot::<Vec<App>>("docker.apps");
    let list = use_snapshot::<Vec<Ctr>>("docker.containers");
    let live = use_events();
    let app = apps.read().as_ref().and_then(|l| l.iter().find(|a| a.name == name).cloned());
    let Some(app) = app else {
        let loading = apps.read().is_none();
        return rsx! {
            PageContainer { width: PageWidth::Full,
                PageHeader { title: name.clone(), eyebrow: "App" }
                if loading {
                    LoadingState {}
                } else {
                    EmptyState { title: "App nicht gefunden", description: "Keine Container mit diesem Compose-Projekt.", icon: rsx! { icons::LayoutGrid {} },
                        Link { to: Route::Apps {}, class: "text-sm underline", "Zu allen Apps" }
                    }
                }
            }
        };
    };
    let (label, tone) = app_state(&app);
    let containers: Vec<Ctr> = list.read().as_ref().map(|l| l.iter().filter(|c| c.app == name).cloned().collect()).unwrap_or_default();
    let mut events: Vec<Event> = live.read().iter().filter(|e| e.labels.get("app") == Some(&name) || app.containers.contains(&e.subject)).cloned().collect();
    events.truncate(50);
    let id = |t: &shared::catalog::Tpl| t.chart_id(Some(&name));
    let up = if app.started > 0 && app.running > 0 { shared::model::duration((crate::fmt::now() - app.started).max(0) as f64) } else { "–".into() };
    rsx! {
        PageContainer { width: PageWidth::Full,
            PageHeader {
                title: name.clone(),
                eyebrow: "App",
                description: if app.dir.is_empty() { format!("{} Container", app.total) } else { app.dir.clone() },
                actions: rsx! {
                    span { class: "inline-flex items-center gap-2 rounded-md border px-2.5 py-1.5", StatusDot { tone, label } }
                },
            }
            div { class: "grid grid-cols-[minmax(0,1fr)] gap-6",
                div { class: "grid grid-cols-2 gap-4 md:grid-cols-3 2xl:grid-cols-6",
                    StatTile { label: "Container", value: "{app.running} / {app.total}", hint: if app.unhealthy > 0 { format!("{} ungesund", app.unhealthy) } else { "laufen".to_string() }, icon: rsx! { icons::Container {} } }
                    StatTile { label: "CPU", value: pct(app.cpu), hint: "100 % = ein Kern", icon: rsx! { icons::Cpu {} } }
                    StatTile { label: "Speicher", value: bytes(app.mem), hint: "ohne Dateicache", icon: rsx! { icons::MemoryStick {} } }
                    StatTile { label: "Netzwerk ↓", value: bits(app.net_rx), hint: "↑ {bits(app.net_tx)} gesendet", icon: rsx! { icons::ArrowDownUp {} } }
                    StatTile { label: "Datenträger", value: bps(app.io_read + app.io_write), hint: "{bps(app.io_read)} lesen · {bps(app.io_write)} schreiben", icon: rsx! { icons::HardDrive {} } }
                    StatTile { label: "Läuft seit", value: up, hint: "{app.restarts} Neustarts", icon: rsx! { icons::Clock {} } }
                }
                ChartGrid {
                    ChartCard { id: id(&APP_CPU), height: 200 }
                    ChartCard { id: id(&APP_MEM), height: 200 }
                    ChartCard { id: id(&APP_NET), height: 200 }
                    ChartCard { id: id(&APP_IO), height: 200 }
                }
                PageSection { title: "Container", description: "Klick öffnet die Einzelheiten",
                    ContainerTable { list: containers.clone(), show_app: false }
                }
                if containers.len() > 1 {
                    PageSection { title: "Speicher je Container", description: "Aktuell, ohne Dateicache",
                        MemBars { list: containers.clone() }
                    }
                }
                PageSection { title: "Ereignisse", description: "Seit dem Öffnen der Oberfläche",
                    EventList { events }
                }
            }
        }
    }
}

/// Rangliste als Balken (Form: Rangliste → Balken).
#[component]
fn MemBars(list: Vec<Ctr>) -> Element {
    let mut l: Vec<Ctr> = list.into_iter().filter(|c| c.running()).collect();
    l.sort_by(|a, b| b.mem.total_cmp(&a.mem));
    let max = l.first().map_or(1.0, |c| c.mem.max(1.0));
    rsx! {
        div { class: "grid gap-2",
            for c in l {
                div { key: "{c.id}", class: "grid grid-cols-[minmax(0,10rem)_minmax(0,1fr)_5rem] items-center gap-3 text-sm",
                    span { class: "truncate", "{c.service.clone()}" }
                    div { class: "h-2 overflow-hidden rounded-full bg-muted",
                        div { class: "h-full rounded-full bg-foreground/60", style: "width:{(c.mem / max * 100.0):.1}%" }
                    }
                    span { class: "text-right tabular-nums", "{bytes(c.mem)}" }
                }
            }
        }
    }
}
