//! Sensoren: alles aus `/sys/class/hwmon` (Temperatur, Lüfter, Spannung, Leistung, Strom) mit Chip- und
//! Fühlernamen und Grenzwerten, thermische Zonen, Akkus. `cpu.temp` kommt aus coretemp/k10temp/zenpower
//! (sonst cpu_thermal oder die Zone `x86_pkg_temp`).

use std::path::{Path, PathBuf};

use shared::{
    catalog::{Tpl, system::*},
    system::{SensorInfo, SensorKind, snap},
};

use super::read;
use crate::{
    collectors::{Collector, Tick},
    core::Core,
    registry::{ChartDef, ChartHandle},
    util::round,
};

/// Eine Messgrösse: Datei, Faktor zur Einheit, Nachkommastellen und Platz in der Fühlerliste.
#[derive(Debug, Clone)]
struct Input {
    path: PathBuf,
    scale: f64,
    decimals: u32,
    sensor: usize,
}

#[derive(Debug)]
struct Group {
    chart: ChartHandle,
    kind: SensorKind,
    inputs: Vec<Input>,
}

/// Messgrössen einer Gruppe: (Datei, Faktor, Stellen, Fühler).
pub type Inputs = Vec<(PathBuf, f64, u32, usize)>;

/// Ergebnis eines Durchgangs durch `/sys`: Fühler und ihre Diagramm-Zuordnung (ohne Diagramme).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Layout {
    pub sensors: Vec<SensorInfo>,
    /// (Diagramm-Kontext, Instanz oder leer, Art, [(Datei, Faktor, Stellen, Fühler)])
    pub groups: Vec<(&'static str, String, SensorKind, Inputs)>,
    /// `cpu.temp`: (Dimension, Anzeigename, Fühler)
    pub cpu: Vec<(String, String, usize)>,
}

fn kind_of(prefix: &str) -> Option<(SensorKind, f64, u32)> {
    Some(match prefix {
        "temp" => (SensorKind::Temp, 1000.0, 1),
        "fan" => (SensorKind::Fan, 1.0, 0),
        "in" => (SensorKind::Voltage, 1000.0, 3),
        "power" => (SensorKind::Power, 1_000_000.0, 1),
        "curr" => (SensorKind::Current, 1000.0, 2),
        _ => return None,
    })
}

fn tpl_of(kind: SensorKind) -> &'static Tpl {
    match kind {
        SensorKind::Fan => &SENSORS_FAN,
        SensorKind::Voltage => &SENSORS_VOLTAGE,
        SensorKind::Power => &SENSORS_POWER,
        SensorKind::Current => &SENSORS_CURRENT,
        SensorKind::Thermal => &SENSORS_THERMAL,
        SensorKind::Battery => &SENSORS_BATTERY,
        SensorKind::Temp => &SENSORS_TEMP,
    }
}

/// `temp1_input` → (`temp`, `temp1`).
fn split_input(file: &str) -> Option<(&str, &str)> {
    let id = file.strip_suffix("_input").or_else(|| file.strip_suffix("_average"))?;
    let prefix = id.trim_end_matches(|c: char| c.is_ascii_digit());
    (prefix.len() < id.len()).then_some((prefix, id))
}

fn chip_label(name: &str, dev: &str, block: Option<&str>) -> String {
    match name {
        "coretemp" => "Intel Core (coretemp)".into(),
        "k10temp" | "zenpower" => format!("AMD Ryzen ({name})"),
        "nvme" => format!("NVMe {dev}"),
        "drivetemp" => format!("SATA {} (drivetemp)", block.unwrap_or(dev)),
        "acpitz" => "ACPI-Thermozone".into(),
        "cpu_thermal" | "soc_thermal" => "CPU (SoC)".into(),
        "amdgpu" | "radeon" => format!("AMD-Grafik ({name})"),
        "nouveau" => "NVIDIA-Grafik".into(),
        n if n.starts_with("pch_") => "Chipsatz (PCH)".into(),
        n if n.starts_with("iwlwifi") => "WLAN (iwlwifi)".into(),
        n if n.starts_with("nct") || n.starts_with("it8") || n.starts_with("f71") => format!("Mainboard ({n})"),
        n if n.starts_with("r8169") || n.starts_with("igc") || n.starts_with("e1000") => format!("Netzwerkkarte ({n})"),
        "BAT0" | "BAT1" | "ACAD" | "AC" => format!("Netzteil/Akku ({name})"),
        n => n.to_string(),
    }
}

