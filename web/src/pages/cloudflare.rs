//! Seite „Cloudflare“: Zonen (Anfragen, Datenmenge, Bedrohungen, Firewall) und Tunnel.

use dioxus::prelude::*;
use shared::{catalog::cloudflare::*, cloudflare::CfStatus, model::Unit};

use super::dns::common::{ListCard, NotConfigured, Notice, use_status};
use crate::{
    components::{
        chart::{ChartCard, section::ChartGrid},
        ui::{
            app_shell::{LoadingState, PageContainer, PageHeader, PageSection, PageWidth},
            badge::{Badge, BadgeVariant},
        },
        widgets::{StatTile, StatusDot, Tone},
    },
    fmt,
};

fn tunnel_tone(s: &str) -> (Tone, &'static str) {
    match s {
        "healthy" => (Tone::Good, "Verbunden"),
        "degraded" => (Tone::Warn, "Eingeschränkt"),
        "down" => (Tone::Bad, "Getrennt"),
        _ => (Tone::Neutral, "Inaktiv"),
    }
}

#[component]
pub fn Cloudflare() -> Element {
    let status = use_status::<CfStatus>("cloudflare.status");
    let mut selected = use_signal(|| None::<String>);
    let Some(st) = status() else {
        return rsx! {
            PageContainer { width: PageWidth::Full,
                PageHeader { title: "Cloudflare", description: "Anfragen, Datenmenge, Bedrohungen und Tunnel" }
                LoadingState {}
            }
        };
    };
    if !st.configured && !st.demo {
        return rsx! {
            PageContainer { width: PageWidth::Full,
                PageHeader { title: "Cloudflare", description: "Anfragen, Datenmenge, Bedrohungen und Tunnel" }
                NotConfigured {
                    title: "Cloudflare ist noch nicht verbunden",
                    description: "Hinterlege ein API-Token mit Leserechten für Zonen, Analytics und Tunnel – die Zonen findet nrzMonitor selbst.",
                    icon: rsx! { icons::Cloud {} },
                }
            }
        };
    }
    let total = |f: fn(&shared::cloudflare::CfZone) -> f64| st.zones.iter().map(f).sum::<f64>();
    let req = total(|z| z.requests_24h);
    let cached = total(|z| z.cached_24h);
    let bytes = total(|z| z.bytes_24h);
    let threats = total(|z| z.threats_24h) + total(|z| z.firewall_24h);
    let up = st.tunnels.iter().filter(|t| t.status == "healthy" || t.status == "degraded").count();
    let zone_names: Vec<String> = st.zones.iter().map(|z| z.name.clone()).collect();
    let current = selected().filter(|s| zone_names.contains(s)).or_else(|| zone_names.first().cloned());
    let zone = st.zones.iter().find(|z| Some(&z.name) == current.as_ref()).cloned();
    rsx! {
        PageContainer { width: PageWidth::Full,
            PageHeader { title: "Cloudflare", description: "Anfragen, Datenmenge, Bedrohungen und Tunnel – Werte je Minute, rund 5 Minuten verzögert" }
            div { class: "grid grid-cols-1 gap-6",
                div { class: "flex flex-wrap items-center gap-x-4 gap-y-2 text-sm",
                    if st.demo {
                        Badge { variant: BadgeVariant::Secondary, "Beispieldaten" }
                    } else if st.ok {
                        StatusDot { tone: Tone::Good, label: format!("Verbunden · zuletzt {}", fmt::ago_ms(st.last_poll)) }
                    } else {
                        StatusDot { tone: Tone::Bad, label: "Keine Verbindung" }
                    }
                    if st.backoff_until > fmt::now() {
                        Badge { variant: BadgeVariant::Warning, {format!("Ratenbegrenzung bis {}", fmt::clock(st.backoff_until))} }
                    }
                }
                if let Some(e) = st.error.clone() {
                    Notice { text: e, error: true }
                }
                for h in st.hints.iter() {
                    Notice { key: "{h}", text: h.clone() }
                }
                div { class: "grid grid-cols-2 gap-4 md:grid-cols-3 xl:grid-cols-5",
                    StatTile { label: "Anfragen (24 h)", value: Unit::Count.format(req), hint: format!("{} Zonen", st.zones.len()) }
                    StatTile { label: "Aus dem Cache", value: if req > 0.0 { Unit::Percent.format(cached * 100.0 / req) } else { "–".into() } }
                    StatTile { label: "Datenmenge (24 h)", value: Unit::Bytes.format(bytes) }
                    StatTile { label: "Abgewehrt (24 h)", value: Unit::Count.format(threats), hint: "Bedrohungen und Firewall" }
                    StatTile { label: "Tunnel", value: format!("{up} / {}", st.tunnels.len()), hint: "verbunden" }
                }
                if !st.tunnels.is_empty() {
                    ListCard { title: "Tunnel",
                        ul { class: "divide-y",
                            for t in st.tunnels.iter() {
                                {
                                    let (tone, label) = tunnel_tone(&t.status);
                                    rsx! {
                                        li { key: "{t.id}", class: "grid gap-2 px-2 py-2.5 sm:grid-cols-[1fr_auto_auto] sm:items-center sm:gap-6",
                                            div { class: "min-w-0",
                                                p { class: "truncate text-sm font-medium", "{t.name}" }
                                                p { class: "truncate text-xs text-muted-foreground", {format!("seit {}", fmt::ago(t.status_since))} }
                                            }
                                            StatusDot { tone, label }
                                            span { class: "text-sm tabular-nums text-muted-foreground",
                                                {format!("{} Verbindungen{}", t.connections, if t.colos.is_empty() { String::new() } else { format!(" · {}", t.colos.join(", ")) })}
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    ChartGrid {
                        for t in st.tunnels.iter() {
                            ChartCard { key: "tc-{t.id}", id: CF_TUNNEL_CONNS.chart_id(Some(&t.name)), title: format!("Tunnel {} · Verbindungen", t.name), height: 140 }
                        }
                    }
                }
                if zone_names.len() > 1 {
                    div { class: "flex flex-wrap gap-1 rounded-lg border bg-card p-1 w-fit",
                        for n in zone_names.iter() {
                            button {
                                key: "{n}",
                                class: if Some(n) == current.as_ref() { "rounded-md bg-muted px-3 py-1.5 text-sm font-medium" } else { "rounded-md px-3 py-1.5 text-sm text-muted-foreground hover:bg-muted/60" },
                                onclick: { let n = n.clone(); move |_| selected.set(Some(n.clone())) },
                                "{n}"
                            }
                        }
                    }
                }
                if let Some(z) = zone {
                    PageSection { key: "{z.id}", title: z.name.clone(),
                        description: format!("{} · {}{}", if z.plan.is_empty() { "Tarif unbekannt" } else { &z.plan }, if z.dataset.is_empty() { "keine HTTP-Analytics" } else { &z.dataset },
                            if z.last_minute > 0 { format!(" · bis {}", fmt::hm(z.last_minute)) } else { String::new() }),
                        if let Some(e) = z.error.clone() {
                            div { class: "mb-4", Notice { text: e } }
                        }
                        div { class: "grid gap-4",
                            ChartGrid {
                                ChartCard { id: CF_REQUESTS.chart_id(Some(&z.name)), height: 200 }
                                ChartCard { id: CF_BANDWIDTH.chart_id(Some(&z.name)), height: 200 }
                                ChartCard { id: CF_STATUS.chart_id(Some(&z.name)), height: 200 }
                                ChartCard { id: CF_VISITORS.chart_id(Some(&z.name)) }
                                ChartCard { id: CF_FIREWALL.chart_id(Some(&z.name)) }
                                ChartCard { id: CF_THREATS.chart_id(Some(&z.name)) }
                                ChartCard { id: CF_COUNTRIES.chart_id(Some(&z.name)) }
                            }
                            if !z.top_countries.is_empty() {
                                div { class: "flex flex-wrap gap-2 text-xs text-muted-foreground",
                                    span { "Länder (24 h):" }
                                    for (c, n) in z.top_countries.iter() {
                                        span { key: "{c}", class: "rounded-md border px-2 py-0.5 tabular-nums", {format!("{c} {}", Unit::Count.format(*n))} }
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
