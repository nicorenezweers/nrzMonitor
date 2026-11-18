//! Seite „Erreichbarkeit“: Karten je Ziel (Ping, TCP, HTTP) und Ziele verwalten.

use dioxus::prelude::*;
use shared::{
    catalog::probes::*,
    dns::{IntegrationsView, TestResult},
    model::Unit,
    probes::{ProbeKind, ProbeSettings, ProbeState, ProbeTarget, ProbesStatus},
};

use super::dns::common::{Notice, use_status};
use crate::{
    api,
    components::{
        chart::ChartCard,
        ui::{
            app_shell::{LoadingState, PageContainer, PageHeader, PageSection, PageWidth},
            badge::{Badge, BadgeVariant},
            button::{Button, ButtonSize, ButtonVariant},
            checkbox::Checkbox,
            input::{Input, InputType},
        },
        widgets::{StatusDot, Tone},
    },
    fmt,
};

const SELECT: &str = "h-9 rounded-md border border-input bg-transparent px-2 text-sm text-foreground outline-none focus-visible:border-ring dark:bg-input/30";

#[component]
pub fn Probes() -> Element {
    let status = use_status::<ProbesStatus>("probes.status");
    let Some(st) = status() else {
        return rsx! {
            PageContainer { width: PageWidth::Full,
                PageHeader { title: "Erreichbarkeit", description: "Ping, TCP und HTTP zu wichtigen Zielen" }
                LoadingState {}
            }
        };
    };
    let up = st.targets.iter().filter(|t| t.up).count();
    rsx! {
        PageContainer { width: PageWidth::Full,
            PageHeader { title: "Erreichbarkeit", description: "Ping, TCP und HTTP zu wichtigen Zielen – alle 10 s (HTTP alle 30 s)" }
            div { class: "grid grid-cols-1 gap-6",
                div { class: "flex flex-wrap items-center gap-x-4 gap-y-2 text-sm",
                    if st.demo {
                        Badge { variant: BadgeVariant::Secondary, "Beispieldaten" }
                    }
                    StatusDot { tone: if up == st.targets.len() { Tone::Good } else { Tone::Bad }, label: format!("{up} von {} Zielen erreichbar", st.targets.len()) }
                }
                if let Some(e) = st.icmp_error.clone() {
                    Notice { text: e }
                }
                if st.targets.is_empty() {
                    p { class: "text-sm text-muted-foreground", "Keine Ziele – unten eines hinzufügen." }
                }
                div { class: "grid gap-4 md:grid-cols-2 2xl:grid-cols-3",
                    for t in st.targets.iter() {
                        TargetCard { key: "{t.id}", t: t.clone() }
                    }
                }
                ManageTargets {}
            }
        }
    }
}

#[component]
fn TargetCard(t: ProbeState) -> Element {
    let (tone, label) = if t.pending {
        (Tone::Neutral, "Wird geprüft")
    } else if t.up {
        (Tone::Good, "Erreichbar")
    } else {
        (Tone::Bad, "Ausgefallen")
    };
    let extra = match (t.kind, t.status) {
        (ProbeKind::Http, Some(s)) => format!("HTTP {s}"),
        _ => format!("{} Verlust", Unit::Percent.format(t.loss)),
    };
    rsx! {
        div { class: "flex min-w-0 flex-col gap-3 rounded-xl border bg-card p-4 shadow-sm",
            div { class: "flex items-start justify-between gap-3",
                div { class: "min-w-0",
                    div { class: "flex items-center gap-2",
                        p { class: "truncate font-medium", "{t.name}" }
                        Badge { variant: BadgeVariant::Outline, class: "font-normal", {t.kind.label()} }
                    }
                    p { class: "truncate text-xs text-muted-foreground", title: "{t.target}", "{t.target}" }
                }
                div { class: "shrink-0 text-right",
                    StatusDot { tone, label }
                    if t.since > 0 {
                        p { class: "text-xs text-muted-foreground", {format!("seit {}", fmt::ago(t.since))} }
                    }
                }
            }
            div { class: "flex items-baseline gap-4",
                span { class: "text-2xl font-semibold tabular-nums", {t.latency_ms.map_or("–".to_string(), |v| Unit::Millis.format(v))} }
                span { class: "text-xs text-muted-foreground tabular-nums", "{extra}" }
                span { class: "ml-auto text-xs text-muted-foreground tabular-nums", {format!("{} verfügbar", Unit::Percent.format(t.uptime_pct))} }
            }
            if let Some(e) = &t.error {
                p { class: "truncate text-xs text-destructive", title: "{e}", "{e}" }
            }
            ChartCard { id: PROBE_LATENCY.chart_id(Some(&t.id)), height: 90, compact: true, title: "Antwortzeit" }
            if t.kind == ProbeKind::Http {
                ChartCard { id: PROBE_HTTP.chart_id(Some(&t.id)), height: 90, title: "Zeitanteile" }
            }
        }
    }
}

