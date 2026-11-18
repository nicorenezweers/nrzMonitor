//! Einstellungen der Integrationen (nrzNetwork-DNS, Cloudflare, NetBird, Erreichbarkeit).

use dioxus::prelude::*;
use shared::dns::{IntegrationState, IntegrationsView, TestResult, TokenForm};

use crate::{
    api,
    components::{
        ui::{
            app_shell::{LoadingState, SettingsGroup, SettingsRow},
            button::{Button, ButtonSize, ButtonVariant},
            checkbox::Checkbox,
            input::{Input, InputType},
        },
        widgets::{StatusDot, Tone},
    },
    fmt,
    route::Route,
};

/// Abschnitt der Einstellungsseite.
#[component]
pub fn IntegrationSettings() -> Element {
    let mut view = use_signal(|| None::<IntegrationsView>);
    let mut error = use_signal(|| None::<String>);
    let _ = use_resource(move || async move {
        match api::get::<IntegrationsView>("/integrations").await {
            Ok(v) => view.set(Some(v)),
            Err(e) => error.set(Some(e)),
        }
    });
    let Some(v) = view() else {
        return rsx! {
            SettingsGroup { title: "Integrationen",
                if let Some(e) = error() {
                    SettingsRow { label: "Nicht geladen", hint: e }
                } else {
                    LoadingState {}
                }
            }
        };
    };
    let probes = v.probes_state.clone();
    rsx! {
        TokenGroup { which: "dns", title: "DNS über nrzNetwork", form: v.dns.clone(), state: v.dns_state.clone(), demo: v.demo,
            onsaved: move |nv| view.set(Some(nv)) }
        TokenGroup { which: "cloudflare", title: "Cloudflare", form: v.cloudflare.clone(), state: v.cloudflare_state.clone(), demo: v.demo,
            onsaved: move |nv| view.set(Some(nv)) }
        TokenGroup { which: "netbird", title: "NetBird", form: v.netbird.clone(), state: v.netbird_state.clone(), demo: v.demo,
            onsaved: move |nv| view.set(Some(nv)) }
        SettingsGroup { title: "Erreichbarkeit",
            footer: "Standardziele: Gateway, 1.1.1.1 und 9.9.9.9 – eigene Ziele (Ping, TCP, HTTP) auf der Seite „Erreichbarkeit“.",
            SettingsRow { label: "Ziele", hint: probes.summary.clone(),
                Button { variant: ButtonVariant::Outline, size: ButtonSize::Sm, onclick: move |_| { navigator().push(Route::Probes {}); },
                    "Ziele verwalten"
                }
            }
        }
    }
}

struct Texts {
    url_label: &'static str,
    url_hint: &'static str,
    token_label: &'static str,
    help: &'static str,
    env: &'static str,
}

fn texts(which: &str) -> Texts {
    match which {
        "dns" => Texts {
            url_label: "MCP-Adresse",
            url_hint: "Endpunkt des MCP-Servers von nrzNetwork",
            token_label: "MCP-Token",
            help: "Token in nrzNetwork unter Einstellungen → MCP erzeugen. nrzMonitor braucht nur lesende Werkzeuge (Abfrageprotokoll, Zustand, Sperren).",
            env: "NRZMON_DNS_MCP_TOKEN",
        },
        "cloudflare" => Texts {
            url_label: "",
            url_hint: "",
            token_label: "API-Token",
            help: "dash.cloudflare.com → Mein Profil → API-Token → eigenes Token mit „Zone → Zone → Lesen“, „Zone → Analytics → Lesen“ und für Tunnel „Konto → Cloudflare Tunnel → Lesen“.",
            env: "NRZMON_CLOUDFLARE_TOKEN",
        },
        _ => Texts {
            url_label: "Management-API",
            url_hint: "Standard https://api.netbird.io – bei eigenem Server dessen Adresse",
            token_label: "Zugangs-Token",
            help: "NetBird-Konsole → Team → Dienstkonten (oder eigener Benutzer) → Zugangs-Token erstellen. Für Benutzer, Regeln und Setup-Schlüssel braucht es Administrator-Rechte.",
            env: "NRZMON_NETBIRD_TOKEN",
        },
    }
}

