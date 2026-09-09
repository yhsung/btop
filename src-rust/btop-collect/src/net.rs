//! Net math mirroring src/osx/btop_collect.cpp:1551-1563 (same block in src/linux).

/// Update one direction counter.
/// Returns `(speed_Bps, total_bytes, rollover, new_offset)` for `(val, last, rollover, offset, dt_ms)` inputs.
/// `dt_ms == 0` yields speed 0 (C++ would divide by zero).
/// Caller-owned offset lifecycle per cpp:1561: `offset > val+rollover → 0` reset happens
/// BEFORE the total calc, and the caller must persist `new_offset` back into saved_stat.
/// DEFERRED divergence vs cpp:1555-1556: u64-overflow reset becomes saturating_add.
/// `as u64` saturation on extreme speed is intentional (C++ out-of-range is UB).
pub fn update_counter(
    val: u64,
    last: u64,
    rollover: u64,
    offset: u64,
    dt_ms: u64,
) -> (u64, u64, u64, u64) {
    let (delta, rollover) = if val < last {
        (val, rollover.saturating_add(last))
    } else {
        (val - last, rollover)
    };
    let speed = if dt_ms == 0 {
        0
    } else {
        (delta as f64 / (dt_ms as f64 / 1000.0)).round() as u64
    };
    let new_offset = if offset > val.saturating_add(rollover) {
        0
    } else {
        offset
    };
    let total = val.saturating_add(rollover).saturating_sub(new_offset);
    (speed, total, rollover, new_offset)
}

pub fn track_top(speed: u64, top: u64) -> u64 {
    speed.max(top)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_from_byte_delta_and_dt() {
        // val 1_000_000 -> 1_002_000 over 1000ms => 2000 B/s; total=no rollover.
        let (speed, total, roll, off) = update_counter(1_002_000, 1_000_000, 0, 0, 1000);
        assert_eq!((speed, total, roll, off), (2000, 1_002_000, 0, 0));
    }

    #[test]
    fn rollover_accumulates_on_counter_wrap() {
        // val drops (wrapped): rollover += last; delta restarts at val.
        let (speed, total, roll, off) = update_counter(100, 1_000_000, 0, 0, 1000);
        assert_eq!(roll, 1_000_000);
        assert_eq!(total, 100 + 1_000_000);
        assert_eq!(speed, 100);
        assert_eq!(off, 0);
    }

    #[test]
    fn zero_dt_gives_zero_speed() {
        let (speed, _, _, _) = update_counter(2000, 1000, 0, 0, 0);
        assert_eq!(speed, 0);
    }

    #[test]
    fn top_tracks_max() {
        assert_eq!(track_top(100, 200), 200);
        assert_eq!(track_top(300, 200), 300);
    }

    #[test]
    fn total_subtracts_offset() {
        let (speed, total, roll, off) = update_counter(1_002_000, 1_000_000, 0, 500, 1000);
        assert_eq!((speed, total, roll, off), (2000, 1_001_500, 0, 500));
    }

    #[test]
    fn offset_resets_when_above_val_plus_rollover() {
        // cpp:1561 — offset reset BEFORE total: 5000 > 1000+200 → offset 0, total 1200.
        let (speed, total, roll, off) = update_counter(1000, 500, 200, 5000, 1000);
        assert_eq!((speed, total, roll, off), (500, 1200, 200, 0));
    }
}