/// `/sys/class/hwmon` und `/sys/class/thermal` durchgehen.
pub fn scan(sys: &Path) -> Layout {
    let mut out = Layout::default();
    let base = sys.join("class/hwmon");
    // (hwmon-Ordner, Name, Gerät)
    let mut chips: Vec<(PathBuf, String, String, Option<String>)> = Vec::new();
    for e in read::dir(&base) {
        let mut dir = base.join(&e);
        let mut name = read::text(dir.join("name"));
        if name.is_none() {
            // Alte Treiber legen alles unter device/ ab.
            dir = dir.join("device");
            name = read::text(dir.join("name"));
        }
        let Some(name) = name else { continue };
        let dev = read::link_name(&base.join(&e).join("device")).unwrap_or_else(|| e.clone());
        let block = read::dir(&base.join(&e).join("device/block")).into_iter().next();
        chips.push((dir, name, dev, block));
    }
    for (dir, name, dev, block) in &chips {
        let dup = chips.iter().filter(|c| &c.1 == name).count() > 1;
        let inst = if dup { format!("{name}-{}", block.as_deref().unwrap_or(dev)) } else { name.clone() };
        let label = chip_label(name, dev, block.as_deref());
        let mut files: Vec<String> = read::dir(dir).into_iter().filter(|f| split_input(f).is_some()).collect();
        // `_input` vor `_average`; je Fühler nur eine Datei.
        files.sort_by(|a, b| read::natural(split_input(a).unwrap().1, split_input(b).unwrap().1).then(b.cmp(a)));
        files.dedup_by(|a, b| split_input(a).map(|s| s.1) == split_input(b).map(|s| s.1));
        for kind_prefix in ["temp", "fan", "in", "power", "curr"] {
            let Some((kind, scale, decimals)) = kind_of(kind_prefix) else { continue };
            let mut inputs = Vec::new();
            for f in &files {
                let Some((prefix, id)) = split_input(f) else { continue };
                if prefix != kind_prefix {
                    continue;
                }
                let path = dir.join(f);
                let Some(raw) = read::int(&path) else { continue }; // Fühler fehlt (ENODATA)
                let value = round(raw as f64 / scale, decimals);
                let custom = read::text(dir.join(format!("{id}_label")));
                if kind == SensorKind::Temp && !(-40.0..=200.0).contains(&value) {
                    continue;
                }
                if kind == SensorKind::Fan && raw == 0 && custom.is_none() {
                    continue; // nicht angeschlossener Lüfteranschluss
                }
                let lim = |s: &str| {
                    let v = read::int(dir.join(format!("{id}_{s}")))? as f64 / scale;
                    let ok = v.is_finite() && v > 0.0 && (kind != SensorKind::Temp || v < 200.0);
                    ok.then(|| round(v, decimals))
                };
                let sensor = SensorInfo {
                    chip: inst.clone(),
                    chip_label: label.clone(),
                    id: id.to_string(),
                    label: custom.unwrap_or_else(|| id.to_string()),
                    kind,
                    value: Some(value),
                    min: lim("min"),
                    max: lim("max").or_else(|| lim("high")),
                    crit: lim("crit"),
                    chart: tpl_of(kind).chart_id(Some(&inst)),
                    dim: inputs.len(),
                };
                inputs.push((path, scale, decimals, out.sensors.len()));
                out.sensors.push(sensor);
            }
            if !inputs.is_empty() {
                out.groups.push((tpl_of(kind).context, inst.clone(), kind, inputs));
            }
        }
    }
    cpu_dims(&mut out);
    thermal(sys, &mut out);
    battery(sys, &mut out);
    out
}

/// Dimensionen von `cpu.temp` aus dem CPU-Chip.
fn cpu_dims(out: &mut Layout) {
    let cpu_chip = ["coretemp", "k10temp", "zenpower", "cpu_thermal", "soc_thermal"]
        .iter()
        .find_map(|c| out.sensors.iter().find(|s| s.chip.split('-').next() == Some(*c)).map(|s| s.chip.clone()));
    let Some(chip) = cpu_chip else { return };
    let mut pkg_n = 0;
    for (i, s) in out.sensors.iter().enumerate() {
        if s.chip != chip || s.kind != SensorKind::Temp {
            continue;
        }
        let l = s.label.as_str();
        let (id, name) = if let Some(n) = l.strip_prefix("Core ") {
            (format!("core{n}"), format!("Kern {n}"))
        } else if let Some(n) = l.strip_prefix("Tccd") {
            (format!("ccd{n}"), format!("CCD {n}"))
        } else if l.starts_with("Package id") || l == "Tctl" || l == "Tdie" || l == "temp1" {
            if l == "Tctl" && out.sensors.iter().any(|x| x.chip == chip && x.label == "Tdie") {
                continue; // Tdie ist genauer (ohne Versatz)
            }
            pkg_n += 1;
            if pkg_n == 1 { ("package".into(), "Gehäuse".into()) } else { (format!("package{pkg_n}"), format!("Gehäuse {pkg_n}")) }
        } else {
            continue;
        };
        out.cpu.push((id, name, i));
    }
}