#[component]
fn TokenGroup(
    which: &'static str,
    title: &'static str,
    form: TokenForm,
    state: IntegrationState,
    demo: bool,
    onsaved: EventHandler<IntegrationsView>,
) -> Element {
    let t = texts(which);
    let mut enabled = use_signal(|| form.enabled);
    let mut url = use_signal(|| form.url.clone());
    let mut token = use_signal(String::new);
    let mut retention = use_signal(|| form.retention_days.to_string());
    let mut zones = use_signal(|| form.zones.join(", "));
    let mut account = use_signal(|| form.account_id.clone());
    let mut busy = use_signal(|| false);
    let mut result = use_signal(|| None::<TestResult>);
    let mut message = use_signal(|| None::<(bool, String)>);

    let build = move |tok: Option<String>| TokenForm {
        enabled: enabled(),
        url: url(),
        token: tok,
        retention_days: retention().trim().parse().unwrap_or(0),
        zones: zones().split([',', ' ', '\n']).map(str::trim).filter(|z| !z.is_empty()).map(String::from).collect(),
        account_id: account(),
        ..TokenForm::default()
    };
    let save = move |tok: Option<String>| async move {
        busy.set(true);
        result.set(None);
        match api::put::<_, IntegrationsView>(&format!("/integrations/{which}"), &build(tok)).await {
            Ok(v) => {
                token.set(String::new());
                message.set(Some((true, "Gespeichert – wirkt sofort.".into())));
                onsaved.call(v);
            }
            Err(e) => message.set(Some((false, e))),
        }
        busy.set(false);
    };
    let test = move || async move {
        busy.set(true);
        message.set(None);
        let tok = Some(token()).filter(|t| !t.trim().is_empty());
        match api::post::<_, TestResult>(&format!("/integrations/{which}/test"), &build(tok)).await {
            Ok(r) => result.set(Some(r)),
            Err(e) => result.set(Some(TestResult { ok: false, message: e, details: vec![] })),
        }
        busy.set(false);
    };

    let (tone, label) = if state.configured && state.ok {
        (Tone::Good, format!("Verbunden · {}", if state.last_ok > 0 { fmt::ago_ms(state.last_ok) } else { String::new() }))
    } else if state.configured {
        (Tone::Bad, state.error.clone().unwrap_or_else(|| "Noch keine Verbindung".into()))
    } else if demo {
        (Tone::Neutral, "Nicht eingerichtet – zeigt Beispieldaten".to_string())
    } else {
        (Tone::Neutral, "Nicht eingerichtet".to_string())
    };
    let placeholder = if form.token_from_env {
        format!("aus {} gesetzt", t.env)
    } else if form.token_set {
        "gesetzt – leer lassen = unverändert".to_string()
    } else {
        "Token einfügen".to_string()
    };
    rsx! {
        SettingsGroup { title, footer: t.help,
            SettingsRow { label: "Zustand", hint: state.summary.clone(),
                span { class: "max-w-xs text-right", StatusDot { tone, label } }
            }
            SettingsRow { label: "Aktiv", hint: "Aus = nicht abfragen, Zugangsdaten bleiben",
                Checkbox { checked: enabled(), aria_label: "Aktiv", on_checked_change: move |v: bool| enabled.set(v) }
            }
            if !t.url_label.is_empty() {
                SettingsRow { label: t.url_label, hint: t.url_hint,
                    Input { class: "w-full sm:w-80", r#type: InputType::Url, value: url(), oninput: move |e: FormEvent| url.set(e.value()) }
                }
            }
            SettingsRow { label: t.token_label, hint: if form.token_from_env { format!("Kommt aus der Umgebung ({}); ein Wert hier geht vor.", t.env) } else { "Wird nur auf dem Server gespeichert und nie angezeigt.".to_string() },
                Input { class: "w-full sm:w-80", r#type: InputType::Password, placeholder, value: token(), oninput: move |e: FormEvent| token.set(e.value()) }
            }
            if which == "dns" {
                SettingsRow { label: "Aufbewahrung", hint: "Tage, die jede Anfrage gespeichert bleibt (abgeschlossene Tage als Parquet)",
                    div { class: "flex items-center gap-2",
                        Input { class: "w-24", r#type: InputType::Number, value: retention(), oninput: move |e: FormEvent| retention.set(e.value()) }
                        span { class: "text-sm text-muted-foreground", "Tage" }
                    }
                }
            }
            if which == "cloudflare" {
                SettingsRow { label: "Zonen", hint: "Leer = alle (Namen oder IDs, mit Komma)",
                    Input { class: "w-full sm:w-80", placeholder: "alle", value: zones(), oninput: move |e: FormEvent| zones.set(e.value()) }
                }
                SettingsRow { label: "Konto-ID", hint: "Für Tunnel; leer = aus den Zonen",
                    Input { class: "w-full sm:w-80", placeholder: "automatisch", value: account(), oninput: move |e: FormEvent| account.set(e.value()) }
                }
            }
            div { class: "flex flex-wrap items-center gap-2 px-3 py-3",
                Button { disabled: busy(), onclick: move |_| { let tok = Some(token()).filter(|t| !t.trim().is_empty()); spawn(save(tok)); },
                    "Speichern"
                }
                Button { variant: ButtonVariant::Outline, disabled: busy(), onclick: move |_| { spawn(test()); },
                    if busy() { icons::LoaderCircle { class: "size-4 animate-spin" } } else { icons::PlugZap { class: "size-4" } }
                    "Verbindung testen"
                }
                if form.token_set && !form.token_from_env {
                    Button { variant: ButtonVariant::Ghost, disabled: busy(), onclick: move |_| { spawn(save(Some(String::new()))); },
                        "Token entfernen"
                    }
                }
                if let Some((ok, m)) = message() {
                    span { class: if ok { "text-sm text-success" } else { "text-sm text-destructive" }, "{m}" }
                }
            }
            if let Some(r) = result() {
                div { class: "grid gap-1 px-3 py-3 text-sm",
                    div { class: "flex items-center gap-2",
                        if r.ok {
                            icons::CircleCheck { class: "size-4 text-success" }
                        } else {
                            icons::CircleAlert { class: "size-4 text-destructive" }
                        }
                        span { class: "font-medium", "{r.message}" }
                    }
                    for d in r.details.iter() {
                        p { class: "pl-6 text-xs text-muted-foreground break-words", "{d}" }
                    }
                }
            }
        }
    }
}
