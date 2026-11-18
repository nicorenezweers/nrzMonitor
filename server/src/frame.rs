//! Ein Takt eines Sammlers: Werte für beliebig viele Diagramme zu einem Zeitpunkt.

use crate::registry::ChartHandle;

pub struct Row {
    pub chart: ChartHandle,
    /// Beginn der Werte dieses Diagramms in [`Frame::values`].
    pub offset: usize,
}

pub struct Frame {
    /// Unix-Sekunden. Darf bis zu 15 Minuten in der Vergangenheit liegen (verzögerte Quellen).
    pub ts: i64,
    pub rows: Vec<Row>,
    pub values: Vec<f64>,
}

impl Frame {
    pub fn new(ts: i64) -> Self {
        Self { ts, rows: Vec::new(), values: Vec::new() }
    }

    /// Werte eines Diagramms abgeben – in der Reihenfolge seiner Dimensionen, `NaN` = kein Wert.
    /// Zu kurze Listen werden mit `NaN` aufgefüllt, zu lange abgeschnitten.
    pub fn put(&mut self, chart: &ChartHandle, values: &[f64]) {
        let n = chart.len();
        if n == 0 {
            return;
        }
        self.rows.push(Row { chart: chart.clone(), offset: self.values.len() });
        self.values.extend(values.iter().copied().take(n));
        for _ in values.len()..n {
            self.values.push(f64::NAN);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Anzahl echter Werte (ohne `NaN`).
    pub fn samples(&self) -> usize {
        self.values.iter().filter(|v| v.is_finite()).count()
    }

    /// Werte eines Diagramms in diesem Takt.
    pub fn values_of(&self, row: &Row) -> &[f64] {
        &self.values[row.offset..row.offset + row.chart.len()]
    }

    /// Alle (Reihe, Wert)-Paare mit echtem Wert.
    pub fn samples_iter(&self) -> impl Iterator<Item = (shared::SeriesId, f64)> + '_ {
        self.rows.iter().flat_map(move |row| {
            row.chart.series.iter().copied().zip(self.values_of(row).iter().copied()).filter(|(_, v)| v.is_finite())
        })
    }
}
