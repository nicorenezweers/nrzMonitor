//! Strom: Einstellungen (Preise mit Gültig-ab-Zeitpunkt, Grundlast, Wirkungsgrad) und die
//! Zusammenfassung von Energie und Kosten (`GET /api/v1/power/summary`).
//!
//! Gemessen wird mit RAPL (CPU, Grafik, RAM). Was RAPL nicht sieht (Mainboard, SSD, Lüfter …), kommt
//! als einstellbare Grundlast dazu; der Netzteilverlust ergibt sich aus dem Wirkungsgrad.
//! Kosten werden immer aus den gespeicherten Leistungswerten neu gerechnet – ein Preis, der
//! nachträglich (mit früherem „gilt ab“) eingetragen wird, wirkt also auch auf alte Werte.

use serde::{Deserialize, Serialize};

/// Strompreis ab einem Zeitpunkt.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PricePoint {
    /// Gilt ab (Unix-Sekunden). 0 = schon immer.
    pub from: i64,
    /// Cent je kWh.
    pub cents: f64,
}

/// Einstellungen (`core.settings`, Schlüssel `power`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PowerSettings {
    /// Preise, nach `from` sortiert. Vor dem ersten Eintrag gilt der erste Preis.
    pub prices: Vec<PricePoint>,
    /// Zusätzliche Leistung, die RAPL nicht misst (Watt).
    pub base_watts: f64,
    /// Wirkungsgrad des Netzteils in Prozent (100 = ohne Verlust).
    pub efficiency: f64,
    /// Leistungsfühler aus hwmon (z. B. Grafikkarte) mitzählen.
    pub include_sensors: bool,
    /// Währungszeichen der Anzeige.
    pub currency: String,
}

impl Default for PowerSettings {
    fn default() -> Self {
        Self { prices: vec![PricePoint { from: 0, cents: 30.0 }], base_watts: 8.0, efficiency: 90.0, include_sensors: false, currency: "€".into() }
    }
}

impl PowerSettings {
    /// Preis (Cent/kWh), der zum Zeitpunkt `ts` gilt.
    #[must_use]
    pub fn price_at(&self, ts: i64) -> f64 {
        let mut price = self.prices.first().map_or(0.0, |p| p.cents);
        for p in &self.prices {
            if p.from <= ts {
                price = p.cents;
            } else {
                break;
            }
        }
        price
    }

    /// Mittlerer Preis (Cent/kWh) zwischen `a` und `b`, nach Zeit gewichtet – für Abschnitte, in
    /// denen der Preis wechselt.
    #[must_use]
    pub fn price_between(&self, a: i64, b: i64) -> f64 {
        if b <= a {
            return self.price_at(a);
        }
        let mut total = 0.0;
        let mut t = a;
        while t < b {
            let next = self.prices.iter().map(|p| p.from).filter(|f| *f > t && *f < b).min().unwrap_or(b);
            total += self.price_at(t) * (next - t) as f64;
            t = next;
        }
        total / (b - a) as f64
    }

    /// Werte in gültige Bereiche bringen und Preise sortieren.
    pub fn normalize(&mut self) {
        self.prices.retain(|p| p.cents.is_finite() && p.cents >= 0.0);
        self.prices.sort_by_key(|p| p.from);
        self.prices.dedup_by_key(|p| p.from);
        if self.prices.is_empty() {
            self.prices.push(PricePoint { from: 0, cents: 30.0 });
        }
        if !self.base_watts.is_finite() {
            self.base_watts = 0.0;
        }
        self.base_watts = self.base_watts.clamp(0.0, 5000.0);
        if !self.efficiency.is_finite() || self.efficiency <= 0.0 {
            self.efficiency = 100.0;
        }
        self.efficiency = self.efficiency.clamp(30.0, 100.0);
        if self.currency.trim().is_empty() {
            self.currency = "€".into();
        }
    }
}

/// Energie und Kosten eines Zeitraums.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EnergyPeriod {
    /// `today`, `yesterday`, `7d`, `month`, `last_month`, `year`, `total`.
    pub key: String,
    pub label: String,
    /// Unix-Sekunden.
    pub from: i64,
    pub to: i64,
    pub kwh: f64,
    pub cost: f64,
}

/// Ein Abschnitt (Stunde, Tag, Monat) mit Energie, Kosten und mittlerer Leistung.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EnergyBucket {
    /// Beginn (Unix-Sekunden, Ortszeit-Grenze).
    pub start: i64,
    pub kwh: f64,
    pub cost: f64,
    /// Mittlere Leistung in Watt, solange Werte da waren.
    pub avg_watts: f64,
    /// Anteil des Abschnitts mit Werten (0–1).
    pub coverage: f64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PowerSummary {
    /// Unix-Sekunden.
    pub now: i64,
    pub currency: String,
    /// Aktueller Preis (Cent/kWh).
    pub price_now: f64,
    /// Es gibt echte Messwerte (RAPL oder Fühler); sonst nur Grundlast.
    pub measured: bool,
    /// Quellen der Messung, lesbar (z. B. `RAPL: Kerne, Grafik, RAM`).
    pub sources: Vec<String>,
    /// Aktuelle Leistung am Netzstecker (geschätzt), Watt.
    pub watts_now: Option<f64>,
    /// Mittel der letzten 24 Stunden, Watt.
    pub avg_watts_24h: Option<f64>,
    pub periods: Vec<EnergyPeriod>,
    /// Letzte 48 Stunden.
    pub hours: Vec<EnergyBucket>,
    /// Letzte 31 Tage (Ortszeit).
    pub days: Vec<EnergyBucket>,
    /// Letzte 12 Monate (Ortszeit).
    pub months: Vec<EnergyBucket>,
    /// Hochrechnung aus dem 24-h-Mittel und dem aktuellen Preis.
    pub projection_month_kwh: f64,
    pub projection_month_cost: f64,
    pub projection_year_kwh: f64,
    pub projection_year_cost: f64,
    /// Erster gespeicherter Wert (Unix-Sekunden).
    pub data_since: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn price_history() {
        let mut s = PowerSettings { prices: vec![PricePoint { from: 1000, cents: 40.0 }, PricePoint { from: 0, cents: 30.0 }], ..PowerSettings::default() };
        s.normalize();
        assert_eq!(s.price_at(-5), 30.0);
        assert_eq!(s.price_at(999), 30.0);
        assert_eq!(s.price_at(1000), 40.0);
        let late = PowerSettings { prices: vec![PricePoint { from: 500, cents: 25.0 }], ..PowerSettings::default() };
        assert_eq!(late.price_at(10), 25.0, "vor dem ersten Eintrag gilt der erste Preis");
        assert!((s.price_between(500, 1500) - 35.0).abs() < 1e-9, "halb 30, halb 40");
    }
}
