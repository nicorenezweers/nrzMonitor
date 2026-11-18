//! Seite „Alarme“: laufende Alarme, Verlauf, Regeln verwalten – und `AlertSettings` (Kanäle).

use dioxus::prelude::*;
use shared::{
    alerts::{ActiveAlert, Channel, ChannelKind, Cmp, Expr, NotifyConfig, Rule, RuleKind, RuleStatus, StateCheck, TestResult},
    events::{Event, EventPage, Severity},
    model::trim,
};

use crate::{
    api,
    components::{
        ui::{
            app_shell::{EmptyState, LoadingState, PageContainer, PageHeader, PageSection, PageWidth, SettingsGroup, SettingsRow},
            button::{Button, ButtonSize, ButtonVariant},
            input::Input,
            toast::{ToastType, show_toast},
        },
        widgets::{StatTile, StatusDot, Tone},
    },
    live::use_events,
    pages::docker::{Segmented, use_snapshot},
    route::Route,
};

fn tone(s: Severity) -> Tone {
    match s {
        Severity::Crit => Tone::Bad,
        Severity::Warn => Tone::Warn,
        Severity::Ok => Tone::Good,
        Severity::Info => Tone::Neutral,
    }
}

fn sev_label(s: Severity) -> &'static str {
    match s {
        Severity::Crit => "Kritisch",
        Severity::Warn => "Warnung",
        Severity::Ok => "Behoben",
        Severity::Info => "Info",
    }
}

/// Ziel eines Alarms in der Oberfläche (Container, App oder Diagramm-Seite).
fn alert_link(a: &ActiveAlert) -> Option<Route> {
    let chart = a.chart.as_deref().unwrap_or("");
    if chart.starts_with("docker.") || a.group == "Container" {
        return Some(Route::Container { name: a.subject.clone() });
    }
    let p = chart.split('.').next().unwrap_or("");
    Some(match p {
        "system" | "cpu" => Route::Cpu {},
        "mem" => Route::Memory {},
        "disk" => Route::Disks {},
        "net" => Route::Network {},
        "sensors" => Route::Sensors {},
        "dns" => Route::Dns {},
        "probe" => Route::Probes {},
        "netbird" => Route::Netbird {},
        "cf" => Route::Cloudflare {},
        _ => return None,
    })
}

#[component]
pub fn Alerts() -> Element {
    let active = use_snapshot::<Vec<ActiveAlert>>("alerts.active");
    let rules = use_resource(|| async { api::get::<Vec<RuleStatus>>("/alerts/rules").await });
    let list = active.read().clone().unwrap_or_default();
    let crit = list.iter().filter(|a| a.severity == Severity::Crit).count();
    let warn = list.iter().filter(|a| a.severity == Severity::Warn).count();
    let (enabled, total) = rules.read().as_ref().and_then(|r| r.as_ref().ok()).map_or((0, 0), |r| (r.iter().filter(|x| x.rule.enabled).count(), r.len()));
    rsx! {
        PageContainer { width: PageWidth::Wide,
            PageHeader {
                title: "Alarme",
                description: "Laufende Alarme, Verlauf und Regeln – geprüft alle 5 Sekunden",
                actions: rsx! {
                    Link { to: Route::Settings {},
                        Button { variant: ButtonVariant::Outline, size: ButtonSize::Sm, icons::BellRing {} "Benachrichtigungen" }
                    }
                },
            }
            div { class: "grid grid-cols-[minmax(0,1fr)] gap-6",
                div { class: "grid grid-cols-2 gap-4 md:grid-cols-4",
                    StatTile { label: "Kritisch", value: "{crit}", icon: rsx! { icons::OctagonAlert {} }, class: if crit > 0 { "border-destructive/60" } else { "" } }
                    StatTile { label: "Warnungen", value: "{warn}", icon: rsx! { icons::TriangleAlert {} }, class: if warn > 0 { "border-warning/60" } else { "" } }
                    StatTile { label: "Regeln aktiv", value: "{enabled} / {total}", icon: rsx! { icons::ListChecks {} } }
                    StatTile {
                        label: "Zustand",
                        value: if crit > 0 { "Kritisch" } else if warn > 0 { "Warnung" } else { "Alles gut" },
                        icon: rsx! { icons::ShieldCheck {} },
                    }
                }
                PageSection { title: "Laufende Alarme", description: if list.is_empty() { "Keine".to_string() } else { format!("{} laufend", list.len()) },
                    if list.is_empty() {
                        EmptyState { title: "Alles in Ordnung", description: "Keine Regel schlägt gerade an.", icon: rsx! { icons::ShieldCheck {} } }
                    } else {
                        ul { class: "divide-y",
                            for a in list {
                                ActiveRow { key: "{a.id}", a }
                            }
                        }
                    }
                }
                div { class: "grid grid-cols-[minmax(0,1fr)] gap-6 xl:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]",
                    RulesSection { rules }
                    History {}
                }
            }
        }
    }
}

