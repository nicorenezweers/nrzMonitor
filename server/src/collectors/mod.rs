//! Sammler: lesen Messwerte und geben sie an den Kern ab.
//!
//! Zwei Arten:
//! - **Im Takt** (lokale Quellen wie `/proc`): [`Collector`] umsetzen und mit [`spawn`] starten. Der
//!   Sammler läuft in einem eigenen Thread, genau auf der Sekundengrenze, und bekommt je Takt einen
//!   [`Tick`], in den er seine Werte legt.
//! - **Frei** (HTTP-APIs, Ereignisströme): mit `tokio::spawn` eine eigene Schleife starten, dort
//!   `Frame::new(ts)` füllen und `core.submit(frame)` rufen.
//!
//! Jeder Bereich startet in seiner `start`-Funktion – bei `core.cfg.demo` mit Beispieldaten.

use std::time::{Duration, Instant};

use crate::{core::Core, frame::Frame, registry::ChartHandle, util};

pub mod cloudflare;
pub mod dns;
pub mod docker;
pub mod monitor;
pub mod netbird;
pub mod power;
pub mod probes;
pub mod system;

/// Alle Sammler starten. Muss in der Tokio-Laufzeit gerufen werden.
pub fn start(core: &Core) {
    monitor::start(core);
    system::start(core);
    docker::start(core);
    dns::start(core);
    cloudflare::start(core);
    netbird::start(core);
    probes::start(core);
    power::start(core);
}

/// Ein Takt: Zeitpunkt, verstrichene Zeit und der Rahmen für die Werte.
pub struct Tick<'a> {
    pub core: &'a Core,
    /// Unix-Sekunden, auf den Takt gerundet.
    pub ts: i64,
    /// Sekunden seit dem letzten Takt dieses Sammlers (für Raten).
    pub dt: f64,
    /// Takt in Sekunden (`NRZMON_INTERVAL`).
    pub interval: u32,
    pub frame: Frame,
}

impl Tick<'_> {
    /// Werte eines Diagramms abgeben (Reihenfolge der Dimensionen, `NaN` = kein Wert).
    pub fn put(&mut self, chart: &ChartHandle, values: &[f64]) {
        self.frame.put(chart, values);
    }

    /// Wahr im ersten Takt jedes `secs`-Fensters – für Messungen, die seltener nötig sind
    /// (passend zu `Tpl::every(secs)` im Katalog).
    pub fn due(&self, secs: u32) -> bool {
        secs <= self.interval || self.ts.rem_euclid(i64::from(secs)) < i64::from(self.interval)
    }
}

pub trait Collector: Send + 'static {
    /// Kurzer Name (Protokoll, Diagramm „Dauer je Sammler“).
    fn name(&self) -> &'static str;

    /// Einmal je Takt. Fehler werden protokolliert, der Sammler läuft weiter.
    fn collect(&mut self, tick: &mut Tick<'_>) -> anyhow::Result<()>;
}

/// Sammler in einem eigenen Thread im Takt laufen lassen.
pub fn spawn(core: &Core, mut collector: impl Collector) {
    let core = core.clone();
    let name = collector.name();
    let builder = std::thread::Builder::new().name(format!("nrz-{name}"));
    let result = builder.spawn(move || {
        let interval = core.cfg.interval;
        let step_ms = i64::from(interval) * 1000;
        let mut last: Option<Instant> = None;
        let mut errors = 0u64;
        loop {
            let now = util::now_ms();
            let next = (now.div_euclid(step_ms) + 1) * step_ms;
            std::thread::sleep(Duration::from_millis((next - now) as u64));
            let started = Instant::now();
            let dt = last.map_or(f64::from(interval), |l| started.duration_since(l).as_secs_f64());
            last = Some(started);
            let mut tick = Tick { core: &core, ts: next / 1000, dt, interval, frame: Frame::new(next / 1000) };
            if let Err(e) = collector.collect(&mut tick) {
                errors += 1;
                // Nicht jede Sekunde dieselbe Meldung.
                if errors == 1 || errors % 300 == 0 {
                    tracing::warn!("Sammler {name}: {e:#} (Fehler Nr. {errors})");
                }
            }
            let frame = tick.frame;
            core.submit(frame);
            core.record_duration(name, started.elapsed());
        }
    });
    if let Err(e) = result {
        tracing::error!("Sammler {name} nicht gestartet: {e}");
    }
}
