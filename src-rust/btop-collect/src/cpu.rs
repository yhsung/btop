//! Cpu math mirroring btop_collect.cpp:1063-1100 (percent), :868-891 (temps).
use std::collections::VecDeque;

fn clamp_pct(v: i64) -> i64 {
    v.clamp(0, 100)
}

/// One core: returns (percent, new_totals, new_idles).
/// Mirrors cpp:1063-1068. Guard `calc_tot <= 0 → 0` is a deliberate safe
/// deviation (C++ would divide by zero; totals always grow in practice).
pub fn update_core(old_tot: i64, old_idle: i64, ticks: [u64; 4]) -> (i64, i64, i64) {
    let tot = ticks.iter().sum::<u64>() as i64;
    let idle = ticks[3] as i64;
    let calc_tot = (tot - old_tot).max(0);
    let calc_idle = (idle - old_idle).max(0);
    let pct = if calc_tot <= 0 {
        0
    } else {
        clamp_pct(((calc_tot - calc_idle) as f64 * 100.0 / calc_tot as f64).round() as i64)
    };
    (pct, tot, idle)
}

/// Global total. Mirrors cpp:1079-1097 (`max(1ll, …)` guards kept verbatim).
pub fn update_total(old_tot: i64, old_idle: i64, new_tot: i64, new_idle: i64) -> i64 {
    let calc_tot = (new_tot - old_tot).max(1);
    let calc_idle = (new_idle - old_idle).max(1);
    clamp_pct(((calc_tot - calc_idle) as f64 * 100.0 / calc_tot as f64).round() as i64)
}

/// sp78 fixed-point decode. Mirrors smc.cpp:80-81
/// (`bytes[0]*256 + (u8)bytes[1]`, `/256.0`, truncates via cast).
pub fn sp78_decode(hi: u8, lo: u8) -> i64 {
    ((hi as i32 * 256 + lo as i32) as f64 / 256.0) as i64
}

/// Sensor interleave index. Mirrors cpp:880.
pub fn sensor_index(core: usize, n_cores: usize, n_sensors: usize) -> usize {
    core * n_sensors / n_cores
}

/// Push + trim history to `cap` (core cap 40 per cpp:1071; temp cap 20 per
/// cpp:870; cpu fields cap width*2 passed by caller per cpp:1088,1100).
pub fn push_trimmed(h: &mut VecDeque<i64>, v: i64, cap: usize) {
    h.push_back(v);
    while h.len() > cap {
        h.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_percent_60_on_100_total_40_idle() {
        // prev_tot=1000, prev_idle=800; cur=[150,10,100,840] sums to 1100, idle 840.
        let (pct, tot, idle) = update_core(1000, 800, [150, 10, 100, 840]);
        assert_eq!((pct, tot, idle), (60, 1100, 840));
    }

    #[test]
    fn core_percent_zero_when_no_progress() {
        let (pct, _, _) = update_core(1000, 800, [100, 0, 100, 800]);
        assert_eq!(pct, 0);
    }

    #[test]
    fn total_percent_uses_max1_guard() {
        // global 5000->5100 (d=100), idle 4000->4040 (d=40) => 60.
        assert_eq!(update_total(5000, 4000, 5100, 4040), 60);
        // no progress at all: max(1,...) guards divide-by-zero, idle delta clamps.
        assert_eq!(update_total(5000, 4000, 5000, 4000), 0);
    }

    #[test]
    fn sp78_truncates_fraction() {
        assert_eq!(sp78_decode(0x1E, 0x00), 30);
        assert_eq!(sp78_decode(0x1E, 0x80), 30); // 30.5 truncates, smc.cpp:80-81
    }

    #[test]
    fn temp_interleave_maps_cores_to_sensors() {
        // cpp:880: sensor_index = core * n_sensors / n_cores
        assert_eq!(sensor_index(0, 6, 3), 0);
        assert_eq!(sensor_index(1, 6, 3), 0);
        assert_eq!(sensor_index(2, 6, 3), 1);
        assert_eq!(sensor_index(5, 6, 3), 2);
    }

    #[test]
    fn push_trimmed_caps_history() {
        let mut h: std::collections::VecDeque<i64> = [1, 2, 3].into_iter().collect();
        push_trimmed(&mut h, 4, 3);
        assert_eq!(h.iter().copied().collect::<Vec<_>>(), vec![2, 3, 4]);
    }
}
