//! Vorlagen der lokalen Sammler: System, CPU, Arbeitsspeicher, Datenträger, Netzwerk, Sensoren, Prozesse.

use super::{Tpl, d, dn};
use crate::model::{ChartKind::*, Unit::*};

// ------------------------------------------------------------------ System (Übersicht)

pub const SYSTEM_CPU: Tpl = Tpl::new(
    "system.cpu", "system", "CPU-Auslastung", Percent, Stacked,
    &[
        d("user", "Benutzer"), d("nice", "Nice"), d("system", "System"), d("iowait", "E/A-Wartezeit"),
        d("irq", "IRQ"), d("softirq", "SoftIRQ"), d("steal", "Steal"), d("guest", "Gast"),
    ],
    100,
);
pub const SYSTEM_LOAD: Tpl = Tpl::new(
    "system.load", "system", "Systemlast", Count, Line,
    &[d("load1", "1 min"), d("load5", "5 min"), d("load15", "15 min")], 110,
);
pub const SYSTEM_RAM: Tpl = Tpl::new(
    "system.ram", "system", "Arbeitsspeicher", Bytes, Stacked,
    &[d("used", "Belegt"), d("buffers", "Puffer"), d("cached", "Cache"), d("free", "Frei")], 120,
);
pub const SYSTEM_SWAP: Tpl = Tpl::new(
    "system.swap", "system", "Auslagerung (Swap)", Bytes, Stacked,
    &[d("used", "Belegt"), d("free", "Frei")], 130,
);
pub const SYSTEM_IO: Tpl = Tpl::new(
    "system.io", "system", "Datenträger-Durchsatz", BytesPerSec, Area,
    &[d("read", "Lesen"), dn("write", "Schreiben")], 140,
);
pub const SYSTEM_NET: Tpl = Tpl::new(
    "system.net", "system", "Netzwerk-Durchsatz", BitsPerSec, Area,
    &[d("received", "Empfangen"), dn("sent", "Gesendet")], 150,
);
pub const SYSTEM_PRESSURE: Tpl = Tpl::new(
    "system.pressure", "system", "Engpässe (PSI, 10 s)", Percent, Line,
    &[
        d("cpu_some", "CPU"), d("mem_some", "Speicher"), d("mem_full", "Speicher (alle)"),
        d("io_some", "E/A"), d("io_full", "E/A (alle)"),
    ],
    160,
);
pub const SYSTEM_PROCESSES: Tpl = Tpl::new(
    "system.processes", "system", "Laufende und blockierte Prozesse", Count, Line,
    &[d("running", "Laufend"), d("blocked", "Blockiert")], 170,
);
pub const SYSTEM_FORKS: Tpl = Tpl::new(
    "system.forks", "system", "Neue Prozesse", PerSec, Line, &[d("started", "Gestartet")], 180,
);
pub const SYSTEM_CTXT: Tpl = Tpl::new(
    "system.ctxt", "system", "Kontextwechsel", PerSec, Line, &[d("switches", "Wechsel")], 190,
);
pub const SYSTEM_INTR: Tpl = Tpl::new(
    "system.intr", "system", "Interrupts", PerSec, Line, &[d("interrupts", "Interrupts")], 200,
);
pub const SYSTEM_UPTIME: Tpl = Tpl::new(
    "system.uptime", "system", "Laufzeit", Seconds, Line, &[d("uptime", "Laufzeit")], 210,
).every(10);
pub const SYSTEM_FDS: Tpl = Tpl::new(
    "system.fds", "system", "Offene Dateien", Count, Line, &[d("used", "Offen")], 220,
).every(10);
pub const SYSTEM_ENTROPY: Tpl = Tpl::new(
    "system.entropy", "system", "Entropie", Count, Line, &[d("entropy", "Bits")], 230,
).every(10);

// ------------------------------------------------------------------ CPU

