//! Arbeitsspeicher: `/proc/meminfo` (Belegung wie `free`, Swap, verfügbar, zugesagt, Kernel, Dirty,
//! grosse Seiten) und `/proc/vmstat` (Paging, Swap-Aktivität, Seitenfehler, OOM).

use std::path::PathBuf;

use shared::catalog::system::*;

use super::read;
use crate::{
    collectors::{Collector, Tick},
    core::Core,
    registry::{ChartDef, ChartHandle},
    util::{Counter, nan},
};

/// Werte aus `/proc/meminfo` in Bytes.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct MemInfo {
    pub total: u64,
    pub free: u64,
    pub available: u64,
    pub buffers: u64,
    pub cached: u64,
    pub sreclaimable: u64,
    pub sunreclaim: u64,
    pub swap_total: u64,
    pub swap_free: u64,
    pub committed: u64,
    pub commit_limit: u64,
    pub kernel_stack: u64,
    pub page_tables: u64,
    pub vmalloc: u64,
    pub dirty: u64,
    pub writeback: u64,
    pub huge_total: u64,
    pub huge_free: u64,
    pub huge_size: u64,
}

impl MemInfo {
    pub fn parse(text: &str) -> Self {
        let mut m = Self::default();
        for (k, v) in read::kv(text) {
            let b = v * 1024;
            match k {
                "MemTotal" => m.total = b,
                "MemFree" => m.free = b,
                "MemAvailable" => m.available = b,
                "Buffers" => m.buffers = b,
                "Cached" => m.cached = b,
                "SReclaimable" => m.sreclaimable = b,
                "SUnreclaim" => m.sunreclaim = b,
                "SwapTotal" => m.swap_total = b,
                "SwapFree" => m.swap_free = b,
                "Committed_AS" => m.committed = b,
                "CommitLimit" => m.commit_limit = b,
                "KernelStack" => m.kernel_stack = b,
                "PageTables" => m.page_tables = b,
                "VmallocUsed" => m.vmalloc = b,
                "Dirty" => m.dirty = b,
                "Writeback" => m.writeback = b,
                "HugePages_Total" => m.huge_total = v,
                "HugePages_Free" => m.huge_free = v,
                "Hugepagesize" => m.huge_size = b,
                _ => {}
            }
        }
        m
    }

    /// Belegt, Puffer, Cache, frei – wie `free` (Cache inkl. freigebbarem Slab).
    pub fn ram(&self) -> [f64; 4] {
        let cache = self.cached + self.sreclaimable;
        let used = self.total.saturating_sub(self.free + self.buffers + cache);
        [used as f64, self.buffers as f64, cache as f64, self.free as f64]
    }
}

/// Zähler aus `/proc/vmstat`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct VmStat {
    pub pgpgin: u64,
    pub pgpgout: u64,
    pub pswpin: u64,
    pub pswpout: u64,
    pub pgfault: u64,
    pub pgmajfault: u64,
    pub oom_kill: Option<u64>,
}

impl VmStat {
    pub fn parse(text: &str) -> Self {
        let mut v = Self::default();
        for (k, n) in read::kv(text) {
            match k {
                "pgpgin" => v.pgpgin = n,
                "pgpgout" => v.pgpgout = n,
                "pswpin" => v.pswpin = n,
                "pswpout" => v.pswpout = n,
                "pgfault" => v.pgfault = n,
                "pgmajfault" => v.pgmajfault = n,
                "oom_kill" => v.oom_kill = Some(n),
                _ => {}
            }
        }
        v
    }
}

pub struct Memory {
    meminfo: PathBuf,
    vmstat: PathBuf,
    buf: String,
    page: f64,
    pgin: Counter,
    pgout: Counter,
    swin: Counter,
    swout: Counter,
    minor: Counter,
    major: Counter,
    oom: Counter,
    ram: ChartHandle,
    swap: Option<ChartHandle>,
    available: ChartHandle,
    committed: ChartHandle,
    kernel: ChartHandle,
    writeback: ChartHandle,
    hugepages: Option<ChartHandle>,
    paging: ChartHandle,
    swapio: ChartHandle,
    faults: ChartHandle,
    c_oom: ChartHandle,
}

impl Memory {
    pub fn new(core: &Core) -> Self {
        // SAFETY: sysconf ist ohne Nebenwirkungen.
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        Self {
            meminfo: core.cfg.proc.join("meminfo"),
            vmstat: core.cfg.proc.join("vmstat"),
            buf: String::with_capacity(8 * 1024),
            page: if page > 0 { page as f64 } else { 4096.0 },
            pgin: Counter::new(),
            pgout: Counter::new(),
            swin: Counter::new(),
            swout: Counter::new(),
            minor: Counter::new(),
            major: Counter::new(),
            oom: Counter::new(),
            ram: core.chart(ChartDef::tpl(&SYSTEM_RAM)),
            swap: None,
            available: core.chart(ChartDef::tpl(&MEM_AVAILABLE)),
            committed: core.chart(ChartDef::tpl(&MEM_COMMITTED)),
            kernel: core.chart(ChartDef::tpl(&MEM_KERNEL)),
            writeback: core.chart(ChartDef::tpl(&MEM_WRITEBACK)),
            hugepages: None,
            paging: core.chart(ChartDef::tpl(&MEM_PAGING)),
            swapio: core.chart(ChartDef::tpl(&MEM_SWAPIO)),
            faults: core.chart(ChartDef::tpl(&MEM_FAULTS)),
            c_oom: core.chart(ChartDef::tpl(&MEM_OOM)),
        }
    }
}

