//! Gemeinsame Helfer der Systemseiten: Kennzahlen-Raster, Diagrammlisten, Werte nach Dimension.

use dioxus::prelude::*;
use shared::query::ChartData;

use crate::{
    components::{
        chart::{ChartCard, color, geom, section::ChartGrid, use_chart},
        lazy::Lazy,
        widgets::use_latest,
    },
    state::use_app,
};

/// Raster für Kennzahlen: 2 Spalten am Handy, bis zu `cols` am Desktop.
#[component]
pub fn KpiRow(#[props(default = 6)] cols: u8, children: Element) -> Element {
    let c = match cols {
        3 => "grid grid-cols-2 gap-4 lg:grid-cols-3",
        4 => "grid grid-cols-2 gap-4 lg:grid-cols-4",
        5 => "grid grid-cols-2 gap-4 md:grid-cols-3 xl:grid-cols-5",
        _ => "grid grid-cols-2 gap-4 md:grid-cols-3 xl:grid-cols-6",
    };
    rsx! { div { class: "{c}", {children} } }
}

/// Kleine Zwischenüberschrift über einer Gruppe.
#[component]
pub fn GroupTitle(#[props(into)] title: String, #[props(into, optional)] hint: Option<String>) -> Element {
    rsx! {
        div { class: "flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1",
            h3 { class: "text-xs font-medium tracking-wider text-muted-foreground uppercase", "{title}" }
            if let Some(h) = hint {
                span { class: "text-xs text-muted-foreground", "{h}" }
            }
        }
    }
}

/// Diagramm-Karten zu festen IDs; was der Server (noch) nicht kennt, fällt weg.
#[component]
pub fn Charts(
    ids: Vec<String>,
    #[props(default = 180)] height: u32,
    /// Höchstens zwei Spalten (für vier Karten: 2 × 2 statt 3 + 1).
    #[props(default)]
    pairs: bool,
) -> Element {
    let app = use_app();
    let ids: Vec<String> = ids.into_iter().filter(|id| app.chart(id).is_some_and(|c| !c.retired)).collect();
    if ids.is_empty() {
        return rsx! {};
    }
    let cards = rsx! {
        for id in ids {
            Lazy { key: "{id}", min_height: height + 110,
                ChartCard { id: id.clone(), height }
            }
        }
    };
    if pairs {
        rsx! { div { class: "grid gap-4 lg:grid-cols-2", {cards} } }
    } else {
        rsx! { ChartGrid { {cards} } }
    }
}

/// Neueste Werte eines Diagramms als (Dimension, Name, Wert) – in der Reihenfolge des Katalogs.
/// Solange noch kein Live-Takt da ist (seltene Diagramme melden sich nur alle 5–10 s), gilt der
/// letzte Wert aus dem geladenen Verlauf.
pub fn use_dims(id: &str) -> Memo<Vec<(String, String, Option<f64>)>> {
    let app = use_app();
    let latest = use_latest(id);
    let data = use_chart(id);
    let id = use_hook(|| id.to_string());
    use_memo(move || {
        let vals = latest.read();
        let Some(meta) = app.chart(&id) else { return Vec::new() };
        let loaded = || data.read();
        meta.dims
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let v = match vals.as_ref() {
                    Some(v) => v.get(i).copied().flatten(),
                    None => geom::latest(&series(&loaded(), &d.id)),
                };
                (d.id.clone(), d.name.clone(), v)
            })
            .collect()
    })
}

/// Wert einer Dimension aus [`use_dims`].
pub fn dim(list: &[(String, String, Option<f64>)], id: &str) -> Option<f64> {
    list.iter().find(|d| d.0 == id).and_then(|d| d.2)
}

/// Summe aller vorhandenen Werte (`None`, wenn keiner da ist).
pub fn sum(list: &[(String, String, Option<f64>)]) -> Option<f64> {
    let vals: Vec<f64> = list.iter().filter_map(|d| d.2).collect();
    (!vals.is_empty()).then(|| vals.iter().sum())
}

/// Mittelwert aller vorhandenen Werte.
pub fn mean(list: &[(String, String, Option<f64>)]) -> Option<f64> {
    let vals: Vec<f64> = list.iter().filter_map(|d| d.2).collect();
    (!vals.is_empty()).then(|| vals.iter().sum::<f64>() / vals.len() as f64)
}

/// Verlauf einer Dimension aus geladenen Diagrammdaten.
pub fn series(data: &Option<ChartData>, dim: &str) -> Vec<Option<f64>> {
    data.as_ref()
        .and_then(|d| d.dims.iter().position(|x| x == dim).and_then(|i| d.values.get(i).cloned()))
        .unwrap_or_default()
}

/// Verlauf für Mini-Diagramme ohne Lücken: seltene Diagramme (alle 5–10 s) liefern auf dem
/// Raster jeden zweiten Punkt leer – die Linie würde sonst in Einzelpunkte zerfallen.
pub fn compact(values: Vec<Option<f64>>) -> Vec<Option<f64>> {
    values.into_iter().filter(Option::is_some).collect()
}

/// Ein Abschnitt eines gestapelten Balkens.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub name: String,
    pub value: f64,
    /// Farbplatz (Reihenfarbe wie im Diagramm); `None` = neutral (z. B. „Frei“).
    pub slot: Option<usize>,
    pub text: String,
}

