//! Standardregeln für alle Bereiche des Katalogs und das Zusammenführen mit eigenen Änderungen.

use shared::{
    alerts::{AlertConfig, Cmp, Rule, RuleKind, StateCheck},
    events::Severity,
};

#[allow(clippy::too_many_arguments)]
fn metric(
    id: &str,
    group: &str,
    name: &str,
    context: &str,
    expr: &str,
    cmp: Cmp,
    warn: Option<f64>,
    crit: Option<f64>,
    window: u32,
    duration: u32,
    hysteresis: f64,
    description: &str,
) -> Rule {
    Rule {
        id: id.to_string(),
        name: name.to_string(),
        group: group.to_string(),
        description: description.to_string(),
        builtin: true,
        kind: RuleKind::Metric,
        context: context.to_string(),
        expr: expr.to_string(),
        cmp,
        warn,
        crit,
        window,
        duration,
        hysteresis,
        ..Rule::default()
    }
}

#[allow(clippy::too_many_arguments)]
fn state(
    id: &str,
    group: &str,
    check: StateCheck,
    severity: Severity,
    warn: Option<f64>,
    crit: Option<f64>,
    window: u32,
    duration: u32,
    context: &str,
    description: &str,
) -> Rule {
    Rule {
        id: id.to_string(),
        name: check.label().to_string(),
        group: group.to_string(),
        description: description.to_string(),
        builtin: true,
        kind: RuleKind::State,
        check,
        severity,
        warn,
        crit,
        window,
        duration,
        context: context.to_string(),
        expr: String::new(),
        ..Rule::default()
    }
}

