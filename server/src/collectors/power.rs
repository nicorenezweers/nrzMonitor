//! Strom: rechnet alle 2 s die Leistung am Netzstecker aus den gemessenen Werten (RAPL `cpu.power`,
//! optional hwmon `sensors.power.*`), der eingestellten Grundlast und dem Netzteil-Wirkungsgrad.
//! Energie und Kosten rechnet `api/power.rs` aus diesem Diagramm (`power.draw`).

use shared::{catalog::power::POWER_DRAW, power::PowerSettings};

use super::{Collector, Tick};
use crate::{
    core::Core,
    registry::{ChartDef, ChartHandle},
    util,
};

/// Werte, die älter sind, gelten als „keine Messung“.
const FRESH_SECS: i64 = 10;

pub fn start(core: &Core) {
    super::spawn(core, Power { chart: core.chart(ChartDef::tpl(&POWER_DRAW)) });
}

struct Power {
    chart: ChartHandle,
}

/// Summe der neuesten Werte aller Reihen eines Diagramms (nur frische Werte).
pub fn latest_sum(core: &Core, chart_id: &str, now: i64) -> Option<f64> {
    let meta = core.registry.get(chart_id)?;
    if meta.retired {
        return None;
    }
    let mut sum = None;
    for d in &meta.dims {
        if let Some((ts, v)) = core.ring.latest(d.series)
            && now - ts <= FRESH_SECS
            && v.is_finite()
        {
            *sum.get_or_insert(0.0) += v;
        }
    }
    sum
}

/// Gemessene Leistung: (RAPL, Fühler) in Watt.
pub fn measured(core: &Core, settings: &PowerSettings, now: i64) -> (Option<f64>, Option<f64>) {
    let cpu = latest_sum(core, "cpu.power", now);
    let sensors = if settings.include_sensors {
        core.registry.by_context("sensors.power").iter().filter_map(|c| latest_sum(core, &c.id, now)).reduce(|a, b| a + b)
    } else {
        None
    };
    (cpu, sensors)
}

impl Collector for Power {
    fn name(&self) -> &'static str {
        "power"
    }

    fn collect(&mut self, t: &mut Tick<'_>) -> anyhow::Result<()> {
        if !t.due(2) {
            return Ok(());
        }
        let settings: PowerSettings = t.core.settings.get("power");
        let (cpu, sensors) = measured(t.core, &settings, t.ts);
        let inner = cpu.unwrap_or(0.0) + sensors.unwrap_or(0.0) + settings.base_watts.max(0.0);
        let eff = (settings.efficiency.clamp(30.0, 100.0)) / 100.0;
        let loss = inner / eff - inner;
        t.put(
            &self.chart,
            &[
                util::round(cpu.unwrap_or(0.0), 2),
                util::round(sensors.unwrap_or(0.0), 2),
                util::round(settings.base_watts.max(0.0), 2),
                util::round(loss, 2),
            ],
        );
        Ok(())
    }
}
