//! Katalog der bekannten Diagramme. Sammler registrieren ihre Diagramme aus diesen Vorlagen,
//! die Oberfläche findet sie über `context` wieder – so passen beide Seiten ohne Absprache zusammen.
//!
//! Regeln:
//! - `context` ist klein geschrieben, mit Punkten gegliedert, und bleibt stabil.
//! - Ein Diagramm ohne Instanz hat `id == context`. Mit Instanz ist die ID `context.<instanz>`
//!   und die Instanz steht zusätzlich als Label unter `instance_label` (`device`, `iface`, …).
//! - Leere `dims` = die Dimensionen entstehen zur Laufzeit (je Kern, je Sensor, je Container).
//! - Zähler werden nie roh gespeichert, immer als Rate (pro Sekunde).
//! - Titel auf Deutsch, Schweizer Schreibweise (ss statt ß).

use crate::model::{ChartKind, Unit};

pub mod cloudflare;
pub mod dns;
pub mod docker;
pub mod monitor;
pub mod netbird;
pub mod power;
pub mod probes;
pub mod system;

/// Dimension einer Vorlage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dim {
    pub id: &'static str,
    pub name: &'static str,
    /// Unter der Nulllinie zeichnen.
    pub negate: bool,
}

/// Dimension, die nach oben gezeichnet wird.
#[must_use]
pub const fn d(id: &'static str, name: &'static str) -> Dim {
    Dim { id, name, negate: false }
}

/// Dimension, die unter der Nulllinie gezeichnet wird (gesendet, schreiben).
#[must_use]
pub const fn dn(id: &'static str, name: &'static str) -> Dim {
    Dim { id, name, negate: true }
}

/// Vorlage eines Diagramms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tpl {
    pub context: &'static str,
    pub section: &'static str,
    pub title: &'static str,
    pub unit: Unit,
    pub kind: ChartKind,
    pub dims: &'static [Dim],
    pub priority: u32,
    /// Label, das Instanzen unterscheidet (`device`, `iface`, `container` …); leer = nur ein Diagramm.
    pub instance_label: &'static str,
    /// Sekunden zwischen zwei Werten (0 = Takt der lokalen Sammler, `NRZMON_INTERVAL`).
    pub interval: u32,
}

impl Tpl {
    #[must_use]
    pub const fn new(
        context: &'static str,
        section: &'static str,
        title: &'static str,
        unit: Unit,
        kind: ChartKind,
        dims: &'static [Dim],
        priority: u32,
    ) -> Self {
        Self { context, section, title, unit, kind, dims, priority, instance_label: "", interval: 0 }
    }

    /// Je Instanz ein Diagramm; die Instanz steht im Label `label`.
    #[must_use]
    pub const fn per(mut self, label: &'static str) -> Self {
        self.instance_label = label;
        self
    }

    #[must_use]
    pub const fn every(mut self, secs: u32) -> Self {
        self.interval = secs;
        self
    }

    /// Diagramm-ID für eine Instanz (`None` = ohne Instanz).
    #[must_use]
    pub fn chart_id(&self, instance: Option<&str>) -> String {
        match instance {
            Some(i) if !i.is_empty() => format!("{}.{}", self.context, sanitize(i)),
            _ => self.context.to_string(),
        }
    }
}

/// Instanzname für eine Diagramm-ID: nur `a-z A-Z 0-9 _ - .`, alles andere wird `_`.
#[must_use]
pub fn sanitize(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.') { c } else { '_' })
        .collect();
    if out.is_empty() {
        out.push('_');
    }
    out
}

/// Hauptbereich der Oberfläche.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Section {
    pub id: &'static str,
    pub title: &'static str,
}

/// Bereiche in Anzeigereihenfolge.
pub const SECTIONS: &[Section] = &[
    Section { id: "system", title: "System" },
    Section { id: "cpu", title: "CPU" },
    Section { id: "memory", title: "Arbeitsspeicher" },
    Section { id: "disk", title: "Datenträger" },
    Section { id: "network", title: "Netzwerk" },
    Section { id: "sensors", title: "Sensoren" },
    Section { id: "power", title: "Strom" },
    Section { id: "processes", title: "Prozesse" },
    Section { id: "docker", title: "Container" },
    Section { id: "apps", title: "Apps" },
    Section { id: "dns", title: "DNS" },
    Section { id: "cloudflare", title: "Cloudflare" },
    Section { id: "netbird", title: "NetBird" },
    Section { id: "probes", title: "Erreichbarkeit" },
    Section { id: "monitor", title: "nrzMonitor" },
];

#[must_use]
pub fn section_title(id: &str) -> &str {
    SECTIONS.iter().find(|s| s.id == id).map_or(id, |s| s.title)
}

/// Position eines Bereichs in der Anzeige (unbekannte ans Ende).
#[must_use]
pub fn section_order(id: &str) -> usize {
    SECTIONS.iter().position(|s| s.id == id).unwrap_or(SECTIONS.len())
}

/// Alle Vorlagen aller Bereiche.
pub fn all() -> impl Iterator<Item = &'static Tpl> {
    system::ALL
        .iter()
        .chain(docker::ALL)
        .chain(dns::ALL)
        .chain(cloudflare::ALL)
        .chain(netbird::ALL)
        .chain(probes::ALL)
        .chain(power::ALL)
        .chain(monitor::ALL)
}

#[must_use]
pub fn find(context: &str) -> Option<&'static Tpl> {
    all().find(|t| t.context == context)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn contexts_are_unique_and_sections_known() {
        let mut seen = BTreeSet::new();
        for t in all() {
            assert!(seen.insert(t.context), "doppelter Kontext {}", t.context);
            assert!(SECTIONS.iter().any(|s| s.id == t.section), "unbekannter Bereich {} in {}", t.section, t.context);
            let mut dims = BTreeSet::new();
            for d in t.dims {
                assert!(dims.insert(d.id), "doppelte Dimension {} in {}", d.id, t.context);
            }
        }
    }

    #[test]
    fn chart_ids() {
        assert_eq!(system::SYSTEM_CPU.chart_id(None), "system.cpu");
        assert_eq!(system::DISK_IO.chart_id(Some("nvme0n1")), "disk.io.nvme0n1");
        assert_eq!(system::DISK_SPACE.chart_id(Some("/mnt/data x")), "disk.space._mnt_data_x");
    }
}