/// Alle Standardregeln (feste IDs – darüber werden Änderungen zugeordnet).
pub fn builtin() -> Vec<Rule> {
    use Cmp::{Above, Below};
    vec![
        // --- System
        metric("cpu_usage", "System", "CPU-Auslastung", "system.cpu", "*", Above, Some(85.0), Some(95.0), 120, 300, 5.0,
            "Mittlere Auslastung aller Kerne über zwei Minuten."),
        metric("load_per_core", "System", "Systemlast je Kern", "system.load", "load15/@cores", Above, Some(150.0), Some(300.0), 0, 300, 10.0,
            "15-Minuten-Last geteilt durch die Anzahl Kerne (100 % = alle Kerne voll beschäftigt)."),
        metric("ram_usage", "System", "Arbeitsspeicher belegt", "system.ram", "used/*", Above, Some(90.0), Some(96.0), 60, 120, 2.0,
            "Belegter Speicher ohne Cache und Puffer."),
        metric("swap_usage", "System", "Swap belegt", "system.swap", "used/*", Above, Some(60.0), Some(85.0), 60, 300, 5.0,
            "Anteil der belegten Auslagerung."),
        metric("host_oom", "System", "Speichermangel (Host)", "mem.oom", "kills", Above, Some(0.0), None, 600, 0, 0.0,
            "Der Kernel hat in den letzten zehn Minuten Prozesse beendet."),
        metric("psi_memory", "System", "Speicher-Engpass (PSI)", "system.pressure", "mem_full", Above, Some(10.0), Some(30.0), 60, 120, 2.0,
            "Anteil der Zeit, in der alle Prozesse auf Speicher warten."),
        // --- Datenträger
        metric("disk_space", "Datenträger", "Dateisystem fast voll", "disk.space", "used/used+avail", Above, Some(85.0), Some(95.0), 0, 60, 1.0,
            "Belegung je Dateisystem."),
        metric("disk_inodes", "Datenträger", "Inodes fast aufgebraucht", "disk.inodes", "used/*", Above, Some(85.0), Some(95.0), 0, 60, 1.0,
            "Anteil belegter Inodes je Dateisystem."),
        metric("disk_util", "Datenträger", "Datenträger ausgelastet", "disk.util", "util", Above, Some(90.0), Some(98.0), 300, 300, 5.0,
            "Mittlere Auslastung über fünf Minuten."),
        metric("disk_temp", "Datenträger", "Datenträger-Temperatur", "disk.temp", "temp", Above, Some(55.0), Some(65.0), 60, 120, 2.0,
            "Temperatur je Laufwerk."),
        // --- Temperaturen
        metric("cpu_temp", "Temperaturen", "CPU-Temperatur", "cpu.temp", "max:*", Above, Some(80.0), Some(90.0), 30, 60, 3.0,
            "Höchste Temperatur von Gehäuse und Kernen."),
        Rule { enabled: false, ..metric("sensor_temp", "Temperaturen", "Sensor-Temperatur", "sensors.temp", "max:*", Above, Some(85.0), Some(95.0), 30, 120, 3.0,
            "Höchster Wert je Sensor-Chip (manche Chips melden unsinnige Werte – darum aus).") },
        // --- Netzwerk
        metric("conntrack", "Netzwerk", "conntrack-Tabelle fast voll", "net.conntrack", "count/max", Above, Some(80.0), Some(92.0), 0, 60, 2.0,
            "Ist sie voll, verwirft der Kernel neue Verbindungen."),
        metric("net_errors", "Netzwerk", "Netzwerkfehler", "net.errors", "rx+tx", Above, Some(5.0), Some(50.0), 120, 120, 1.0,
            "Fehlerhafte Pakete je Sekunde und Schnittstelle."),
        Rule { enabled: false, ..metric("net_drops", "Netzwerk", "Verworfene Pakete", "net.drops", "rx+tx", Above, Some(50.0), Some(500.0), 120, 300, 5.0,
            "Verworfene Pakete je Sekunde (manche Schnittstellen verwerfen dauernd ein paar – darum aus).") },
        // --- Container
        state("container_exited", "Container", StateCheck::ContainerExited, Severity::Crit, None, None, 0, 0, "",
            "Container mit Fehlercode beendet und nicht neu gestartet."),
        state("container_unhealthy", "Container", StateCheck::ContainerUnhealthy, Severity::Warn, None, None, 0, 60, "",
            "Der Healthcheck des Containers schlägt fehl."),
        state("container_restart_loop", "Container", StateCheck::RestartLoop, Severity::Warn, Some(2.0), Some(5.0), 900, 0, "",
            "Mehr Neustarts als die Schwelle in 15 Minuten."),
        state("container_oom", "Container", StateCheck::ContainerOom, Severity::Crit, None, None, 900, 0, "",
            "Ein Prozess im Container wurde wegen Speichermangel beendet (bleibt 15 Minuten stehen)."),
        state("container_mem_limit", "Container", StateCheck::MemLimit, Severity::Warn, Some(90.0), Some(97.0), 0, 60, "",
            "Speicher in Prozent der eigenen Grenze des Containers."),
        metric("container_throttle", "Container", "CPU gedrosselt", "docker.throttle", "throttled", Above, Some(25.0), Some(60.0), 300, 300, 5.0,
            "Anteil der Zeitabschnitte, in denen die CPU-Grenze des Containers griff."),
        Rule { enabled: false, ..metric("container_cpu", "Container", "Container-CPU hoch", "docker.cpu", "*", Above, Some(200.0), Some(350.0), 120, 600, 10.0,
            "CPU je Container (100 % = ein Kern).") },
        // --- DNS
        metric("dns_servfail", "DNS", "DNS-Fehlerantworten", "dns.rcodes", "servfail/*", Above, Some(5.0), Some(20.0), 300, 120, 1.0,
            "Anteil SERVFAIL an allen Antworten."),
        metric("dns_upstream", "DNS", "Weiterleitung fehlerhaft", "dns.upstream.answers", "failed/*", Above, Some(10.0), Some(30.0), 300, 0, 2.0,
            "Anteil fehlgeschlagener Antworten je Upstream."),
        metric("dns_latency", "DNS", "DNS langsam", "dns.latency", "avg", Above, Some(150.0), Some(500.0), 300, 300, 10.0,
            "Mittlere Antwortzeit."),
        // --- Erreichbarkeit
        metric("probe_down", "Erreichbarkeit", "Ziel nicht erreichbar", "probe.up", "up", Below, None, Some(0.5), 0, 30, 0.0,
            "Prüfung schlägt fehl."),
        metric("probe_loss", "Erreichbarkeit", "Paketverlust", "probe.loss", "loss", Above, Some(10.0), Some(50.0), 300, 0, 2.0,
            "Mittlerer Verlust über fünf Minuten."),
        metric("probe_latency", "Erreichbarkeit", "Antwortzeit hoch", "probe.latency", "latency", Above, Some(500.0), Some(2000.0), 300, 0, 20.0,
            "Mittlere Antwortzeit über fünf Minuten."),
        // --- Peers und Tunnel
        metric("peer_offline", "Peers", "NetBird-Peer getrennt", "netbird.peer.online", "online", Below, Some(0.5), None, 0, 300, 0.0,
            "Peer ist seit fünf Minuten nicht verbunden."),
        metric("tunnel_down", "Peers", "Cloudflare-Tunnel getrennt", "cf.tunnel.up", "up", Below, None, Some(0.5), 0, 120, 0.0,
            "Tunnel hat keine Verbindung."),
        // --- Daten
        state("no_data", "Daten", StateCheck::NoData, Severity::Warn, Some(30.0), Some(180.0), 0, 0, "system.cpu, docker.containers",
            "Sekunden ohne neue Werte – ein Sammler steht still."),
    ]
}

