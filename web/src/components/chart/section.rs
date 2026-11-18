//! Diagramme automatisch aus dem Katalog aufbauen – ohne Wissen über den Bereich.

use dioxus::prelude::*;
use shared::model::ChartMeta;

use super::ChartCard;
use crate::{
    components::{lazy::Lazy, ui::app_shell::EmptyState},
    state::use_app,
};

/// Raster für Diagramm-Karten: 1 Spalte am Handy, 2 ab `lg`, 3 ab `2xl`.
#[component]
pub fn ChartGrid(children: Element) -> Element {
    rsx! {
        div { class: "grid gap-4 lg:grid-cols-2 2xl:grid-cols-3", {children} }
    }
}

/// Aufeinanderfolgende Diagramme mit gleicher Familie zusammenfassen (Reihenfolge bleibt).
pub fn by_family(charts: Vec<ChartMeta>) -> Vec<(String, Vec<ChartMeta>)> {
    let mut groups: Vec<(String, Vec<ChartMeta>)> = Vec::new();
    for c in charts {
        match groups.iter_mut().find(|(f, _)| *f == c.family) {
            Some((_, list)) => list.push(c),
            None => groups.push((c.family.clone(), vec![c])),
        }
    }
    // Diagramme ohne Familie zuerst.
    groups.sort_by_key(|(f, _)| !f.is_empty());
    groups
}

/// Passt ein Diagramm zur Suche? Jedes Wort muss in Titel, Familie, ID oder einem Label vorkommen.
pub fn matches_filter(c: &ChartMeta, filter: &str) -> bool {
    let words: Vec<String> = filter.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return true;
    }
    let hay = format!(
        "{} {} {} {} {}",
        c.title,
        c.family,
        c.id,
        shared::catalog::section_title(&c.section),
        c.labels.values().cloned().collect::<Vec<_>>().join(" ")
    )
    .to_lowercase();
    words.iter().all(|w| hay.contains(w.as_str()))
}

/// Alle Diagramme eines Bereichs (`cpu`, `disk`, `docker` …), nach Familie gruppiert.
/// Was ein Sammler anmeldet, erscheint hier von selbst.
#[component]
pub fn SectionCharts(
    section: String,
    #[props(default = 180)] height: u32,
    /// Stillgelegte Diagramme (Container entfernt …) mit anzeigen.
    #[props(default)]
    retired: bool,
    /// Nur Diagramme, die zu diesen Suchwörtern passen.
    #[props(into, default)]
    filter: String,
    /// Familien einklappbar (bei vielen Familien anfangs eingeklappt).
    #[props(default)]
    collapsible: bool,
) -> Element {
    let app = use_app();
    let charts: Vec<ChartMeta> =
        app.by_section(&section).into_iter().filter(|c| (retired || !c.retired) && matches_filter(c, &filter)).collect();
    if charts.is_empty() {
        return rsx! {
            EmptyState {
                title: if filter.is_empty() { "Noch keine Daten" } else { "Nichts gefunden" },
                description: if filter.is_empty() { "Für diesen Bereich hat noch kein Sammler Werte geliefert." } else { "Kein Diagramm in diesem Bereich passt zur Suche." },
                icon: rsx! { icons::ChartLine {} },
            }
        };
    }
    let groups = by_family(charts);
    let many = groups.iter().filter(|(f, _)| !f.is_empty()).count() > 10;
    let searching = !filter.trim().is_empty();
    rsx! {
        div { class: if collapsible { "grid gap-3" } else { "grid gap-8" },
            for (family, list) in groups {
                if collapsible && !family.is_empty() {
                    FamilyGroup { key: "{family}", open: searching || !many, family, charts: list, height }
                } else {
                    div { key: "{family}", class: "grid gap-3",
                        if !family.is_empty() {
                            h3 { class: "text-xs font-medium tracking-wider text-muted-foreground uppercase", "{family}" }
                        }
                        ChartGrid {
                            for c in list {
                                Lazy { key: "{c.id}", min_height: height + 110,
                                    ChartCard { id: c.id.clone(), height }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Eine einklappbare Familie (Gerät, Schnittstelle, Container …).
#[component]
pub fn FamilyGroup(family: String, charts: Vec<ChartMeta>, #[props(default = 180)] height: u32, #[props(default = true)] open: bool) -> Element {
    let mut is_open = use_signal(|| open);
    use_effect(use_reactive!(|open| is_open.set(open)));
    let n = charts.len();
    let retired = charts.iter().all(|c| c.retired);
    rsx! {
        section { class: "rounded-xl border bg-card/40", "data-name": "FamilyGroup",
            button {
                r#type: "button",
                class: "flex w-full items-center gap-2 rounded-xl px-3 py-2.5 text-left outline-none hover:bg-accent/30 focus-visible:ring-[3px] focus-visible:ring-ring/50 [&_svg]:size-4",
                "aria-expanded": "{is_open()}",
                onclick: move |_| is_open.toggle(),
                span { class: if is_open() { "rotate-90 text-muted-foreground transition-transform" } else { "text-muted-foreground transition-transform" },
                    icons::ChevronRight {}
                }
                span { class: "truncate text-sm font-medium", "{family}" }
                if retired {
                    span { class: "rounded bg-muted px-1.5 text-[11px] text-muted-foreground", "stillgelegt" }
                }
                span { class: "ml-auto text-xs text-muted-foreground tabular-nums",
                    if n == 1 { "1 Diagramm" } else { "{n} Diagramme" }
                }
            }
            if is_open() {
                div { class: "px-3 pb-3",
                    ChartGrid {
                        for c in charts {
                            Lazy { key: "{c.id}", min_height: height + 110,
                                ChartCard { id: c.id.clone(), height, subtitle: String::new() }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Alle Instanzen eines Kontexts nebeneinander (z. B. `net.traffic` → je Schnittstelle eine Karte,
/// betitelt mit der Instanz).
#[component]
pub fn ContextCharts(context: String, #[props(default = 160)] height: u32) -> Element {
    let app = use_app();
    let charts: Vec<ChartMeta> = app.by_context(&context).into_iter().filter(|c| !c.retired).collect();
    rsx! {
        ChartGrid {
            for c in charts {
                Lazy { key: "{c.id}", min_height: height + 110,
                    ChartCard { id: c.id.clone(), height }
                }
            }
        }
    }
}
