//! Mem math mirroring btop_collect.cpp:1252-1281, :1385-1390, :1210-1226.
use crate::types::DiskInfo;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VmDerived {
    pub used: u64,
    pub avail: u64,
    pub cached: u64,
    pub free: u64,
}

/// active+wire=used, external=cached. Mirrors cpp:1252-1255.
pub fn vm_stats(
    active: u64,
    wired: u64,
    free_pages: u64,
    external: u64,
    page: u64,
    total: u64,
) -> VmDerived {
    let used = (active + wired).saturating_mul(page);
    VmDerived {
        used,
        avail: total.saturating_sub(used),
        cached: external.saturating_mul(page),
        free: free_pages.saturating_mul(page),
    }
}

/// round(stat*100/total). Mirrors cpp:1270,1279.
pub fn mem_percent(stat: u64, total: u64) -> i64 {
    if total == 0 {
        return 0;
    }
    (stat as f64 * 100.0 / total as f64).round() as i64
}

/// Mirrors cpp:1385-1390.
pub fn disk_usage(blocks: u64, bfree: u64, frsize: u64) -> DiskInfo {
    let total = blocks.saturating_mul(frsize);
    let free = bfree.saturating_mul(frsize);
    let used = total.saturating_sub(free);
    let used_percent = mem_percent(used, total);
    DiskInfo {
        total,
        used,
        free,
        used_percent,
    }
}

/// max(0, new-old). Mirrors cpp:1210,1218.
pub fn io_delta(new_bytes: u64, old_bytes: u64) -> u64 {
    new_bytes.saturating_sub(old_bytes)
}

/// clamp(round((r+w)/1MiB),0,100). Mirrors cpp:1226.
pub fn io_activity(read_delta: u64, write_delta: u64) -> i64 {
    const MIB: f64 = 1_048_576.0;
    (((read_delta.saturating_add(write_delta)) as f64 / MIB).round() as i64).clamp(0, 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vm_stats_derive_used_avail() {
        // page=4096; active=100, wired=50, free=200, external=30, total=2_000_000:
        // used=(100+50)*4096=614400; cached=30*4096=122880; free=819200; avail=total-used.
        let s = vm_stats(100, 50, 200, 30, 4096, 2_000_000);
        assert_eq!(s.used, 614_400);
        assert_eq!(s.cached, 122_880);
        assert_eq!(s.free, 819_200);
        assert_eq!(s.avail, 2_000_000 - 614_400);
    }

    #[test]
    fn percent_rounds_half_up() {
        assert_eq!(mem_percent(1, 200), 1); // 0.5 rounds to 1 (Rust round half away)
        assert_eq!(mem_percent(0, 0), 0); // guard, C++ swap_total==0 impossible in practice
    }

    #[test]
    fn disk_usage_splits_total() {
        let d = disk_usage(1000, 100, 10); // blocks, bfree, frsize
        assert_eq!(
            (d.total, d.free, d.used, d.used_percent),
            (10_000, 1_000, 9_000, 90)
        );
    }

    #[test]
    fn io_delta_never_negative() {
        assert_eq!(io_delta(500, 700), 0); // counter reset/rollover
        assert_eq!(io_delta(700, 500), 200);
    }

    #[test]
    fn io_activity_clamps() {
        assert_eq!(io_activity(0, 0), 0);
        assert_eq!(io_activity(u64::MAX, u64::MAX), 100);
    }
}