/// Standardregeln mit den gespeicherten Änderungen, dahinter die eigenen Regeln.
pub fn effective(cfg: &AlertConfig) -> Vec<Rule> {
    let mut out = builtin();
    for r in &mut out {
        if let Some(o) = cfg.rules.iter().find(|x| x.builtin && x.id == r.id) {
            apply(r, o);
        }
    }
    out.extend(cfg.rules.iter().filter(|r| !r.builtin).cloned());
    out
}

/// Änderbare Felder einer Standardregel übernehmen (Kontext, Ausdruck und Art bleiben fest).
pub fn apply(base: &mut Rule, o: &Rule) {
    base.enabled = o.enabled;
    base.warn = o.warn;
    base.crit = o.crit;
    base.window = o.window;
    base.duration = o.duration;
    base.hysteresis = o.hysteresis;
    base.filter.clone_from(&o.filter);
    base.notify = o.notify;
    base.severity = o.severity;
    base.cmp = o.cmp;
}

#[cfg(test)]
mod tests {
    use shared::alerts::Expr;

    use super::*;

    #[test]
    fn builtin_rules_are_valid() {
        let rules = builtin();
        let mut ids = std::collections::HashSet::new();
        for r in &rules {
            assert!(ids.insert(r.id.clone()), "doppelte ID {}", r.id);
            if r.kind == RuleKind::Metric {
                Expr::parse(&r.expr).unwrap_or_else(|e| panic!("{}: {e}", r.id));
                let ctx = shared::catalog::find(&r.context).unwrap_or_else(|| panic!("{}: Kontext {} fehlt im Katalog", r.id, r.context));
                for part in r.expr.trim_start_matches("max:").split(['/', '+']) {
                    if part != "*" && part != "@cores" && !ctx.dims.is_empty() {
                        assert!(ctx.dims.iter().any(|d| d.id == part), "{}: Dimension {part} fehlt in {}", r.id, r.context);
                    }
                }
            }
            assert!(r.warn.is_some() || r.crit.is_some() || r.kind == RuleKind::State);
        }
    }

    #[test]
    fn overrides_apply() {
        let mut o = builtin().into_iter().find(|r| r.id == "disk_space").expect("Regel");
        o.warn = Some(70.0);
        o.context = "anders".into();
        let cfg = AlertConfig { rules: vec![o, Rule { id: "custom-1".into(), ..Rule::default() }], ..AlertConfig::default() };
        let eff = effective(&cfg);
        let d = eff.iter().find(|r| r.id == "disk_space").expect("Regel");
        assert_eq!(d.warn, Some(70.0));
        assert_eq!(d.context, "disk.space", "Kontext bleibt fest");
        assert!(eff.iter().any(|r| r.id == "custom-1"));
    }
}
