//! Output structs mirroring btop_shared.hpp collectors (AppleSi scope).
use std::collections::{HashMap, VecDeque};

#[derive(Debug, Clone, Default)]
pub struct CpuInfo {
    pub total: VecDeque<i64>,
    pub fields: HashMap<String, VecDeque<i64>>,
    pub cores: Vec<VecDeque<i64>>,
    pub temp: Vec<VecDeque<i64>>,
    pub temp_max: i64,
    pub load_avg: [f64; 3],
}

#[derive(Debug, Clone, Default)]
pub struct MemInfo {
    pub used: u64,
    pub avail: u64,
    pub cached: u64,
    pub free: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub swap_free: u64,
    pub percent_used: VecDeque<i64>,
    pub disks: HashMap<String, DiskInfo>,
}

#[derive(Debug, Clone, Default)]
pub struct DiskInfo {
    pub total: u64,
    pub used: u64,
    pub free: u64,
    pub used_percent: i64,
}

#[derive(Debug, Clone, Default)]
pub struct NetCounters {
    pub down_bytes: u64,
    pub up_bytes: u64,
}

#[derive(Debug, Clone, Default)]
pub struct NetInfo {
    pub down_speed: u64,
    pub up_speed: u64,
    pub down_total: u64,
    pub up_total: u64,
    pub down_top: u64,
    pub up_top: u64,
}

#[derive(Debug, Clone, Default)]
pub struct ProcEntry {
    pub pid: u64,
    pub name: String,
    pub cpu_p: f64,
    pub mem_bytes: u64,
    pub threads: u64,
}

#[derive(Debug, Clone, Default)]
pub struct GpuInfo {
    pub util: VecDeque<i64>,
    pub clock_mhz: u64,
    pub power_mw: u64,
    pub vram_used: u64,
    pub vram_total: u64,
    pub temp: VecDeque<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CollectError {
    Unsupported(&'static str),
    Syscall(&'static str, i32),
}
