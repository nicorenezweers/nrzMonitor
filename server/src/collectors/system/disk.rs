//! Datenträger: `/proc/diskstats` je echtem Gerät (ohne loop/ram/zram, Partitionen nicht doppelt),
//! Temperatur je Gerät über hwmon, Belegung und Inodes je Dateisystem (mountinfo + statvfs unter
//! `core.cfg.root`, ohne Pseudo-Dateisysteme, Overlays und Bind-Mounts).

use std::{
    collections::HashMap,
    ffi::CString,
    path::{Path, PathBuf},
};

use shared::{
    catalog::system::*,
    system::{DiskInfo, FsInfo, snap},
};

use super::read;
use crate::{
    collectors::{Collector, Tick},
    core::Core,
    registry::{ChartDef, ChartHandle},
    util::round,
};

/// Eine Zeile aus `/proc/diskstats`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DiskStat {
    pub reads: u64,
    pub read_sectors: u64,
    pub read_ms: u64,
    pub writes: u64,
    pub write_sectors: u64,
    pub write_ms: u64,
    pub io_ms: u64,
    pub weighted_ms: u64,
}

/// `/proc/diskstats` → (Name, Werte).
pub fn parse_diskstats(text: &str, mut f: impl FnMut(&str, DiskStat)) {
    for line in text.lines() {
        let mut it = line.split_ascii_whitespace();
        let (Some(_), Some(_), Some(name)) = (it.next(), it.next(), it.next()) else { continue };
        let v: Vec<u64> = it.take(11).map(|x| x.parse().unwrap_or(0)).collect();
        if v.len() < 11 {
            continue;
        }
        f(name, DiskStat {
            reads: v[0],
            read_sectors: v[2],
            read_ms: v[3],
            writes: v[4],
            write_sectors: v[6],
            write_ms: v[7],
            io_ms: v[9],
            weighted_ms: v[10],
        });
    }
}

/// Geräte, die keine eigenen Datenträger sind.
pub fn skip_device(name: &str) -> bool {
    ["loop", "ram", "zram", "fd", "sr", "nbd", "md127p"].iter().any(|p| name.starts_with(p))
}

/// Dateisysteme ohne eigenen Speicherplatz (oder Netz-/Gastfreigaben, deren `statvfs` hängen kann).
pub fn skip_fstype(fs: &str) -> bool {
    const PSEUDO: &[&str] = &[
        "proc", "sysfs", "tmpfs", "devtmpfs", "devpts", "cgroup", "cgroup2", "overlay", "aufs", "squashfs", "nsfs",
        "autofs", "mqueue", "debugfs", "tracefs", "securityfs", "pstore", "bpf", "fusectl", "configfs", "hugetlbfs",
        "binfmt_misc", "efivarfs", "ramfs", "rpc_pipefs", "nfsd", "selinuxfs", "iso9660", "shm", "none", "zfs",
        // Netz und Gastfreigaben
        "nfs", "nfs4", "cifs", "smb3", "smbfs", "9p", "virtiofs", "ceph", "glusterfs", "afs",
    ];
    PSEUDO.contains(&fs) || (fs.starts_with("fuse.") && fs != "fuse.mergerfs") || fs == "fuse"
}

fn skip_mount(mount: &str) -> bool {
    const PREFIX: &[&str] = &["/proc", "/sys", "/dev", "/run", "/var/lib/docker", "/var/lib/containerd", "/snap", "/var/snap", "/var/lib/kubelet"];
    PREFIX.iter().any(|p| mount == *p || mount.starts_with(&format!("{p}/")))
}

