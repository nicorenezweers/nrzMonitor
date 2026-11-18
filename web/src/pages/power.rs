//! Seite „Strom“: aktuelle Leistung, Energie (kWh) und Kosten je Tag, Monat und Jahr; Strompreise
//! mit Gültig-ab-Zeitpunkt (auch rückwirkend), Grundlast und Netzteil-Wirkungsgrad.

use dioxus::prelude::*;
use gloo_timers::future::TimeoutFuture;
use js_sys::Date;
use shared::{
    Unit,
    catalog::power::POWER_DRAW,
    power::{EnergyBucket, PowerSettings, PowerSummary, PricePoint},
};
use wasm_bindgen::JsValue;

use crate::{
    api,
    components::{
        chart::{ChartCard, color, section::ChartGrid},
        ui::{
            app_shell::{LoadingState, PageContainer, PageHeader, PageSection, PageWidth, SettingsGroup, SettingsRow},
            button::{Button, ButtonSize, ButtonVariant},
            toast::{ToastType, show_toast},
        },
        widgets::{StatTile, use_latest},
    },
    fmt,
    state::use_app,
};

const INPUT: &str = "h-9 rounded-md border border-input bg-background px-3 text-sm tabular-nums outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50";
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mär", "Apr", "Mai", "Jun", "Jul", "Aug", "Sep", "Okt", "Nov", "Dez"];

/// Abstand der Ortszeit zu UTC in Minuten.
fn tz_minutes() -> i64 {
    -(Date::new_0().get_timezone_offset() as i64)
}

/// `1.23 kWh` bzw. `420 Wh`.
fn energy(kwh: f64) -> String {
    if kwh.abs() < 1.0 { format!("{} Wh", shared::model::trim(kwh * 1000.0, if kwh.abs() < 0.01 { 1 } else { 0 })) } else { format!("{} kWh", shared::model::trim(kwh, 2)) }
}

/// `1.23 €`; Beträge unter einem Cent in Cent (`0.04 ct`).
fn money(v: f64, cur: &str) -> String {
    if v != 0.0 && v.abs() < 0.01 {
        return format!("{} ct", shared::model::trim(v * 100.0, 2));
    }
    format!("{v:.2} {cur}")
}

fn cents(v: f64) -> String {
    format!("{} ct/kWh", shared::model::trim(v, 2))
}

fn month_label(ts: i64) -> String {
    let d = Date::new(&JsValue::from_f64(ts as f64 * 1000.0));
    format!("{} {}", MONTHS.get(d.get_month() as usize).copied().unwrap_or(""), d.get_full_year())
}

/// Wert für `<input type="datetime-local">` (Ortszeit).
fn datetime_value(ts: i64) -> String {
    let d = Date::new(&JsValue::from_f64(ts as f64 * 1000.0));
    format!("{:04}-{:02}-{:02}T{:02}:{:02}", d.get_full_year(), d.get_month() + 1, d.get_date(), d.get_hours(), d.get_minutes())
}

/// `<input type="datetime-local">` → Unix-Sekunden (Ortszeit des Browsers).
fn parse_datetime(v: &str) -> Option<i64> {
    let ms = Date::new(&JsValue::from_str(v)).get_time();
    ms.is_finite().then(|| (ms / 1000.0) as i64)
}

