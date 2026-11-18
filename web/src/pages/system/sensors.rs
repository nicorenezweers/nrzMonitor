//! Sensoren: Kacheln je Fühler (nach Art und Chip), Abstand zum Grenzwert, darunter die Verläufe.

use dioxus::prelude::*;
use shared::{
    catalog::system::*,
    model::Unit,
    system::{SensorInfo, SensorKind, snap},
};

use super::common::{Charts, GroupTitle, KpiRow, compact, series};
use crate::{
    components::{
        chart::{ChartCard, section::ChartGrid, use_chart},
        lazy::Lazy,
        ui::app_shell::{EmptyState, LoadingState, PageContainer, PageHeader, PageWidth},
        widgets::{Meter, Sparkline, StatTile, StatusDot, Tone, show},
    },
    live::use_snap,
    state::use_app,
};

/// Reihenfolge der Arten auf der Seite.
const ORDER: [SensorKind; 7] = [
    SensorKind::Temp,
    SensorKind::Fan,
    SensorKind::Voltage,
    SensorKind::Power,
    SensorKind::Current,
    SensorKind::Thermal,
    SensorKind::Battery,
];

fn kind_title(k: SensorKind) -> &'static str {
    match k {
        SensorKind::Temp => "Temperaturen",
        SensorKind::Fan => "Lüfter",
        SensorKind::Voltage => "Spannungen",
        SensorKind::Power => "Leistung",
        SensorKind::Current => "Strom",
        SensorKind::Thermal => "Thermische Zonen",
        SensorKind::Battery => "Akku",
    }
}

pub fn unit(k: SensorKind) -> Unit {
    match k {
        SensorKind::Temp | SensorKind::Thermal => Unit::Celsius,
        SensorKind::Fan => Unit::Rpm,
        SensorKind::Voltage => Unit::Volts,
        SensorKind::Power => Unit::Watts,
        SensorKind::Current => Unit::Amps,
        SensorKind::Battery => Unit::Percent,
    }
}

/// Grenzwerte über 200 °C meldet manche Hardware als Platzhalter – nicht ernst nehmen.
fn sane(k: SensorKind, v: Option<f64>) -> Option<f64> {
    v.filter(|v| v.is_finite() && *v > 0.0 && !(matches!(k, SensorKind::Temp | SensorKind::Thermal) && *v > 200.0))
}

/// Beurteilung eines Fühlers.
#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    pub tone: Tone,
    pub state: &'static str,
    /// z. B. „noch 38 °C bis kritisch“.
    pub text: String,
    /// Füllstand bezogen auf die Grenze: (Wert %, Warnung ab %, kritisch ab %).
    pub meter: Option<(f64, f64, f64)>,
}

pub fn judge(s: &SensorInfo) -> Verdict {
    let u = unit(s.kind);
    let (max, crit) = (sane(s.kind, s.max), sane(s.kind, s.crit));
    let Some(v) = s.value else {
        return Verdict { tone: Tone::Neutral, state: "kein Wert", text: "Fühler liefert gerade nichts".into(), meter: None };
    };
    match s.kind {
        SensorKind::Battery => {
            let (tone, state) = match v {
                v if v < 10.0 => (Tone::Bad, "fast leer"),
                v if v < 20.0 => (Tone::Warn, "niedrig"),
                _ => (Tone::Good, "geladen"),
            };
            // Leer ist schlecht: der Balken zeigt die Ladung, Warnfarben passen hier nicht.
            Verdict { tone, state, text: format!("{} geladen", u.format(v)), meter: Some((v, 1000.0, 1000.0)) }
        }
        SensorKind::Fan => {
            let min = sane(s.kind, s.min);
            match min {
                Some(m) if v < m => Verdict { tone: Tone::Warn, state: "zu langsam", text: format!("Minimum {}", u.format(m)), meter: None },
                Some(m) => Verdict { tone: Tone::Good, state: "normal", text: format!("{} über Minimum", u.format(v - m)), meter: None },
                None if v <= 0.0 => Verdict { tone: Tone::Neutral, state: "steht", text: "dreht gerade nicht".into(), meter: None },
                None => Verdict { tone: Tone::Good, state: "dreht", text: "kein Minimum gemeldet".into(), meter: None },
            }
        }
        _ => {
            let Some(limit) = crit.or(max) else {
                let text = if s.crit.or(s.max).is_some() { "Grenzwert unbrauchbar" } else { "kein Grenzwert gemeldet" };
                return Verdict { tone: Tone::Neutral, state: "ohne Grenze", text: text.into(), meter: None };
            };
            let what = if crit.is_some() { "kritisch" } else { "Warngrenze" };
            let warn_pct = match (max, crit) {
                (Some(m), Some(c)) if m < c => m / c * 100.0,
                _ => 90.0,
            };
            let (tone, state) = if crit.is_some_and(|c| v >= c) {
                (Tone::Bad, "kritisch")
            } else if max.is_some_and(|m| v >= m) {
                (Tone::Warn, "über Warngrenze")
            } else if s.kind != SensorKind::Voltage && v >= limit * 0.9 {
                (Tone::Warn, "nahe Grenze")
            } else if sane(s.kind, s.min).is_some_and(|m| v < m) {
                (Tone::Warn, "unter Minimum")
            } else {
                (Tone::Good, "normal")
            };
            let text = if v >= limit {
                format!("{} über {what}", u.format(v - limit))
            } else {
                format!("noch {} bis {what}", u.format(limit - v))
            };
            // Spannungen liegen im Normalfall knapp unter der Grenze – ein Füllstand führt dort in die Irre.
            let meter = (s.kind != SensorKind::Voltage).then_some((v / limit * 100.0, warn_pct, 100.0));
            Verdict { tone, state, text, meter }
        }
    }
}

