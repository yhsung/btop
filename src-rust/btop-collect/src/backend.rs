use crate::gpu::EnergyUnit;
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
    /// Returns (active, wired, free_pages, external, page_size).
    fn vm_raw(&mut self) -> Result<(u64, u64, u64, u64, u64), CollectError>;
    /// Returns (total, avail, used).
    fn swap_raw(&mut self) -> Result<(u64, u64, u64), CollectError>;
    /// Returns (blocks, bfree, frsize). `mount` is ignored by ReplayBackend (FIFO regardless).
    fn disk_raw(&mut self, mount: &str) -> Result<(u64, u64, u64), CollectError>;
    /// Returns (iface_name, ibytes, obytes) per interface.
    fn if_counters(&mut self) -> Result<Vec<(String, u64, u64)>, CollectError>;
    fn proc_list(&mut self) -> Result<Vec<ProcRaw>, CollectError>;
    /// AppleSi-only; Intel returns Unsupported. Returns (name, residency, freq_hz).
    /// AppleSi-only; Intel returns Unsupported.
    /// Returns (name, residency, freq_hz).
    fn gpu_residency(&mut self) -> Result<Vec<(String, u64, u64)>, CollectError>;
    /// Returns (raw_value, unit). AppleSi-only; Intel returns Unsupported.
    fn gpu_energy(&mut self) -> Result<(u64, EnergyUnit), CollectError>;
    /// Returns Celsius readings. AppleSi-only; Intel returns Unsupported.
    fn hid_temps(&mut self) -> Result<Vec<f64>, CollectError>;
}

#[derive(Debug, Clone, Default)]
pub struct ProcRaw {
    pub pid: u64,
    pub name: String,
    pub cpu_ticks: u64,
    pub mem_bytes: u64,
    pub threads: u64,
}

/// Deterministic replay source for tests. Queues drain FIFO in call order.
/// Empty-queue fallback per method: `cpu_ticks` → `Err(Unsupported)`,
/// `load_avg` → `Ok([0,0,0])`, `package_temp` → `Ok(None)`, `core_temps` → `Ok(vec![])`,
/// `vm_raw` → `Err(Unsupported)`, `swap_raw` → `Ok((0,0,0))`, `disk_raw` → `Ok((0,0,0))`,
/// `if_counters` → `Ok(vec![])`, `proc_list` → `Ok(vec![])`,
/// `gpu_residency` → `Ok(vec![])`, `gpu_energy` → `Ok((0, Nano))`, `hid_temps` → `Ok(vec![])`.
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
    pub if_counters_q: VecDeque<Vec<(String, u64, u64)>>,
    pub proc_list_q: VecDeque<Vec<ProcRaw>>,
    pub gpu_residency_q: VecDeque<Vec<(String, u64, u64)>>,
    pub gpu_energy_q: VecDeque<(u64, EnergyUnit)>,
    pub hid_temps_q: VecDeque<Vec<f64>>,
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
    fn if_counters(&mut self) -> Result<Vec<(String, u64, u64)>, CollectError> {
        Ok(self.if_counters_q.pop_front().unwrap_or_default())
    }
    fn proc_list(&mut self) -> Result<Vec<ProcRaw>, CollectError> {
        Ok(self.proc_list_q.pop_front().unwrap_or_default())
    }
    fn gpu_residency(&mut self) -> Result<Vec<(String, u64, u64)>, CollectError> {
        Ok(self.gpu_residency_q.pop_front().unwrap_or_default())
    }
    fn gpu_energy(&mut self) -> Result<(u64, EnergyUnit), CollectError> {
        Ok(self
            .gpu_energy_q
            .pop_front()
            .unwrap_or((0, EnergyUnit::Nano)))
    }
    fn hid_temps(&mut self) -> Result<Vec<f64>, CollectError> {
        Ok(self.hid_temps_q.pop_front().unwrap_or_default())
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