#[component]
pub fn Power() -> Element {
    let app = use_app();
    let mut summary = use_signal(|| None::<PowerSummary>);
    let mut error = use_signal(|| None::<String>);
    let mut reload = use_signal(|| 0u32);

    use_effect(move || {
        let _ = reload();
        spawn(async move {
            match api::get::<PowerSummary>(&format!("/power/summary?tz={}", tz_minutes())).await {
                Ok(s) => {
                    summary.set(Some(s));
                    error.set(None);
                }
                Err(e) => error.set(Some(e)),
            }
        });
    });
    use_future(move || async move {
        loop {
            TimeoutFuture::new(30_000).await;
            reload += 1;
        }
    });

    let latest = use_latest(POWER_DRAW.context);
    let live_watts = latest.read().as_ref().map(|v| v.iter().flatten().sum::<f64>());
    let has_rapl = app.chart("cpu.power").is_some();

    let Some(s) = summary() else {
        return rsx! {
            PageContainer { width: PageWidth::Full,
                PageHeader { title: "Strom", description: "Aktuelle Leistung, Energie und Kosten" }
                if let Some(e) = error() {
                    p { class: "rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2.5 text-sm", "{e}" }
                } else {
                    LoadingState {}
                }
            }
        };
    };
    let cur = s.currency.clone();
    let period = |key: &str| s.periods.iter().find(|p| p.key == key).cloned().unwrap_or_default();
    let (today, month, year, total) = (period("today"), period("month"), period("year"), period("total"));
    let watts = live_watts.or(s.watts_now);

    rsx! {
        PageContainer { width: PageWidth::Full,
            PageHeader { title: "Strom", description: "Aktuelle Leistung, Energie und Kosten" }

            div { class: "mb-6 rounded-lg border bg-muted/50 px-3 py-2.5 text-sm text-muted-foreground",
                if s.measured {
                    "Gemessen: {s.sources.join(\" · \")}. Was RAPL nicht sieht (Mainboard, SSD, Lüfter) kommt als Grundlast dazu, der Netzteilverlust aus dem Wirkungsgrad – einstellbar unten."
                } else {
                    "Keine Leistungsmessung verfügbar (RAPL fehlt oder ist nicht lesbar) – gerechnet wird nur mit der eingestellten Grundlast."
                }
            }

            div { class: "mb-6 grid grid-cols-2 gap-4 md:grid-cols-3 xl:grid-cols-6",
                StatTile { label: "Aktuell", value: watts.map_or("–".into(), |w| Unit::Watts.format(w)),
                    hint: s.avg_watts_24h.map_or("Noch kein 24-h-Mittel".into(), |w| format!("Mittel 24 h: {}", Unit::Watts.format(w))),
                    icon: rsx! { icons::Zap {} } }
                StatTile { label: "Heute", value: energy(today.kwh), hint: money(today.cost, &cur), icon: rsx! { icons::Calendar {} } }
                StatTile { label: "Dieser Monat", value: energy(month.kwh), hint: money(month.cost, &cur), icon: rsx! { icons::ChartColumn {} } }
                StatTile { label: "Dieses Jahr", value: energy(year.kwh), hint: money(year.cost, &cur), icon: rsx! { icons::ChartBar {} } }
                StatTile { label: "Hochrechnung", value: money(s.projection_month_cost, &cur),
                    hint: format!("je Monat · {} je Jahr", money(s.projection_year_cost, &cur)), icon: rsx! { icons::TrendingUp {} } }
                StatTile { label: "Strompreis", value: format!("{} ct", shared::model::trim(s.price_now, 2)), hint: format!("je kWh · bisher {}", money(total.cost, &cur)), icon: rsx! { icons::Coins {} } }
            }

            div { class: "grid gap-6",
                ChartGrid {
                    ChartCard { id: POWER_DRAW.context, title: "Leistung am Netzstecker" }
                    if has_rapl {
                        ChartCard { id: "cpu.power", title: "Gemessen mit RAPL" }
                    }
                }

                PageSection { title: "Energie und Kosten je Tag", description: "Die letzten 31 Tage – Kosten und mittlere Leistung beim Darüberfahren",
                    Columns { buckets: s.days.clone(), currency: cur.clone(), daily: true }
                }

                div { class: "grid gap-6 xl:grid-cols-2",
                    PageSection { title: "Zeiträume",
                        PeriodTable { rows: s.periods.iter().map(|p| (p.label.clone(), p.kwh, p.cost)).collect(), currency: cur.clone() }
                    }
                    PageSection { title: "Monate", description: "Die letzten 12 Monate",
                        PeriodTable {
                            rows: s.months.iter().rev().map(|b| (month_label(b.start), b.kwh, b.cost)).collect(),
                            currency: cur.clone(),
                        }
                    }
                }

                PageSection { title: "Letzte 48 Stunden", description: "Energie je Stunde – Kosten beim Darüberfahren",
                    Columns { buckets: s.hours.clone(), currency: cur.clone(), daily: false }
                }

                PriceSettings { onsaved: move |_| reload += 1 }
            }
        }
    }
}