/// Fühler einer Art aus einem Diagramm (= ein Chip).
#[derive(Debug, Clone, PartialEq)]
struct Group {
    kind: SensorKind,
    chart: String,
    label: String,
    sensors: Vec<SensorInfo>,
}

fn groups(list: &[SensorInfo]) -> Vec<Group> {
    let mut out: Vec<Group> = Vec::new();
    for k in ORDER {
        for s in list.iter().filter(|s| s.kind == k) {
            match out.iter_mut().find(|g| g.kind == k && g.chart == s.chart && g.label == s.chip_label) {
                Some(g) => g.sensors.push(s.clone()),
                None => out.push(Group { kind: k, chart: s.chart.clone(), label: s.chip_label.clone(), sensors: vec![s.clone()] }),
            }
        }
    }
    out
}

#[component]
pub fn Sensors() -> Element {
    let app = use_app();
    let snap = use_snap::<Vec<SensorInfo>>(snap::SENSORS);
    let loaded = use_memo(move || snap.read().is_some());
    let list = use_memo(move || snap().unwrap_or_default());
    let grouped = use_memo(move || groups(&list.read()));

    let l = list.read();
    let temps: Vec<&SensorInfo> = l.iter().filter(|s| matches!(s.kind, SensorKind::Temp | SensorKind::Thermal) && s.value.is_some()).collect();
    let hottest = temps.iter().max_by(|a, b| a.value.unwrap_or(0.0).total_cmp(&b.value.unwrap_or(0.0)));
    // Kleinster Abstand zur Grenze (nur Temperaturen mit brauchbarer Grenze).
    let closest = temps
        .iter()
        .filter_map(|s| sane(s.kind, s.limit()).map(|lim| (s, lim - s.value.unwrap_or(0.0))))
        .min_by(|a, b| a.1.total_cmp(&b.1));
    let fans: Vec<f64> = l.iter().filter(|s| s.kind == SensorKind::Fan).filter_map(|s| s.value).collect();
    let power: Vec<f64> = l.iter().filter(|s| s.kind == SensorKind::Power).filter_map(|s| s.value).collect();
    let alerts = l.iter().filter(|s| matches!(judge(s).tone, Tone::Warn | Tone::Bad)).count();

    let contexts = [&SENSORS_TEMP, &SENSORS_FAN, &SENSORS_VOLTAGE, &SENSORS_POWER, &SENSORS_CURRENT, &SENSORS_BATTERY];
    let history: Vec<(&'static str, &'static str)> =
        contexts.iter().filter(|t| !app.by_context(t.context).is_empty()).map(|t| (t.context, t.title)).collect();

    rsx! {
        PageContainer { width: PageWidth::Full,
            PageHeader { title: "Sensoren", description: "Temperaturen, Lüfter, Spannungen und Leistung" }
            if !loaded() {
                LoadingState { text: "Lade Sensoren …" }
            } else if l.is_empty() {
                EmptyState {
                    title: "Keine Sensoren",
                    description: "Der Host meldet keine Fühler (hwmon, thermische Zonen) – in virtuellen Maschinen ist das üblich.",
                    icon: rsx! { icons::Thermometer {} },
                }
            } else {
                div { class: "grid gap-6",
                    KpiRow { cols: 4,
                        StatTile {
                            label: "Heissester Fühler",
                            value: show(Unit::Celsius, hottest.and_then(|s| s.value)),
                            hint: hottest.map_or_else(String::new, |s| format!("{} · {}", s.label, s.chip_label)),
                        }
                        StatTile {
                            label: "Knappster Abstand",
                            value: closest.map_or_else(|| "–".to_string(), |(_, d)| Unit::Celsius.format(d)),
                            hint: closest.map_or_else(|| "keine Grenzwerte gemeldet".to_string(), |(s, _)| format!("bis Grenze · {} ({})", s.label, s.chip_label)),
                        }
                        if !fans.is_empty() {
                            StatTile {
                                label: "Lüfter",
                                value: Unit::Rpm.format(fans.iter().sum::<f64>() / fans.len() as f64),
                                hint: format!("Mittel aus {} Lüftern", fans.len()),
                            }
                        } else if !power.is_empty() {
                            StatTile { label: "Leistung", value: Unit::Watts.format(power.iter().sum()), hint: "Summe der Fühler" }
                        } else {
                            StatTile { label: "Temperaturfühler", value: "{temps.len()}", hint: "mit aktuellem Wert" }
                        }
                        StatTile {
                            label: "Zustand",
                            value: if alerts == 0 { "Alles normal".to_string() } else { format!("{alerts} auffällig") },
                            hint: format!("{} Fühler überwacht", l.len()),
                            StatusDot {
                                tone: if alerts == 0 { Tone::Good } else { Tone::Warn },
                                label: if alerts == 0 { "keine Grenze in Sicht" } else { "nahe oder über einer Grenze" },
                            }
                        }
                    }
                    for k in ORDER {
                        if grouped.read().iter().any(|g| g.kind == k) {
                            div { key: "{kind_title(k)}", class: "grid gap-3",
                                GroupTitle { title: kind_title(k) }
                                div { class: "flex flex-wrap gap-x-3 gap-y-4 sm:gap-x-6",
                                    for g in grouped.read().iter().filter(|g| g.kind == k) {
                                        ChipGroup { key: "{g.chart}-{g.label}", group: g.clone() }
                                    }
                                }
                            }
                        }
                    }
                    div { class: "grid gap-6 border-t pt-6",
                        for (ctx, title) in history {
                            div { key: "{ctx}", class: "grid gap-3",
                                GroupTitle { title: "Verlauf: {title}" }
                                ChipCharts { context: ctx }
                            }
                        }
                        div { class: "grid gap-3",
                            if app.chart(&SENSORS_THERMAL.chart_id(None)).is_some() {
                                GroupTitle { title: "Verlauf: thermische Zonen" }
                            }
                            Charts { ids: vec![SENSORS_THERMAL.chart_id(None)], height: 170 }
                        }
                    }
                }
            }
        }
    }
}

