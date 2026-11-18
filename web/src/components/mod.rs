//! Bausteine der Oberfläche.
//!
//! - `ui`: nrzUI-Komponenten (kopiert, nicht verändern), `hooks`: deren Hooks.
//! - `chart`: Diagramme. `widgets`: Kennzahlen, Füllstand, Zustand. `lazy`: Aufbau erst im Bild.
//! - `layout`: Rahmen der App (Seitenleiste, Zeitraum-Leiste).

pub mod chart;
pub mod hooks;
pub mod layout;
pub mod lazy;
pub mod ui;
pub mod widgets;
