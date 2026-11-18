//! DNS: Übersicht und Abfrageprotokoll (nrzNetwork über MCP).

pub mod common;
mod queries;

use dioxus::prelude::*;
use shared::{
    catalog::dns::*,
    dns::{DnsStatus, DnsTop, QueryFilter},
    model::Unit,
};

pub use self::queries::DnsQueries;
use self::common::{BarList, ListCard, NotConfigured, Notice, pct, range_params, use_api, use_status};
use crate::{
    api,
    components::{
        chart::{ChartCard, section::ChartGrid},
        ui::{
            app_shell::{LoadingState, PageContainer, PageHeader, PageWidth},
            badge::{Badge, BadgeVariant},
            button::{Button, ButtonVariant},
            table::{Table, TableBody, TableCell, TableHead, TableHeader, TableRow},
        },
        widgets::{StatTile, StatusDot, Tone},
    },
    fmt,
    route::Route,
    state::use_app,
};

/// Filter, mit dem das Abfrageprotokoll öffnet (Klick auf einen Client oder eine Domain).
pub static QUERY_FILTER: GlobalSignal<QueryFilter> = Signal::global(QueryFilter::default);

/// Zum Abfrageprotokoll mit Filter springen.
pub fn open_queries(f: QueryFilter) {
    *QUERY_FILTER.write() = f;
    navigator().push(Route::DnsQueries {});
}