/// Gestapelter Balken mit Legende (Anteile an der Summe aller Abschnitte).
#[component]
pub fn StackBar(segments: Vec<Segment>, #[props(default = 16)] height: u32) -> Element {
    let total: f64 = segments.iter().map(|s| s.value.max(0.0)).sum();
    rsx! {
        div { class: "grid gap-3",
            div { class: "flex w-full gap-0.5 overflow-hidden rounded-md bg-muted", style: "height:{height}px",
                // Immer alle Abschnitte, Schlüssel = Platz: so werden nur Breiten aktualisiert,
                // nie Knoten umgehängt (sonst bleiben Farben am falschen Abschnitt hängen).
                if total > 0.0 {
                    for (i, s) in segments.iter().enumerate() {
                        div {
                            key: "{i}",
                            // Klasse fest, die Reihenfarbe kommt per `style` darüber.
                            class: "h-full bg-foreground/10 transition-[width] duration-500",
                            style: fill(s.value.max(0.0) / total * 100.0, s.slot),
                            title: "{s.name}: {s.text}",
                        }
                    }
                }
            }
            div { class: "flex flex-wrap gap-x-5 gap-y-2 text-xs",
                for s in segments.iter() {
                    div { key: "{s.name}", class: "flex items-center gap-1.5",
                        if let Some(slot) = s.slot {
                            span { class: "size-2.5 shrink-0 rounded-[3px]", style: "background:{color(slot)}" }
                        } else {
                            span { class: "size-2.5 shrink-0 rounded-[3px] border bg-muted" }
                        }
                        span { class: "text-muted-foreground", "{s.name}" }
                        span { class: "font-medium tabular-nums", "{s.text}" }
                        if total > 0.0 {
                            span { class: "text-muted-foreground tabular-nums", "{s.value / total * 100.0:.0} %" }
                        }
                    }
                }
            }
        }
    }
}

/// Zeile einer Balkenliste (Rangliste).
#[derive(Debug, Clone, PartialEq)]
pub struct BarRow {
    pub name: String,
    pub value: f64,
    pub text: String,
    pub slot: Option<usize>,
}

/// Balkenliste: Name, Balken relativ zum grössten Wert, Wert rechts.
#[component]
pub fn BarList(rows: Vec<BarRow>) -> Element {
    let max = rows.iter().map(|r| r.value).fold(0.0_f64, f64::max);
    rsx! {
        div { class: "grid gap-2.5",
            // Schlüssel = Rang: beim Umsortieren ändern sich nur Inhalte, keine Knoten.
            for (i, r) in rows.iter().enumerate() {
                div { key: "{i}", class: "grid gap-1",
                    div { class: "flex items-baseline justify-between gap-3 text-sm",
                        span { class: "flex min-w-0 items-center gap-1.5",
                            if let Some(slot) = r.slot {
                                span { class: "size-2.5 shrink-0 rounded-[3px]", style: "background:{color(slot)}" }
                            }
                            span { class: "truncate", "{r.name}" }
                        }
                        span { class: "shrink-0 font-medium tabular-nums", "{r.text}" }
                    }
                    div { class: "h-1.5 w-full overflow-hidden rounded-full bg-muted",
                        div {
                            // Ohne Übergang: Ranglisten sortieren sich laufend um.
                            class: "h-full rounded-full bg-foreground/60",
                            style: fill(pct(r.value, max), r.slot),
                        }
                    }
                }
            }
        }
    }
}

/// Breite (und Reihenfarbe, falls ein Farbplatz da ist) eines Balkenstücks.
fn fill(width_pct: f64, slot: Option<usize>) -> String {
    match slot {
        Some(slot) => format!("width:{width_pct:.2}%;background:{}", color(slot)),
        None => format!("width:{width_pct:.2}%"),
    }
}

/// Anteil in Prozent (0, wenn die Basis fehlt).
pub fn pct(v: f64, base: f64) -> f64 {
    if base > 0.0 { (v / base * 100.0).clamp(0.0, 100.0) } else { 0.0 }
}

/// Tönung einer Kachel nach Last (0–100): Abstufungen des Vordergrunds, keine festen Farben.
pub fn heat(p: f64) -> &'static str {
    match p {
        p if p >= 90.0 => "bg-foreground/30",
        p if p >= 75.0 => "bg-foreground/25",
        p if p >= 60.0 => "bg-foreground/20",
        p if p >= 45.0 => "bg-foreground/15",
        p if p >= 30.0 => "bg-foreground/10",
        p if p >= 15.0 => "bg-foreground/[0.07]",
        _ => "bg-foreground/[0.04]",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(v: &[Option<f64>]) -> Vec<(String, String, Option<f64>)> {
        v.iter().enumerate().map(|(i, x)| (format!("d{i}"), String::new(), *x)).collect()
    }

    #[test]
    fn summen() {
        assert_eq!(sum(&l(&[Some(1.0), None, Some(2.5)])), Some(3.5));
        assert_eq!(sum(&l(&[None])), None);
        assert_eq!(mean(&l(&[Some(1.0), Some(3.0), None])), Some(2.0));
        assert_eq!(dim(&l(&[None, Some(4.0)]), "d1"), Some(4.0));
    }

    #[test]
    fn luecken() {
        assert_eq!(compact(vec![None, Some(1.0), None, Some(2.0)]), vec![Some(1.0), Some(2.0)]);
    }

    #[test]
    fn anteile() {
        assert_eq!(pct(5.0, 0.0), 0.0);
        assert_eq!(pct(5.0, 10.0), 50.0);
        assert_eq!(pct(50.0, 10.0), 100.0);
        assert_eq!(heat(95.0), "bg-foreground/30");
        assert_eq!(heat(1.0), "bg-foreground/[0.04]");
    }
}
