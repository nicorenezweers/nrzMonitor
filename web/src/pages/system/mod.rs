//! Seiten des Hosts: CPU, Arbeitsspeicher, Datenträger, Netzwerk, Sensoren, Prozesse.

mod common;
mod cpu;
mod disks;
mod memory;
mod network;
mod processes;
mod sensors;

pub use cpu::Cpu;
pub use disks::Disks;
pub use memory::Memory;
pub use network::Network;
pub use processes::Processes;
pub use sensors::Sensors;