/// Säulen für Tage oder Stunden: Höhe = kWh, Tooltip mit Kosten und mittlerer Leistung.
#[component]
fn Columns(buckets: Vec<EnergyBucket>, currency: String, daily: bool) -> Element {
    let max = buckets.iter().map(|b| b.kwh).fold(0.0, f64::max);
    if max <= 0.0 {
        return rsx! { p { class: "py-10 text-center text-sm text-muted-foreground", "Noch keine Werte in diesem Zeitraum." } };
    }
    let n = buckets.len();
    let every = if daily { 5 } else { 6 };
    rsx! {
        div { class: "flex h-48 items-end gap-0.5 sm:gap-1",
            for (i , b) in buckets.iter().enumerate() {
                {
                    let h = (b.kwh / max * 100.0).max(if b.kwh > 0.0 { 1.5 } else { 0.0 });
                    let when = if daily { format!("{} {}", fmt::day(b.start), weekday(b.start)) } else { format!("{} {}–{} Uhr", fmt::day(b.start), fmt::hm(b.start), fmt::hm(b.start + 3600)) };
                    let tip = format!(
                        "{when}\n{} · {}\nMittel {}{}",
                        energy(b.kwh),
                        money(b.cost, &currency),
                        Unit::Watts.format(b.avg_watts),
                        if b.coverage < 0.98 && b.coverage > 0.0 { format!(" · {} % erfasst", (b.coverage * 100.0).round()) } else { String::new() },
                    );
                    let opacity = if i + 1 == n { "0.55" } else { "1" };
                    rsx! {
                        div { key: "{b.start}", class: "group relative flex h-full min-w-0 flex-1 flex-col justify-end", title: "{tip}",
                            div {
                                class: "mx-auto w-full max-w-6 rounded-t-[4px] transition-opacity group-hover:opacity-80",
                                style: "height:{h:.1}%;background:{color(0)};opacity:{opacity}",
                            }
                        }
                    }
                }
            }
        }
        div { class: "mt-1.5 flex gap-0.5 text-[10px] text-muted-foreground tabular-nums sm:gap-1",
            for (i , b) in buckets.iter().enumerate() {
                span { key: "{b.start}", class: "min-w-0 flex-1 text-center whitespace-nowrap",
                    if (n - 1 - i) % every == 0 {
                        if daily { "{fmt::day(b.start)}" } else { "{fmt::hm(b.start)}" }
                    }
                }
            }
        }
    }
}

fn weekday(ts: i64) -> &'static str {
    const D: [&str; 7] = ["So", "Mo", "Di", "Mi", "Do", "Fr", "Sa"];
    D[Date::new(&JsValue::from_f64(ts as f64 * 1000.0)).get_day() as usize % 7]
}

#[component]
fn PeriodTable(rows: Vec<(String, f64, f64)>, currency: String) -> Element {
    rsx! {
        div { class: "-mx-4 overflow-x-auto sm:-mx-6",
            table { class: "w-full text-sm",
                thead {
                    tr { class: "border-b text-xs text-muted-foreground",
                        th { class: "px-4 py-2 text-left font-medium sm:px-6", "Zeitraum" }
                        th { class: "px-4 py-2 text-right font-medium", "Energie" }
                        th { class: "px-4 py-2 text-right font-medium sm:px-6", "Kosten" }
                    }
                }
                tbody { class: "divide-y",
                    for (label , kwh , cost) in rows {
                        tr { key: "{label}",
                            td { class: "px-4 py-2 sm:px-6", "{label}" }
                            td { class: "px-4 py-2 text-right tabular-nums", "{energy(kwh)}" }
                            td { class: "px-4 py-2 text-right font-medium tabular-nums sm:px-6", "{money(cost, &currency)}" }
                        }
                    }
                }
            }
        }
    }
}

