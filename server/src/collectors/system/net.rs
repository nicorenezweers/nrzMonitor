//! Netzwerk: `/proc/net/dev` für physische Schnittstellen, Bridges, VPN (`wt0`, `wg*`, `tailscale0`)
//! und `docker0` – keine einzelnen veth*. `system.net` ist die Summe der physischen. Dazu TCP/UDP aus
//! `/proc/net/snmp`, Sockets aus `sockstat` und conntrack.

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    path::{Path, PathBuf},
};

use shared::{
    catalog::system::*,
    system::{IfaceInfo, snap},
};

use super::read;
use crate::{
    collectors::{Collector, Tick},
    core::Core,
    registry::{ChartDef, ChartHandle},
    util::{Counter, nan},
};

/// Zähler einer Schnittstelle aus `/proc/net/dev`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct IfStat {
    pub rx_bytes: u64,
    pub rx_packets: u64,
    pub rx_errs: u64,
    pub rx_drop: u64,
    pub tx_bytes: u64,
    pub tx_packets: u64,
    pub tx_errs: u64,
    pub tx_drop: u64,
}

pub fn parse_net_dev(text: &str, mut f: impl FnMut(&str, IfStat)) {
    for line in text.lines().skip(2) {
        let Some((name, rest)) = line.split_once(':') else { continue };
        let v: Vec<u64> = rest.split_ascii_whitespace().take(16).map(|x| x.parse().unwrap_or(0)).collect();
        if v.len() < 12 {
            continue;
        }
        f(name.trim(), IfStat {
            rx_bytes: v[0],
            rx_packets: v[1],
            rx_errs: v[2],
            rx_drop: v[3],
            tx_bytes: v[8],
            tx_packets: v[9],
            tx_errs: v[10],
            tx_drop: v[11],
        });
    }
}

/// Art einer Schnittstelle; `None` = nicht anzeigen (lo, veth*, Tunnel-Platzhalter, leere Bridges).
pub fn classify(sys_net: &Path, name: &str) -> Option<&'static str> {
    const SKIP: &[&str] = &["lo", "sit0", "tunl0", "ip6tnl0", "ip_vti0", "ip6_vti0", "gre0", "gretap0", "erspan0", "ip6gre0", "bonding_masters"];
    if SKIP.contains(&name) || name.starts_with("veth") || name.starts_with("dummy") {
        return None;
    }
    let dir = sys_net.join(name);
    let uevent = read::text(dir.join("uevent")).unwrap_or_default();
    let devtype = uevent.lines().find_map(|l| l.strip_prefix("DEVTYPE=")).unwrap_or("");
    let vpn = ["wt", "wg", "tailscale", "tun", "tap", "ppp", "zt", "utun", "nordlynx"];
    if devtype == "wireguard" || vpn.iter().any(|p| name.starts_with(p)) {
        return Some("vpn");
    }
    if dir.join("bridge").exists() || devtype == "bridge" {
        if name == "docker0" {
            return Some("docker");
        }
        // Docker legt je Netz eine Bridge an – ohne angeschlossene Ports fliesst nichts.
        return (!read::dir(&dir.join("brif")).is_empty()).then_some("bridge");
    }
    if dir.join("wireless").exists() || dir.join("phy80211").exists() || devtype == "wlan" {
        return Some("wireless");
    }
    if dir.join("device").exists() {
        return Some("physical");
    }
    if devtype == "vlan" || devtype == "bond" || name.starts_with("bond") {
        return Some("virtual");
    }
    None
}

