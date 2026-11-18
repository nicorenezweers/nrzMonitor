//! Vorlage für den Stromverbrauch (gemessen + geschätzt).

use super::{Tpl, d};
use crate::model::{ChartKind::*, Unit::*};

/// Leistung am Netzstecker: gemessener Teil (RAPL, Fühler) plus Grundlast und Netzteilverlust.
pub const POWER_DRAW: Tpl = Tpl::new(
    "power.draw", "power", "Leistungsaufnahme", Watts, Stacked,
    &[
        d("cpu", "CPU & RAM (gemessen)"),
        d("sensors", "Weitere Fühler"),
        d("base", "Grundlast (geschätzt)"),
        d("loss", "Netzteilverlust (geschätzt)"),
    ],
    100,
).every(2);

pub const ALL: &[Tpl] = &[POWER_DRAW];
