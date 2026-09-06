#![allow(dead_code)] // M2g removes this if clippy stays clean; see plan header.
use crate::types::CollectError;
use std::collections::VecDeque;

/// Raw per-core ticks: [user, nice, system, idle]. Mirrors
/// `processor_cpu_load_info_data_t.cpu_ticks[4]` (btop_collect.cpp:1045-1048).
pub type CpuTicks = Vec<[u64; 4]>;

/// Source of raw OS numbers. Pure logic never touches the OS directly.
/// Grows additively: Tasks 3–6 append mem/net/proc/gpu methods here.
pub trait MacOsBackend {
    fn cpu_ticks(&mut self) -> Result<CpuTicks, CollectError>;
    fn load_avg(&mut self) -> Result<[f64; 3], CollectError>;
    fn package_temp(&mut self) -> Result<Option<i64>, CollectError>;
    fn core_temps(&mut self) -> Result<Vec<i64>, CollectError>;
    fn vm_raw(&mut self) -> Result<(u64, u64, u64, u64, u64), CollectError>;
    fn swap_raw(&mut self) -> Result<(u64, u64, u64), CollectError>;
    fn disk_raw(&mut self, mount: &str) -> Result<(u64, u64, u64), CollectError>;
}

/// Deterministic replay source for tests. Queues drain FIFO in call order.
/// Empty-queue fallback per method: `cpu_ticks` → `Err(Unsupported)`,
/// `load_avg` → `Ok([0,0,0])`, `package_temp` → `Ok(None)`, `core_temps` → `Ok(vec![])`,
/// `vm_raw` → `Err(Unsupported)`, `swap_raw` → `Ok((0,0,0))`, `disk_raw` → `Ok((0,0,0))`.
/// Tasks 4–6 append their methods here with the same documented fallback.
#[derive(Debug, Default)]
pub struct ReplayBackend {
    pub cpu_ticks_q: VecDeque<CpuTicks>,
    pub load_avg_q: VecDeque<[f64; 3]>,
    pub package_temp_q: VecDeque<Option<i64>>,
    pub core_temps_q: VecDeque<Vec<i64>>,
    pub vm_raw_q: VecDeque<(u64, u64, u64, u64, u64)>,
    pub swap_raw_q: VecDeque<(u64, u64, u64)>,
    pub disk_raw_q: VecDeque<(u64, u64, u64)>,
}

impl MacOsBackend for ReplayBackend {
    fn cpu_ticks(&mut self) -> Result<CpuTicks, CollectError> {
        self.cpu_ticks_q
            .pop_front()
            .ok_or(CollectError::Unsupported("cpu_ticks queue empty"))
    }
    fn load_avg(&mut self) -> Result<[f64; 3], CollectError> {
        Ok(self.load_avg_q.pop_front().unwrap_or([0.0, 0.0, 0.0]))
    }
    fn package_temp(&mut self) -> Result<Option<i64>, CollectError> {
        Ok(self.package_temp_q.pop_front().flatten())
    }
    fn core_temps(&mut self) -> Result<Vec<i64>, CollectError> {
        Ok(self.core_temps_q.pop_front().unwrap_or_default())
    }
    fn vm_raw(&mut self) -> Result<(u64, u64, u64, u64, u64), CollectError> {
        self.vm_raw_q
            .pop_front()
            .ok_or(CollectError::Unsupported("vm_raw queue empty"))
    }
    fn swap_raw(&mut self) -> Result<(u64, u64, u64), CollectError> {
        Ok(self.swap_raw_q.pop_front().unwrap_or((0, 0, 0)))
    }
    fn disk_raw(&mut self, _mount: &str) -> Result<(u64, u64, u64), CollectError> {
        Ok(self.disk_raw_q.pop_front().unwrap_or((0, 0, 0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_serves_queued_ticks_in_order() {
        let mut b = ReplayBackend {
            cpu_ticks_q: vec![vec![[150, 10, 100, 840]], vec![[160, 10, 100, 850]]]
                .into_iter()
                .collect(),
            load_avg_q: vec![[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]].into_iter().collect(),
            ..Default::default()
        };
        // FIFO: first call returns first sample, second returns second.
        assert_eq!(b.cpu_ticks().unwrap(), vec![[150, 10, 100, 840]]);
        assert_eq!(b.cpu_ticks().unwrap(), vec![[160, 10, 100, 850]]);
        assert!(b.cpu_ticks().is_err());
        assert_eq!(b.load_avg().unwrap(), [1.0, 2.0, 3.0]);
        assert_eq!(b.load_avg().unwrap(), [4.0, 5.0, 6.0]);
        assert_eq!(b.load_avg().unwrap(), [0.0, 0.0, 0.0]);
    }
}
