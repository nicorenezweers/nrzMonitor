//! nrzMonitor über sich selbst: Dauer der Sammler, Anzahl Reihen, eigene Last.

use shared::catalog::monitor::*;

use super::{Collector, Tick};
use crate::{
    core::Core,
    registry::{ChartDef, ChartHandle},
    util::{self, Counter},
};

pub fn start(core: &Core) {
    super::spawn(
        core,
        Monitor {
            series: core.chart(ChartDef::tpl(&MONITOR_SERIES)),
            ingest: core.chart(ChartDef::tpl(&MONITOR_INGEST)),
            cpu: core.chart(ChartDef::tpl(&MONITOR_CPU)),
            mem: core.chart(ChartDef::tpl(&MONITOR_MEM)),
            clients: core.chart(ChartDef::tpl(&MONITOR_CLIENTS)),
            samples: Counter::new(),
            user: Counter::new(),
            system: Counter::new(),
        },
    );
}

struct Monitor {
    series: ChartHandle,
    ingest: ChartHandle,
    cpu: ChartHandle,
    mem: ChartHandle,
    clients: ChartHandle,
    samples: Counter,
    user: Counter,
    system: Counter,
}

impl Collector for Monitor {
    fn name(&self) -> &'static str {
        "monitor"
    }

    fn collect(&mut self, t: &mut Tick<'_>) -> anyhow::Result<()> {
        let core = t.core;

        // Dauer je Sammler: Dimensionen entstehen mit den Sammlern.
        let durations = core.durations.lock().clone();
        if !durations.is_empty() {
            let chart = core.chart(ChartDef::tpl(&MONITOR_COLLECTORS).dims(durations.keys().map(|k| (k.clone(), k.clone()))));
            let values: Vec<f64> = durations.values().copied().collect();
            t.put(&chart, &values);
        }

        let rate = self.samples.rate(core.store.samples_total(), t.dt);
        t.put(&self.ingest, &[util::round(util::nan(rate), 0)]);

        let (user_us, system_us, max_rss) = rusage();
        let user = self.user.rate(user_us, t.dt).map(|us| us / 10_000.0);
        let system = self.system.rate(system_us, t.dt).map(|us| us / 10_000.0);
        t.put(&self.cpu, &[util::round(util::nan(user), 2), util::round(util::nan(system), 2)]);

        if t.due(5) {
            t.put(&self.mem, &[rss_bytes().unwrap_or(max_rss) as f64]);
        }
        if t.due(10) {
            let (charts, series) = core.registry.counts();
            t.put(&self.series, &[charts as f64, series as f64]);
            t.put(&self.clients, &[core.live_clients() as f64]);
        }
        Ok(())
    }
}

/// (Benutzerzeit µs, Systemzeit µs, grösster Speicherbedarf in Bytes) dieses Prozesses.
fn rusage() -> (u64, u64, u64) {
    // SAFETY: `ru` ist ein gültiger, genullter Puffer; getrusage füllt ihn nur.
    let ru = unsafe {
        let mut ru: libc::rusage = std::mem::zeroed();
        if libc::getrusage(libc::RUSAGE_SELF, &mut ru) != 0 {
            return (0, 0, 0);
        }
        ru
    };
    let us = |tv: libc::timeval| tv.tv_sec as u64 * 1_000_000 + tv.tv_usec as u64;
    // Linux zählt in KiB, macOS in Bytes.
    let max_rss = if cfg!(target_os = "macos") { ru.ru_maxrss as u64 } else { ru.ru_maxrss as u64 * 1024 };
    (us(ru.ru_utime), us(ru.ru_stime), max_rss)
}

/// Aktuell belegter Speicher (nur Linux).
fn rss_bytes() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * 4096)
}
