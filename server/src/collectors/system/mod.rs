//! Lokale Sammler: System, CPU, Arbeitsspeicher, Datenträger, Netzwerk, Sensoren, Prozesse.
//! Quellen sind `core.cfg.proc`, `core.cfg.sys` und `core.cfg.root` (im Container die des Hosts).
//! Jeder Sammler läuft in seinem eigenen Takt-Thread – so blockiert ein langsamer Durchgang
//! (Prozesse, statvfs) die anderen nicht, und `monitor.collectors` zeigt die Dauer je Teil.

use crate::{collectors, core::Core};

mod cpu;
mod demo;
mod disk;
mod memory;
mod net;
mod procs;
mod read;
mod sensors;

pub fn start(core: &Core) {
    if core.cfg.demo {
        demo::start(core);
        return;
    }
    if !core.cfg.proc.join("stat").exists() {
        tracing::warn!("System: {} fehlt – keine Systemdiagramme", core.cfg.proc.join("stat").display());
        return;
    }
    collectors::spawn(core, cpu::Cpu::new(core));
    collectors::spawn(core, memory::Memory::new(core));
    collectors::spawn(core, disk::Disks::new(core));
    collectors::spawn(core, net::Net::new(core));
    collectors::spawn(core, sensors::Sensors::new(core));
    collectors::spawn(core, procs::Procs::new(core));
}
