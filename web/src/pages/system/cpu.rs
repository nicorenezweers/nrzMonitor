//! CPU: Kennzahlen, Kerne als Kacheln mit Mini-Verlauf, dann die Verläufe.

use dioxus::prelude::*;
use shared::{catalog::system::*, model::Unit, system::{HostInfo, snap}};

use super::common::{Charts, GroupTitle, KpiRow, compact, dim, heat, mean, series, sum, use_dims};
use crate::{
    components::{
        chart::use_chart,
        ui::app_shell::{PageContainer, PageHeader, PageWidth},
        widgets::{Meter, Sparkline, StatTile, show},
    },
    live::use_snap,
    state::use_app,
};

#[component]
pub fn Cpu() -> Element {
    let info = use_snap::<HostInfo>(snap::INFO);
    let cpu = use_dims(&SYSTEM_CPU.chart_id(None));
    let load = use_dims(&SYSTEM_LOAD.chart_id(None));
    let freq = use_dims(&CPU_FREQ.chart_id(None));
    let temp = use_dims(&CPU_TEMP.chart_id(None));
    let power = use_dims(&CPU_POWER.chart_id(None));

    let description = match info() {
        Some(i) if !i.cpu_model.is_empty() => format!("{} · {} logische Kerne · {}", i.cpu_model, i.cpu_cores, i.arch),
        _ => "Auslastung, Kerne, Takt und Temperatur".to_string(),
    };
    let total = sum(&cpu.read());
    let (l1, l5, l15) = {
        let l = load.read();
        (dim(&l, "load1"), dim(&l, "load5"), dim(&l, "load15"))
    };
    let cores = info().map(|i| i.cpu_cores).filter(|c| *c > 0);
    let load_hint = match (l1, cores) {
        (Some(l), Some(c)) => format!("{:.0} %", l / f64::from(c) * 100.0),
        _ => String::new(),
    };
    let f = freq.read();
    let f_max = f.iter().filter_map(|d| d.2).fold(None, |a: Option<f64>, v| Some(a.map_or(v, |a| a.max(v))));
    let t = temp.read();
    let t_pkg = dim(&t, "package").or_else(|| t.iter().filter_map(|d| d.2).reduce(f64::max));
    let t_hot = t.iter().filter(|d| d.0 != "package").filter_map(|d| d.2.map(|v| (d.1.clone(), v))).max_by(|a, b| a.1.total_cmp(&b.1));
    let p = power.read();
    let p_total = sum(&p);

    rsx! {
        PageContainer { width: PageWidth::Full,
            PageHeader { title: "CPU", description }
            div { class: "grid gap-6",
                KpiRow {
                    StatTile { label: "Auslastung", value: show(Unit::Percent, total), hint: "alle Kerne",
                        Meter { value: total.unwrap_or(0.0) }
                    }
                    StatTile {
                        label: "Last (1 min)",
                        value: load_num(l1),
                        hint: format!("5 / 15 min: {} / {}", load_num(l5), load_num(l15)),
                    }
                    StatTile { label: "Takt", value: show(Unit::Mhz, mean(&f)), hint: format!("Mittel · höchster {}", show(Unit::Mhz, f_max)) }
                    StatTile {
                        label: "Temperatur",
                        value: show(Unit::Celsius, t_pkg),
                        hint: t_hot.map_or_else(|| "Gehäuse".to_string(), |(n, v)| format!("Gehäuse · max. {n} {}", Unit::Celsius.format(v))),
                    }
                    StatTile {
                        label: "Leistung (RAPL)",
                        value: show(Unit::Watts, p_total),
                        hint: dim(&p, "cores").map_or_else(|| "Gehäuse gesamt".to_string(), |c| format!("davon Kerne {}", Unit::Watts.format(c))),
                    }
                    StatTile {
                        label: "Kerne",
                        value: cores.map_or_else(|| "–".to_string(), |c| c.to_string()),
                        hint: if load_hint.is_empty() { "logisch".to_string() } else { format!("Last: {load_hint}") },
                    }
                }
                CoreTiles {}
                div { class: "grid gap-3",
                    GroupTitle { title: "Auslastung und Engpässe" }
                    Charts { ids: vec![SYSTEM_CPU.chart_id(None), SYSTEM_LOAD.chart_id(None), SYSTEM_PRESSURE.chart_id(None)], height: 200 }
                }
                div { class: "grid gap-3",
                    GroupTitle { title: "Temperatur, Takt und Leistung" }
                    Charts { ids: vec![CPU_TEMP.chart_id(None), CPU_FREQ.chart_id(None), CPU_POWER.chart_id(None)] }
                }
                div { class: "grid gap-3",
                    GroupTitle { title: "Unterbrechungen, Prozesse und Drosselung" }
                    Charts {
                        ids: vec![
                            CPU_SOFTIRQS.chart_id(None),
                            SYSTEM_CTXT.chart_id(None),
                            SYSTEM_INTR.chart_id(None),
                            SYSTEM_FORKS.chart_id(None),
                            SYSTEM_PROCESSES.chart_id(None),
                            CPU_THROTTLE.chart_id(None),
                        ],
                        height: 160,
                    }
                }
            }
        }
    }
}