fn thermal(sys: &Path, out: &mut Layout) {
    let base = sys.join("class/thermal");
    let mut inputs = Vec::new();
    for e in read::dir(&base) {
        if !e.starts_with("thermal_zone") {
            continue;
        }
        let dir = base.join(&e);
        let path = dir.join("temp");
        let Some(raw) = read::int(&path) else { continue };
        let value = round(raw as f64 / 1000.0, 1);
        if !(-40.0..=200.0).contains(&value) {
            continue;
        }
        let ty = read::text(dir.join("type")).unwrap_or_else(|| e.clone());
        let mut id = ty.clone();
        let mut n = 2;
        while out.sensors.iter().any(|s| s.kind == SensorKind::Thermal && s.id == id) {
            id = format!("{ty}_{n}");
            n += 1;
        }
        let mut crit = None;
        let mut max = None;
        for i in 0..12 {
            let Some(t) = read::text(dir.join(format!("trip_point_{i}_type"))) else { break };
            let v = read::int(dir.join(format!("trip_point_{i}_temp"))).map(|v| round(v as f64 / 1000.0, 1)).filter(|v| *v > 0.0 && *v < 200.0);
            match t.as_str() {
                "critical" => crit = crit.or(v),
                "hot" | "passive" => max = max.or(v),
                _ => {}
            }
        }
        inputs.push((path, 1000.0, 1, out.sensors.len()));
        out.sensors.push(SensorInfo {
            chip: "thermal".into(),
            chip_label: "Thermische Zonen".into(),
            label: ty,
            id,
            kind: SensorKind::Thermal,
            value: Some(value),
            max,
            crit,
            chart: SENSORS_THERMAL.chart_id(None),
            dim: inputs.len() - 1,
            ..Default::default()
        });
    }
    if !inputs.is_empty() {
        out.groups.push((SENSORS_THERMAL.context, String::new(), SensorKind::Thermal, inputs));
    }
    // Ohne CPU-Chip: Zone x86_pkg_temp als Gehäuse.
    if out.cpu.is_empty()
        && let Some(i) = out.sensors.iter().position(|s| s.kind == SensorKind::Thermal && s.label == "x86_pkg_temp") {
            out.cpu.push(("package".into(), "Gehäuse".into(), i));
        }
}

fn battery(sys: &Path, out: &mut Layout) {
    let base = sys.join("class/power_supply");
    for e in read::dir(&base) {
        let dir = base.join(&e);
        if read::text(dir.join("type")).as_deref() != Some("Battery") {
            continue;
        }
        let path = dir.join("capacity");
        let Some(v) = read::int(&path) else { continue };
        out.groups.push((SENSORS_BATTERY.context, e.clone(), SensorKind::Battery, vec![(path, 1.0, 0, out.sensors.len())]));
        out.sensors.push(SensorInfo {
            chip: e.clone(),
            chip_label: format!("Akku {e}"),
            id: "capacity".into(),
            label: read::text(dir.join("model_name")).filter(|m| !m.is_empty()).unwrap_or_else(|| "Ladung".into()),
            kind: SensorKind::Battery,
            value: Some(v as f64),
            chart: SENSORS_BATTERY.chart_id(Some(&e)),
            ..Default::default()
        });
    }
}

pub struct Sensors {
    sys: PathBuf,
    buf: String,
    layout: Layout,
    groups: Vec<Group>,
    cpu: Option<(ChartHandle, Vec<usize>)>,
    hwmon_dirs: Vec<String>,
    checked: i64,
}

impl Sensors {
    pub fn new(core: &Core) -> Self {
        let mut me = Self {
            sys: core.cfg.sys.clone(),
            buf: String::with_capacity(64),
            layout: Layout::default(),
            groups: Vec::new(),
            cpu: None,
            hwmon_dirs: Vec::new(),
            checked: 0,
        };
        me.rebuild(core);
        me
    }