/// Adressen aller Schnittstellen (`192.168.1.10/24`) über getifaddrs.
fn addresses() -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs füllt eine verkettete Liste, die wir unten mit freeifaddrs freigeben; die
    // Zeiger werden nur innerhalb der Liste gelesen und auf ihre Familie geprüft.
    unsafe {
        if libc::getifaddrs(&mut ifap) != 0 {
            return out;
        }
        let mut cur = ifap;
        while !cur.is_null() {
            let ifa = &*cur;
            cur = ifa.ifa_next;
            if ifa.ifa_addr.is_null() || ifa.ifa_name.is_null() {
                continue;
            }
            let name = std::ffi::CStr::from_ptr(ifa.ifa_name).to_string_lossy().into_owned();
            let fam = i32::from((*ifa.ifa_addr).sa_family);
            let (ip, prefix) = if fam == libc::AF_INET {
                let a = &*(ifa.ifa_addr as *const libc::sockaddr_in);
                let ip = IpAddr::V4(Ipv4Addr::from(u32::from_be(a.sin_addr.s_addr)));
                let p = if ifa.ifa_netmask.is_null() {
                    32
                } else {
                    (*(ifa.ifa_netmask as *const libc::sockaddr_in)).sin_addr.s_addr.count_ones()
                };
                (ip, p)
            } else if fam == libc::AF_INET6 {
                let a = &*(ifa.ifa_addr as *const libc::sockaddr_in6);
                let ip = IpAddr::V6(Ipv6Addr::from(a.sin6_addr.s6_addr));
                let p = if ifa.ifa_netmask.is_null() {
                    128
                } else {
                    (*(ifa.ifa_netmask as *const libc::sockaddr_in6)).sin6_addr.s6_addr.iter().map(|b| b.count_ones()).sum()
                };
                (ip, p)
            } else {
                continue;
            };
            out.entry(name).or_default().push(format!("{ip}/{prefix}"));
        }
        libc::freeifaddrs(ifap);
    }
    out
}

struct Iface {
    kind: &'static str,
    prev: Option<IfStat>,
    traffic: ChartHandle,
    packets: ChartHandle,
    errors: ChartHandle,
    drops: ChartHandle,
    rx: f64,
    tx: f64,
    totals: (u64, u64),
}

pub struct Net {
    net_dev: PathBuf,
    snmp: PathBuf,
    sockstat: PathBuf,
    proc: PathBuf,
    sys_net: PathBuf,
    buf: String,
    /// Schnittstellen in Reihenfolge des Auftretens; `None` = bewusst übergangen.
    ifaces: Vec<(String, Option<Iface>)>,
    scanned: i64,
    total: ChartHandle,
    counters: HashMap<&'static str, Counter>,
    tcp_conn: ChartHandle,
    tcp_opens: ChartHandle,
    tcp_errors: ChartHandle,
    udp_packets: ChartHandle,
    udp_errors: ChartHandle,
    sockets: ChartHandle,
    conntrack: Option<ChartHandle>,
}

/// Erst die Sicht von PID 1 (Host-Netz, auch wenn der Container ein eigenes hätte), sonst die eigene.
fn host_net(proc: &Path, file: &str) -> PathBuf {
    let p = proc.join("1/net").join(file);
    if std::fs::metadata(&p).is_ok() && std::fs::File::open(&p).is_ok() { p } else { proc.join("net").join(file) }
}

impl Net {
    pub fn new(core: &Core) -> Self {
        let proc = core.cfg.proc.clone();
        let conntrack = proc.join("sys/net/netfilter/nf_conntrack_count").exists();
        Self {
            net_dev: host_net(&proc, "dev"),
            snmp: host_net(&proc, "snmp"),
            sockstat: host_net(&proc, "sockstat"),
            sys_net: core.cfg.sys.join("class/net"),
            proc,
            buf: String::with_capacity(16 * 1024),
            ifaces: Vec::new(),
            scanned: 0,
            total: core.chart(ChartDef::tpl(&SYSTEM_NET)),
            counters: HashMap::new(),
            tcp_conn: core.chart(ChartDef::tpl(&NET_TCP_CONN)),
            tcp_opens: core.chart(ChartDef::tpl(&NET_TCP_OPENS)),
            tcp_errors: core.chart(ChartDef::tpl(&NET_TCP_ERRORS)),
            udp_packets: core.chart(ChartDef::tpl(&NET_UDP_PACKETS)),
            udp_errors: core.chart(ChartDef::tpl(&NET_UDP_ERRORS)),
            sockets: core.chart(ChartDef::tpl(&NET_SOCKETS)),
            conntrack: conntrack.then(|| core.chart(ChartDef::tpl(&NET_CONNTRACK))),
        }
    }