fn load_num(v: Option<f64>) -> String {
    v.map_or_else(|| "–".to_string(), |v| format!("{v:.2}"))
}

/// Kerne als Kacheln: Last als Tönung und Füllhöhe, Wert, Takt/Temperatur und Mini-Verlauf.
#[component]
fn CoreTiles() -> Element {
    let app = use_app();
    let id = CPU_CORES.chart_id(None);
    let cores = use_dims(&id);
    let freq = use_dims(&CPU_FREQ.chart_id(None));
    let temp = use_dims(&CPU_TEMP.chart_id(None));
    let data = use_chart(&id);
    // Verläufe nur neu schneiden, wenn neue Daten da sind.
    let lines = use_memo(move || {
        let d = data.read();
        let Some(meta) = app.chart(&CPU_CORES.chart_id(None)) else { return Vec::new() };
        meta.dims.iter().map(|m| compact(series(&d, &m.id))).collect::<Vec<_>>()
    });

    let list = cores.read();
    if list.is_empty() {
        return rsx! {};
    }
    let f = freq.read();
    let t = temp.read();
    let lines = lines.read();
    // Wenige Kerne füllen die Breite, viele brechen um.
    let cols = match list.len() {
        0..=4 => "grid grid-cols-2 gap-3 sm:grid-cols-4",
        5..=8 => "grid grid-cols-2 gap-3 sm:grid-cols-4 xl:grid-cols-8",
        _ => "grid grid-cols-2 gap-3 sm:grid-cols-[repeat(auto-fill,minmax(10rem,1fr))]",
    };
    let hottest = list.iter().filter_map(|d| d.2).fold(0.0_f64, f64::max);
    rsx! {
        div { class: "grid gap-3",
            GroupTitle { title: "Kerne", hint: format!("{} Kerne · höchster {}", list.len(), Unit::Percent.format(hottest)) }
            div { class: "{cols}",
                for (i, (dim_id, name, v)) in list.iter().enumerate() {
                    {
                        let p = v.unwrap_or(0.0);
                        // `cpu3` → Takt `cpu3`, Temperatur `core3`.
                        let n = dim_id.trim_start_matches("cpu");
                        let mhz = dim(&f, dim_id);
                        let deg = dim(&t, &format!("core{n}"));
                        let detail = [mhz.map(|v| Unit::Mhz.format(v)), deg.map(|v| Unit::Celsius.format(v))]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>()
                            .join(" · ");
                        let line = lines.get(i).cloned().unwrap_or_default();
                        rsx! {
                            div {
                                key: "{dim_id}",
                                class: "relative overflow-hidden rounded-xl border p-3 shadow-sm transition-colors duration-500 {heat(p)}",
                                title: "{name}: {show(Unit::Percent, *v)}",
                                // Füllhöhe = Last
                                div {
                                    class: "pointer-events-none absolute inset-x-0 bottom-0 bg-foreground/[0.06] transition-[height] duration-500",
                                    style: "height:{p.clamp(0.0, 100.0):.1}%",
                                }
                                div { class: "relative",
                                    div { class: "flex items-baseline justify-between gap-2",
                                        span { class: "text-xs text-muted-foreground", "{name}" }
                                        span { class: "truncate text-[11px] text-muted-foreground tabular-nums", "{detail}" }
                                    }
                                    p { class: "mt-1 text-xl font-semibold tracking-tight tabular-nums", "{show(Unit::Percent, *v)}" }
                                    div { class: "mt-2",
                                        Sparkline { values: line, height: 28, slot: 0 }
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