/// Dimensionen: je Kern eine (`cpu0`, `cpu1` …).
pub const CPU_CORES: Tpl = Tpl::new("cpu.cores", "cpu", "Auslastung je Kern", Percent, Line, &[], 100);
/// Dimensionen: je Kern eine.
pub const CPU_FREQ: Tpl = Tpl::new("cpu.freq", "cpu", "Taktfrequenz", Mhz, Line, &[], 110).every(5);
/// Dimensionen: `package` und je Kern eine.
pub const CPU_TEMP: Tpl = Tpl::new("cpu.temp", "cpu", "CPU-Temperatur", Celsius, Line, &[], 120).every(2);
/// Dimensionen: je RAPL-Zone eine (`package`, `cores`, `dram` …).
pub const CPU_POWER: Tpl = Tpl::new("cpu.power", "cpu", "Leistungsaufnahme (RAPL)", Watts, Stacked, &[], 130).every(2);
/// Dimensionen: je SoftIRQ-Art eine.
pub const CPU_SOFTIRQS: Tpl = Tpl::new("cpu.softirqs", "cpu", "SoftIRQs nach Art", PerSec, Stacked, &[], 140);
pub const CPU_THROTTLE: Tpl = Tpl::new(
    "cpu.throttle", "cpu", "Drosselung (thermisch)", PerSec, Line,
    &[d("core", "Kern"), d("package", "Gehäuse")], 150,
).every(5);

// ------------------------------------------------------------------ Arbeitsspeicher

pub const MEM_AVAILABLE: Tpl = Tpl::new(
    "mem.available", "memory", "Verfügbarer Speicher", Bytes, Area, &[d("available", "Verfügbar")], 100,
);
pub const MEM_COMMITTED: Tpl = Tpl::new(
    "mem.committed", "memory", "Zugesagter Speicher", Bytes, Line,
    &[d("committed", "Zugesagt"), d("limit", "Grenze")], 110,
).every(5);
pub const MEM_KERNEL: Tpl = Tpl::new(
    "mem.kernel", "memory", "Kernel-Speicher", Bytes, Stacked,
    &[
        d("slab_reclaimable", "Slab (freigebbar)"), d("slab_unreclaimable", "Slab (fest)"),
        d("kernel_stack", "Kernel-Stack"), d("page_tables", "Seitentabellen"), d("vmalloc", "vmalloc"),
    ],
    120,
).every(5);
pub const MEM_WRITEBACK: Tpl = Tpl::new(
    "mem.writeback", "memory", "Noch zu schreiben", Bytes, Line,
    &[d("dirty", "Geändert"), d("writeback", "Wird geschrieben")], 130,
);
pub const MEM_PAGING: Tpl = Tpl::new(
    "mem.paging", "memory", "Seiten von/zu Datenträger", BytesPerSec, Area,
    &[d("in", "Gelesen"), dn("out", "Geschrieben")], 140,
);
pub const MEM_SWAPIO: Tpl = Tpl::new(
    "mem.swapio", "memory", "Swap-Aktivität", BytesPerSec, Area,
    &[d("in", "Eingelagert"), dn("out", "Ausgelagert")], 150,
);
pub const MEM_FAULTS: Tpl = Tpl::new(
    "mem.faults", "memory", "Seitenfehler", PerSec, Line, &[d("minor", "Leicht"), d("major", "Schwer")], 160,
);
pub const MEM_OOM: Tpl = Tpl::new(
    "mem.oom", "memory", "Vom Kernel beendet (OOM)", PerSec, Line, &[d("kills", "Beendet")], 170,
);
pub const MEM_HUGEPAGES: Tpl = Tpl::new(
    "mem.hugepages", "memory", "Grosse Seiten", Bytes, Stacked, &[d("used", "Belegt"), d("free", "Frei")], 180,
).every(10);

// ------------------------------------------------------------------ Datenträger

pub const DISK_IO: Tpl = Tpl::new(
    "disk.io", "disk", "Durchsatz", BytesPerSec, Area, &[d("read", "Lesen"), dn("write", "Schreiben")], 100,
).per("device");
pub const DISK_OPS: Tpl = Tpl::new(
    "disk.ops", "disk", "Zugriffe", PerSec, Line, &[d("read", "Lesen"), dn("write", "Schreiben")], 110,
).per("device");
pub const DISK_AWAIT: Tpl = Tpl::new(
    "disk.await", "disk", "Wartezeit je Zugriff", Millis, Line, &[d("read", "Lesen"), d("write", "Schreiben")], 120,
).per("device");
pub const DISK_UTIL: Tpl = Tpl::new(
    "disk.util", "disk", "Auslastung", Percent, Area, &[d("util", "Beschäftigt")], 130,
).per("device");
pub const DISK_QUEUE: Tpl = Tpl::new(
    "disk.queue", "disk", "Warteschlange", Count, Line, &[d("queue", "Wartend")], 140,
).per("device");
pub const DISK_TEMP: Tpl = Tpl::new(
    "disk.temp", "disk", "Temperatur", Celsius, Line, &[d("temp", "Temperatur")], 150,
).per("device").every(10);
pub const DISK_SPACE: Tpl = Tpl::new(
    "disk.space", "disk", "Belegung", Bytes, Stacked,
    &[d("used", "Belegt"), d("avail", "Frei"), d("reserved", "Reserviert")], 200,
).per("mount").every(10);
pub const DISK_INODES: Tpl = Tpl::new(
    "disk.inodes", "disk", "Inodes", Count, Stacked, &[d("used", "Belegt"), d("free", "Frei")], 210,
).per("mount").every(10);

