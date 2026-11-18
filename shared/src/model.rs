//! Metrik-Modell: Diagramme (Charts), Dimensionen (Reihen) und Einheiten.
//!
//! Ein **Diagramm** hat eine eindeutige `id` (z. B. `disk.io.nvme0n1`), gehört zu einem **Kontext**
//! (`disk.io` – gleiche Dimensionen und Einheit bei allen Instanzen) und besteht aus
//! **Dimensionen** (`read`, `write`). Jede Dimension ist eine Zeitreihe mit einer festen `series`-Nummer.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Nummer einer Zeitreihe. Bleibt über Neustarts gleich (Tabelle `series` in DuckDB).
pub type SeriesId = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChartKind {
    /// Linien.
    #[default]
    Line,
    /// Gefüllte Flächen, übereinander gelegt (nicht gestapelt).
    Area,
    /// Gestapelte Flächen (Summe ist aussagekräftig, z. B. CPU-Zustände).
    Stacked,
}

/// Einheit der Werte. Bestimmt Achsenbeschriftung und Formatierung.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    /// Reine Anzahl.
    #[default]
    Count,
    /// 0–100 (bei CPU je Kern auch darüber).
    Percent,
    Bytes,
    BytesPerSec,
    BitsPerSec,
    /// Ereignisse je Sekunde.
    PerSec,
    /// Ereignisse je Minute.
    PerMin,
    Seconds,
    Millis,
    Celsius,
    Rpm,
    Volts,
    Amps,
    Watts,
    Mhz,
    /// Zustand als Zahl (0/1).
    State,
}

impl Unit {
    /// Kurze Achsenbeschriftung.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Count | Self::State => "",
            Self::Percent => "%",
            Self::Bytes => "B",
            Self::BytesPerSec => "B/s",
            Self::BitsPerSec => "bit/s",
            Self::PerSec => "/s",
            Self::PerMin => "/min",
            Self::Seconds => "s",
            Self::Millis => "ms",
            Self::Celsius => "°C",
            Self::Rpm => "rpm",
            Self::Volts => "V",
            Self::Amps => "A",
            Self::Watts => "W",
            Self::Mhz => "MHz",
        }
    }

    /// Wert lesbar formatieren, z. B. `1.4 GiB`, `23.5 %`, `12 ms`, `3 d 4 h`.
    #[must_use]
    pub fn format(self, v: f64) -> String {
        if !v.is_finite() {
            return "–".to_string();
        }
        match self {
            Self::Count | Self::State => scaled(v, 1000.0, &["", " k", " M", " G"], ""),
            Self::Percent => format!("{} %", trim(v, if v.abs() < 10.0 { 2 } else { 1 })),
            Self::Bytes => scaled(v, 1024.0, &[" B", " KiB", " MiB", " GiB", " TiB", " PiB"], ""),
            Self::BytesPerSec => scaled(v, 1024.0, &[" B", " KiB", " MiB", " GiB", " TiB"], "/s"),
            Self::BitsPerSec => scaled(v, 1000.0, &[" bit", " kbit", " Mbit", " Gbit", " Tbit"], "/s"),
            Self::PerSec => scaled(v, 1000.0, &["", " k", " M", " G"], "/s"),
            Self::PerMin => scaled(v, 1000.0, &["", " k", " M", " G"], "/min"),
            Self::Seconds => duration(v),
            Self::Millis => {
                if v.abs() >= 1000.0 {
                    format!("{} s", trim(v / 1000.0, 2))
                } else {
                    format!("{} ms", trim(v, if v.abs() < 10.0 { 2 } else { 1 }))
                }
            }
            Self::Celsius => format!("{} °C", trim(v, 1)),
            Self::Rpm => format!("{} rpm", trim(v, 0)),
            Self::Volts => format!("{} V", trim(v, 2)),
            Self::Amps => format!("{} A", trim(v, 2)),
            Self::Watts => format!("{} W", trim(v, 1)),
            Self::Mhz => {
                if v.abs() >= 1000.0 {
                    format!("{} GHz", trim(v / 1000.0, 2))
                } else {
                    format!("{} MHz", trim(v, 0))
                }
            }
        }
    }
}