    fn make(&self, core: &Core, name: &str) -> Option<Iface> {
        let kind = classify(&self.sys_net, name)?;
        let def = |tpl| ChartDef::tpl(tpl).instance(name).label("kind", kind);
        Some(Iface {
            kind,
            prev: None,
            traffic: core.chart(def(&NET_TRAFFIC)),
            packets: core.chart(def(&NET_PACKETS)),
            errors: core.chart(def(&NET_ERRORS)),
            drops: core.chart(def(&NET_DROPS)),
            rx: 0.0,
            tx: 0.0,
            totals: (0, 0),
        })
    }

    fn rate(&mut self, key: &'static str, v: u64, dt: f64) -> f64 {
        nan(self.counters.entry(key).or_default().rate(v, dt).map(f64::round))
    }

    fn snmp(&mut self, t: &mut Tick<'_>) {
        if !read::into(&mut self.buf, &self.snmp) {
            return;
        }
        let mut vals: HashMap<&'static str, u64> = HashMap::new();
        const KEYS: &[&str] = &[
            "Tcp.CurrEstab", "Tcp.ActiveOpens", "Tcp.PassiveOpens", "Tcp.RetransSegs", "Tcp.InErrs", "Tcp.OutRsts",
            "Tcp.AttemptFails", "Tcp.EstabResets", "Udp.InDatagrams", "Udp.OutDatagrams", "Udp.InErrors", "Udp.NoPorts",
            "Udp.RcvbufErrors", "Udp.SndbufErrors",
        ];
        read::header_table(&self.buf, |p, k, v| {
            if let Some(key) = KEYS.iter().find(|key| key.len() == p.len() + 1 + k.len() && key.starts_with(p) && key.ends_with(k)) {
                vals.insert(key, v);
            }
        });
        if vals.is_empty() {
            return;
        }
        let dt = t.dt;
        let g = |k: &str| vals.get(k).copied().unwrap_or(0);
        let mut r = |k: &'static str| {
            let v = g(k);
            self.rate(k, v, dt)
        };
        let opens = [r("Tcp.ActiveOpens"), r("Tcp.PassiveOpens")];
        let errs = [r("Tcp.RetransSegs"), r("Tcp.InErrs"), r("Tcp.OutRsts"), r("Tcp.AttemptFails"), r("Tcp.EstabResets")];
        let udp = [r("Udp.InDatagrams"), r("Udp.OutDatagrams")];
        let uerr = [r("Udp.InErrors"), r("Udp.NoPorts"), r("Udp.RcvbufErrors"), r("Udp.SndbufErrors")];
        t.put(&self.tcp_conn, &[g("Tcp.CurrEstab") as f64]);
        t.put(&self.tcp_opens, &opens);
        t.put(&self.tcp_errors, &errs);
        t.put(&self.udp_packets, &udp);
        t.put(&self.udp_errors, &uerr);
    }

    fn slow(&mut self, t: &mut Tick<'_>) {
        if read::into(&mut self.buf, &self.sockstat)
            && let Some(s) = parse_sockstat(&self.buf) {
                t.put(&self.sockets, &s);
            }
        if let Some(c) = &self.conntrack {
            let base = self.proc.join("sys/net/netfilter");
            if let (Some(n), Some(m)) = (read::int(base.join("nf_conntrack_count")), read::int(base.join("nf_conntrack_max"))) {
                t.put(c, &[n as f64, m as f64]);
            }
        }
        // Momentaufnahme der Schnittstellen.
        let addrs = addresses();
        let list: Vec<IfaceInfo> = self
            .ifaces
            .iter()
            .filter_map(|(name, i)| {
                let i = i.as_ref()?;
                let dir = self.sys_net.join(name);
                let state = read::text(dir.join("operstate")).unwrap_or_default();
                let flags = read::text(dir.join("flags")).and_then(|f| i64::from_str_radix(f.trim_start_matches("0x"), 16).ok()).unwrap_or(0);
                Some(IfaceInfo {
                    name: name.clone(),
                    kind: i.kind.to_string(),
                    up: state == "up" || (state == "unknown" && flags & 1 == 1),
                    state,
                    mac: read::text(dir.join("address")).filter(|m| m != "00:00:00:00:00:00").unwrap_or_default(),
                    mtu: read::int(dir.join("mtu")).unwrap_or(0).max(0) as u32,
                    speed: read::int(dir.join("speed")).unwrap_or(0).max(0) as u32,
                    addrs: addrs.get(name).cloned().unwrap_or_default(),
                    rx: i.rx,
                    tx: i.tx,
                    rx_total: i.totals.0,
                    tx_total: i.totals.1,
                    ports: read::dir(&dir.join("brif")).len() as u32,
                })
            })
            .collect();
        t.core.snapshot(snap::INTERFACES, &list);
    }
}

