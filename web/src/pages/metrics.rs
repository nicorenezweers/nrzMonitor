//! Alle Metriken: jeder Bereich mit allem, was die Sammler liefern – ohne Auswahl von Hand.
//! Bereiche als Navigation, Suche über alles, Familien einklappbar (auch bei 80 Containern übersichtlich).

use dioxus::prelude::*;
use shared::catalog;

use crate::{
    components::{
        chart::{SectionCharts, section::matches_filter},
        ui::app_shell::{EmptyState, PageContainer, PageHeader, PageWidth},
    },
    state::use_app,
};

#[component]
pub fn Metrics() -> Element {
    let app = use_app();
    let mut current = use_signal(|| None::<String>);
    let mut query = use_signal(String::new);
    let mut retired = use_signal(|| false);

    let charts = app.charts.read();
    let q = query();
    let searching = !q.trim().is_empty();
    // Bereiche mit (sichtbaren, passenden) Diagrammen.
    let mut sections: Vec<(String, usize)> = Vec::new();
    let mut total = 0usize;
    for c in charts.iter().filter(|c| retired() || !c.retired) {
        total += 1;
        if !matches_filter(c, &q) {
            continue;
        }
        match sections.iter_mut().find(|(s, _)| *s == c.section) {
            Some((_, n)) => *n += 1,
            None => sections.push((c.section.clone(), 1)),
        }
    }
    drop(charts);
    sections.sort_by_key(|(s, _)| catalog::section_order(s));
    let hits: usize = sections.iter().map(|(_, n)| n).sum();
    let active = current().filter(|s| sections.iter().any(|(x, _)| x == s)).or_else(|| sections.first().map(|(s, _)| s.clone()));
    let n_sections = sections.len();

    rsx! {
        PageContainer { width: PageWidth::Full,
            PageHeader {
                title: "Alle Metriken",
                description: if searching { format!("{hits} von {total} Diagrammen passen") } else { format!("{total} Diagramme in {n_sections} Bereichen") },
            }
            div { class: "mb-5 flex flex-wrap items-center gap-2",
                div { class: "relative min-w-0 flex-1 sm:max-w-sm",
                    icons::Search { class: "pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" }
                    input {
                        class: "h-9 w-full rounded-md border bg-card pr-8 pl-8 text-sm outline-none placeholder:text-muted-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50",
                        r#type: "search",
                        placeholder: "Diagramme, Geräte, Container filtern …",
                        "aria-label": "Diagramme filtern",
                        value: "{q}",
                        oninput: move |e| query.set(e.value()),
                    }
                    if searching {
                        button {
                            r#type: "button",
                            class: "absolute top-1/2 right-1.5 grid size-6 -translate-y-1/2 place-items-center rounded text-muted-foreground hover:text-foreground [&_svg]:size-3.5",
                            "aria-label": "Suche leeren",
                            onclick: move |_| query.set(String::new()),
                            icons::X {}
                        }
                    }
                }
                label { class: "inline-flex h-9 cursor-pointer items-center gap-2 rounded-md border bg-card px-3 text-xs text-muted-foreground select-none",
                    input {
                        r#type: "checkbox",
                        class: "accent-foreground",
                        checked: retired(),
                        onchange: move |e| retired.set(e.checked()),
                    }
                    "Stillgelegte zeigen"
                }
            }
            if sections.is_empty() {
                EmptyState {
                    title: if searching { "Nichts gefunden" } else { "Noch keine Diagramme" },
                    description: if searching { "Kein Diagramm passt zur Suche." } else { "Sobald die Sammler Werte liefern, erscheinen sie hier." },
                    icon: rsx! { icons::ChartLine {} },
                }
            } else {
                div { class: "lg:grid lg:grid-cols-[11rem_1fr] lg:gap-8",
                    // Bereiche: am Desktop links und mitlaufend, am Handy als waagrechte Leiste.
                    nav { class: "-mx-4 mb-5 flex gap-1.5 overflow-x-auto px-4 pb-1 [scrollbar-width:none] lg:sticky lg:top-16 lg:mx-0 lg:mb-0 lg:flex-col lg:self-start lg:overflow-visible lg:px-0 [&::-webkit-scrollbar]:hidden",
                        "aria-label": "Bereiche",
                        for (s, n) in sections.iter().cloned() {
                            button {
                                key: "{s}",
                                r#type: "button",
                                class: "flex shrink-0 items-center justify-between gap-3 rounded-md px-2.5 py-1.5 text-left text-sm outline-none transition-colors focus-visible:ring-[3px] focus-visible:ring-ring/50 max-lg:border max-lg:text-xs",
                                class: if active.as_deref() == Some(s.as_str()) { "bg-accent font-medium text-accent-foreground" } else { "text-muted-foreground hover:bg-accent/50 hover:text-foreground max-lg:bg-card" },
                                "aria-current": if active.as_deref() == Some(s.as_str()) { "true" } else { "false" },
                                onclick: {
                                    let s = s.clone();
                                    move |_| {
                                        current.set(Some(s.clone()));
                                        if let Some(main) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.get_element_by_id("app-main")) {
                                            main.set_scroll_top(0);
                                        }
                                    }
                                },
                                span { class: "truncate", "{catalog::section_title(&s)}" }
                                span { class: "text-xs text-muted-foreground tabular-nums", "{n}" }
                            }
                        }
                    }
                    div { class: "min-w-0",
                        if let Some(section) = active {
                            div { key: "{section}",
                                h2 { class: "mb-3 text-sm font-semibold", "{catalog::section_title(&section)}" }
                                SectionCharts { section, filter: q.clone(), retired: retired(), collapsible: true }
                            }
                        }
                    }
                }
            }
        }
    }
}