// ------------------------------------------------------------------ Netzwerk

pub const NET_TRAFFIC: Tpl = Tpl::new(
    "net.traffic", "network", "Durchsatz", BitsPerSec, Area,
    &[d("received", "Empfangen"), dn("sent", "Gesendet")], 100,
).per("iface");
pub const NET_PACKETS: Tpl = Tpl::new(
    "net.packets", "network", "Pakete", PerSec, Line,
    &[d("received", "Empfangen"), dn("sent", "Gesendet")], 110,
).per("iface");
pub const NET_ERRORS: Tpl = Tpl::new(
    "net.errors", "network", "Fehler", PerSec, Line,
    &[d("rx", "Empfang"), dn("tx", "Senden")], 120,
).per("iface");
pub const NET_DROPS: Tpl = Tpl::new(
    "net.drops", "network", "Verworfene Pakete", PerSec, Line,
    &[d("rx", "Empfang"), dn("tx", "Senden")], 130,
).per("iface");
pub const NET_TCP_CONN: Tpl = Tpl::new(
    "net.tcp.conn", "network", "TCP-Verbindungen", Count, Line, &[d("established", "Offen")], 300,
);
pub const NET_TCP_OPENS: Tpl = Tpl::new(
    "net.tcp.opens", "network", "TCP: neue Verbindungen", PerSec, Line,
    &[d("active", "Ausgehend"), dn("passive", "Eingehend")], 310,
);
pub const NET_TCP_ERRORS: Tpl = Tpl::new(
    "net.tcp.errors", "network", "TCP: Fehler", PerSec, Line,
    &[
        d("retrans", "Wiederholt"), d("in_errors", "Fehlerhaft"), d("resets_out", "Reset gesendet"),
        d("attempt_fails", "Fehlgeschlagen"), d("estab_resets", "Abgebrochen"),
    ],
    320,
);
pub const NET_UDP_PACKETS: Tpl = Tpl::new(
    "net.udp.packets", "network", "UDP-Pakete", PerSec, Line,
    &[d("received", "Empfangen"), dn("sent", "Gesendet")], 330,
);
pub const NET_UDP_ERRORS: Tpl = Tpl::new(
    "net.udp.errors", "network", "UDP: Fehler", PerSec, Line,
    &[d("in_errors", "Fehlerhaft"), d("no_port", "Kein Port"), d("rcvbuf", "Puffer voll (Empfang)"), d("sndbuf", "Puffer voll (Senden)")],
    340,
);
pub const NET_SOCKETS: Tpl = Tpl::new(
    "net.sockets", "network", "Sockets", Count, Line,
    &[d("tcp", "TCP"), d("udp", "UDP"), d("orphan", "Verwaist"), d("tw", "Time-Wait")], 350,
).every(5);
pub const NET_CONNTRACK: Tpl = Tpl::new(
    "net.conntrack", "network", "Verbindungsverfolgung (conntrack)", Count, Line,
    &[d("count", "Einträge"), d("max", "Grenze")], 360,
).every(5);

// ------------------------------------------------------------------ Sensoren