#[component]
fn ManageTargets() -> Element {
    let mut settings = use_signal(|| None::<ProbeSettings>);
    let mut msg = use_signal(|| None::<(bool, String)>);
    let _ = use_resource(move || async move {
        if let Ok(v) = api::get::<IntegrationsView>("/integrations").await {
            settings.set(Some(v.probes));
        }
    });
    let mut name = use_signal(String::new);
    let mut kind = use_signal(|| ProbeKind::Ping);
    let mut host = use_signal(String::new);
    let mut port = use_signal(|| "443".to_string());
    let mut busy = use_signal(|| false);
    let mut result = use_signal(|| None::<TestResult>);
    let target = move || ProbeTarget {
        id: String::new(),
        name: name(),
        kind: kind(),
        host: host(),
        port: port().trim().parse().unwrap_or(0),
        enabled: true,
    };
    let mut apply = move |v: Result<IntegrationsView, String>| match v {
        Ok(v) => {
            settings.set(Some(v.probes));
            msg.set(Some((true, "Gespeichert – wirkt sofort.".into())));
        }
        Err(e) => msg.set(Some((false, e))),
    };
    let Some(s) = settings() else { return rsx! {} };
    let placeholder = match kind() {
        ProbeKind::Ping => "Rechner oder IP, z. B. nas.local",
        ProbeKind::Tcp => "Rechner oder IP",
        ProbeKind::Http => "https://beispiel.ch/gesund",
    };
    rsx! {
        PageSection { title: "Ziele verwalten", description: "Eigene Ziele kommen zu den Standardzielen dazu.",
            div { class: "grid gap-4",
                label { class: "flex cursor-pointer items-center gap-2 text-sm",
                    Checkbox { checked: s.defaults, aria_label: "Standardziele",
                        on_checked_change: move |v: bool| {
                            let mut s = settings().unwrap_or_default();
                            s.defaults = v;
                            spawn(async move { apply(api::put::<_, IntegrationsView>("/integrations/probes", &s).await) });
                        } }
                    "Standardziele prüfen (Gateway, 1.1.1.1, 9.9.9.9)"
                }
                if !s.targets.is_empty() {
                    ul { class: "divide-y rounded-lg border",
                        for t in s.targets.iter() {
                            li { key: "{t.id}", class: "flex items-center justify-between gap-3 px-3 py-2",
                                div { class: "min-w-0",
                                    p { class: "truncate text-sm font-medium", "{t.name}" }
                                    p { class: "truncate text-xs text-muted-foreground",
                                        {format!("{} · {}", t.kind.label(), if t.kind == ProbeKind::Tcp { format!("{}:{}", t.host, t.port) } else { t.host.clone() })}
                                    }
                                }
                                Button { variant: ButtonVariant::Ghost, size: ButtonSize::Sm, aria_label: "Entfernen",
                                    onclick: { let id = t.id.clone(); move |_| { let id = id.clone(); spawn(async move {
                                        let r = match api::delete(&format!("/integrations/probes/targets/{id}")).await {
                                            Ok(()) => api::get::<IntegrationsView>("/integrations").await,
                                            Err(e) => Err(e),
                                        };
                                        apply(r);
                                    }); } },
                                    icons::Trash2 { class: "size-4" }
                                    "Entfernen"
                                }
                            }
                        }
                    }
                }
                div { class: "grid gap-2 sm:grid-cols-[10rem_7rem_1fr_6rem_auto] sm:items-center",
                    Input { placeholder: "Name", value: name(), oninput: move |e: FormEvent| name.set(e.value()) }
                    select { class: SELECT, "aria-label": "Art",
                        onchange: move |e| kind.set(match e.value().as_str() { "tcp" => ProbeKind::Tcp, "http" => ProbeKind::Http, _ => ProbeKind::Ping }),
                        option { value: "ping", selected: kind() == ProbeKind::Ping, "Ping" }
                        option { value: "tcp", selected: kind() == ProbeKind::Tcp, "TCP" }
                        option { value: "http", selected: kind() == ProbeKind::Http, "HTTP(S)" }
                    }
                    Input { placeholder, value: host(), oninput: move |e: FormEvent| host.set(e.value()) }
                    if kind() == ProbeKind::Tcp {
                        Input { r#type: InputType::Number, placeholder: "Port", value: port(), oninput: move |e: FormEvent| port.set(e.value()) }
                    } else {
                        span {}
                    }
                    div { class: "flex gap-2",
                        Button { variant: ButtonVariant::Outline, disabled: busy(),
                            onclick: move |_| { spawn(async move {
                                busy.set(true);
                                let r = api::post::<_, TestResult>("/integrations/probes/test", &target()).await;
                                result.set(Some(r.unwrap_or_else(|e| TestResult { ok: false, message: e, details: vec![] })));
                                busy.set(false);
                            }); },
                            "Testen"
                        }
                        Button { disabled: busy() || host().trim().is_empty(),
                            onclick: move |_| { spawn(async move {
                                let r = api::post::<_, IntegrationsView>("/integrations/probes/targets", &target()).await;
                                if r.is_ok() { name.set(String::new()); host.set(String::new()); result.set(None); }
                                apply(r);
                            }); },
                            icons::Plus { class: "size-4" }
                            "Hinzufügen"
                        }
                    }
                }
                if let Some(r) = result() {
                    div { class: "text-sm",
                        span { class: if r.ok { "text-success" } else { "text-destructive" }, "{r.message}" }
                        for d in r.details.iter() { p { class: "text-xs text-muted-foreground", "{d}" } }
                    }
                }
                if let Some((ok, m)) = msg() {
                    p { class: if ok { "text-sm text-success" } else { "text-sm text-destructive" }, "{m}" }
                }
            }
        }
    }
}
