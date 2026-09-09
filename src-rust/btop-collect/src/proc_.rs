//! Proc math mirroring src/osx/btop_collect.cpp:1876-1892, gate :1764.

/// cpu_p.
/// `factor` = machTck/clkTck from the OS (cpp:679-691), passed in rather than read from globals.
/// Fidelity vs cpp:1889 (`round(A)*cmult/1000`, cmult per cpp:1758 `per_core ? coreCount : 1`):
/// this port computes `(A.round() * cmult as f64 / 1000.0).clamp(0.0, 100.0 * ncore as f64)`
/// where A = delta_proc*factor/delta_total — round-before-rescale, matching C++ `round` then
/// `* cmult / 1000.0`. Zero-total guard stays (C++ would divide doubles by zero; guard → 0.0).
pub fn proc_cpu_percent(
    delta_proc: u64,
    delta_total: u64,
    factor: f64,
    cmult: i64,
    ncore: u64,
) -> f64 {
    if delta_total == 0 {
        return 0.0;
    }
    let a = (delta_proc as f64 * factor) / delta_total as f64;
    (a.round() * cmult as f64 / 1000.0).clamp(0.0, 100.0 * ncore as f64)
}

/// Cache gate. Mirrors cpp:1764 `no_update and not current_procs.empty()`.
pub fn should_use_cache(no_update: bool, procs_empty: bool) -> bool {
    no_update && !procs_empty
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proc_cpu_round_before_rescale_with_cmult() {
        // A = 4000*1.0/8000 = 0.5 -> round half-away -> 1; 1*cmult/1000.
        assert_eq!(proc_cpu_percent(4000, 8000, 1.0, 1, 8), 0.001);
        assert_eq!(proc_cpu_percent(4000, 8000, 1.0, 8, 8), 0.008);
    }

    #[test]
    fn proc_cpu_clamps_to_ncore_x_100() {
        assert_eq!(proc_cpu_percent(1_000_000, 1, 1.0, 8, 8), 800.0);
        assert_eq!(proc_cpu_percent(0, 0, 1.0, 1, 8), 0.0); // zero total guard
    }

    #[test]
    fn proc_cache_gate_semantics() {
        // Mirrors cpp:1764: no_update && !empty => cached (re-sort only).
        assert!(should_use_cache(true, false));
        assert!(!should_use_cache(true, true));
        assert!(!should_use_cache(false, false));
    }
}