    fn rebuild(&mut self, core: &Core) {
        self.hwmon_dirs = read::dir(&self.sys.join("class/hwmon"));
        self.layout = scan(&self.sys);
        let sensors = &self.layout.sensors;
        self.groups = self
            .layout
            .groups
            .iter()
            .map(|(_, inst, kind, inputs)| {
                let mut def = ChartDef::tpl(tpl_of(*kind));
                if !inst.is_empty() {
                    def = def.instance(inst).label("chip_label", sensors[inputs[0].3].chip_label.clone());
                }
                if !matches!(kind, SensorKind::Battery) {
                    def = def.dims(inputs.iter().map(|i| (sensors[i.3].id.clone(), sensors[i.3].label.clone())));
                }
                Group {
                    chart: core.chart(def),
                    kind: *kind,
                    inputs: inputs.iter().map(|(p, s, d, i)| Input { path: p.clone(), scale: *s, decimals: *d, sensor: *i }).collect(),
                }
            })
            .collect();
        self.cpu = (!self.layout.cpu.is_empty()).then(|| {
            let dims = self.layout.cpu.iter().map(|(id, name, _)| (id.clone(), name.clone()));
            (core.chart(ChartDef::tpl(&CPU_TEMP).dims(dims)), self.layout.cpu.iter().map(|c| c.2).collect())
        });
    }
}