impl Collector for Memory {
    fn name(&self) -> &'static str {
        "memory"
    }

    fn collect(&mut self, t: &mut Tick<'_>) -> anyhow::Result<()> {
        anyhow::ensure!(read::into(&mut self.buf, &self.meminfo), "/proc/meminfo nicht lesbar");
        let m = MemInfo::parse(&self.buf);
        t.put(&self.ram, &m.ram());
        if m.swap_total > 0 {
            let c = self.swap.get_or_insert_with(|| t.core.chart(ChartDef::tpl(&SYSTEM_SWAP)));
            t.put(c, &[(m.swap_total - m.swap_free.min(m.swap_total)) as f64, m.swap_free as f64]);
        }
        t.put(&self.available, &[m.available as f64]);
        t.put(&self.writeback, &[m.dirty as f64, m.writeback as f64]);
        if t.due(5) {
            t.put(&self.committed, &[m.committed as f64, m.commit_limit as f64]);
            t.put(
                &self.kernel,
                &[m.sreclaimable as f64, m.sunreclaim as f64, m.kernel_stack as f64, m.page_tables as f64, m.vmalloc as f64],
            );
        }
        if t.due(10) && m.huge_total > 0 {
            let used = (m.huge_total - m.huge_free.min(m.huge_total)) * m.huge_size;
            let c = self.hugepages.get_or_insert_with(|| t.core.chart(ChartDef::tpl(&MEM_HUGEPAGES)));
            t.put(c, &[used as f64, (m.huge_free * m.huge_size) as f64]);
        }

        if read::into(&mut self.buf, &self.vmstat) {
            let v = VmStat::parse(&self.buf);
            let dt = t.dt;
            let r = |c: &mut Counter, n: u64, mult: f64| nan(c.rate(n, dt).map(|x| (x * mult).round()));
            // pgpgin/out zählen KiB, pswpin/out Seiten.
            t.put(&self.paging, &[r(&mut self.pgin, v.pgpgin, 1024.0), r(&mut self.pgout, v.pgpgout, 1024.0)]);
            t.put(&self.swapio, &[r(&mut self.swin, v.pswpin, self.page), r(&mut self.swout, v.pswpout, self.page)]);
            let minor = v.pgfault.saturating_sub(v.pgmajfault);
            t.put(&self.faults, &[r(&mut self.minor, minor, 1.0), r(&mut self.major, v.pgmajfault, 1.0)]);
            if let Some(o) = v.oom_kill {
                t.put(&self.c_oom, &[r(&mut self.oom, o, 1.0)]);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEMINFO: &str = "MemTotal:       16000000 kB
MemFree:         2000000 kB
MemAvailable:    9000000 kB
Buffers:          300000 kB
Cached:          5000000 kB
SwapCached:         1000 kB
SwapTotal:       4000000 kB
SwapFree:        3500000 kB
Dirty:              2048 kB
Writeback:             0 kB
SReclaimable:     700000 kB
SUnreclaim:       200000 kB
KernelStack:       20000 kB
PageTables:        60000 kB
CommitLimit:    12000000 kB
Committed_AS:   14000000 kB
VmallocUsed:       50000 kB
HugePages_Total:       4
HugePages_Free:        1
Hugepagesize:       2048 kB
";

    #[test]
    fn meminfo_like_free() {
        let m = MemInfo::parse(MEMINFO);
        let k = 1024.0;
        // used = 16000000 - 2000000 - 300000 - (5000000 + 700000) = 8000000 kB
        assert_eq!(m.ram(), [8_000_000.0 * k, 300_000.0 * k, 5_700_000.0 * k, 2_000_000.0 * k]);
        assert_eq!(m.available, 9_000_000 * 1024);
        assert_eq!((m.huge_total, m.huge_free, m.huge_size), (4, 1, 2048 * 1024));
        assert_eq!(m.committed, 14_000_000 * 1024);
    }

    #[test]
    fn vmstat() {
        let v = VmStat::parse("pgpgin 100\npgpgout 200\npswpin 3\npswpout 4\npgfault 1000\npgmajfault 10\noom_kill 2\n");
        assert_eq!((v.pgpgin, v.pgpgout, v.pswpin, v.pswpout, v.pgfault, v.pgmajfault, v.oom_kill), (100, 200, 3, 4, 1000, 10, Some(2)));
        assert_eq!(VmStat::parse("pgfault 1\n").oom_kill, None);
    }
}
