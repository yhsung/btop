//! Proc math mirroring btop_collect.cpp:1876-1892, gate :1764.

/// cpu_p (PORT SCALE — see PARITY-DEBT).
/// `factor` = machTck/clkTck from the OS (cpp:679-691), passed in rather than read from globals.
/// PARITY-DEBT vs cpp:1889 (`round(A)*cmult/1000`, cmult per cpp:1758 `per_core ? coreCount : 1`):
/// this port computes `round(A*100)` instead. Wiring task MUST (a) plumb `cmult`/per_core,
/// (b) move to round-before-rescale, (c) update the 3.0 test vector. Zero-total guard stays.
pub fn proc_cpu_percent(delta_proc: u64, delta_total: u64, factor: f64, ncore: u64) -> f64 {
    if delta_total == 0 {
        return 0.0;
    }
    let raw = (delta_proc as f64 * factor) / delta_total as f64 * 100.0;
    raw.round().clamp(0.0, 100.0 * ncore as f64)
}

/// Cache gate. Mirrors cpp:1764 `no_update and not current_procs.empty()`.
pub fn should_use_cache(no_update: bool, procs_empty: bool) -> bool {
    no_update && !procs_empty
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proc_cpu_scales_by_tick_factor() {
        // delta_proc=200, delta_total=8000, factor=1.0, ncore=8 => 2.5 -> 3 (round half away).
        assert_eq!(proc_cpu_percent(200, 8000, 1.0, 8), 3.0);
    }

    #[test]
    fn proc_cpu_clamps_to_ncore_x_100() {
        assert_eq!(proc_cpu_percent(1_000_000, 1, 1.0, 8), 800.0);
        assert_eq!(proc_cpu_percent(0, 0, 1.0, 8), 0.0); // zero total guard
    }

    #[test]
    fn proc_cache_gate_semantics() {
        // Mirrors cpp:1764: no_update && !empty => cached (re-sort only).
        assert!(should_use_cache(true, false));
        assert!(!should_use_cache(true, true));
        assert!(!should_use_cache(false, false));
    }
}
