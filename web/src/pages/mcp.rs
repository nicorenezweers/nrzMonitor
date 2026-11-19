//! Einstellungen des MCP-Servers: Adresse, Schlüssel, Schalter und Beispiele zum Verbinden.

use dioxus::prelude::*;
use shared::mcp::{McpToken, McpTokenReq, McpUpdate, McpView};

use crate::{
    api,
    components::{
        ui::{
            app_shell::{SettingsGroup, SettingsRow},
            button::{Button, ButtonSize, ButtonVariant},
            checkbox::Checkbox,
            toast::{ToastType, show_toast},
        },
        widgets::{StatusDot, Tone},
    },
};

fn origin() -> String {
    web_sys::window().and_then(|w| w.location().origin().ok()).unwrap_or_default()
}

fn copy(text: &str) {
    if let Some(w) = web_sys::window() {
        let _ = w.navigator().clipboard().write_text(text);
        show_toast(ToastType::Success, "In die Zwischenablage kopiert");
    }
}

/// Text in Festbreitenschrift mit Knopf zum Kopieren.
#[component]
fn CopyLine(#[props(into)] text: String) -> Element {
    let t = text.clone();
    rsx! {
        div { class: "flex w-full min-w-0 items-center gap-2",
            code { class: "w-44 min-w-0 truncate sm:w-80 lg:w-96 rounded-md bg-muted px-2 py-1 font-mono text-xs", title: "{text}", "{text}" }
            Button { variant: ButtonVariant::Outline, size: ButtonSize::Sm, onclick: move |_| copy(&t), "Kopieren" }
        }
    }
}

