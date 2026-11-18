//! Prozesse: Kennzahlen, sortier- und durchsuchbare Live-Tabelle, Rangliste der Programme, Verläufe.

use dioxus::prelude::*;
use shared::{
    catalog::system::*,
    model::Unit,
    system::{ProcInfo, ProcessList, snap},
};

use super::common::{BarList, BarRow, Charts, GroupTitle, KpiRow, dim, pct, use_dims};
use crate::{
    components::{
        ui::{
            app_shell::{LoadingState, PageContainer, PageHeader, PageSection, PageWidth},
            badge::{Badge, BadgeSize, BadgeVariant},
            input::{Input, InputType},
            table::{Table, TableBody, TableCell, TableHead, TableHeader, TableRow, TableWrapper},
        },
        widgets::{Meter, StatTile, show},
    },
    live::use_snap,
    state::use_app,
};

/// Höchstens so viele Zeilen zeichnen.
const SHOWN: usize = 150;

/// Sortierspalte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Col {
    Pid,
    Name,
    User,
    Cpu,
    Mem,
    State,
    Started,
    Container,
}

/// Treffer zur Suche, sortiert, gekürzt auf `limit`; dazu die Zahl aller Treffer.
pub fn select(procs: &[ProcInfo], query: &str, col: Col, desc: bool, limit: usize) -> (Vec<ProcInfo>, usize) {
    let q = query.trim().to_lowercase();
    let mut hits: Vec<&ProcInfo> = procs
        .iter()
        .filter(|p| {
            q.is_empty()
                || p.name.to_lowercase().contains(&q)
                || p.cmd.to_lowercase().contains(&q)
                || p.user.to_lowercase().contains(&q)
                || p.container.to_lowercase().contains(&q)
                || p.pid.to_string() == q
        })
        .collect();
    hits.sort_by(|a, b| {
        let o = match col {
            Col::Pid => a.pid.cmp(&b.pid),
            Col::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            Col::User => a.user.cmp(&b.user),
            Col::Cpu => a.cpu.total_cmp(&b.cpu),
            Col::Mem => a.mem.cmp(&b.mem),
            Col::State => a.state.cmp(&b.state),
            // Laufzeit: früher gestartet = länger gelaufen.
            Col::Started => b.started.cmp(&a.started),
            Col::Container => a.container.cmp(&b.container),
        };
        let o = if desc { o.reverse() } else { o };
        o.then(a.pid.cmp(&b.pid))
    });
    let n = hits.len();
    (hits.into_iter().take(limit).cloned().collect(), n)
}

fn state_label(s: &str) -> (&'static str, BadgeVariant) {
    match s {
        "R" => ("laufend", BadgeVariant::Secondary),
        "S" => ("schlafend", BadgeVariant::Muted),
        "D" => ("wartet auf E/A", BadgeVariant::Outline),
        "I" => ("Leerlauf", BadgeVariant::Muted),
        "T" | "t" => ("angehalten", BadgeVariant::Outline),
        "Z" => ("Zombie", BadgeVariant::Warning),
        _ => ("unbekannt", BadgeVariant::Muted),
    }
}