/// Zahl mit höchstens `decimals` Nachkommastellen, ohne überflüssige Nullen.
#[must_use]
pub fn trim(v: f64, decimals: usize) -> String {
    let s = format!("{v:.decimals$}");
    if s.contains('.') {
        let t = s.trim_end_matches('0').trim_end_matches('.');
        if t == "-0" { "0".to_string() } else { t.to_string() }
    } else {
        s
    }
}

fn scaled(v: f64, base: f64, units: &[&str], suffix: &str) -> String {
    let mut x = v;
    let mut i = 0;
    while x.abs() >= base && i + 1 < units.len() {
        x /= base;
        i += 1;
    }
    let decimals = if i == 0 {
        if x.abs() < 10.0 && x.fract() != 0.0 { 2 } else if x.fract() != 0.0 { 1 } else { 0 }
    } else if x.abs() < 10.0 {
        2
    } else if x.abs() < 100.0 {
        1
    } else {
        0
    };
    format!("{}{}{}", trim(x, decimals), units.get(i).copied().unwrap_or(""), suffix)
}

/// Dauer in Sekunden lesbar: `45 s`, `12 min`, `3 h 20 min`, `5 d 2 h`.
#[must_use]
pub fn duration(secs: f64) -> String {
    let s = secs.abs();
    if s < 1.0 {
        return format!("{} ms", trim(secs * 1000.0, 0));
    }
    if s < 60.0 {
        return format!("{} s", trim(secs, if s < 10.0 { 1 } else { 0 }));
    }
    let total = s as u64;
    let (d, h, m) = (total / 86_400, (total % 86_400) / 3600, (total % 3600) / 60);
    if d > 0 {
        format!("{d} d {h} h")
    } else if h > 0 {
        format!("{h} h {m} min")
    } else {
        format!("{m} min")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DimMeta {
    /// Technischer Name, eindeutig im Diagramm (`user`, `read`).
    pub id: String,
    /// Anzeigename.
    pub name: String,
    pub series: SeriesId,
    /// Unter der Nulllinie zeichnen (z. B. „gesendet“, „schreiben“). Der Wert selbst ist positiv.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub negate: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChartMeta {
    /// Eindeutig, z. B. `system.cpu`, `disk.io.nvme0n1`, `docker.cpu.nrz-git_web_1`.
    pub id: String,
    /// Vorlage aus dem Katalog, z. B. `disk.io`.
    pub context: String,
    /// Hauptbereich (Menü): `system`, `cpu`, `memory`, `disk`, `network`, `sensors`, `processes`,
    /// `docker`, `apps`, `dns`, `cloudflare`, `netbird`, `probes`, `monitor`.
    pub section: String,
    /// Untergruppe im Bereich: Gerät, Schnittstelle, Container … (leer = keine).
    pub family: String,
    pub title: String,
    pub unit: Unit,
    pub kind: ChartKind,
    /// Reihenfolge im Bereich (klein = oben).
    pub priority: u32,
    /// Sekunden zwischen zwei Werten.
    pub interval: u32,
    pub dims: Vec<DimMeta>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub labels: BTreeMap<String, String>,
    /// Zeit des letzten Werts (Unix-Sekunden, 0 = noch keiner).
    #[serde(default)]
    pub last: i64,
    /// Liefert nichts mehr (Container entfernt, Gerät weg). Bleibt für alte Zeiträume abfragbar.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub retired: bool,
}

impl ChartMeta {
    #[must_use]
    pub fn label(&self, key: &str) -> Option<&str> {
        self.labels.get(key).map(String::as_str)
    }

    #[must_use]
    pub fn dim_index(&self, id: &str) -> Option<usize> {
        self.dims.iter().position(|d| d.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(Unit::Bytes.format(1536.0), "1.5 KiB");
        assert_eq!(Unit::Bytes.format(0.0), "0 B");
        assert_eq!(Unit::Percent.format(23.456), "23.5 %");
        assert_eq!(Unit::BitsPerSec.format(12_500_000.0), "12.5 Mbit/s");
        assert_eq!(Unit::Seconds.format(90_000.0), "1 d 1 h");
        assert_eq!(Unit::Millis.format(1500.0), "1.5 s");
        assert_eq!(Unit::Count.format(f64::NAN), "–");
        assert_eq!(Unit::Mhz.format(2400.0), "2.4 GHz");
    }
}
