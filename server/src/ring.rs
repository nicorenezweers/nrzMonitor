//! Ringpuffer im Arbeitsspeicher: die letzten Minuten jeder Reihe in voller Auflösung.
//! Daraus kommen die Live-Diagramme – ohne einen einzigen Zugriff auf den Datenträger.

use parking_lot::RwLock;
use shared::SeriesId;

use crate::frame::Frame;

/// Mindestens so viele Punkte je Reihe – langsame Reihen (60 s) bleiben so stundenlang im Speicher.
const MIN_POINTS: u32 = 240;

struct Buf {
    step: u32,
    /// `ts / step` des neuesten Werts.
    last: i64,
    data: Box<[f32]>,
}

impl Buf {
    fn new(step: u32, window: u32) -> Self {
        let cap = (window / step.max(1)).max(MIN_POINTS) as usize;
        Self { step: step.max(1), last: i64::MIN, data: vec![f32::NAN; cap].into_boxed_slice() }
    }

    fn write(&mut self, ts: i64, v: f64) {
        let cap = self.data.len() as i64;
        let slot = ts.div_euclid(i64::from(self.step));
        if self.last == i64::MIN || slot > self.last {
            // Übersprungene Plätze leeren.
            let from = if self.last == i64::MIN { slot - cap + 1 } else { self.last + 1 };
            for s in from.max(slot - cap + 1)..slot {
                self.data[s.rem_euclid(cap) as usize] = f32::NAN;
            }
            self.last = slot;
        } else if slot <= self.last - cap {
            return; // zu alt
        }
        self.data[slot.rem_euclid(cap) as usize] = v as f32;
    }

    fn at(&self, slot: i64) -> Option<f64> {
        let cap = self.data.len() as i64;
        if self.last == i64::MIN || slot > self.last || slot <= self.last - cap {
            return None;
        }
        let v = self.data[slot.rem_euclid(cap) as usize];
        if v.is_nan() { None } else { Some(widen(v)) }
    }
}

/// `f32` → `f64` ohne Rundungsreste: 10.3 bleibt 10.3 (nicht 10.300000190734863).
fn widen(v: f32) -> f64 {
    let direct = f64::from(v);
    if direct.fract() == 0.0 {
        return direct;
    }
    v.to_string().parse().unwrap_or(direct)
}

pub struct Ring {
    window: u32,
    bufs: RwLock<Vec<Option<Buf>>>,
}

impl Ring {
    pub fn new(window_secs: u32) -> Self {
        Self { window: window_secs, bufs: RwLock::new(Vec::new()) }
    }

    pub fn write(&self, frame: &Frame) {
        let mut bufs = self.bufs.write();
        for row in &frame.rows {
            let step = row.chart.interval.max(1);
            for (series, v) in row.chart.series.iter().zip(frame.values_of(row)) {
                if !v.is_finite() {
                    continue;
                }
                let i = *series as usize;
                if bufs.len() <= i {
                    bufs.resize_with(i + 1, || None);
                }
                let buf = match &mut bufs[i] {
                    Some(b) if b.step == step => b,
                    slot => slot.insert(Buf::new(step, self.window)),
                };
                buf.write(frame.ts, *v);
            }
        }
    }

    /// Zeit des neuesten Werts einer Reihe (Unix-Sekunden, auf ihr Raster gerundet).
    pub fn last_ts(&self, series: SeriesId) -> Option<i64> {
        let bufs = self.bufs.read();
        let b = bufs.get(series as usize)?.as_ref()?;
        (b.last != i64::MIN).then(|| b.last * i64::from(b.step))
    }

    /// Neuester Wert einer Reihe.
    pub fn latest(&self, series: SeriesId) -> Option<(i64, f64)> {
        let bufs = self.bufs.read();
        let b = bufs.get(series as usize)?.as_ref()?;
        if b.last == i64::MIN {
            return None;
        }
        // Rückwärts den letzten echten Wert suchen (höchstens ein paar Plätze).
        (0..8).find_map(|back| b.at(b.last - back).map(|v| ((b.last - back) * i64::from(b.step), v)))
    }