#[component]
fn ActiveRow(a: ActiveAlert) -> Element {
    let link = alert_link(&a);
    rsx! {
        li { class: "flex flex-wrap items-start gap-x-4 gap-y-1 py-3",
            div { class: "w-20 shrink-0 pt-0.5", StatusDot { tone: tone(a.severity), label: sev_label(a.severity) } }
            div { class: "min-w-0 flex-1 basis-60",
                p { class: "font-medium break-words",
                    if let Some(to) = link {
                        Link { to, class: "hover:underline", "{a.title}" }
                    } else {
                        "{a.title}"
                    }
                }
                p { class: "text-sm text-muted-foreground", "{a.message}" }
            }
            div { class: "ml-auto text-right text-xs text-muted-foreground",
                p { class: "tabular-nums", title: "{crate::fmt::full_ms(a.since)}", "seit {shared::model::duration(((crate::fmt::now_ms() - a.since) / 1000).max(0) as f64)}" }
                p { "{a.group}" }
            }
        }
    }
}

/// Verlauf: Alarm-Ereignisse aus dem Speicher und laufend neue.
#[component]
fn History() -> Element {
    let live = use_events();
    let past = use_resource(|| async { api::get::<EventPage>(&api::qs("/events", &[("source", "alert".into()), ("limit", "100".into())])).await });
    let mut list: Vec<Event> = live.read().iter().filter(|e| e.source == "alert").cloned().collect();
    if let Some(Ok(p)) = past.read().as_ref() {
        for e in &p.events {
            if !list.iter().any(|x| x.id == e.id && x.ts == e.ts) {
                list.push(e.clone());
            }
        }
    }
    list.sort_by(|a, b| b.ts.cmp(&a.ts));
    list.truncate(100);
    rsx! {
        PageSection { title: "Verlauf", description: "Ausgelöst und behoben",
            if list.is_empty() {
                p { class: "text-sm text-muted-foreground", "Noch keine Alarme." }
            }
            ul { class: "max-h-[40rem] divide-y overflow-auto",
                for e in list {
                    li { key: "{e.id}-{e.ts}", class: "flex items-start gap-3 py-2 text-sm",
                        span { class: "mt-0.5", StatusDot { tone: tone(e.severity), label: "" } }
                        div { class: "min-w-0 flex-1",
                            p { class: "truncate", "{e.title}" }
                            if !e.message.is_empty() {
                                p { class: "truncate text-xs text-muted-foreground", "{e.message}" }
                            }
                        }
                        span { class: "shrink-0 text-xs text-muted-foreground tabular-nums", title: "{crate::fmt::full_ms(e.ts)}", "{crate::fmt::ago_ms(e.ts)}" }
                    }
                }
            }
        }
    }
}