impl Collector for Sensors {
    fn name(&self) -> &'static str {
        "sensors"
    }

    fn collect(&mut self, t: &mut Tick<'_>) -> anyhow::Result<()> {
        if !t.due(2) {
            return Ok(());
        }
        // Neue oder entfernte Chips (USB, Treiber nachgeladen): alle 5 min prüfen.
        if t.ts - self.checked >= 300 {
            self.checked = t.ts;
            if read::dir(&self.sys.join("class/hwmon")) != self.hwmon_dirs {
                self.rebuild(t.core);
            }
        }
        let slow = t.due(5);
        let battery = t.due(30);
        for g in &self.groups {
            let due = match g.kind {
                SensorKind::Voltage | SensorKind::Current => slow,
                SensorKind::Battery => battery,
                _ => true,
            };
            if !due {
                continue;
            }
            let mut vals = Vec::with_capacity(g.inputs.len());
            for i in &g.inputs {
                let v = read::num::<f64>(&mut self.buf, &i.path).map(|raw| round(raw / i.scale, i.decimals));
                self.layout.sensors[i.sensor].value = v;
                vals.push(v.unwrap_or(f64::NAN));
            }
            t.put(&g.chart, &vals);
        }
        if let Some((chart, idx)) = &self.cpu {
            let vals: Vec<f64> = idx.iter().map(|&i| self.layout.sensors[i].value.unwrap_or(f64::NAN)).collect();
            t.put(chart, &vals);
        }
        t.core.snapshot(snap::SENSORS, &self.layout.sensors);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(p: &Path, s: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, s).unwrap();
    }

    /// Nachgebauter `/sys` mit coretemp, NVMe, einem Mainboard-Chip, Zonen und Akku.
    fn fake_sys() -> PathBuf {
        let sys = std::env::temp_dir().join(format!("nrzmon-sensors-{}-{}", std::process::id(), fastrand::u32(..)));
        let h0 = sys.join("class/hwmon/hwmon0");
        w(&h0.join("name"), "coretemp\n");
        w(&h0.join("temp1_input"), "45000\n");
        w(&h0.join("temp1_label"), "Package id 0\n");
        w(&h0.join("temp1_max"), "84000\n");
        w(&h0.join("temp1_crit"), "100000\n");
        w(&h0.join("temp2_input"), "43000\n");
        w(&h0.join("temp2_label"), "Core 0\n");
        w(&h0.join("temp3_input"), "44000\n");
        w(&h0.join("temp3_label"), "Core 1\n");
        let h1 = sys.join("class/hwmon/hwmon1");
        w(&h1.join("name"), "nvme\n");
        w(&h1.join("temp1_input"), "38850\n");
        w(&h1.join("temp1_label"), "Composite\n");
        w(&h1.join("temp1_max"), "81850\n");
        w(&h1.join("temp1_crit"), "84850\n");
        w(&h1.join("temp2_input"), "41850\n");
        w(&h1.join("temp2_max"), "65261850\n");
        let h2 = sys.join("class/hwmon/hwmon2");
        w(&h2.join("name"), "nct6793\n");
        w(&h2.join("fan1_input"), "1200\n");
        w(&h2.join("fan2_input"), "0\n");
        w(&h2.join("in0_input"), "832\n");
        w(&h2.join("in0_label"), "Vcore\n");
        w(&h2.join("in0_max"), "1488\n");
        w(&h2.join("power1_average"), "12500000\n");
        let z0 = sys.join("class/thermal/thermal_zone0");
        w(&z0.join("type"), "acpitz\n");
        w(&z0.join("temp"), "27800\n");
        w(&z0.join("trip_point_0_type"), "critical\n");
        w(&z0.join("trip_point_0_temp"), "119000\n");
        let z1 = sys.join("class/thermal/thermal_zone1");
        w(&z1.join("type"), "x86_pkg_temp\n");
        w(&z1.join("temp"), "46000\n");
        let b = sys.join("class/power_supply/BAT0");
        w(&b.join("type"), "Battery\n");
        w(&b.join("capacity"), "87\n");
        w(&sys.join("class/power_supply/AC/type"), "Mains\n");
        sys
    }

    #[test]
    fn scan_fake_hwmon() {
        let sys = fake_sys();
        let l = scan(&sys);
        let find = |chip: &str, id: &str| l.sensors.iter().find(|s| s.chip == chip && s.id == id).unwrap();
        let pkg = find("coretemp", "temp1");
        assert_eq!((pkg.label.as_str(), pkg.value, pkg.max, pkg.crit), ("Package id 0", Some(45.0), Some(84.0), Some(100.0)));
        assert_eq!(pkg.chart, "sensors.temp.coretemp");
        assert_eq!(pkg.limit(), Some(100.0));
        // Unsinnige Grenze (65261 °C) fällt weg, Name ohne Label = ID.
        let s2 = find("nvme", "temp2");
        assert_eq!((s2.label.as_str(), s2.max, s2.dim), ("temp2", None, 1));
        assert_eq!(find("nvme", "temp1").value, Some(38.9));
        // Lüfter 0 ohne Label ist nicht angeschlossen.
        assert!(l.sensors.iter().all(|s| !(s.chip == "nct6793" && s.id == "fan2")));
        let v = find("nct6793", "in0");
        assert_eq!((v.kind, v.value, v.max, v.label.as_str()), (SensorKind::Voltage, Some(0.832), Some(1.488), "Vcore"));
        assert_eq!(find("nct6793", "power1").value, Some(12.5));
        assert_eq!(find("nct6793", "fan1").chart, "sensors.fan.nct6793");
        // cpu.temp: Gehäuse + Kerne.
        let cpu: Vec<&str> = l.cpu.iter().map(|c| c.0.as_str()).collect();
        assert_eq!(cpu, ["package", "core0", "core1"]);
        // Zonen und Akku.
        let z = find("thermal", "acpitz");
        assert_eq!((z.value, z.crit, z.chart.as_str()), (Some(27.8), Some(119.0), "sensors.thermal"));
        let bat = find("BAT0", "capacity");
        assert_eq!((bat.kind, bat.value), (SensorKind::Battery, Some(87.0)));
        // Gruppen: coretemp, nvme, nct6793 (fan, in, power), thermal, battery
        let groups: Vec<(&str, &str)> = l.groups.iter().map(|g| (g.0, g.1.as_str())).collect();
        assert_eq!(
            groups,
            [
                ("sensors.temp", "coretemp"),
                ("sensors.temp", "nvme"),
                ("sensors.fan", "nct6793"),
                ("sensors.voltage", "nct6793"),
                ("sensors.power", "nct6793"),
                ("sensors.thermal", ""),
                ("sensors.battery", "BAT0"),
            ]
        );
        let _ = std::fs::remove_dir_all(sys);
    }

    #[test]
    fn duplicate_chips_get_device_names() {
        let sys = std::env::temp_dir().join(format!("nrzmon-sensors-dup-{}", std::process::id()));
        for (h, dev) in [("hwmon3", "nvme0"), ("hwmon4", "nvme1")] {
            let d = sys.join("class/hwmon").join(h);
            w(&d.join("name"), "nvme\n");
            w(&d.join("temp1_input"), "40000\n");
            std::fs::create_dir_all(sys.join("devices").join(dev)).unwrap();
            let _ = std::os::unix::fs::symlink(sys.join("devices").join(dev), d.join("device"));
        }
        let l = scan(&sys);
        let chips: Vec<&str> = l.sensors.iter().map(|s| s.chip.as_str()).collect();
        assert_eq!(chips, ["nvme-nvme0", "nvme-nvme1"]);
        assert_eq!(l.sensors[1].chip_label, "NVMe nvme1");
        let _ = std::fs::remove_dir_all(sys);
    }
}