/// Abschnitt der Einstellungsseite.
#[component]
pub fn McpSettings() -> Element {
    let mut view = use_signal(|| None::<McpView>);
    let mut error = use_signal(|| None::<String>);
    // Klartext erst nach „Anzeigen“ oder „Neu erzeugen“.
    let mut token = use_signal(|| None::<String>);
    let _ = use_resource(move || async move {
        match api::get::<McpView>("/mcp").await {
            Ok(v) => view.set(Some(v)),
            Err(e) => error.set(Some(e)),
        }
    });

    let save = move |u: McpUpdate| {
        spawn(async move {
            match api::put::<McpUpdate, McpView>("/mcp", &u).await {
                Ok(v) => {
                    view.set(Some(v));
                    show_toast(ToastType::Success, "MCP-Einstellungen gespeichert");
                }
                Err(e) => show_toast(ToastType::Error, format!("Nicht gespeichert: {e}")),
            }
        });
    };
    let fetch_token = move |action: &'static str| {
        spawn(async move {
            match api::post::<McpTokenReq, McpToken>("/mcp/token", &McpTokenReq { action: action.to_string() }).await {
                Ok(t) => {
                    token.set(Some(t.token));
                    if action == "new" {
                        show_toast(ToastType::Success, "Neuer Schlüssel – bestehende Verbindungen müssen ihn übernehmen");
                        if let Ok(v) = api::get::<McpView>("/mcp").await {
                            view.set(Some(v));
                        }
                    }
                }
                Err(e) => show_toast(ToastType::Error, e),
            }
        });
    };

    let Some(v) = view.read().clone() else {
        return rsx! {
            SettingsGroup { title: "MCP-Server & Dashboards",
                SettingsRow { label: if error.read().is_some() { "Nicht geladen" } else { "Lade …" }, hint: error.read().clone().unwrap_or_default(), span {} }
            }
        };
    };
    let base = origin();
    let url = format!("{base}/mcp");
    let shown = token.read().clone();
    let tok = shown.clone().unwrap_or_else(|| "<SCHLÜSSEL>".to_string());
    let masked = shown.clone().unwrap_or_else(|| format!("••••••••••••{}", v.token_hint));
    let internal: Vec<String> = v.bind.iter().map(|b| format!("http://{}/mcp", b.replace("0.0.0.0", "<host>"))).collect();
    let claude = format!("claude mcp add --transport http nrzmonitor {url} --header \"Authorization: Bearer {tok}\"");
    let json_cfg = format!("{{\"mcpServers\": {{\"nrzmonitor\": {{\"type\": \"http\", \"url\": \"{url}\", \"headers\": {{\"Authorization\": \"Bearer {tok}\"}}}}}}}}");
    let stream = format!("{base}/mcp/stream?section=system&snaps=docker.containers&events=1&token={tok}");
    let rest = format!("curl -H \"Authorization: Bearer {tok}\" {base}/mcp/api/v1/snap/docker.containers");
    let names: Vec<String> = v.tools.iter().map(|t| t.name.clone()).collect();
    let (enabled, allow_write) = (v.enabled, v.allow_write);
    let footer = format!(
        "Protokoll {} (Streamable HTTP) mit Werkzeugen, Ressourcen zum Abonnieren, Prompts und Live-Benachrichtigungen. \
         Zugang nur mit Schlüssel (Authorization: Bearer, X-Api-Key oder ?token=). Andere Container im selben Netz: {}.",
        v.protocol,
        internal.join(", ")
    );

    rsx! {
        SettingsGroup {
            title: "MCP-Server & Dashboards",
            footer,
            actions: rsx! {
                StatusDot {
                    tone: if enabled { Tone::Good } else { Tone::Neutral },
                    label: if enabled { format!("{} Sitzungen · {} Ströme", v.sessions, v.streams) } else { "aus".to_string() },
                }
            },
            SettingsRow { label: "MCP anbieten", hint: "Werkzeuge, Live-Strom und API unter /mcp",
                Checkbox { checked: enabled, aria_label: "MCP anbieten", on_change: move |c: bool| save(McpUpdate { enabled: c, allow_write }) }
            }
            SettingsRow { label: "Ändern erlauben", hint: "Auch Einstellungen und Alarmregeln über MCP ändern (sonst nur lesen)",
                Checkbox { checked: allow_write, aria_label: "Ändern erlauben", on_change: move |c: bool| save(McpUpdate { enabled, allow_write: c }) }
            }
            SettingsRow { label: "Adresse", hint: "Für MCP-Programme (Claude, Cursor, n8n …)",
                CopyLine { text: url.clone() }
            }
            SettingsRow { label: "Schlüssel", hint: if v.token_env { "Aus NRZMON_MCP_TOKEN" } else { "Neu erzeugen trennt alle bestehenden Verbindungen" },
                div { class: "flex w-full min-w-0 flex-wrap items-center gap-2",
                    code { class: "w-44 min-w-0 truncate sm:w-80 lg:w-96 rounded-md bg-muted px-2 py-1 font-mono text-xs", "{masked}" }
                    if let Some(t) = shown.clone() {
                        Button { variant: ButtonVariant::Outline, size: ButtonSize::Sm, onclick: move |_| copy(&t), "Kopieren" }
                    } else {
                        Button { variant: ButtonVariant::Outline, size: ButtonSize::Sm, onclick: move |_| fetch_token("reveal"), "Anzeigen" }
                    }
                    if !v.token_env {
                        Button { variant: ButtonVariant::Outline, size: ButtonSize::Sm, onclick: move |_| fetch_token("new"), "Neu erzeugen" }
                    }
                }
            }
            SettingsRow { label: "Claude Code", hint: "Befehl zum Verbinden",
                CopyLine { text: claude }
            }
            SettingsRow { label: "Konfiguration", hint: "JSON für Claude Desktop, Cursor und andere",
                CopyLine { text: json_cfg }
            }
            SettingsRow { label: "Ohne HTTP (stdio)", hint: "Brücke im Container, findet Adresse und Schlüssel selbst",
                CopyLine { text: "docker exec -i nrz-monitor_app_1 /app/nrzmonitor mcp" }
            }
            SettingsRow { label: "Live-Strom", hint: "Server-Sent Events für Dashboards (EventSource): tick, snap, event",
                CopyLine { text: stream }
            }
            SettingsRow { label: "REST-API", hint: "Die ganze API unter /mcp/api/v1 mit dem Schlüssel",
                CopyLine { text: rest }
            }
            SettingsRow { label: "Werkzeuge", hint: names.join(", "),
                span { class: "text-sm text-muted-foreground tabular-nums", "{names.len()}" }
            }
        }
    }
}
