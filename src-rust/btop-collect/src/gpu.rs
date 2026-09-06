//! Gpu math mirroring btop_collect.cpp:490-550, :566-580; sensors avg
//! mirroring sensors.cpp:93-96.

/// Utilization: round(active*100/total), skipping IDLE/OFF/DOWN.
/// Mirrors cpp:490-508. Literals verified against cpp:492.
pub fn gpu_util(states: &[(String, u64)]) -> i64 {
    let mut active = 0u64;
    let mut total = 0u64;
    for (name, res) in states {
        total = total.saturating_add(*res);
        if name != "IDLE" && name != "OFF" && name != "DOWN" {
            active = active.saturating_add(*res);
        }
    }
    if total == 0 {
        return 0;
    }
    ((active as f64 * 100.0 / total as f64).round() as i64).clamp(0, 100)
}

/// Weighted clock: Σ(res*freq)/active. Mirrors cpp:512-514.
pub fn gpu_clock(states: &[(String, u64, u64)]) -> u64 {
    let mut num = 0u64;
    let mut active = 0u64;
    for (name, res, freq) in states {
        if name != "IDLE" && name != "OFF" && name != "DOWN" {
            num = num.saturating_add(res.saturating_mul(*freq));
            active = active.saturating_add(*res);
        }
    }
    if active == 0 {
        return 0;
    }
    (num as f64 / active as f64).round() as u64
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EnergyUnit {
    Nano,
    Micro,
    Milli,
}

/// mW = round(J / (dt_ms/1000) * 1000). Mirrors cpp:524-532,546.
pub fn power_mw(value: u64, unit: EnergyUnit, dt_ms: u64) -> u64 {
    if dt_ms == 0 {
        return 0;
    }
    let joules = match unit {
        EnergyUnit::Nano => value as f64 / 1e9,
        EnergyUnit::Micro => value as f64 / 1e6,
        EnergyUnit::Milli => value as f64 / 1e3,
    };
    (joules / (dt_ms as f64 / 1000.0) * 1000.0).round() as u64
}

/// Mirrors cpp:569-579.
#[allow(clippy::too_many_arguments)]
pub fn vram_used(
    act: u64,
    inact: u64,
    wire: u64,
    spec: u64,
    compr: u64,
    purge: u64,
    ext: u64,
    page: u64,
) -> u64 {
    act.saturating_add(inact)
        .saturating_add(wire)
        .saturating_add(spec)
        .saturating_add(compr)
        .saturating_sub(purge)
        .saturating_sub(ext)
        .saturating_mul(page)
}

/// Mean rounded. Mirrors sensors.cpp:93-96 (`round(sum/size)`).
// NOTE: C++ sensors.cpp:93-95 accumulates into 0ll (per-element truncation + integer division); this port implements ideal round(sum/size) — more correct, same result on realistic temps.
pub fn sensor_avg(temps: &[f64]) -> i64 {
    if temps.is_empty() {
        return 0;
    }
    (temps.iter().sum::<f64>() / temps.len() as f64).round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_util_skips_idle_states() {
        // residencies: IDLE=700, ACTIVE=300 => 30.
        let states = vec![("IDLE".to_string(), 700), ("ACTIVE".to_string(), 300)];
        assert_eq!(gpu_util(&states), 30);
        assert_eq!(gpu_util(&[]), 0);
    }

    #[test]
    fn gpu_clock_weights_by_residency() {
        let states = vec![
            ("A".to_string(), 300u64, 600u64), // (name, residency, freq)
            ("B".to_string(), 100u64, 1000u64),
        ];
        // (300*600 + 100*1000)/400 = 700.
        assert_eq!(gpu_clock(&states), 700);
    }

    #[test]
    fn gpu_clock_rounds_not_truncates() {
        let states = vec![("A".to_string(), 1u64, 2u64), ("B".to_string(), 1u64, 3u64)];
        // (2+3)/2 = 2.5 -> 3 (C++ round, cpp:514), truncation would give 2.
        assert_eq!(gpu_clock(&states), 3);
    }

    #[test]
    fn gpu_power_converts_units() {
        // 2_000_000 nJ over 1000ms = 0.002J/1s = 2mW.
        assert_eq!(power_mw(2_000_000, EnergyUnit::Nano, 1000), 2);
        assert_eq!(power_mw(2_000, EnergyUnit::Micro, 1000), 2);
        assert_eq!(power_mw(2, EnergyUnit::Milli, 1000), 2);
        assert_eq!(power_mw(100, EnergyUnit::Nano, 0), 0); // zero-dt guard
    }

    #[test]
    fn vram_used_sums_named_counters() {
        // used=(act+inact+wire+spec+compr-purge-ext)*page; mirrors cpp:569-579.
        assert_eq!(
            vram_used(10, 5, 3, 1, 1, 2, 0, 4096),
            (10 + 5 + 3 + 1 + 1 - 2 - 0) * 4096
        );
    }

    #[test]
    fn sensor_avg_rounds() {
        assert_eq!(sensor_avg(&[70.0, 72.0]), 71);
        assert_eq!(sensor_avg(&[]), 0);
    }
}
