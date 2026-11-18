//! Diagramme. Übersicht der Bausteine:
//!
//! - [`ChartCard`]: fertige Karte zu einem Diagramm aus dem Katalog (`id`), folgt dem globalen
//!   Zeitraum, läuft live, mit Legende, Fadenkreuz und Zoom. **Der Normalfall.**
//! - [`SectionCharts`] / [`ContextCharts`]: alle Diagramme eines Bereichs bzw. Kontexts im Raster.
//! - [`Plot`]: nur die Zeichenfläche für eigene Daten ([`PlotData`]), z. B. aus einem eigenen Endpunkt.
//! - [`use_chart`]: nur die Daten eines Diagramms (für eigene Darstellungen, Kennzahlen).
//! - [`Heatmap`], [`StateBand`], [`BarList`]: weitere Formen (viele Reihen, an/aus, Rangliste).
//! - [`ChartDialog`] / [`expand`]: grosse Ansicht mit Statistik, Tabelle und CSV.
//! - [`nav`]: Zeitraum zoomen, verschieben, anhalten (mit „Zurück“-Verlauf).
//! - [`marks`]: Ereignis-Marken; [`Threshold`]: Grenzlinien.
//! - [`geom`]: Rechenteil (Wertebereich, Pfade, Zeitachse, Statistik, CSV).

pub mod card;
pub mod data;
pub mod forms;
pub mod geom;
pub mod marks;
pub mod nav;
pub mod plot;
pub mod section;

pub use card::{ChartCard, ChartDialog, ChartView, expand};
pub use data::{use_chart, use_feed};
pub use forms::{BarItem, BarList, Heatmap, StateBand};
pub use geom::{PlotData, PlotSeries};
pub use plot::{Mark, Plot, Threshold};
pub use section::{ContextCharts, SectionCharts};

/// Farbe eines Farbplatzes: acht feste Töne in fester Reihenfolge, alles darüber neutral.
/// Die Farbe gehört zur Reihe (Platz in der Liste der Dimensionen), nie zu ihrem Rang.
pub fn color(slot: usize) -> &'static str {
    const COLORS: [&str; 8] = [
        "var(--series-1)",
        "var(--series-2)",
        "var(--series-3)",
        "var(--series-4)",
        "var(--series-5)",
        "var(--series-6)",
        "var(--series-7)",
        "var(--series-8)",
    ];
    COLORS.get(slot).copied().unwrap_or("var(--series-other)")
}