/// Verläufe eines Kontexts je Chip, betitelt mit dem lesbaren Chipnamen.
#[component]
fn ChipCharts(context: String) -> Element {
    let app = use_app();
    let charts: Vec<(String, String)> = app
        .by_context(&context)
        .into_iter()
        .filter(|c| !c.retired)
        .map(|c| {
            let name = c.label("chip_label").map_or_else(|| c.family.clone(), str::to_string);
            (c.id, format!("{} · {name}", c.title))
        })
        .collect();
    rsx! {
        ChartGrid {
            for (id, title) in charts {
                Lazy { key: "{id}", min_height: 280,
                    ChartCard { id: id.clone(), title, height: 170 }
                }
            }
        }
    }
}

/// Fühler eines Chips: ein Diagramm für alle Mini-Verläufe.
#[component]
fn ChipGroup(group: Group) -> Element {
    let app = use_app();
    let data = use_chart(&group.chart);
    let chart = group.chart.clone();
    let ids = group.sensors.iter().map(|s| (s.id.clone(), s.dim)).collect::<Vec<_>>();
    // Verläufe nur neu schneiden, wenn neue Daten kommen.
    let lines = use_memo(move || {
        let d = data.read();
        let meta = app.chart(&chart);
        ids.iter()
            .map(|(id, i)| {
                // Dimension laut Katalog (`dim`), sonst der technische Name.
                let dim = meta.as_ref().and_then(|m| m.dims.get(*i)).map_or(id.as_str(), |d| d.id.as_str());
                compact(series(&d, dim))
            })
            .collect::<Vec<_>>()
    });
    let lines = lines.read();
    rsx! {
        // Am Handy stehen Chips mit nur einem Fühler zu zweit nebeneinander.
        div {
            class: "grid min-w-0 content-start gap-2",
            class: if group.sensors.len() == 1 { "max-sm:w-[calc(50%-0.375rem)]" } else { "max-sm:w-full" },
            p { class: "truncate text-sm font-medium",
                "{group.label}"
                span { class: "ml-2 text-xs font-normal text-muted-foreground", "{group.sensors.len()} Fühler" }
            }
            div {
                class: "grid gap-3 sm:flex sm:flex-wrap",
                class: if group.sensors.len() == 1 { "grid-cols-1" } else { "grid-cols-2" },
                for (i, s) in group.sensors.iter().enumerate() {
                    SensorTile { key: "{s.id}", sensor: s.clone(), line: lines.get(i).cloned().unwrap_or_default() }
                }
            }
        }
    }
}