#[component]
pub fn Processes() -> Element {
    let app = use_app();
    let snap = use_snap::<ProcessList>(snap::PROCESSES);
    let states = use_dims(&PROC_STATES.chart_id(None));
    let mut query = use_signal(String::new);
    let mut sort = use_signal(|| (Col::Cpu, true));

    // Teure Arbeit nur, wenn Liste, Suche oder Sortierung sich ändern.
    let rows = use_memo(move || {
        let (col, desc) = sort();
        let q = query();
        snap.read().as_ref().map_or_else(|| (Vec::new(), 0), |l| select(&l.procs, &q, col, desc, SHOWN))
    });
    let ranking = use_memo(move || {
        let Some(l) = snap.read().clone() else { return (Vec::new(), Vec::new()) };
        // Farbe wie die Reihe des Programms in den Diagrammen.
        let slots: Vec<String> = app.chart(&PROC_CPU.chart_id(None)).map(|m| m.dims.into_iter().map(|d| d.id).collect()).unwrap_or_default();
        let slot = |n: &str| slots.iter().position(|d| d == n);
        let label = |n: &str, c: u32| format!("{} ({c})", if n == "other" { "Andere" } else { n });
        let mut by_cpu = l.groups.clone();
        by_cpu.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
        let mut by_mem = l.groups.clone();
        by_mem.sort_by_key(|g| std::cmp::Reverse(g.mem));
        let cpu = by_cpu
            .iter()
            .take(10)
            .map(|g| BarRow { name: label(&g.name, g.count), value: g.cpu, text: Unit::Percent.format(g.cpu), slot: slot(&g.name) })
            .collect();
        let mem = by_mem
            .iter()
            .take(10)
            .map(|g| BarRow { name: label(&g.name, g.count), value: g.mem as f64, text: Unit::Bytes.format(g.mem as f64), slot: slot(&g.name) })
            .collect();
        (cpu, mem)
    });

    let Some(list) = snap() else {
        return rsx! {
            PageContainer { width: PageWidth::Full,
                PageHeader { title: "Prozesse", description: "Was auf dem Host läuft und wie viel es braucht" }
                LoadingState { text: "Lade Prozessliste …" }
            }
        };
    };
    let zombies = dim(&states.read(), "zombie").or_else(|| Some(list.procs.iter().filter(|p| p.state == "Z").count() as f64));
    let cpu_sum: f64 = list.groups.iter().map(|g| g.cpu).sum();
    let mem_sum: u64 = list.groups.iter().map(|g| g.mem).sum();
    let cores = f64::from(list.cores.max(1));
    let (shown, hits) = rows();
    let (col, desc) = sort();
    let (rank_cpu, rank_mem) = ranking();
    let ts = list.ts;

    let head = move |label: &'static str, c: Col, class: &'static str| {
        let active = col == c;
        rsx! {
            TableHead { class: "{class}",
                button {
                    r#type: "button",
                    class: "inline-flex items-center gap-1 rounded outline-none hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 [&_svg]:size-3.5",
                    class: if active { "text-foreground" },
                    onclick: move |_| {
                        let (cur, d) = *sort.peek();
                        // Zahlen zuerst absteigend, Texte aufsteigend.
                        let first_desc = matches!(c, Col::Cpu | Col::Mem | Col::Started);
                        sort.set(if cur == c { (c, !d) } else { (c, first_desc) });
                    },
                    "{label}"
                    if active {
                        if desc {
                            icons::ArrowDown {}
                        } else {
                            icons::ArrowUp {}
                        }
                    }
                }
            }
        }
    };

    rsx! {
        PageContainer { width: PageWidth::Full,
            PageHeader { title: "Prozesse", description: "Was auf dem Host läuft und wie viel es braucht" }
            div { class: "grid gap-6",
                KpiRow {
                    StatTile { label: "Prozesse", value: Unit::Count.format(f64::from(list.total)), hint: format!("{} Programme", list.groups.len()) }
                    StatTile { label: "Threads", value: Unit::Count.format(f64::from(list.threads)), hint: format!("{:.1} je Prozess", f64::from(list.threads) / f64::from(list.total.max(1))) }
                    StatTile { label: "Laufend", value: "{list.running}", hint: "gerade auf einem Kern" }
                    StatTile { label: "Zombies", value: show(Unit::Count, zombies), hint: if zombies.unwrap_or(0.0) > 0.0 { "beendet, Eltern holen nicht ab" } else { "keine" } }
                    StatTile { label: "CPU gesamt", value: Unit::Percent.format(cpu_sum / cores), hint: format!("{} Kerne · 100 % = alle", list.cores),
                        Meter { value: cpu_sum / cores }
                    }
                    StatTile { label: "Speicher (RSS)", value: Unit::Bytes.format(mem_sum as f64), hint: format!("von {}", Unit::Bytes.format(list.mem_total as f64)),
                        Meter { value: pct(mem_sum as f64, list.mem_total as f64) }
                    }
                }
                PageSection {
                    title: "Prozessliste",
                    description: if hits > SHOWN { format!("{hits} Treffer, die ersten {SHOWN} gezeigt · CPU: 100 % = ein Kern") } else { format!("{hits} Treffer · CPU: 100 % = ein Kern") },
                    actions: rsx! {
                        div { class: "relative w-full sm:w-72",
                            span { class: "pointer-events-none absolute top-1/2 left-2.5 -translate-y-1/2 text-muted-foreground [&_svg]:size-4", icons::Search {} }
                            Input {
                                r#type: InputType::Search,
                                class: "pl-8",
                                placeholder: "Suchen: Name, Befehl, Benutzer, PID …",
                                value: query(),
                                oninput: move |e: FormEvent| query.set(e.value()),
                            }
                        }
                    },
                    class: "[&>div:last-child]:p-0 [&>div:last-child]:sm:p-0",
                    if shown.is_empty() {
                        p { class: "px-4 py-10 text-center text-sm text-muted-foreground", "Kein Prozess passt zur Suche." }
                    } else {
                        TableWrapper { class: "max-h-[38rem] rounded-none border-0",
                            Table { class: "max-w-none text-[13px]",
                                TableHeader {
                                    TableRow {
                                        {head("PID", Col::Pid, "hidden w-16 sm:table-cell")}
                                        {head("Name", Col::Name, "max-sm:w-full")}
                                        {head("Benutzer", Col::User, "hidden lg:table-cell")}
                                        {head("CPU", Col::Cpu, "text-right")}
                                        {head("Speicher", Col::Mem, "text-right")}
                                        {head("Zustand", Col::State, "hidden md:table-cell")}
                                        {head("Laufzeit", Col::Started, "hidden md:table-cell")}
                                        {head("Container", Col::Container, "hidden xl:table-cell")}
                                        TableHead { class: "hidden 2xl:table-cell", "Befehl" }
                                    }
                                }
                                TableBody {
                                    for p in shown.iter() {
                                        {
                                            let (st, variant) = state_label(&p.state);
                                            let age = (ts - p.started).max(0) as f64;
                                            rsx! {
                                                TableRow { key: "{p.pid}",
                                                    TableCell { class: "hidden px-2 py-1.5 text-muted-foreground tabular-nums sm:table-cell", "{p.pid}" }
                                                    TableCell { class: "max-w-0 px-2 py-1.5 sm:max-w-none",
                                                        div { class: "truncate font-medium", title: "{p.cmd}", "{p.name}" }
                                                        // Am Handy und ohne Befehlsspalte: Befehl klein darunter.
                                                        if !p.cmd.is_empty() {
                                                            div { class: "truncate font-mono text-[11px] text-muted-foreground 2xl:hidden", title: "{p.cmd}", "{p.cmd}" }
                                                        }
                                                    }
                                                    TableCell { class: "hidden px-2 py-1.5 lg:table-cell", "{p.user}" }
                                                    TableCell { class: "px-2 py-1.5 text-right whitespace-nowrap tabular-nums", "{Unit::Percent.format(p.cpu)}" }
                                                    TableCell { class: "px-2 py-1.5 text-right whitespace-nowrap tabular-nums", "{Unit::Bytes.format(p.mem as f64)}" }
                                                    TableCell { class: "hidden px-2 py-1.5 md:table-cell",
                                                        Badge { variant, size: BadgeSize::Sm, "{st}" }
                                                    }
                                                    TableCell { class: "hidden px-2 py-1.5 whitespace-nowrap text-muted-foreground tabular-nums md:table-cell", "{Unit::Seconds.format(age)}" }
                                                    TableCell { class: "hidden px-2 py-1.5 text-muted-foreground xl:table-cell",
                                                        div { class: "max-w-44 truncate", title: "{p.container}",
                                                            if p.container.is_empty() { "–" } else { "{p.container}" }
                                                        }
                                                    }
                                                    // Nimmt den Restplatz, ohne die Tabelle zu verbreitern (max-w-0 + w-full).
                                                    TableCell { class: "hidden w-full max-w-0 px-2 py-1.5 font-mono text-xs text-muted-foreground 2xl:table-cell",
                                                        div { class: "truncate", title: "{p.cmd}", "{p.cmd}" }
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
                div { class: "grid gap-4 lg:grid-cols-2",
                    PageSection { title: "CPU nach Programm", description: "Summe aller Prozesse, 100 % = ein Kern",
                        BarList { rows: rank_cpu }
                    }
                    PageSection { title: "Speicher nach Programm", description: "Summe des belegten Arbeitsspeichers (RSS)",
                        BarList { rows: rank_mem }
                    }
                }
                div { class: "grid gap-3",
                    GroupTitle { title: "Verlauf nach Programm" }
                    Charts { ids: vec![PROC_CPU.chart_id(None), PROC_MEM.chart_id(None), PROC_IO_READ.chart_id(None), PROC_IO_WRITE.chart_id(None), PROC_THREADS.chart_id(None), PROC_FDS.chart_id(None)], height: 200 }
                }
                div { class: "grid gap-3",
                    GroupTitle { title: "Anzahl und Zustände" }
                    Charts { ids: vec![PROC_COUNT.chart_id(None), PROC_STATES.chart_id(None)], height: 160, pairs: true }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, name: &str, cpu: f64, user: &str) -> ProcInfo {
        ProcInfo { pid, name: name.into(), cpu, user: user.into(), ..Default::default() }
    }

    #[test]
    fn suchen_und_sortieren() {
        let l = [p(1, "systemd", 0.1, "root"), p(20, "node", 5.0, "1000"), p(30, "Nginx", 1.0, "www-data")];
        let (r, n) = select(&l, "", Col::Cpu, true, 10);
        assert_eq!((r[0].pid, n), (20, 3));
        let (r, _) = select(&l, "nginx", Col::Cpu, true, 10);
        assert_eq!(r.len(), 1);
        let (r, _) = select(&l, "root", Col::Cpu, true, 10);
        assert_eq!(r[0].pid, 1);
        let (r, _) = select(&l, "20", Col::Cpu, true, 10);
        assert_eq!(r[0].pid, 20);
        let (r, n) = select(&l, "", Col::Name, false, 2);
        assert_eq!((r.iter().map(|p| p.pid).collect::<Vec<_>>(), n), (vec![30, 20], 3));
    }
}