/// `\040` usw. in mountinfo zurückwandeln.
fn unescape(s: &str) -> String {
    if !s.contains('\\') {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
            out.push((b[i + 1] - b'0') * 64 + (b[i + 2] - b'0') * 8 + (b[i + 3] - b'0'));
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Eingehängtes Dateisystem aus mountinfo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub dev: String,
    pub mount: String,
    pub fstype: String,
    pub source: String,
    /// Auf dem Host nur lesend eingehängt (aus den Optionen; `statvfs` sieht im Container immer ro).
    pub readonly: bool,
}

/// Echte Dateisysteme aus `/proc/1/mountinfo`: je Gerät nur einmal (Bind-Mounts und btrfs-Subvolumes
/// fallen weg, bevorzugt wird die Wurzel und der kürzeste Pfad).
pub fn parse_mountinfo(text: &str) -> Vec<Mount> {
    let mut best: Vec<(bool, Mount)> = Vec::new();
    for line in text.lines() {
        let Some((left, right)) = line.split_once(" - ") else { continue };
        let l: Vec<&str> = left.split(' ').collect();
        let r: Vec<&str> = right.split(' ').collect();
        if l.len() < 5 || r.len() < 2 {
            continue;
        }
        let (dev, root, mount, fstype, source) = (l[2], unescape(l[3]), unescape(l[4]), r[0], unescape(r[1]));
        if skip_fstype(fstype) || skip_mount(&mount) {
            continue;
        }
        let is_root = root == "/";
        let readonly = l.get(5).is_some_and(|o| o.split(',').any(|x| x == "ro"));
        let m = Mount { dev: dev.to_string(), mount, fstype: fstype.to_string(), source, readonly };
        match best.iter_mut().find(|(_, b)| b.dev == m.dev) {
            Some(slot) => {
                let better = (is_root && !slot.0) || (is_root == slot.0 && m.mount.len() < slot.1.mount.len());
                if better {
                    *slot = (is_root, m);
                }
            }
            None => best.push((is_root, m)),
        }
    }
    let mut out: Vec<Mount> = best.into_iter().map(|(_, m)| m).collect();
    out.sort_by(|a, b| a.mount.cmp(&b.mount));
    out
}

/// Werte von `statvfs` in Bytes bzw. Inodes.
#[derive(Debug, Clone, Copy, Default)]
pub struct Space {
    pub size: u64,
    pub used: u64,
    pub avail: u64,
    pub reserved: u64,
    pub inodes: u64,
    pub inodes_free: u64,
    pub readonly: bool,
}

pub fn statvfs(path: &Path) -> Option<Space> {
    use std::os::unix::ffi::OsStrExt;
    let c = CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: gültiger, nullterminierter Pfad; `st` wird vollständig von statvfs gefüllt.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let f = st.f_frsize as u64;
    let size = st.f_blocks as u64 * f;
    let free = st.f_bfree as u64 * f;
    let avail = (st.f_bavail as u64 * f).min(free);
    Some(Space {
        size,
        used: size.saturating_sub(free),
        avail,
        reserved: free - avail,
        inodes: st.f_files as u64,
        inodes_free: st.f_ffree as u64,
        readonly: st.f_flag as u64 & libc::ST_RDONLY != 0,
    })
}

/// Temperatur-Datei eines Blockgeräts (NVMe-Controller oder drivetemp).
fn find_temp(sys: &Path, name: &str) -> Option<PathBuf> {
    let block = sys.join("block").join(name);
    let mut dirs = vec![block.join("device"), block.join("device/hwmon")];
    if name.starts_with("nvme") {
        let ctrl = name.rfind('n').filter(|&i| i > 0).map_or(name, |i| &name[..i]); // nvme0n1 → nvme0
        dirs.push(sys.join("class/nvme").join(ctrl));
        dirs.push(block.join("device/device/hwmon"));
    }
    for d in dirs {
        for e in read::dir(&d) {
            if e.starts_with("hwmon") {
                let p = d.join(&e).join("temp1_input");
                if p.exists() {
                    return Some(p);
                }
            }
        }
    }
    None
}

struct Dev {
    name: String,
    /// Echtes Gerät (zählt in `system.io`), nicht dm/md.
    physical: bool,
    prev: Option<DiskStat>,
    temp_path: Option<PathBuf>,
    info: DiskInfo,
    io: ChartHandle,
    ops: ChartHandle,
    wait: ChartHandle,
    util: ChartHandle,
    queue: ChartHandle,
    temp: Option<ChartHandle>,
    seen: bool,
}

struct Fs {
    space: ChartHandle,
    inodes: Option<ChartHandle>,
    seen: bool,
}

pub struct Disks {
    proc: PathBuf,
    sys: PathBuf,
    root: PathBuf,
    buf: String,
    devs: Vec<Dev>,
    fs: HashMap<String, Fs>,
    io: ChartHandle,
    scanned: i64,
}

impl Disks {
    pub fn new(core: &Core) -> Self {
        Self {
            proc: core.cfg.proc.clone(),
            sys: core.cfg.sys.clone(),
            root: core.cfg.root.clone(),
            buf: String::with_capacity(8 * 1024),
            devs: Vec::new(),
            fs: HashMap::new(),
            io: core.chart(ChartDef::tpl(&SYSTEM_IO)),
            scanned: 0,
        }
    }

    /// Geräte aus `/sys/block` (ganze Datenträger; Partitionen stehen darunter).
    fn scan(&mut self, core: &Core) {
        let block = self.sys.join("block");
        let mut names = read::dir(&block);
        names.retain(|n| !skip_device(n) && read::int(block.join(n).join("size")).unwrap_or(0) > 0);
        for d in &mut self.devs {
            d.seen = names.contains(&d.name);
        }
        for d in self.devs.iter().filter(|d| !d.seen) {
            for id in [&d.io, &d.ops, &d.wait, &d.util, &d.queue] {
                core.registry.retire(&id.id);
            }
            if let Some(t) = &d.temp {
                core.registry.retire(&t.id);
            }
        }
        self.devs.retain(|d| d.seen);
        for name in names {
            if self.devs.iter().any(|d| d.name == name) {
                continue;
            }
            let dir = block.join(&name);
            let dev = dir.join("device");
            let physical = dev.exists();
            let link = std::fs::read_link(&dir).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
            let rotational = read::int(dir.join("queue/rotational")) == Some(1);
            let kind = if name.starts_with("nvme") {
                "nvme"
            } else if name.starts_with("mmcblk") {
                "mmc"
            } else if link.contains("/usb") {
                "usb"
            } else if name.starts_with("vd") || name.starts_with("xvd") || link.contains("/virtio") || !physical {
                "virtual"
            } else if rotational {
                "hdd"
            } else {
                "ssd"
            };
            let text = |rel: &str| read::text(dev.join(rel)).unwrap_or_default();
            let mut model = text("model");
            if model.is_empty() {
                model = read::text(dir.join("dm/name")).or_else(|| read::text(dev.join("name"))).unwrap_or_default();
            }
            let mut partitions: Vec<String> = read::dir(&dir).into_iter().filter(|p| p.starts_with(&name) && p != &name).collect();
            partitions.sort_by(|a, b| read::natural(a, b));
            let info = DiskInfo {
                name: name.clone(),
                model,
                serial: text("serial"),
                kind: kind.to_string(),
                size: read::int(dir.join("size")).unwrap_or(0).max(0) as u64 * 512,
                partitions,
                ..Default::default()
            };
            let temp_path = find_temp(&self.sys, &name);
            let def = |tpl| ChartDef::tpl(tpl).instance(&name);
            self.devs.push(Dev {
                physical,
                prev: None,
                temp: temp_path.as_ref().map(|_| core.chart(def(&DISK_TEMP))),
                temp_path,
                info,
                io: core.chart(def(&DISK_IO)),
                ops: core.chart(def(&DISK_OPS)),
                wait: core.chart(def(&DISK_AWAIT)),
                util: core.chart(def(&DISK_UTIL)),
                queue: core.chart(def(&DISK_QUEUE)),
                seen: true,
                name,
            });
        }
    }

    fn filesystems(&mut self, t: &mut Tick<'_>) {
        if !read::into(&mut self.buf, &self.proc.join("1/mountinfo")) && !read::into(&mut self.buf, &self.proc.join("self/mountinfo")) {
            return;
        }
        let mounts = parse_mountinfo(&self.buf);
        for f in self.fs.values_mut() {
            f.seen = false;
        }
        let mut list = Vec::new();
        for m in mounts {
            let Some(s) = statvfs(&read::under(&self.root, &m.mount)) else { continue };
            if s.size == 0 {
                continue;
            }
            let entry = self.fs.entry(m.mount.clone()).or_insert_with(|| Fs {
                space: t.core.chart(ChartDef::tpl(&DISK_SPACE).instance(&m.mount).label("device", m.source.clone())),
                inodes: (s.inodes > 0).then(|| t.core.chart(ChartDef::tpl(&DISK_INODES).instance(&m.mount))),
                seen: true,
            });
            entry.seen = true;
            t.put(&entry.space, &[s.used as f64, s.avail as f64, s.reserved as f64]);
            if let Some(c) = &entry.inodes {
                t.put(c, &[(s.inodes - s.inodes_free.min(s.inodes)) as f64, s.inodes_free as f64]);
            }
            list.push(FsInfo {
                mount: m.mount,
                device: m.source,
                fstype: m.fstype,
                size: s.size,
                used: s.used,
                avail: s.avail,
                inodes: s.inodes,
                inodes_used: s.inodes - s.inodes_free.min(s.inodes),
                readonly: m.readonly,
            });
        }
        self.fs.retain(|_, f| {
            if !f.seen {
                t.core.registry.retire(&f.space.id);
                if let Some(i) = &f.inodes {
                    t.core.registry.retire(&i.id);
                }
            }
            f.seen
        });
        t.core.snapshot(snap::FILESYSTEMS, &list);
    }
}

impl Collector for Disks {
    fn name(&self) -> &'static str {
        "disk"
    }

    fn collect(&mut self, t: &mut Tick<'_>) -> anyhow::Result<()> {
        if self.scanned == 0 || t.ts - self.scanned >= 60 {
            self.scanned = t.ts;
            self.scan(t.core);
        }
        anyhow::ensure!(read::into(&mut self.buf, &self.proc.join("diskstats")), "/proc/diskstats nicht lesbar");
        let mut now: Vec<(usize, DiskStat)> = Vec::with_capacity(self.devs.len());
        let devs = &self.devs;
        parse_diskstats(&self.buf, |name, s| {
            if let Some(i) = devs.iter().position(|d| d.name == name) {
                now.push((i, s));
            }
        });
        let dt = t.dt.max(1e-3);
        let (mut sum_r, mut sum_w, mut any) = (0.0, 0.0, false);
        for (i, s) in now {
            let d = &mut self.devs[i];
            let Some(p) = d.prev.replace(s) else { continue };
            if s.reads < p.reads || s.writes < p.writes || s.io_ms < p.io_ms {
                continue;
            }
            let rd = ((s.read_sectors - p.read_sectors) as f64 * 512.0 / dt).round();
            let wr = ((s.write_sectors - p.write_sectors) as f64 * 512.0 / dt).round();
            let (dr, dw) = (s.reads - p.reads, s.writes - p.writes);
            let wait = |ms: u64, ops: u64| if ops == 0 { 0.0 } else { round(ms as f64 / ops as f64, 2) };
            let util = round(((s.io_ms - p.io_ms) as f64 / (dt * 1000.0) * 100.0).min(100.0), 2);
            t.put(&d.io, &[rd, wr]);
            t.put(&d.ops, &[(dr as f64 / dt).round(), (dw as f64 / dt).round()]);
            t.put(&d.wait, &[wait(s.read_ms.saturating_sub(p.read_ms), dr), wait(s.write_ms.saturating_sub(p.write_ms), dw)]);
            t.put(&d.util, &[util]);
            t.put(&d.queue, &[round(s.weighted_ms.saturating_sub(p.weighted_ms) as f64 / (dt * 1000.0), 2)]);
            (d.info.read, d.info.write, d.info.util) = (rd, wr, util);
            if d.physical {
                sum_r += rd;
                sum_w += wr;
                any = true;
            }
        }
        if any {
            t.put(&self.io, &[sum_r, sum_w]);
        }
        if t.due(10) {
            for d in &mut self.devs {
                if let (Some(p), Some(c)) = (&d.temp_path, &d.temp) {
                    let v = read::num::<f64>(&mut self.buf, p).map(|mc| round(mc / 1000.0, 1));
                    d.info.temp = v;
                    if let Some(v) = v {
                        t.put(c, &[v]);
                    }
                }
            }
            self.filesystems(t);
        }
        if t.due(5) {
            let list: Vec<DiskInfo> = self.devs.iter().map(|d| d.info.clone()).collect();
            t.core.snapshot(snap::DISKS, &list);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diskstats() {
        let text = " 259       0 nvme0n1 1000 10 80000 500 2000 20 160000 900 0 1200 1500 0 0 0 0
 259       1 nvme0n1p1 10 0 80 5 0 0 0 0 0 5 5
   7       0 loop0 5 0 10 1 0 0 0 0 0 1 1
";
        let mut got = Vec::new();
        parse_diskstats(text, |n, s| got.push((n.to_string(), s)));
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].1, DiskStat { reads: 1000, read_sectors: 80000, read_ms: 500, writes: 2000, write_sectors: 160000, write_ms: 900, io_ms: 1200, weighted_ms: 1500 });
        assert!(skip_device("loop0") && skip_device("zram0") && skip_device("ram1") && !skip_device("sda") && !skip_device("nvme0n1"));
    }

    #[test]
    fn mountinfo_filters() {
        let text = r"22 1 259:2 / / rw,relatime shared:1 - ext4 /dev/nvme0n1p2 rw
23 22 0:21 / /proc rw - proc proc rw
24 22 259:1 / /boot/efi rw - vfat /dev/nvme0n1p1 rw
25 22 0:30 / /run rw - tmpfs tmpfs rw
26 22 259:2 /home/umbrel/data /mnt/bind rw - ext4 /dev/nvme0n1p2 rw
27 22 0:50 / /var/lib/docker/overlay2/abc/merged rw - overlay overlay rw
28 22 8:1 / /mnt/my\040disk rw - ext4 /dev/sda1 rw
29 22 0:60 / /Users rw - virtiofs mount0 rw
30 22 0:70 /@ /data rw - btrfs /dev/sdb1 rw
31 22 0:70 /@home /data/home rw - btrfs /dev/sdb1 rw
";
        let m = parse_mountinfo(text);
        let mounts: Vec<&str> = m.iter().map(|m| m.mount.as_str()).collect();
        assert_eq!(mounts, ["/", "/boot/efi", "/data", "/mnt/my disk"]);
        assert_eq!(m[0].source, "/dev/nvme0n1p2");
        assert_eq!(m[3].fstype, "ext4");
        assert!(!m[0].readonly);
    }

    #[test]
    fn statvfs_works() {
        let s = statvfs(Path::new("/")).unwrap();
        assert!(s.size > 0 && s.used + s.avail + s.reserved == s.size);
    }
}
