//! Kleine Helfer für alle Sammler.

use std::time::{SystemTime, UNIX_EPOCH};

/// Jetzt in Unix-Sekunden.
pub fn now() -> i64 {
    now_ms() / 1000
}

/// Jetzt in Unix-Millisekunden.
pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// Auf `decimals` Nachkommastellen runden. Gerundete Werte lassen sich viel besser komprimieren –
/// darum **vor** dem Speichern runden (Prozent: 2, Bytes und Raten: 0–1, Temperatur: 1).
pub fn round(v: f64, decimals: u32) -> f64 {
    if !v.is_finite() {
        return v;
    }
    let f = 10f64.powi(decimals as i32);
    (v * f).round() / f
}

/// Macht aus einem Zähler (wächst nur) eine Rate je Sekunde.
#[derive(Debug, Default, Clone, Copy)]
pub struct Counter {
    prev: Option<u64>,
}

impl Counter {
    pub const fn new() -> Self {
        Self { prev: None }
    }

    /// Rate seit dem letzten Aufruf. `None` beim ersten Wert und wenn der Zähler zurückgesetzt wurde.
    pub fn rate(&mut self, current: u64, dt_secs: f64) -> Option<f64> {
        let prev = self.prev.replace(current)?;
        if current < prev || dt_secs <= 0.0 {
            return None;
        }
        Some((current - prev) as f64 / dt_secs)
    }

    /// Zuwachs seit dem letzten Aufruf (ohne Zeitbezug).
    pub fn delta(&mut self, current: u64) -> Option<u64> {
        let prev = self.prev.replace(current)?;
        current.checked_sub(prev)
    }
}

/// `Some(rate)` → Wert, `None` → NaN (= kein Wert in diesem Takt).
pub fn nan(v: Option<f64>) -> f64 {
    v.unwrap_or(f64::NAN)
}

/// Zufallsbewegung für Beispieldaten: bleibt zwischen `min` und `max`, zieht zur Mitte `base`.
#[derive(Debug, Clone)]
pub struct Walk {
    pub value: f64,
    base: f64,
    min: f64,
    max: f64,
    step: f64,
}

impl Walk {
    pub fn new(base: f64, min: f64, max: f64, step: f64) -> Self {
        Self { value: base, base, min, max, step }
    }

    pub fn next(&mut self) -> f64 {
        let pull = (self.base - self.value) * 0.05;
        self.value = (self.value + pull + (fastrand::f64() - 0.5) * 2.0 * self.step).clamp(self.min, self.max);
        self.value
    }

    /// Wie [`Walk::next`], gelegentlich mit einem Ausschlag (für Lastspitzen).
    pub fn next_spiky(&mut self, chance: f64, height: f64) -> f64 {
        let v = self.next();
        if fastrand::f64() < chance {
            self.value = (v + height * (0.5 + fastrand::f64())).clamp(self.min, self.max);
        }
        self.value
    }
}

/// `"90s"`, `"15m"`, `"24h"`, `"30d"`, `"2y"` → Sekunden.
pub fn parse_duration(s: &str) -> Option<u64> {
    let s = s.trim();
    let split = s.find(|c: char| !c.is_ascii_digit() && c != '.')?;
    let (n, unit) = s.split_at(split);
    let n: f64 = n.parse().ok()?;
    let mult = match unit.trim() {
        "s" => 1.0,
        "m" | "min" => 60.0,
        "h" => 3600.0,
        "d" => 86_400.0,
        "w" => 604_800.0,
        "y" => 31_536_000.0,
        _ => return None,
    };
    Some((n * mult) as u64)
}

/// `"512MB"`, `"2GiB"`, `"1.5G"` → Bytes.
pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let split = s.find(|c: char| !c.is_ascii_digit() && c != '.').unwrap_or(s.len());
    let (n, unit) = s.split_at(split);
    let n: f64 = n.parse().ok()?;
    let mult = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1.0,
        "k" | "kb" | "kib" => 1024.0,
        "m" | "mb" | "mib" => 1024.0 * 1024.0,
        "g" | "gb" | "gib" => 1024.0 * 1024.0 * 1024.0,
        "t" | "tb" | "tib" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    Some((n * mult) as u64)
}

/// Sichere Zufallsbytes als Hex (Sitzungen, Schlüssel).
pub fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    if getrandom::fill(&mut buf).is_err() {
        for b in &mut buf {
            *b = fastrand::u8(..);
        }
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Vergleich ohne Zeitunterschied (Passwörter, Schlüssel).
pub fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_rates() {
        let mut c = Counter::new();
        assert_eq!(c.rate(100, 1.0), None);
        assert_eq!(c.rate(150, 1.0), Some(50.0));
        assert_eq!(c.rate(250, 2.0), Some(50.0));
        assert_eq!(c.rate(10, 1.0), None);
        assert_eq!(c.rate(20, 1.0), Some(10.0));
    }

    #[test]
    fn parsing() {
        assert_eq!(parse_duration("24h"), Some(86_400));
        assert_eq!(parse_duration("30d"), Some(2_592_000));
        assert_eq!(parse_duration("x"), None);
        assert_eq!(parse_size("2GiB"), Some(2 * 1024 * 1024 * 1024));
        assert_eq!(parse_size("512mb"), Some(512 * 1024 * 1024));
        assert_eq!(round(12.3456, 2), 12.35);
    }
}