/// Je Chip ein Diagramm, Dimensionen = Fühler des Chips.
pub const SENSORS_TEMP: Tpl = Tpl::new("sensors.temp", "sensors", "Temperatur", Celsius, Line, &[], 100).per("chip").every(2);
pub const SENSORS_FAN: Tpl = Tpl::new("sensors.fan", "sensors", "Lüfter", Rpm, Line, &[], 110).per("chip").every(2);
pub const SENSORS_VOLTAGE: Tpl = Tpl::new("sensors.voltage", "sensors", "Spannung", Volts, Line, &[], 120).per("chip").every(5);
pub const SENSORS_POWER: Tpl = Tpl::new("sensors.power", "sensors", "Leistung", Watts, Line, &[], 130).per("chip").every(2);
pub const SENSORS_CURRENT: Tpl = Tpl::new("sensors.current", "sensors", "Strom", Amps, Line, &[], 140).per("chip").every(5);
/// Dimensionen: je thermischer Zone eine.
pub const SENSORS_THERMAL: Tpl = Tpl::new("sensors.thermal", "sensors", "Thermische Zonen", Celsius, Line, &[], 150).every(2);
pub const SENSORS_BATTERY: Tpl = Tpl::new(
    "sensors.battery", "sensors", "Akku", Percent, Area, &[d("capacity", "Ladung")], 160,
).per("battery").every(30);

// ------------------------------------------------------------------ Prozesse

pub const PROC_COUNT: Tpl = Tpl::new(
    "proc.count", "processes", "Prozesse und Threads", Count, Line,
    &[d("processes", "Prozesse"), d("threads", "Threads")], 100,
).every(5);
pub const PROC_STATES: Tpl = Tpl::new(
    "proc.states", "processes", "Prozesse nach Zustand", Count, Stacked,
    &[d("running", "Laufend"), d("sleeping", "Schlafend"), d("idle", "Leerlauf"), d("stopped", "Angehalten"), d("zombie", "Zombie")],
    110,
).every(5);
/// Dimensionen: je Programmgruppe eine (grösste zuerst, Rest als `other`).
pub const PROC_CPU: Tpl = Tpl::new("proc.cpu", "processes", "CPU nach Programm", Percent, Stacked, &[], 120).every(5);
pub const PROC_MEM: Tpl = Tpl::new("proc.mem", "processes", "Speicher nach Programm", Bytes, Stacked, &[], 130).every(5);
pub const PROC_IO_READ: Tpl = Tpl::new("proc.io_read", "processes", "Lesen nach Programm", BytesPerSec, Stacked, &[], 140).every(5);
pub const PROC_IO_WRITE: Tpl = Tpl::new("proc.io_write", "processes", "Schreiben nach Programm", BytesPerSec, Stacked, &[], 150).every(5);
pub const PROC_THREADS: Tpl = Tpl::new("proc.threads", "processes", "Threads nach Programm", Count, Stacked, &[], 160).every(5);
pub const PROC_FDS: Tpl = Tpl::new("proc.fds", "processes", "Offene Dateien nach Programm", Count, Stacked, &[], 170).every(5);

pub const ALL: &[Tpl] = &[
    SYSTEM_CPU, SYSTEM_LOAD, SYSTEM_RAM, SYSTEM_SWAP, SYSTEM_IO, SYSTEM_NET, SYSTEM_PRESSURE, SYSTEM_PROCESSES,
    SYSTEM_FORKS, SYSTEM_CTXT, SYSTEM_INTR, SYSTEM_UPTIME, SYSTEM_FDS, SYSTEM_ENTROPY,
    CPU_CORES, CPU_FREQ, CPU_TEMP, CPU_POWER, CPU_SOFTIRQS, CPU_THROTTLE,
    MEM_AVAILABLE, MEM_COMMITTED, MEM_KERNEL, MEM_WRITEBACK, MEM_PAGING, MEM_SWAPIO, MEM_FAULTS, MEM_OOM, MEM_HUGEPAGES,
    DISK_IO, DISK_OPS, DISK_AWAIT, DISK_UTIL, DISK_QUEUE, DISK_TEMP, DISK_SPACE, DISK_INODES,
    NET_TRAFFIC, NET_PACKETS, NET_ERRORS, NET_DROPS, NET_TCP_CONN, NET_TCP_OPENS, NET_TCP_ERRORS,
    NET_UDP_PACKETS, NET_UDP_ERRORS, NET_SOCKETS, NET_CONNTRACK,
    SENSORS_TEMP, SENSORS_FAN, SENSORS_VOLTAGE, SENSORS_POWER, SENSORS_CURRENT, SENSORS_THERMAL, SENSORS_BATTERY,
    PROC_COUNT, PROC_STATES, PROC_CPU, PROC_MEM, PROC_IO_READ, PROC_IO_WRITE, PROC_THREADS, PROC_FDS,
];
