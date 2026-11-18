//! Seite „NetBird“: Peers, Bestand und Aktivität.

use dioxus::prelude::*;
use shared::{
    catalog::netbird::*,
    netbird::{NetbirdStatus, Peer},
};

use super::dns::common::{ListCard, NotConfigured, Notice, use_status};
use crate::{
    components::{
        chart::{ChartCard, section::ChartGrid},
        ui::{
            app_shell::{LoadingState, PageContainer, PageHeader, PageWidth},
            badge::{Badge, BadgeVariant},
            input::Input,
        },
        widgets::{StatTile, StatusDot, Tone},
    },
    fmt,
};

#[component]
pub fn Netbird() -> Element {
    let status = use_status::<NetbirdStatus>("netbird.status");
    let peers = use_status::<Vec<Peer>>("netbird.peers");
    let mut search = use_signal(String::new);
    let Some(st) = status() else {
        return rsx! {
            PageContainer { width: PageWidth::Full,
                PageHeader { title: "NetBird", description: "Peers, Gruppen, Regeln und Aktivität" }
                LoadingState {}
            }
        };
    };
    if !st.configured && !st.demo {
        return rsx! {
            PageContainer { width: PageWidth::Full,
                PageHeader { title: "NetBird", description: "Peers, Gruppen, Regeln und Aktivität" }
                NotConfigured {
                    title: "NetBird ist noch nicht verbunden",
                    description: "Hinterlege ein Zugangs-Token der Management-API – dann erscheinen hier alle Peers, ob sie verbunden sind und was sich tut.",
                    icon: rsx! { icons::Waypoints {} },
                }
            }
        };
    }
    let q = search().to_lowercase();
    let list: Vec<Peer> = peers()
        .unwrap_or_default()
        .into_iter()
        .filter(|p| {
            q.is_empty()
                || p.name.to_lowercase().contains(&q)
                || p.ip.contains(&q)
                || p.os.to_lowercase().contains(&q)
                || p.groups.iter().any(|g| g.to_lowercase().contains(&q))
        })
        .collect();
    let inv = st.inventory.clone();
    rsx! {
        PageContainer { width: PageWidth::Full,
            PageHeader { title: "NetBird", description: "Peers, Gruppen, Regeln und Aktivität im NetBird-Netz" }
            div { class: "grid grid-cols-1 gap-6",
                div { class: "flex flex-wrap items-center gap-x-4 gap-y-2 text-sm",
                    if st.demo {
                        Badge { variant: BadgeVariant::Secondary, "Beispieldaten" }
                    } else if st.ok {
                        StatusDot { tone: Tone::Good, label: format!("Verbunden mit {} · zuletzt {}", st.url, fmt::ago_ms(st.last_poll)) }
                    } else {
                        StatusDot { tone: Tone::Bad, label: "Keine Verbindung" }
                    }
                }
                if let Some(e) = st.error.clone() {
                    Notice { text: e, error: true }
                }
                for h in st.hints.iter() {
                    Notice { key: "{h}", text: h.clone() }
                }
                div { class: "grid grid-cols-2 gap-4 md:grid-cols-3 xl:grid-cols-6",
                    StatTile { label: "Peers", value: format!("{}", st.peers), hint: format!("{} verbunden", st.connected) }
                    StatTile { label: "Getrennt", value: format!("{}", st.peers.saturating_sub(st.connected)) }
                    StatTile { label: "Benutzer", value: format!("{}", inv.users) }
                    StatTile { label: "Gruppen", value: format!("{}", inv.groups) }
                    StatTile { label: "Regeln", value: format!("{}", inv.policies), hint: format!("{} aktiv", st.policies.iter().filter(|p| p.1).count()) }
                    StatTile { label: "Routen und Netzwerke", value: format!("{}", inv.routes + inv.networks), hint: format!("{} Setup-Schlüssel", inv.setup_keys) }
                }
                ChartGrid {
                    ChartCard { id: NETBIRD_PEERS.context, height: 180 }
                    ChartCard { id: NETBIRD_EVENTS.context, height: 180 }
                    ChartCard { id: NETBIRD_INVENTORY.context, height: 180 }
                }
                div { class: "rounded-xl border bg-card shadow-sm",
                    div { class: "flex flex-wrap items-center justify-between gap-2 border-b px-4 py-3",
                        h3 { class: "text-sm font-semibold", "Peers" }
                        div { class: "w-full sm:w-64", Input { placeholder: "Suchen (Name, IP, OS, Gruppe)", value: search(), oninput: move |e: FormEvent| search.set(e.value()) } }
                    }
                    div { class: "overflow-x-auto",
                        table { class: "w-full text-sm",
                            thead { class: "border-b text-left text-xs text-muted-foreground",
                                tr {
                                    th { class: "px-3 py-2 font-medium", "Peer" }
                                    th { class: "px-3 py-2 font-medium", "Zustand" }
                                    th { class: "hidden px-3 py-2 font-medium md:table-cell", "IP" }
                                    th { class: "hidden px-3 py-2 font-medium lg:table-cell", "System" }
                                    th { class: "hidden px-3 py-2 font-medium xl:table-cell", "Gruppen" }
                                    th { class: "hidden px-3 py-2 font-medium lg:table-cell", "Ort" }
                                }
                            }
                            tbody { class: "divide-y",
                                for p in list.iter() {
                                    tr { key: "{p.id}", class: "hover:bg-muted/40",
                                        td { class: "px-3 py-2",
                                            p { class: "font-medium", "{p.name}" }
                                            p { class: "text-xs text-muted-foreground", {if p.dns_label.is_empty() { p.hostname.clone() } else { p.dns_label.clone() }} }
                                            p { class: "text-xs text-muted-foreground tabular-nums md:hidden", "{p.ip}" }
                                        }
                                        td { class: "px-3 py-2 whitespace-nowrap",
                                            if p.connected {
                                                StatusDot { tone: Tone::Good, label: "Verbunden" }
                                            } else if p.login_expired {
                                                StatusDot { tone: Tone::Warn, label: "Anmeldung abgelaufen" }
                                            } else {
                                                StatusDot { tone: Tone::Neutral, label: "Getrennt" }
                                            }
                                            if !p.connected && p.last_seen > 0 {
                                                p { class: "text-xs text-muted-foreground", {format!("zuletzt {}", fmt::ago(p.last_seen))} }
                                            }
                                        }
                                        td { class: "hidden px-3 py-2 tabular-nums md:table-cell",
                                            p { "{p.ip}" }
                                            if !p.connection_ip.is_empty() {
                                                p { class: "text-xs text-muted-foreground", "{p.connection_ip}" }
                                            }
                                        }
                                        td { class: "hidden px-3 py-2 lg:table-cell",
                                            p { "{p.os}" }
                                            p { class: "text-xs text-muted-foreground", {format!("NetBird {}", p.version)} }
                                        }
                                        td { class: "hidden px-3 py-2 xl:table-cell",
                                            div { class: "flex flex-wrap gap-1",
                                                for g in p.groups.iter() {
                                                    Badge { key: "{g}", variant: BadgeVariant::Outline, class: "font-normal", "{g}" }
                                                }
                                            }
                                        }
                                        td { class: "hidden px-3 py-2 text-muted-foreground lg:table-cell",
                                            {[p.city.clone(), p.country.clone()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ")}
                                        }
                                    }
                                }
                                if list.is_empty() {
                                    tr { td { colspan: "6", class: "px-3 py-8 text-center text-muted-foreground", "Keine Peers" } }
                                }
                            }
                        }
                    }
                }
                div { class: "grid gap-4 lg:grid-cols-3",
                    div { class: "lg:col-span-2",
                        ListCard { title: "Aktivität", hint: "Aktivitätsprotokoll der Management-API, neueste zuerst",
                            if st.activity.is_empty() {
                                p { class: "py-6 text-center text-sm text-muted-foreground", "Keine Einträge" }
                            }
                            ul { class: "divide-y",
                                for a in st.activity.iter().take(30) {
                                    li { key: "{a.id}", class: "flex items-start justify-between gap-3 px-2 py-2",
                                        div { class: "min-w-0",
                                            p { class: "truncate text-sm", "{a.activity}" }
                                            p { class: "truncate text-xs text-muted-foreground",
                                                {[a.target.clone(), a.initiator.clone()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · von ")}
                                            }
                                        }
                                        span { class: "shrink-0 text-xs tabular-nums text-muted-foreground", title: "{fmt::full(a.ts)}", {fmt::ago(a.ts)} }
                                    }
                                }
                            }
                        }
                    }
                    div { class: "grid content-start gap-4",
                        ListCard { title: "Gruppen",
                            ul { class: "divide-y",
                                for (g, n) in st.groups.iter() {
                                    li { key: "{g}", class: "flex justify-between gap-2 px-2 py-1.5 text-sm",
                                        span { class: "truncate", "{g}" }
                                        span { class: "tabular-nums text-muted-foreground", {format!("{n} Peers")} }
                                    }
                                }
                            }
                        }
                        ListCard { title: "Regeln",
                            ul { class: "divide-y",
                                for (p, on) in st.policies.iter() {
                                    li { key: "{p}", class: "flex justify-between gap-2 px-2 py-1.5 text-sm",
                                        span { class: "truncate", "{p}" }
                                        span { class: "text-xs text-muted-foreground", if *on { "aktiv" } else { "aus" } }
                                    }
                                }
                            }
                        }
                        ListCard { title: "Routen, Netzwerke, DNS",
                            ul { class: "grid gap-1 px-2 py-1 text-sm",
                                for r in st.routes.iter() { li { key: "r-{r}", class: "truncate", {format!("Route · {r}")} } }
                                for n in st.networks.iter() { li { key: "n-{n}", class: "truncate", {format!("Netzwerk · {n}")} } }
                                for d in st.nameservers.iter() { li { key: "d-{d}", class: "truncate", {format!("DNS · {d}")} } }
                            }
                        }
                    }
                }
            }
        }
    }
}