    /// Ältester Zeitpunkt, der für diese Reihe noch im Speicher liegt.
    pub fn oldest_ts(&self, series: SeriesId) -> Option<i64> {
        let bufs = self.bufs.read();
        let b = bufs.get(series as usize)?.as_ref()?;
        (b.last != i64::MIN).then(|| (b.last - b.data.len() as i64 + 1) * i64::from(b.step))
    }

    /// Reihe auf ein Raster verdichten: `n` Punkte ab `start`, je `step` Sekunden.
    /// `fold` bekommt die Rohwerte eines Punkts (nie leer). Liefert auch Min und Max je Punkt.
    pub fn read(
        &self,
        series: SeriesId,
        start: i64,
        step: u32,
        n: usize,
        fold: impl Fn(&[f64]) -> f64,
    ) -> (Vec<Option<f64>>, Vec<Option<f64>>, Vec<Option<f64>>) {
        let bufs = self.bufs.read();
        let Some(Some(b)) = bufs.get(series as usize) else {
            return (vec![None; n], vec![None; n], vec![None; n]);
        };
        let (mut out, mut lo, mut hi) = (Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n));
        let mut scratch = Vec::new();
        let own = i64::from(b.step);
        for i in 0..n {
            let from = start + i as i64 * i64::from(step);
            let to = from + i64::from(step);
            scratch.clear();
            let mut slot = from.div_euclid(own);
            if slot * own < from {
                slot += 1;
            }
            while slot * own < to {
                if let Some(v) = b.at(slot) {
                    scratch.push(v);
                }
                slot += 1;
            }
            if scratch.is_empty() {
                out.push(None);
                lo.push(None);
                hi.push(None);
            } else {
                out.push(Some(fold(&scratch)));
                lo.push(scratch.iter().copied().reduce(f64::min));
                hi.push(scratch.iter().copied().reduce(f64::max));
            }
        }
        (out, lo, hi)
    }

    pub fn window(&self) -> u32 {
        self.window
    }

    /// Belegter Arbeitsspeicher in Bytes (ungefähr).
    pub fn bytes(&self) -> usize {
        self.bufs.read().iter().flatten().map(|b| b.data.len() * 4 + 24).sum()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::registry::ChartHandle;

    fn handle(series: &[u32], interval: u32) -> ChartHandle {
        ChartHandle { id: Arc::from("t"), series: series.to_vec().into(), interval }
    }

    #[test]
    fn writes_and_reads() {
        let ring = Ring::new(300);
        let h = handle(&[1, 2], 1);
        for t in 0..10 {
            let mut f = Frame::new(1000 + t);
            f.put(&h, &[t as f64, f64::NAN]);
            ring.write(&f);
        }
        assert_eq!(ring.latest(1), Some((1009, 9.0)));
        assert_eq!(ring.latest(2), None);
        let avg = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
        let (vals, lo, hi) = ring.read(1, 1000, 5, 3, avg);
        assert_eq!(vals, vec![Some(2.0), Some(7.0), None]);
        assert_eq!(lo[0], Some(0.0));
        assert_eq!(hi[1], Some(9.0));
    }

    #[test]
    fn gaps_and_late_values() {
        let ring = Ring::new(300);
        let h = handle(&[0], 10);
        for (t, v) in [(100, 1.0), (130, 4.0), (120, 3.0)] {
            let mut f = Frame::new(t);
            f.put(&h, &[v]);
            ring.write(&f);
        }
        let last = |v: &[f64]| v[v.len() - 1];
        let (vals, _, _) = ring.read(0, 100, 10, 4, last);
        assert_eq!(vals, vec![Some(1.0), None, Some(3.0), Some(4.0)]);
    }
}