/// Strompreise (mit „gilt ab“), Grundlast, Wirkungsgrad, Fühler, Währung.
#[component]
fn PriceSettings(onsaved: EventHandler<()>) -> Element {
    let mut settings = use_signal(|| None::<PowerSettings>);
    let mut new_cents = use_signal(|| "30".to_string());
    let mut new_from = use_signal(|| datetime_value(fmt::now()));
    let mut from_start = use_signal(|| false);
    let mut busy = use_signal(|| false);

    use_future(move || async move {
        if let Ok(s) = api::get::<PowerSettings>("/power/settings").await {
            if let Some(p) = s.prices.last() {
                new_cents.set(shared::model::trim(p.cents, 2));
            }
            settings.set(Some(s));
        }
    });

    let save = use_callback(move |next: PowerSettings| {
        busy.set(true);
        spawn(async move {
            match api::put::<_, PowerSettings>("/power/settings", &next).await {
                Ok(s) => {
                    settings.set(Some(s));
                    show_toast(ToastType::Success, "Strom-Einstellungen gespeichert – Kosten neu gerechnet");
                    onsaved.call(());
                }
                Err(e) => show_toast(ToastType::Error, format!("Nicht gespeichert: {e}")),
            }
            busy.set(false);
        });
    });

    let Some(s) = settings() else { return rsx! {} };
    let cur = s.currency.clone();
    rsx! {
        div { class: "grid gap-6 xl:grid-cols-2",
            SettingsGroup { title: "Strompreis", footer: "Ein Preis gilt ab seinem Zeitpunkt bis zum nächsten. Wählst du einen früheren Zeitpunkt, werden die bereits erfassten Werte mit dem neuen Preis neu gerechnet. Vor dem ersten Eintrag gilt der erste Preis.",
                for (i , p) in s.prices.iter().enumerate() {
                    SettingsRow {
                        key: "{p.from}",
                        label: cents(p.cents),
                        hint: if p.from <= 0 { "gilt seit Beginn".to_string() } else { format!("gilt ab {}", fmt::full(p.from)) },
                        Button {
                            variant: ButtonVariant::Ghost,
                            size: ButtonSize::Sm,
                            disabled: busy() || s.prices.len() <= 1,
                            onclick: {
                                let s = s.clone();
                                move |_| {
                                    let mut next = s.clone();
                                    next.prices.remove(i);
                                    save.call(next);
                                }
                            },
                            icons::Trash2 { class: "size-4" }
                            "Entfernen"
                        }
                    }
                }
                div { class: "grid gap-3 px-3 py-3",
                    p { class: "text-sm font-medium", "Neuer Preis" }
                    div { class: "flex flex-wrap items-center gap-2",
                        div { class: "flex items-center gap-1.5",
                            input { class: "{INPUT} w-24", r#type: "number", step: "0.01", min: "0", value: "{new_cents}", oninput: move |e| new_cents.set(e.value()) }
                            span { class: "text-sm text-muted-foreground", "ct/kWh" }
                        }
                        input {
                            class: "{INPUT}",
                            r#type: "datetime-local",
                            disabled: from_start(),
                            value: "{new_from}",
                            oninput: move |e| new_from.set(e.value()),
                        }
                        label { class: "flex items-center gap-2 text-sm text-muted-foreground",
                            input { r#type: "checkbox", class: "size-4 accent-foreground", checked: from_start(), onchange: move |e| from_start.set(e.checked()) }
                            "seit Beginn"
                        }
                    }
                    div { class: "flex gap-2",
                        Button {
                            size: ButtonSize::Sm,
                            disabled: busy(),
                            onclick: {
                                let s = s.clone();
                                move |_| {
                                    let Ok(c) = new_cents().replace(',', ".").parse::<f64>() else {
                                        show_toast(ToastType::Error, "Preis bitte als Zahl in Cent angeben");
                                        return;
                                    };
                                    let from = if from_start() { 0 } else { parse_datetime(&new_from()).unwrap_or_else(fmt::now) };
                                    let mut next = s.clone();
                                    next.prices.retain(|p| p.from != from);
                                    next.prices.push(PricePoint { from, cents: c });
                                    save.call(next);
                                }
                            },
                            icons::Plus { class: "size-4" }
                            "Preis übernehmen"
                        }
                        Button { variant: ButtonVariant::Outline, size: ButtonSize::Sm, onclick: move |_| new_from.set(datetime_value(fmt::now())), "Ab jetzt" }
                    }
                }
            }

            SettingsGroup { title: "Schätzung", footer: "RAPL misst nur CPU, Grafik und RAM. Grundlast und Wirkungsgrad machen daraus den Verbrauch am Netzstecker – mit einem Strommessgerät lässt sich die Grundlast genau abgleichen.",
                NumberRow { label: "Grundlast", hint: "Mainboard, SSD, Lüfter, Netzwerk (Watt)", value: s.base_watts, unit: "W", step: "0.5", busy: busy(),
                    onsave: {
                        let s = s.clone();
                        move |v: f64| { let mut n = s.clone(); n.base_watts = v; save.call(n); }
                    }
                }
                NumberRow { label: "Wirkungsgrad Netzteil", hint: "100 % = ohne Verlust", value: s.efficiency, unit: "%", step: "1", busy: busy(),
                    onsave: {
                        let s = s.clone();
                        move |v: f64| { let mut n = s.clone(); n.efficiency = v; save.call(n); }
                    }
                }
                SettingsRow { label: "Weitere Leistungsfühler", hint: "hwmon-Fühler (z. B. Grafikkarte) mitzählen – nur, wenn sie nicht schon in RAPL stecken",
                    input {
                        r#type: "checkbox",
                        class: "size-4 accent-foreground",
                        checked: s.include_sensors,
                        disabled: busy(),
                        onchange: {
                            let s = s.clone();
                            move |e: FormEvent| { let mut n = s.clone(); n.include_sensors = e.checked(); save.call(n); }
                        },
                    }
                }
                SettingsRow { label: "Währung", hint: "Zeichen für die Kosten",
                    input {
                        class: "{INPUT} w-20 text-center",
                        value: "{cur}",
                        maxlength: "4",
                        onchange: {
                            let s = s.clone();
                            move |e: FormEvent| { let mut n = s.clone(); n.currency = e.value(); save.call(n); }
                        },
                    }
                }
            }
        }
    }
}

/// Zahl mit Einheit, gespeichert beim Verlassen des Felds.
#[component]
fn NumberRow(label: String, hint: String, value: f64, unit: String, step: String, busy: bool, onsave: EventHandler<f64>) -> Element {
    rsx! {
        SettingsRow { label, hint,
            div { class: "flex items-center gap-1.5",
                input {
                    class: "{INPUT} w-24 text-right",
                    r#type: "number",
                    step: "{step}",
                    min: "0",
                    disabled: busy,
                    value: "{shared::model::trim(value, 2)}",
                    onchange: move |e| {
                        if let Ok(v) = e.value().replace(',', ".").parse::<f64>() {
                            onsave.call(v);
                        }
                    },
                }
                span { class: "w-6 text-sm text-muted-foreground", "{unit}" }
            }
        }
    }
}
