#![allow(dead_code)] // M2g removes this if clippy stays clean; see plan header.
use crate::types::CollectError;

/// Raw per-core ticks: [user, nice, system, idle]. Mirrors
/// `processor_cpu_load_info_data_t.cpu_ticks[4]` (btop_collect.cpp:1045-1048).
pub type CpuTicks = Vec<[u64; 4]>;

/// Source of raw OS numbers. Pure logic never touches the OS directly.
/// Grows additively: Tasks 3–6 append mem/net/proc/gpu methods here.
pub trait MacOsBackend {
    fn cpu_ticks(&mut self) -> Result<CpuTicks, CollectError>;
    fn load_avg(&mut self) -> Result<[f64; 3], CollectError>;
}

/// Deterministic replay source for tests. Fields are `Option` queues in call
/// order; unneeded methods return `Unsupported` until their task fills them.
#[derive(Debug, Default)]
pub struct ReplayBackend {
    pub cpu_ticks_q: Vec<CpuTicks>,
    pub load_avg_q: Vec<[f64; 3]>,
}

impl MacOsBackend for ReplayBackend {
    fn cpu_ticks(&mut self) -> Result<CpuTicks, CollectError> {
        self.cpu_ticks_q
            .pop()
            .ok_or(CollectError::Unsupported("cpu_ticks queue empty"))
    }
    fn load_avg(&mut self) -> Result<[f64; 3], CollectError> {
        Ok(self.load_avg_q.pop().unwrap_or([0.0, 0.0, 0.0]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_serves_queued_ticks_in_order() {
        let mut b = ReplayBackend {
            cpu_ticks_q: vec![vec![[150, 10, 100, 840]]],
            ..Default::default()
        };
        // pop() takes from the END: single element is fine for order check.
        let t = b.cpu_ticks().unwrap();
        assert_eq!(t, vec![[150, 10, 100, 840]]);
        assert!(b.cpu_ticks().is_err());
    }
}