/// Ein-/Ausschalter (die nrzUI-Switch meldet keine Änderung – darum hier ein eigener Knopf im gleichen Stil).
#[component]
pub fn Toggle(checked: bool, onchange: EventHandler<bool>, #[props(into, default)] label: String) -> Element {
    let state = if checked { "checked" } else { "unchecked" };
    rsx! {
        button {
            r#type: "button",
            role: "switch",
            "aria-checked": "{checked}",
            "aria-label": "{label}",
            "data-state": state,
            class: "inline-flex h-5 w-9 shrink-0 cursor-pointer items-center rounded-full border-2 border-transparent transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring data-[state=checked]:bg-primary data-[state=unchecked]:bg-input",
            onclick: move |e| {
                e.stop_propagation();
                onchange.call(!checked);
            },
            span { "data-state": state, class: "pointer-events-none block size-4 rounded-full bg-background shadow-lg ring-0 transition-transform data-[state=checked]:translate-x-4 data-[state=unchecked]:translate-x-0" }
        }
    }
}

#[component]
fn RulesSection(rules: Resource<Result<Vec<RuleStatus>, String>>) -> Element {
    let mut editing = use_signal(|| None::<Rule>);
    let filter = use_signal(|| "all".to_string());
    let mut rules = rules;
    let save = move |r: Rule| {
        spawn(async move {
            match api::put::<Rule, RuleStatus>("/alerts/rules", &r).await {
                Ok(_) => {
                    rules.restart();
                    editing.set(None);
                }
                Err(e) => show_toast(ToastType::Error, e),
            }
        });
    };
    let body = match rules.read().as_ref() {
        None => rsx! { LoadingState {} },
        Some(Err(e)) => rsx! { p { class: "text-sm text-destructive", "{e}" } },
        Some(Ok(list)) => {
            let mut groups: Vec<(String, Vec<RuleStatus>)> = Vec::new();
            for r in list.iter().filter(|r| match filter().as_str() {
                "firing" => r.firing > 0,
                "off" => !r.rule.enabled,
                "custom" => !r.rule.builtin,
                _ => true,
            }) {
                match groups.iter_mut().find(|(g, _)| *g == r.rule.group) {
                    Some((_, l)) => l.push(r.clone()),
                    None => groups.push((r.rule.group.clone(), vec![r.clone()])),
                }
            }
            rsx! {
                if groups.is_empty() {
                    p { class: "text-sm text-muted-foreground", "Keine Regeln für diesen Filter." }
                }
                div { class: "grid grid-cols-[minmax(0,1fr)] gap-5",
                    for (g, l) in groups {
                        div { key: "{g}",
                            p { class: "mb-1 text-xs font-medium tracking-wider text-muted-foreground uppercase", "{g}" }
                            ul { class: "divide-y rounded-lg border",
                                for r in l {
                                    RuleRow {
                                        key: "{r.rule.id}",
                                        status: r.clone(),
                                        on_toggle: move |on: bool| {
                                            let mut x = r.rule.clone();
                                            x.enabled = on;
                                            save(x);
                                        },
                                        on_edit: move |rule: Rule| editing.set(Some(rule)),
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    };
    rsx! {
        PageSection {
            title: "Regeln",
            description: "Standardregeln für alle Bereiche – ein-/ausschaltbar, Schwellen änderbar, eigene Regeln möglich",
            actions: rsx! {
                Button {
                    size: ButtonSize::Sm,
                    onclick: move |_| editing.set(Some(Rule { name: String::new(), context: String::new(), warn: Some(80.0), crit: Some(90.0), ..Rule::default() })),
                    icons::Plus {}
                    "Eigene Regel"
                }
            },
            div { class: "mb-4",
                Segmented {
                    options: vec![("all".into(), "Alle".into()), ("firing".into(), "Laufend".into()), ("off".into(), "Aus".into()), ("custom".into(), "Eigene".into())],
                    value: filter,
                }
            }
            if let Some(r) = editing() {
                RuleEditor {
                    key: "{r.id}",
                    rule: r,
                    on_save: save,
                    on_cancel: move |_| editing.set(None),
                    on_changed: move |_| {
                        rules.restart();
                        editing.set(None);
                    },
                }
            }
            {body}
        }
    }
}

#[component]
fn RuleRow(status: RuleStatus, on_toggle: EventHandler<bool>, on_edit: EventHandler<Rule>) -> Element {
    let r = status.rule.clone();
    let unit = status.unit;
    let f = |v: Option<f64>| v.map_or_else(|| "–".to_string(), |v| unit.format(v));
    let cond = if r.kind == RuleKind::State && !r.check.uses_thresholds() {
        sev_label(r.severity).to_string()
    } else {
        format!("{} {} · {}", r.cmp.symbol(), f(r.warn), f(r.crit))
    };
    let dim = if r.enabled { "" } else { "opacity-60" };
    let rr = r.clone();
    rsx! {
        li { class: "flex items-center gap-3 px-3 py-2.5 {dim}",
            Toggle { checked: r.enabled, label: "{r.name} ein/aus", onchange: move |v| on_toggle.call(v) }
            button { class: "block min-w-0 flex-1 overflow-hidden text-left", onclick: move |_| on_edit.call(rr.clone()),
                p { class: "flex items-center gap-2 text-sm font-medium",
                    span { class: "truncate", "{r.name}" }
                    if status.firing > 0 {
                        span { class: "rounded bg-warning-light px-1.5 text-xs text-warning-dark", "{status.firing} laufend" }
                    }
                    if !r.builtin {
                        span { class: "rounded bg-muted px-1.5 text-xs text-muted-foreground", "eigene" }
                    }
                }
                p { class: "truncate text-xs text-muted-foreground",
                    if let Some(e) = status.error.as_ref() {
                        span { class: "text-destructive", "Fehler: {e}" }
                    } else {
                        "{r.description}"
                    }
                }
            }
            div { class: "hidden shrink-0 text-right text-xs sm:block",
                p { class: "tabular-nums", "{cond}" }
                p { class: "text-muted-foreground", if r.enabled { "{status.checked} geprüft" } else { "aus" } }
            }
            button { class: "shrink-0 rounded-md p-1.5 text-muted-foreground hover:bg-muted hover:text-foreground", "aria-label": "Bearbeiten", onclick: move |_| on_edit.call(r.clone()),
                icons::Pencil { class: "size-4" }
            }
        }
    }
}

fn num(s: &str) -> Option<f64> {
    let s = s.trim().replace(',', ".");
    if s.is_empty() { None } else { s.parse().ok() }
}

fn opt(v: Option<f64>) -> String {
    v.map(|v| trim(v, 4)).unwrap_or_default()
}

/// Formular für eine Regel (Standardregel: nur Schwellen, Zeiten, Filter; eigene: alles).
#[component]
fn RuleEditor(rule: Rule, on_save: EventHandler<Rule>, on_cancel: EventHandler<()>, on_changed: EventHandler<()>) -> Element {
    let builtin = rule.builtin;
    let is_new = rule.id.is_empty();
    let mut name = use_signal(|| rule.name.clone());
    let mut kind = use_signal(|| if rule.kind == RuleKind::State { "state".to_string() } else { "metric".to_string() });
    let mut check = use_signal(|| rule.check.key().to_string());
    let mut context = use_signal(|| rule.context.clone());
    let mut expr = use_signal(|| rule.expr.clone());
    let mut filter = use_signal(|| rule.filter.clone());
    let mut cmp = use_signal(|| if rule.cmp == Cmp::Below { "below".to_string() } else { "above".to_string() });
    let mut warn = use_signal(|| opt(rule.warn));
    let mut crit = use_signal(|| opt(rule.crit));
    let mut window = use_signal(|| rule.window.to_string());
    let mut duration = use_signal(|| rule.duration.to_string());
    let mut hyst = use_signal(|| trim(rule.hysteresis, 4));
    let mut severity = use_signal(|| if rule.severity == Severity::Crit { "crit".to_string() } else { "warn".to_string() });
    let mut notify = use_signal(|| rule.notify);
    let mut description = use_signal(|| rule.description.clone());
    let expr_err = (kind() == "metric").then(|| Expr::parse(&expr()).err()).flatten();
    let state_kind = kind() == "state";
    let chk = StateCheck::from_key(&check()).unwrap_or_default();
    let thresholds = !state_kind || chk.uses_thresholds();
    let base = rule.clone();
    let submit = move |_| {
        let mut r = base.clone();
        r.name = name();
        r.description = description();
        r.kind = if kind() == "state" { RuleKind::State } else { RuleKind::Metric };
        r.check = StateCheck::from_key(&check()).unwrap_or_default();
        r.context = context();
        r.expr = expr();
        r.filter = filter();
        r.cmp = if cmp() == "below" { Cmp::Below } else { Cmp::Above };
        r.warn = num(&warn());
        r.crit = num(&crit());
        r.window = window().trim().parse().unwrap_or(0);
        r.duration = duration().trim().parse().unwrap_or(0);
        r.hysteresis = num(&hyst()).unwrap_or(0.0);
        r.severity = if severity() == "crit" { Severity::Crit } else { Severity::Warn };
        r.notify = notify();
        on_save.call(r);
    };
    let id = rule.id.clone();
    let reset = move |_| {
        let id = id.clone();
        spawn(async move {
            match api::post_empty(&format!("/alerts/rules/{id}/reset"), &()).await {
                Ok(()) => on_changed.call(()),
                Err(e) => show_toast(ToastType::Error, e),
            }
        });
    };
    let id2 = rule.id.clone();
    let remove = move |_| {
        let id = id2.clone();
        spawn(async move {
            match api::delete(&format!("/alerts/rules/{id}")).await {
                Ok(()) => on_changed.call(()),
                Err(e) => show_toast(ToastType::Error, e),
            }
        });
    };
    let label = "grid gap-1 text-xs text-muted-foreground";
    let select_cls = "h-9 rounded-md border border-input bg-background px-2 text-sm text-foreground";
    rsx! {
        div { class: "mb-5 rounded-lg border bg-muted/30 p-4",
            div { class: "mb-3 flex items-center justify-between gap-2",
                p { class: "font-medium", if is_new { "Neue Regel" } else { "Regel bearbeiten: {rule.name}" } }
                if builtin {
                    span { class: "text-xs text-muted-foreground", "Standardregel · Kontext {rule.context} · Wert {rule.expr}" }
                }
            }
            div { class: "grid gap-3 sm:grid-cols-2 lg:grid-cols-4",
                if !builtin {
                    label { class: "{label} sm:col-span-2",
                        "Name"
                        Input { value: name(), oninput: move |e: FormEvent| name.set(e.value()) }
                    }
                    label { class: label,
                        "Art"
                        select { class: select_cls, value: "{kind}", onchange: move |e| kind.set(e.value()),
                            option { value: "metric", "Messwert" }
                            option { value: "state", "Zustand" }
                        }
                    }
                    if state_kind {
                        label { class: label,
                            "Prüfung"
                            select { class: select_cls, value: "{check}", onchange: move |e| check.set(e.value()),
                                for c in StateCheck::ALL {
                                    option { key: "{c.key()}", value: c.key(), "{c.label()}" }
                                }
                            }
                        }
                    } else {
                        label { class: label,
                            "Vergleich"
                            select { class: select_cls, value: "{cmp}", onchange: move |e| cmp.set(e.value()),
                                option { value: "above", "über (>)" }
                                option { value: "below", "unter (<)" }
                            }
                        }
                    }
                    if !state_kind || chk == StateCheck::NoData {
                        label { class: "{label} sm:col-span-2",
                            "Kontext (z. B. disk.space, docker.*)"
                            Input { value: context(), placeholder: "system.cpu", oninput: move |e: FormEvent| context.set(e.value()) }
                        }
                    }
                    if !state_kind {
                        label { class: "{label} sm:col-span-2",
                            "Wert: used · rx+tx · * · max:* · used/used+avail (Anteil in %)"
                            Input { value: expr(), oninput: move |e: FormEvent| expr.set(e.value()) }
                            if let Some(e) = expr_err.clone() {
                                span { class: "text-destructive", "{e}" }
                            }
                        }
                    }
                }
                label { class: "{label} sm:col-span-2",
                    "Filter (Labels, z. B. app=nrz-*, device!=loop*)"
                    Input { value: filter(), placeholder: "alle", oninput: move |e: FormEvent| filter.set(e.value()) }
                }
                if thresholds {
                    label { class: label,
                        "Warnung ab"
                        Input { value: warn(), placeholder: "–", oninput: move |e: FormEvent| warn.set(e.value()) }
                    }
                    label { class: label,
                        "Kritisch ab"
                        Input { value: crit(), placeholder: "–", oninput: move |e: FormEvent| crit.set(e.value()) }
                    }
                } else {
                    label { class: label,
                        "Schwere"
                        select { class: select_cls, value: "{severity}", onchange: move |e| severity.set(e.value()),
                            option { value: "warn", "Warnung" }
                            option { value: "crit", "Kritisch" }
                        }
                    }
                }
                label { class: label,
                    "Mittel über (s, 0 = neuester Wert)"
                    Input { value: window(), oninput: move |e: FormEvent| window.set(e.value()) }
                }
                label { class: label,
                    "Mindestdauer (s)"
                    Input { value: duration(), oninput: move |e: FormEvent| duration.set(e.value()) }
                }
                if thresholds {
                    label { class: label,
                        "Hysterese (Abstand zum Beenden)"
                        Input { value: hyst(), oninput: move |e: FormEvent| hyst.set(e.value()) }
                    }
                }
                label { class: "flex items-center gap-2 self-end pb-2 text-sm",
                    Toggle { checked: notify(), label: "Benachrichtigen", onchange: move |v| notify.set(v) }
                    "Benachrichtigen"
                }
                if !builtin {
                    label { class: "{label} sm:col-span-2 lg:col-span-4",
                        "Beschreibung"
                        Input { value: description(), oninput: move |e: FormEvent| description.set(e.value()) }
                    }
                }
            }
            div { class: "mt-4 flex flex-wrap items-center gap-2",
                Button { size: ButtonSize::Sm, disabled: expr_err.is_some() && !builtin, onclick: submit, icons::Check {} "Speichern" }
                Button { size: ButtonSize::Sm, variant: ButtonVariant::Ghost, onclick: move |_| on_cancel.call(()), "Abbrechen" }
                div { class: "flex-1" }
                if builtin {
                    Button { size: ButtonSize::Sm, variant: ButtonVariant::Outline, onclick: reset, icons::RotateCcw {} "Standard" }
                } else if !is_new {
                    Button { size: ButtonSize::Sm, variant: ButtonVariant::Destructive, onclick: remove, icons::Trash2 {} "Löschen" }
                }
            }
        }
    }
}

// ------------------------------------------------------------------ Einstellungen: Kanäle

/// Abschnitt der Einstellungsseite: Benachrichtigungen.
#[component]
pub fn AlertSettings() -> Element {
    let mut cfg = use_resource(|| async { api::get::<NotifyConfig>("/alerts/config").await });
    let mut draft = use_signal(|| None::<NotifyConfig>);
    let mut saving = use_signal(|| false);
    use_effect(move || {
        if let Some(Ok(c)) = cfg.read().as_ref() {
            draft.set(Some(c.clone()));
        }
    });
    let Some(d) = draft() else {
        return rsx! {
            SettingsGroup { title: "Benachrichtigungen",
                SettingsRow { label: "Kanäle", hint: "Lade …" }
            }
        };
    };
    let dirty = cfg.read().as_ref().and_then(|r| r.as_ref().ok()) != Some(&d);
    let save = move |_| {
        let Some(c) = draft() else { return };
        saving.set(true);
        spawn(async move {
            match api::put::<NotifyConfig, NotifyConfig>("/alerts/config", &c).await {
                Ok(_) => {
                    show_toast(ToastType::Success, "Benachrichtigungen gespeichert");
                    cfg.restart();
                }
                Err(e) => show_toast(ToastType::Error, e),
            }
            saving.set(false);
        });
    };
    let mut add = move |kind: ChannelKind| {
        if let Some(c) = draft.write().as_mut() {
            c.channels.push(Channel { kind, name: kind.label().to_string(), ..Channel::default() });
        }
    };
    rsx! {
        SettingsGroup {
            title: "Benachrichtigungen",
            footer: "Geheimnisse (Tokens, Webhook-URLs) werden nur maskiert angezeigt; leer lassen = unverändert.",
            actions: rsx! {
                Button { size: ButtonSize::Sm, disabled: !dirty || saving(), onclick: save, if saving() { "Speichert …" } else { "Speichern" } }
            },
            for (i, c) in d.channels.iter().enumerate() {
                ChannelRow { key: "{i}-{c.id}", index: i, channel: c.clone(), draft }
            }
            SettingsRow { label: "Kanal hinzufügen", hint: "ntfy, Webhook, Discord oder Telegram",
                div { class: "flex flex-wrap gap-1",
                    for k in ChannelKind::ALL {
                        Button { key: "{k.key()}", size: ButtonSize::Sm, variant: ButtonVariant::Outline, onclick: move |_| add(k), icons::Plus {} "{k.label()}" }
                    }
                }
            }
            SettingsRow { label: "Erinnern", hint: "Bei laufendem Alarm erneut senden (Minuten, 0 = nie)",
                div { class: "flex items-center gap-2",
                    Input {
                        class: "w-20",
                        value: d.repeat_minutes.to_string(),
                        oninput: move |e: FormEvent| if let Some(c) = draft.write().as_mut() { c.repeat_minutes = e.value().trim().parse().unwrap_or(0) },
                    }
                    span { class: "text-xs text-muted-foreground", "höchstens" }
                    Input {
                        class: "w-16",
                        value: d.max_repeats.to_string(),
                        oninput: move |e: FormEvent| if let Some(c) = draft.write().as_mut() { c.max_repeats = e.value().trim().parse().unwrap_or(0) },
                    }
                    span { class: "text-xs text-muted-foreground", "mal" }
                }
            }
            SettingsRow { label: "„Behoben“ melden", hint: "Nachricht, wenn ein gemeldeter Alarm endet",
                Toggle { checked: d.notify_resolved, label: "Behoben melden", onchange: move |v| if let Some(c) = draft.write().as_mut() { c.notify_resolved = v } }
            }
        }
    }
}

#[component]
fn ChannelRow(index: usize, channel: Channel, draft: Signal<Option<NotifyConfig>>) -> Element {
    let mut testing = use_signal(|| false);
    let mut result = use_signal(|| None::<TestResult>);
    let c = channel.clone();
    let mut edit = move |f: &dyn Fn(&mut Channel)| {
        if let Some(cfg) = draft.write().as_mut() {
            if let Some(ch) = cfg.channels.get_mut(index) {
                f(ch);
            }
        }
    };
    let test = {
        let c = channel.clone();
        move |_| {
            let c = c.clone();
            testing.set(true);
            spawn(async move {
                let r = api::post::<Channel, TestResult>("/alerts/test", &c).await.unwrap_or_else(|e| TestResult { ok: false, message: e });
                result.set(Some(r));
                testing.set(false);
            });
        }
    };
    let field = "grid gap-1 text-xs text-muted-foreground";
    let (url_label, url_ph) = match c.kind {
        ChannelKind::Ntfy => ("Server", "https://ntfy.sh"),
        ChannelKind::Discord => ("Webhook-URL", "https://discord.com/api/webhooks/…"),
        ChannelKind::Webhook => ("URL (POST, JSON)", "https://example.com/hook"),
        ChannelKind::Telegram => ("", ""),
    };
    rsx! {
        div { class: "grid gap-3 px-3 py-3",
            div { class: "flex flex-wrap items-center gap-2",
                Toggle { checked: c.enabled, label: "Kanal ein/aus", onchange: move |v| edit(&|ch: &mut Channel| ch.enabled = v) }
                span { class: "rounded bg-muted px-1.5 py-0.5 text-xs", "{c.kind.label()}" }
                div { class: "min-w-0 flex-1",
                    Input { value: c.name.clone(), placeholder: "Name", oninput: move |e: FormEvent| { let v = e.value(); edit(&|ch: &mut Channel| ch.name = v.clone()) } }
                }
                Button { size: ButtonSize::Sm, variant: ButtonVariant::Outline, disabled: testing(), onclick: test, icons::Send {} if testing() { "Sendet …" } else { "Test" } }
                Button {
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    aria_label: "Kanal entfernen",
                    onclick: move |_| if let Some(cfg) = draft.write().as_mut() { if index < cfg.channels.len() { cfg.channels.remove(index); } },
                    icons::Trash2 {}
                }
            }
            div { class: "grid gap-2 sm:grid-cols-2",
                if c.kind != ChannelKind::Telegram {
                    label { class: field,
                        "{url_label}"
                        Input { value: c.url.clone(), placeholder: url_ph, oninput: move |e: FormEvent| { let v = e.value(); edit(&|ch: &mut Channel| ch.url = v.clone()) } }
                    }
                }
                if c.kind == ChannelKind::Ntfy {
                    label { class: field,
                        "Topic"
                        Input { value: c.topic.clone(), placeholder: "nrzmonitor", oninput: move |e: FormEvent| { let v = e.value(); edit(&|ch: &mut Channel| ch.topic = v.clone()) } }
                    }
                }
                if matches!(c.kind, ChannelKind::Ntfy | ChannelKind::Telegram) {
                    label { class: field,
                        if c.kind == ChannelKind::Ntfy { "Zugangstoken (optional)" } else { "Bot-Token" }
                        Input { value: c.token.clone(), placeholder: "leer", oninput: move |e: FormEvent| { let v = e.value(); edit(&|ch: &mut Channel| ch.token = v.clone()) } }
                    }
                }
                if c.kind == ChannelKind::Telegram {
                    label { class: field,
                        "Chat-ID"
                        Input { value: c.chat_id.clone(), oninput: move |e: FormEvent| { let v = e.value(); edit(&|ch: &mut Channel| ch.chat_id = v.clone()) } }
                    }
                }
                label { class: field,
                    "Senden ab"
                    select {
                        class: "h-9 rounded-md border border-input bg-background px-2 text-sm text-foreground",
                        value: if c.min_severity == Severity::Crit { "crit" } else { "warn" },
                        onchange: move |e| { let crit = e.value() == "crit"; edit(&|ch: &mut Channel| ch.min_severity = if crit { Severity::Crit } else { Severity::Warn }) },
                        option { value: "warn", "Warnung" }
                        option { value: "crit", "Nur kritisch" }
                    }
                }
            }
            if let Some(r) = result() {
                p { class: if r.ok { "text-xs text-success" } else { "text-xs text-destructive" }, "{r.message}" }
            }
        }
    }
}