/// „51 – 66.6 °C“: Einheit nur einmal, wenn sie hinten steht.
fn range(u: Unit, lo: f64, hi: f64) -> String {
    let (a, b) = (u.format(lo), u.format(hi));
    match (a.rsplit_once(' '), b.rsplit_once(' ')) {
        (Some((x, ua)), Some((_, ub))) if ua == ub => format!("{x} – {b}"),
        _ => format!("{a} – {b}"),
    }
}

#[component]
fn SensorTile(sensor: SensorInfo, line: Vec<Option<f64>>) -> Element {
    let u = unit(sensor.kind);
    let v = judge(&sensor);
    let lo_hi = line.iter().flatten().fold(None, |a: Option<(f64, f64)>, &x| Some(a.map_or((x, x), |(l, h)| (l.min(x), h.max(x)))));
    rsx! {
        div { class: "flex min-w-0 flex-col rounded-xl border bg-card p-3 shadow-sm sm:w-56 sm:p-4",
            div { class: "flex items-start justify-between gap-2",
                span { class: "truncate text-xs text-muted-foreground", title: "{sensor.label} ({sensor.id})", "{sensor.label}" }
                span { class: "hidden sm:inline", StatusDot { tone: v.tone, label: v.state } }
            }
            p { class: "mt-1 text-2xl font-semibold tracking-tight tabular-nums sm:text-3xl", "{show(u, sensor.value)}" }
            div { class: "mt-2",
                Sparkline { values: line.clone(), height: 30, slot: sensor.dim }
            }
            if let Some((lo, hi)) = lo_hi {
                p { class: "truncate text-[11px] text-muted-foreground tabular-nums", title: "Tiefst- und Höchstwert im Zeitraum",
                    "{range(u, lo, hi)}"
                    span { class: "hidden sm:inline", " im Zeitraum" }
                }
            }
            div { class: "mt-auto grid gap-1.5 pt-3",
                if let Some((p, warn, crit)) = v.meter {
                    Meter { value: p, warn, crit }
                }
                p { class: "truncate text-xs", title: "{v.text}", "{v.text}" }
                span { class: "sm:hidden", StatusDot { tone: v.tone, label: v.state } }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(v: f64, max: Option<f64>, crit: Option<f64>) -> SensorInfo {
        SensorInfo { kind: SensorKind::Temp, value: Some(v), max, crit, ..Default::default() }
    }

    #[test]
    fn grenzen() {
        let n = judge(&temp(46.0, Some(84.0), Some(100.0)));
        assert_eq!(n.tone, Tone::Good);
        assert_eq!(n.text, "noch 54 °C bis kritisch");
        assert_eq!(n.meter, Some((46.0, 84.0, 100.0)));
        assert_eq!(judge(&temp(86.0, Some(84.0), Some(100.0))).tone, Tone::Warn);
        assert_eq!(judge(&temp(101.0, Some(84.0), Some(100.0))).state, "kritisch");
        // Unsinnige Grenze (Platzhalter der Hardware) wird ignoriert.
        let mut volt = temp(12.13, Some(12.6), None);
        volt.kind = SensorKind::Voltage;
        let vv = judge(&volt);
        assert_eq!((vv.tone, vv.meter), (Tone::Good, None));
        let x = judge(&temp(40.0, Some(65261.8), Some(65261.8)));
        assert_eq!(x.tone, Tone::Neutral);
        assert!(x.meter.is_none());
        // Nur Warngrenze.
        assert_eq!(judge(&temp(60.0, Some(70.0), None)).text, "noch 10 °C bis Warngrenze");
    }

    #[test]
    fn spanne() {
        assert_eq!(range(Unit::Celsius, 51.0, 66.6), "51 – 66.6 °C");
        assert_eq!(range(Unit::Rpm, 600.0, 1200.0), "600 – 1200 rpm");
    }

    #[test]
    fn luefter_und_akku() {
        let f = SensorInfo { kind: SensorKind::Fan, value: Some(400.0), min: Some(600.0), ..Default::default() };
        assert_eq!(judge(&f).tone, Tone::Warn);
        let b = SensorInfo { kind: SensorKind::Battery, value: Some(8.0), ..Default::default() };
        assert_eq!(judge(&b).tone, Tone::Bad);
    }

    #[test]
    fn gruppen_nach_art() {
        let mk = |k, chart: &str| SensorInfo { kind: k, chart: chart.into(), chip_label: chart.into(), ..Default::default() };
        let g = groups(&[mk(SensorKind::Fan, "f"), mk(SensorKind::Temp, "a"), mk(SensorKind::Temp, "a"), mk(SensorKind::Temp, "b")]);
        assert_eq!(g.iter().map(|g| (g.chart.as_str(), g.sensors.len())).collect::<Vec<_>>(), [("a", 2), ("b", 1), ("f", 1)]);
    }
}