/// `/proc/net/sockstat` → [TCP in Gebrauch, UDP, verwaist, Time-Wait].
pub fn parse_sockstat(text: &str) -> Option<[f64; 4]> {
    let mut out = [f64::NAN; 4];
    for line in text.lines() {
        let mut it = line.split_ascii_whitespace();
        let proto = it.next()?;
        let pairs: Vec<&str> = it.collect();
        let get = |k: &str| pairs.chunks(2).find(|c| c[0] == k).and_then(|c| c.get(1)?.parse::<f64>().ok());
        match proto {
            "TCP:" => {
                out[0] = get("inuse").unwrap_or(f64::NAN);
                out[2] = get("orphan").unwrap_or(f64::NAN);
                out[3] = get("tw").unwrap_or(f64::NAN);
            }
            "UDP:" => out[1] = get("inuse").unwrap_or(f64::NAN),
            _ => {}
        }
    }
    (!out[0].is_nan()).then_some(out)
}

impl Collector for Net {
    fn name(&self) -> &'static str {
        "network"
    }

    fn collect(&mut self, t: &mut Tick<'_>) -> anyhow::Result<()> {
        // Neu prüfen: Bridges bekommen Ports, VPNs kommen und gehen.
        if self.scanned == 0 || t.ts - self.scanned >= 30 {
            self.scanned = t.ts;
            let names: Vec<String> = self.ifaces.iter().filter(|(_, i)| i.is_none()).map(|(n, _)| n.clone()).collect();
            for n in names {
                if let Some(i) = self.make(t.core, &n)
                    && let Some(slot) = self.ifaces.iter_mut().find(|(m, _)| *m == n) {
                        slot.1 = Some(i);
                    }
            }
        }
        anyhow::ensure!(read::into(&mut self.buf, &self.net_dev), "/proc/net/dev nicht lesbar");
        let mut stats: Vec<(String, IfStat)> = Vec::with_capacity(self.ifaces.len());
        parse_net_dev(&self.buf, |n, s| stats.push((n.to_string(), s)));
        // Verschwundene Schnittstellen stilllegen.
        let core = t.core;
        self.ifaces.retain(|(n, i)| {
            let alive = stats.iter().any(|(m, _)| m == n);
            if !alive
                && let Some(i) = i {
                    for c in [&i.traffic, &i.packets, &i.errors, &i.drops] {
                        core.registry.retire(&c.id);
                    }
                }
            alive
        });
        let dt = t.dt.max(1e-3);
        let (mut rx_sum, mut tx_sum, mut any) = (0.0, 0.0, false);
        for (name, s) in stats {
            let pos = match self.ifaces.iter().position(|(n, _)| *n == name) {
                Some(p) => p,
                None => {
                    let i = self.make(t.core, &name);
                    self.ifaces.push((name, i));
                    self.ifaces.len() - 1
                }
            };
            let Some(i) = self.ifaces[pos].1.as_mut() else { continue };
            i.totals = (s.rx_bytes, s.tx_bytes);
            let Some(p) = i.prev.replace(s) else { continue };
            if s.rx_bytes < p.rx_bytes || s.tx_bytes < p.tx_bytes {
                continue;
            }
            let r = |a: u64, b: u64| (a.saturating_sub(b) as f64 / dt).round();
            let (rx, tx) = (r(s.rx_bytes, p.rx_bytes) * 8.0, r(s.tx_bytes, p.tx_bytes) * 8.0);
            (i.rx, i.tx) = (rx, tx);
            t.put(&i.traffic, &[rx, tx]);
            t.put(&i.packets, &[r(s.rx_packets, p.rx_packets), r(s.tx_packets, p.tx_packets)]);
            t.put(&i.errors, &[r(s.rx_errs, p.rx_errs), r(s.tx_errs, p.tx_errs)]);
            t.put(&i.drops, &[r(s.rx_drop, p.rx_drop), r(s.tx_drop, p.tx_drop)]);
            if matches!(i.kind, "physical" | "wireless") {
                rx_sum += rx;
                tx_sum += tx;
                any = true;
            }
        }
        if any {
            t.put(&self.total, &[rx_sum, tx_sum]);
        }
        self.snmp(t);
        if t.due(5) {
            self.slow(t);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NET_DEV: &str = "Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
    lo: 1000      10    0    0    0     0          0         0     1000      10    0    0    0     0       0          0
enp1s0:123456789 1000 1 2 0 0 0 5 987654 900 3 4 0 0 0 0
";

    #[test]
    fn net_dev() {
        let mut got = Vec::new();
        parse_net_dev(NET_DEV, |n, s| got.push((n.to_string(), s)));
        assert_eq!(got.len(), 2);
        assert_eq!(got[1].0, "enp1s0");
        assert_eq!(got[1].1, IfStat { rx_bytes: 123_456_789, rx_packets: 1000, rx_errs: 1, rx_drop: 2, tx_bytes: 987_654, tx_packets: 900, tx_errs: 3, tx_drop: 4 });
    }

    #[test]
    fn classify_with_fake_sysfs() {
        let dir = std::env::temp_dir().join(format!("nrzmon-net-{}", std::process::id()));
        let mk = |p: &str| std::fs::create_dir_all(dir.join(p)).unwrap();
        mk("enp1s0/device");
        mk("docker0/bridge");
        mk("br-empty/bridge");
        mk("br-full/bridge");
        mk("br-full/brif/veth123");
        mk("wt0");
        mk("wlan0/wireless");
        mk("veth123");
        mk("lo");
        mk("wg-x");
        std::fs::write(dir.join("wg-x/uevent"), "DEVTYPE=wireguard\nINTERFACE=wg-x\n").unwrap();
        assert_eq!(classify(&dir, "enp1s0"), Some("physical"));
        assert_eq!(classify(&dir, "docker0"), Some("docker"));
        assert_eq!(classify(&dir, "br-empty"), None);
        assert_eq!(classify(&dir, "br-full"), Some("bridge"));
        assert_eq!(classify(&dir, "wt0"), Some("vpn"));
        assert_eq!(classify(&dir, "wg-x"), Some("vpn"));
        assert_eq!(classify(&dir, "wlan0"), Some("wireless"));
        assert_eq!(classify(&dir, "veth123"), None);
        assert_eq!(classify(&dir, "lo"), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn sockstat() {
        let s = parse_sockstat("sockets: used 300\nTCP: inuse 25 orphan 1 tw 7 alloc 30 mem 4\nUDP: inuse 6 mem 2\n").unwrap();
        assert_eq!(s, [25.0, 6.0, 1.0, 7.0]);
    }
}