#[component]
pub fn Dns() -> Element {
    let status = use_status::<DnsStatus>("dns.status");
    let app = use_app();
    let (top, err, _) = use_api::<DnsTop>(
        move || {
            let mut p = range_params(app);
            p.push(("limit", "10".into()));
            api::qs("/dns/top", &p)
        },
        30_000,
    );
    let Some(st) = status() else {
        return rsx! {
            PageContainer { width: PageWidth::Full,
                PageHeader { title: "DNS", description: "Alle Anfragen im Netzwerk" }
                LoadingState {}
            }
        };
    };
    let actions = rsx! {
        Button { variant: ButtonVariant::Outline, onclick: move |_| open_queries(QueryFilter::default()),
            icons::ListFilter { class: "size-4" }
            "Abfrageprotokoll"
        }
    };
    if !st.configured && !st.demo {
        return rsx! {
            PageContainer { width: PageWidth::Full,
                PageHeader { title: "DNS", description: "Alle Anfragen im Netzwerk" }
                NotConfigured {
                    title: "DNS ist noch nicht verbunden",
                    description: "Hinterlege in den Einstellungen das MCP-Token von nrzNetwork – danach wird jede DNS-Anfrage im Netzwerk mitgeschrieben.",
                    icon: rsx! { icons::Globe {} },
                }
            }
        };
    }
    let t = top().unwrap_or_default();
    let blocklists = st.blocklists.clone();
    let upstreams = st.upstreams.clone();
    let upstream_charts = app.by_context(DNS_UPSTREAM_LATENCY.context);
    let state_line = if st.demo {
        rsx! { Badge { variant: BadgeVariant::Secondary, "Beispieldaten" } }
    } else if st.connected {
        rsx! { StatusDot { tone: Tone::Good, label: format!("Verbunden · nrzNetwork {} · zuletzt {}", st.version, fmt::ago_ms(st.last_poll)) } }
    } else {
        rsx! { StatusDot { tone: Tone::Bad, label: "Getrennt – Protokoll wird nach der Rückkehr nachgeholt" } }
    };
    rsx! {
        PageContainer { width: PageWidth::Full,
            PageHeader { title: "DNS", description: "Alle Anfragen im Netzwerk über nrzNetwork", actions }
            div { class: "grid grid-cols-1 gap-6",
                div { class: "flex flex-wrap items-center gap-x-4 gap-y-2 text-sm",
                    {state_line}
                    if st.catching_up {
                        Badge { variant: BadgeVariant::Warning, "Holt Rückstand auf" }
                    }
                    if !st.blocking_enabled && st.version.len() > 0 {
                        Badge { variant: BadgeVariant::Warning, "Sperren aus" }
                    }
                    if let Some(p) = st.paused_until {
                        Badge { variant: BadgeVariant::Warning, {format!("Sperren pausiert bis {}", fmt::clock(p))} }
                    }
                }
                if let Some(e) = st.error.clone() {
                    Notice { text: e, error: true }
                }
                if let Some(e) = err() {
                    Notice { text: e, error: true }
                }
                div { class: "grid grid-cols-2 gap-4 md:grid-cols-3 xl:grid-cols-6",
                    StatTile { label: "Anfragen", value: Unit::Count.format(t.total as f64), hint: "im Zeitraum" }
                    StatTile { label: "Gesperrt", value: pct(t.blocked, t.total), hint: format!("{} Anfragen", Unit::Count.format(t.blocked as f64)) }
                    StatTile { label: "Aus dem Cache", value: pct(t.cached, t.total), hint: format!("{} Einträge im Cache", Unit::Count.format(st.cache_entries as f64)) }
                    StatTile { label: "Clients", value: Unit::Count.format(t.clients_count as f64), hint: format!("{} Domains", Unit::Count.format(t.domains_count as f64)) }
                    StatTile { label: "Antwortzeit", value: if t.total == 0 { "–".to_string() } else { Unit::Millis.format(t.avg_ms) }, hint: format!("{} Fehler", Unit::Count.format(t.failed as f64)) }
                    StatTile { label: "Sperrliste", value: Unit::Count.format(st.blocked_domains as f64), hint: format!("{} Listen", blocklists.iter().filter(|l| l.enabled).count()) }
                }
                ChartGrid {
                    ChartCard { id: DNS_QUERIES.context, height: 220 }
                    ChartCard { id: DNS_BLOCKED_PCT.context, height: 220 }
                    ChartCard { id: DNS_LATENCY.context, height: 220 }
                }
                div { class: "grid gap-4 md:grid-cols-2 2xl:grid-cols-4",
                    ListCard { title: "Aktivste Clients", hint: "Klick zeigt die Anfragen",
                        BarList { items: t.clients.clone(), show_blocked: true,
                            onpick: move |c: String| open_queries(QueryFilter { client: c, ..QueryFilter::default() }) }
                    }
                    ListCard { title: "Häufigste Domains",
                        BarList { items: t.domains.clone(), show_blocked: true,
                            onpick: move |n: String| open_queries(QueryFilter { name: n, ..QueryFilter::default() }) }
                    }
                    ListCard { title: "Gesperrte Domains",
                        BarList { items: t.blocked_domains.clone(), show_blocked: true, empty: "Nichts gesperrt",
                            onpick: move |n: String| open_queries(QueryFilter { name: n, kind: "blocked".into(), ..QueryFilter::default() }) }
                    }
                    ListCard { title: "Abfragetypen",
                        BarList { items: t.qtypes.clone(),
                            onpick: move |q: String| open_queries(QueryFilter { qtype: q, ..QueryFilter::default() }) }
                    }
                }
                ChartGrid {
                    ChartCard { id: DNS_RCODES.context }
                    ChartCard { id: DNS_QTYPES.context }
                    ChartCard { id: DNS_CLIENT_QUERIES.context }
                    ChartCard { id: DNS_CLIENTS.context }
                    ChartCard { id: DNS_CACHE.context }
                    ChartCard { id: DNS_BLOCKLIST.context }
                }
                div { class: "grid gap-4 xl:grid-cols-2",
                    ListCard { title: "Weiterleitungen", hint: "Vorgelagerte Server laut nrzNetwork (seit dem Start)",
                        if upstreams.is_empty() {
                            p { class: "py-6 text-center text-sm text-muted-foreground", "Keine Weiterleitungen gemeldet" }
                        } else {
                            div { class: "overflow-x-auto",
                                Table {
                                    TableHeader {
                                        TableRow {
                                            TableHead { "Server" }
                                            TableHead { class: "text-right", "Antworten" }
                                            TableHead { class: "text-right", "Fehler" }
                                            TableHead { class: "text-right", "Mittel" }
                                        }
                                    }
                                    TableBody {
                                        for u in upstreams.iter() {
                                            TableRow { key: "{u.upstream}",
                                                TableCell { class: "px-2 py-2",
                                                    button { class: "text-left hover:underline",
                                                        onclick: { let up = u.upstream.clone(); move |_| open_queries(QueryFilter { upstream: up.clone(), ..QueryFilter::default() }) },
                                                        p { class: "font-medium", "{u.name}" }
                                                        p { class: "text-xs text-muted-foreground", "{u.upstream}" }
                                                    }
                                                    if let Some(e) = &u.last_error {
                                                        p { class: "text-xs text-warning",
                                                            {format!("{e}{}", u.last_error_at.map(|t| format!(" · {}", fmt::ago(t))).unwrap_or_default())}
                                                        }
                                                    }
                                                }
                                                TableCell { class: "px-2 py-2 text-right tabular-nums", {Unit::Count.format(u.ok as f64)} }
                                                TableCell { class: "px-2 py-2 text-right tabular-nums", {pct(u.failed, u.ok + u.failed)} }
                                                TableCell { class: "px-2 py-2 text-right tabular-nums", {Unit::Millis.format(u.avg_ms)} }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    ListCard { title: "Sperrlisten",
                        hint: format!("{} gesperrte Domains · {} eigene Sperren · {} Ausnahmen", Unit::Count.format(st.blocked_domains as f64), st.custom_blocked, st.allowed),
                        if blocklists.is_empty() {
                            p { class: "py-6 text-center text-sm text-muted-foreground", "Keine Sperrlisten" }
                        } else {
                            ul { class: "divide-y",
                                for l in blocklists.iter() {
                                    li { key: "{l.url}", class: "flex items-center justify-between gap-3 px-2 py-2",
                                        div { class: "min-w-0",
                                            p { class: "truncate text-sm font-medium", "{l.name}" }
                                            p { class: "truncate text-xs text-muted-foreground", title: "{l.url}", "{l.url}" }
                                            if let Some(e) = &l.error {
                                                p { class: "text-xs text-destructive", {format!("Fehler: {e}")} }
                                            }
                                        }
                                        div { class: "shrink-0 text-right",
                                            p { class: "text-sm tabular-nums", {Unit::Count.format(l.domains as f64)} }
                                            p { class: "text-xs text-muted-foreground",
                                                if !l.enabled { "aus" } else if let Some(u) = l.updated_at { {fmt::ago(u)} } else { "–" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                if !upstream_charts.is_empty() {
                    ChartGrid {
                        for c in upstream_charts.iter() {
                            ChartCard { key: "{c.id}", id: c.id.clone(), title: format!("Antwortzeit · {}", c.family) }
                        }
                    }
                }
                p { class: "text-xs text-muted-foreground",
                    {format!(
                        "Gespeichert: {} Anfragen · {} · {:.1} Bytes je Anfrage · Aufbewahrung {} Tage{}",
                        Unit::Count.format(st.stored as f64),
                        Unit::Bytes.format(st.stored_bytes as f64),
                        st.bytes_per_query,
                        st.retention_days,
                        if st.synced_until > 0 { format!(" · lückenlos bis {}", fmt::clock(st.synced_until)) } else { String::new() },
                    )}
                }
            }
        }
    }
}
