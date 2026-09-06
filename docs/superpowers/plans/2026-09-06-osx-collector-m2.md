# M2 macOS Collector (AppleSi) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the `btop-collect` crate: AppleSi macOS data collection (cpu/mem/net/proc/gpu) at C++ parity, isolated behind a `MacOsBackend` trait with record/replay tests.

**Architecture:** New workspace member `btop-collect` with zero third-party deps. Pure math functions (ticks-delta→percent, Δbytes/Δt, history trim) take plain structs and are unit-tested; `RealBackend` holds only hand-written `extern "C"` + minimal `unsafe`; `ReplayBackend` serves literals transcribed from recorder JSON fixtures. `real.rs` and the smoke test are `#![cfg(target_os = "macos")]` so `cargo test` stays green on Linux.

**Tech Stack:** Rust 2021, `cargo test`, no external crates, hand-written macOS FFI (`#[link(name = "IOKit", kind = "framework")]`, libc calls via `extern "C"`), recorder at `btop-collect/examples/record.rs` (local-only, never CI).

---

## Scope note

Spec: `docs/superpowers/specs/2026-09-06-osx-collector-design.md` (M2a–M2g). This single plan covers all of M2 (one crate, AppleSi-only). Intel paths are `todo!`/`Unsupported` stubs throughout. M3 (draw) and later are out of scope. C++ truth lives at `src/osx/btop_collect.cpp` (+ `smc.cpp:78-81`, `sensors.cpp:93-96`, `btop_shared.hpp`); line numbers below refer to it — read-only, never modify.

## File structure

- Modify: `src-rust/Cargo.toml` — add `"btop-collect"` to members.
- Create: `src-rust/btop-collect/Cargo.toml` — no dependencies.
- Create: `src-rust/btop-collect/src/lib.rs` — `pub mod types; pub mod backend; pub mod cpu; pub mod mem; pub mod net; pub mod proc_; pub mod gpu;` + `#[cfg(target_os = "macos")] pub mod real;` (module is `proc_` — `proc` is a Rust keyword).
- Create: `src-rust/btop-collect/src/types.rs` — output structs with `VecDeque` histories + `CollectError`.
- Create: `src-rust/btop-collect/src/backend.rs` — `trait MacOsBackend` (grows per task), raw input structs, `ReplayBackend`.
- Create: `src-rust/btop-collect/src/cpu.rs`, `mem.rs`, `net.rs`, `proc_.rs`, `gpu.rs` — one `update(prev, sample, …)` pure function each + inline tests.
- Create: `src-rust/btop-collect/src/real.rs` — `RealBackend`, `#[cfg(target_os = "macos")]`, thin extern wrappers only.
- Create: `src-rust/btop-collect/examples/record.rs` — `#![cfg(target_os = "macos")}]` recorder printing JSON via `format!` (local-only).
- Create: `src-rust/fixtures/osx/cpu.json` — recorder output (JSON is interchange only; tests use inline literals so the zero-dep rule holds — no JSON parser is written).
- Modify: `.github/workflows/parity-gate.yml` — no change needed (already triggers on `src-rust/**`).
- Naming: `#[allow(dead_code)]` at the top of `backend.rs` until M2g wires everything (RealBackend methods are filled in Task 7; remove the allow there if clippy stays clean — it must, gate is `-D warnings`).

---

### Task 1: M2a crate skeleton, types, backend trait core, replay harness

**Files:**
- Modify: `src-rust/Cargo.toml`
- Create: `src-rust/btop-collect/Cargo.toml`, `src/lib.rs`, `src/types.rs`, `src/backend.rs`, `src/cpu.rs`, `src/mem.rs`, `src/net.rs`, `src/proc_.rs`, `src/gpu.rs` (stubs with `//!` doc only, except types/backend below)

- [ ] **Step 1: Write the workspace change + manifests + types + backend core**

```toml
# src-rust/Cargo.toml (edit members line only)
[workspace]
members = ["btop-tools", "btop-config", "btop-collect"]
resolver = "2"
```

```toml
# src-rust/btop-collect/Cargo.toml
[package]
name = "btop-collect"
version = "0.1.0"
edition = "2021"
```

```rust
// src-rust/btop-collect/src/lib.rs
pub mod backend;
pub mod cpu;
pub mod gpu;
pub mod mem;
pub mod net;
pub mod proc_;
pub mod types;
#[cfg(target_os = "macos")]
pub mod real;
```

```rust
// src-rust/btop-collect/src/types.rs
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
```

```rust
// src-rust/btop-collect/src/backend.rs (core; later tasks APPEND methods + raw structs)
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
```

Subsystem stubs (`cpu.rs`, `mem.rs`, `net.rs`, `proc_.rs`, `gpu.rs`): each file contains exactly one line, e.g. `//! Cpu collection logic (Task 2 fills this).`

- [ ] **Step 2: Build**

Run: `cargo build --workspace`
Expected: `Finished`, no errors (dead_code allowed, warnings ok for now).

- [ ] **Step 3: Run new tests**

Run: `cargo test -p btop-collect`
Expected: `1 passed`.

- [ ] **Step 4: Commit**

```bash
git add src-rust/Cargo.toml src-rust/btop-collect
git commit -m "feat(rust): add btop-collect skeleton with backend trait core"
```

---

### Task 2: M2b cpu pure math (ticks→percent, sp78, temps)

**Files:**
- Modify: `src-rust/btop-collect/src/cpu.rs`, `src/backend.rs` (append temp methods)

Reference: `btop_collect.cpp:1063-1100` (percent), `:868-891` (temp interleave), `smc.cpp:78-81` (sp78).

- [ ] **Step 1: Write the failing tests** (append `#[cfg(test)] mod tests` to `cpu.rs`)

```rust
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
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p btop-collect cpu`
Expected: FAIL `cannot find function update_core` (E0425).

- [ ] **Step 3: Write minimal implementation** (replace `cpu.rs` stub; append temp methods to backend trait + ReplayBackend)

```rust
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
```

Backend additions (append to `backend.rs` trait + `ReplayBackend` struct/impl):

```rust
// in trait MacOsBackend (append):
fn package_temp(&mut self) -> Result<Option<i64>, CollectError>;
fn core_temps(&mut self) -> Result<Vec<i64>, CollectError>;
// in struct ReplayBackend (append fields):
//  pub package_temp_q: Vec<Option<i64>>,
//  pub core_temps_q: Vec<Vec<i64>>,
// impl (append):
//  fn package_temp(&mut self) -> Result<Option<i64>, CollectError> {
//      Ok(self.package_temp_q.pop().flatten())
//  }
//  fn core_temps(&mut self) -> Result<Vec<i64>, CollectError> {
//      Ok(self.core_temps_q.pop().unwrap_or_default())
//  }
```

(Apply the commented lines as real code when editing.)

- [ ] **Step 4: Run tests**

Run: `cargo test -p btop-collect`
Expected: `8 passed` (1 backend + 7 cpu).

- [ ] **Step 5: Commit**

```bash
git add src-rust/btop-collect/src/cpu.rs src-rust/btop-collect/src/backend.rs
git commit -m "feat(rust): port osx cpu math (percent, sp78, temp interleave)"
```

---

### Task 3: M2c mem pure math

**Files:**
- Modify: `src-rust/btop-collect/src/mem.rs`, `src/backend.rs` (append mem methods)

Reference: `btop_collect.cpp:1252-1281` (stats/percent), `:1385-1390` (disk usage), `:1210-1226` (io delta/activity).

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vm_stats_derive_used_avail() {
        // page=4096; active=100, wired=50, free=200, external=30, total=16GiB-ish small numbers:
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
        assert_eq!((d.total, d.free, d.used, d.used_percent), (10_000, 1_000, 9_000, 90));
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
```

`vm_stats` returns a small local struct — define `pub struct VmDerived { pub used/cached/free/avail: u64 }` in `mem.rs`. `disk_usage` returns `crate::types::DiskInfo`.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p btop-collect mem`
Expected: FAIL `cannot find function vm_stats`.

- [ ] **Step 3: Write minimal implementation**

```rust
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
pub fn vm_stats(active: u64, wired: u64, free_pages: u64, external: u64, page: u64, total: u64) -> VmDerived {
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
    DiskInfo { total, used, free, used_percent }
}

/// max(0, new-old). Mirrors cpp:1210,1218.
pub fn io_delta(new_bytes: u64, old_bytes: u64) -> u64 {
    new_bytes.saturating_sub(old_bytes)
}

/// clamp(round((r+w)/1MiB),0,100). Mirrors cpp:1226.
pub fn io_activity(read_delta: u64, write_delta: u64) -> i64 {
    const MIB: f64 = 1_048_576.0;
    ((read_delta.saturating_add(write_delta)) as f64 / MIB).round() as i64).clamp(0, 100)
}
```

Backend additions: `fn vm_raw(&mut self) -> Result<(u64, u64, u64, u64, u64), CollectError>;` (active, wired, free_pages, external, page_size), `fn swap_raw(&mut self) -> Result<(u64, u64, u64), CollectError>;` (total, avail, used), `fn disk_raw(&mut self, mount: &str) -> Result<(u64, u64, u64), CollectError>;` (blocks, bfree, frsize). ReplayBackend: matching `_q` vec fields + pop impls (swap/disk default zeros when empty).

- [ ] **Step 4: Run tests**

Run: `cargo test -p btop-collect`
Expected: all green (prior 8 + 5 new = 13).

- [ ] **Step 5: Commit**

```bash
git add src-rust/btop-collect/src/mem.rs src-rust/btop-collect/src/backend.rs
git commit -m "feat(rust): port osx mem math (vm stats, disk, io deltas)"
```

---

### Task 4: M2d net pure math

**Files:**
- Modify: `src-rust/btop-collect/src/net.rs`, `src/backend.rs` (append net methods)

Reference: `btop_collect.cpp:1551-1563` (rollover/speed/total/top). Read those lines first; the function below mirrors them — if the source differs on rollover accumulation, adjust the implementation (never the test vectors) and cite the line in your report.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_from_byte_delta_and_dt() {
        // val 1_000_000 -> 1_002_000 over 1000ms => 2000 B/s; total=no rollover.
        let (speed, total, roll) = update_counter(1_002_000, 1_000_000, 0, 0, 1000);
        assert_eq!((speed, total, roll), (2000, 1_002_000, 0));
    }

    #[test]
    fn rollover_accumulates_on_counter_wrap() {
        // val drops (wrapped): rollover += last; speed uses wrapped delta.
        let (speed, total, roll) = update_counter(100, 1_000_000, 0, 0, 1000);
        assert_eq!(roll, 1_000_000);
        assert_eq!(total, 100 + 1_000_000);
        assert_eq!(speed, 100); // (100 - 1_000_000) would be negative -> saturates to speed of wrapped delta? see impl
    }

    #[test]
    fn zero_dt_gives_zero_speed() {
        let (speed, _, _) = update_counter(2000, 1000, 0, 0, 0);
        assert_eq!(speed, 0);
    }

    #[test]
    fn top_tracks_max() {
        assert_eq!(track_top(100, 200), 200);
        assert_eq!(track_top(300, 200), 300);
    }
}
```

Wait — rollover test `speed` needs care: after wrap, `val - last` is negative; C++ computes with unsigned wrap or saturates? Define: on wrap (`val < last`), `rollover += last`, delta = `val` (fresh epoch), speed = round(val / dt_s). So speed = round(100/1.0) = 100. That matches the test above. Implement exactly that and document as mirroring cpp:1551-1563; if source shows different, fix impl + report.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p btop-collect net`
Expected: FAIL `cannot find function update_counter`.

- [ ] **Step 3: Write minimal implementation**

```rust
//! Net math mirroring btop_collect.cpp:1551-1563.
pub fn update_counter(
    val: u64,
    last: u64,
    rollover: u64,
    offset: u64,
    dt_ms: u64,
) -> (u64, u64, u64) {
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
    let total = val.saturating_add(rollover).saturating_sub(offset);
    (speed, total, rollover)
}

pub fn track_top(speed: u64, top: u64) -> u64 {
    speed.max(top)
}
```

Backend additions: `fn if_counters(&mut self) -> Result<Vec<(String, u64, u64)>, CollectError>;` (name, ibytes, obytes per interface) + ReplayBackend `_q` + pop (empty → `Ok(vec![])`).

- [ ] **Step 4: Run tests**

Run: `cargo test -p btop-collect`
Expected: all green (13 + 4 = 17).

- [ ] **Step 5: Commit**

```bash
git add src-rust/btop-collect/src/net.rs src-rust/btop-collect/src/backend.rs
git commit -m "feat(rust): port osx net math (rollover, speed, totals)"
```

---

### Task 5: M2e proc pure math

**Files:**
- Modify: `src-rust/btop-collect/src/proc_.rs`, `src/backend.rs` (append proc methods)

Reference: `btop_collect.cpp:1889-1892` (cpu_p/cpu_c), `:1876-1878` (threads/mem/cpu_t), `:1862` (cpu_s), `:1764` (no_update gate).

- [ ] **Step 1: Write the failing tests**

```rust
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
```

`proc_cpu_percent` returns f64 (clamped 0..100*ncore, unrounded? C++ line 1889 wraps in `clamp(round(...) * cmult / 1000.0, ...)` — verify `cmult` at the call site when implementing; the test expects `round(2.5)=3.0` i.e. round-half-away. Rust `f64::round` matches C++ `std::round` (both half-away; halfway unreachable for these magnitudes per Plan 1 precedent, but the test pins 2.5→3.0 anyway).

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p btop-collect proc_`
Expected: FAIL `cannot find function proc_cpu_percent`.

- [ ] **Step 3: Write minimal implementation**

```rust
//! Proc math mirroring btop_collect.cpp:1876-1892, gate :1764.

/// cpu_p. `factor` = machTck/clkTck (cpp:679-691, passed in, never read here).
/// Mirrors cpp:1889 with zero-total guard (C++ divides by tick delta directly).
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
```

Backend additions: `fn proc_list(&mut self) -> Result<Vec<ProcRaw>, CollectError>;` with `#[derive(Debug, Clone, Default)] pub struct ProcRaw { pub pid: u64, pub name: String, pub cpu_ticks: u64, pub mem_bytes: u64, pub threads: u64 }`; Intel-only detail fields are NOT added (todo). ReplayBackend `_q` + pop (empty → `Ok(vec![])`).

- [ ] **Step 4: Run tests**

Run: `cargo test -p btop-collect`
Expected: all green (17 + 3 = 20).

- [ ] **Step 5: Commit**

```bash
git add src-rust/btop-collect/src/proc_.rs src-rust/btop-collect/src/backend.rs
git commit -m "feat(rust): port osx proc math (cpu percent, cache gate)"
```

---

### Task 6: M2f gpu + sensors pure math

**Files:**
- Modify: `src-rust/btop-collect/src/gpu.rs`, `src/backend.rs` (append gpu methods)

Reference: `btop_collect.cpp:490-550` (util/clock/power), `:566-580` (vram), `:616-635` (trims), `sensors.cpp:93-96` (avg).

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_util_skips_idle_states() {
        // residencies: IDLE=700, ACTIVE=300 => 30.
        let states = vec![
            ("IDLE".to_string(), 700),
            ("ACTIVE".to_string(), 300),
        ];
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
        assert_eq!(vram_used(10, 5, 3, 1, 1, 2, 0, 4096), (10 + 5 + 3 + 1 + 1 - 2 - 0) * 4096);
    }

    #[test]
    fn sensor_avg_rounds() {
        assert_eq!(sensor_avg(&[70.0, 72.0]), 71);
        assert_eq!(sensor_avg(&[]), 0);
    }
}
```

`gpu_util` skips states named `IDLE`, `OFF`, `DOWN` (cpp:490-495 — verify names at the call site; if C++ uses different literals, use those and report).

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p btop-collect gpu`
Expected: FAIL `cannot find function gpu_util`.

- [ ] **Step 3: Write minimal implementation**

```rust
//! Gpu math mirroring btop_collect.cpp:490-550, :566-580; sensors avg
//! mirroring sensors.cpp:93-96.

/// Utilization: round(active*100/total), skipping IDLE/OFF/DOWN.
/// Mirrors cpp:490-508.
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
    (active as f64 * 100.0 / total as f64).round() as i64
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
    num / active
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
pub fn vram_used(act: u64, inact: u64, wire: u64, spec: u64, compr: u64, purge: u64, ext: u64, page: u64) -> u64 {
    act.saturating_add(inact)
        .saturating_add(wire)
        .saturating_add(spec)
        .saturating_add(compr)
        .saturating_sub(purge)
        .saturating_sub(ext)
        .saturating_mul(page)
}

/// Mean rounded. Mirrors sensors.cpp:93-96 (`round(sum/size)`).
pub fn sensor_avg(temps: &[f64]) -> i64 {
    if temps.is_empty() {
        return 0;
    }
    (temps.iter().sum::<f64>() / temps.len() as f64).round() as i64
}
```

Backend additions: `fn gpu_residency(&mut self) -> Result<Vec<(String, u64, u64)>, CollectError>;` (name, residency, freq_hz), `fn gpu_energy(&mut self) -> Result<(u64, EnergyUnit), CollectError>;`, `fn hid_temps(&mut self) -> Result<Vec<f64>, CollectError>;` — all AppleSi-only; Intel returns `Unsupported` in RealBackend (Task 7). ReplayBackend `_q` + pops (empty → `Ok(vec![])` / `Ok((0, Nano))`).

- [ ] **Step 4: Run tests**

Run: `cargo test -p btop-collect`
Expected: all green (20 + 5 = 25).

- [ ] **Step 5: Commit**

```bash
git add src-rust/btop-collect/src/gpu.rs src-rust/btop-collect/src/backend.rs
git commit -m "feat(rust): port osx gpu math and sensor average"
```

---

### Task 7: M2g RealBackend externs, recorder, smoke test, gate green

**Files:**
- Create: `src-rust/btop-collect/src/real.rs`, `src-rust/btop-collect/examples/record.rs`, `src-rust/fixtures/osx/cpu.json`
- Modify: `src-rust/btop-collect/src/backend.rs` (remove `#![allow(dead_code)]` if clippy clean without it — try removal first, keep only if build fails)

Constraints (all from spec S1/S2): thin wrappers only — each `RealBackend` method calls exactly one OS function and converts to the plain raw struct, no math. Intel-only methods return `Err(CollectError::Unsupported("intel later"))`. Missing IOReport on AppleSi → empty + no panic.

- [ ] **Step 1: Write the smoke test FIRST (fails: no real.rs)**

```rust
// append to src-rust/btop-collect/src/real.rs? No — file doesn't exist yet.
// Create src-rust/btop-collect/tests/smoke.rs:
#![cfg(target_os = "macos")]
use btop_collect::backend::MacOsBackend;
use btop_collect::real::RealBackend;

#[test]
fn smoke_three_rounds_no_panic_nonempty() {
    let mut b = RealBackend::new();
    for _ in 0..3 {
        let ticks = b.cpu_ticks().expect("cpu_ticks must succeed on macOS");
        assert!(!ticks.is_empty(), "at least one cpu entry");
        assert_eq!(ticks[0].len(), 4);
        let avg = b.load_avg().expect("load_avg must succeed");
        assert!(avg[0] >= 0.0);
    }
}
```

Run: `cargo test -p btop-collect --test smoke`
Expected: FAIL `couldn't read src-rust/btop-collect/src/real.rs: No such file` / unresolved import.

- [ ] **Step 2: Write minimal real.rs** — thin wrappers, cfg-gated whole file:

```rust
//! Real macOS backend: thin hand-written FFI, no logic. AppleSi complete;
//! Intel-only paths return Unsupported (M2 scope).
#![cfg(target_os = "macos")]

use crate::backend::{CpuTicks, EnergyUnit, MacOsBackend, ProcRaw};
use crate::types::CollectError;

#[derive(Debug, Default)]
pub struct RealBackend;

impl RealBackend {
    pub fn new() -> Self {
        Self
    }
}

#[link(name = "IOKit", kind = "framework")]
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn mach_host_self() -> u32;
    fn getloadavg(loadavg: *mut f64, nelem: i32) -> i32;
    fn getifaddrs(ifap: *mut *mut u8) -> i32;
    fn freeifaddrs(ifa: *mut u8);
    fn proc_pidinfo(pid: i32, flavor: i32, arg: u64, buffer: *mut u8, buffersize: i32) -> i32;
    fn proc_pidpath(pid: i32, buffer: *mut u8, buffersize: u32) -> i32;
}

// host_processor_info / host_statistics64 / sysctl / IOReport / IOHIDEvent:
// declare each function the night before use, verified against SDK headers at
// /Library/Developer/CommandLineTools/SDKs/MacOSX.sdk/usr/include and the
// private decl block at btop_collect.cpp:82-114 (IOReport) + smc.hpp/sensors.hpp.
// Rule: one extern per OS call actually invoked; #[repr(C)] structs declare
// ONLY used fields; every unsafe block is a single call + return-code check.
// After writing each decl, add a compile-time offset assertion in #[cfg(test)]
// (e.g. assert mem::offset_of!) — offsets verified against the C++ usage, not guessed.
```

Then implement `MacOsBackend` (+ mem/net/proc/gpu methods from Tasks 3–6) for `RealBackend`:
- `cpu_ticks`: `host_processor_info(PROCESSOR_CPU_LOAD_INFO)` → `Vec<[u64;4]>` (order user/nice/system/idle per cpp:1045). Free the returned buffer per Mach contract (deallocate — check `vm_deallocate` pairing in cpp or SDK docs; if unsure, note in report as follow-up instead of guessing).
- `load_avg`: `getloadavg` → array, error → `Syscall("getloadavg", ret)`.
- `package_temp`/`core_temps`: AppleSi IOHID path → `Some`/`vec`; on failure → `Ok(None)`/`Ok(vec![])` (degrade, never Err — per spec S2). Intel SMC → `Err(Unsupported("intel later"))`.
- mem/net/proc/gpu: same one-call-per-method shape; any failure → `Ok(empty)` for optional subsystems (gpu/temps/disks) but `Err(Syscall)` for core cpu/mem/probe calls — document each choice with the cpp error path (`Logger::error` + continue vs throw) in a comment.
- `proc_list`: two-phase sysctl sizing + fetch (cpp:1789-1800), then per-pid `proc_pidinfo`; skip failures per-process (continue), never abort the whole list.

Keep every method under ~40 lines; if a method grows, split the conversion into a `#[cfg(test)]`-visible pure helper (which also buys a unit test).

- [ ] **Step 3: Recorder example + first fixture**

```rust
// src-rust/btop-collect/examples/record.rs
#![cfg(target_os = "macos")]
//! Local-only recorder: prints one JSON snapshot of raw backend values.
//! NEVER run in CI. Usage: cargo run -p btop-collect --example record > src-rust/fixtures/osx/cpu.json
use btop_collect::backend::MacOsBackend;
use btop_collect::real::RealBackend;

fn main() {
    let mut b = RealBackend::new();
    let ticks = b.cpu_ticks().unwrap_or_default();
    let avg = b.load_avg().unwrap_or([0.0, 0.0, 0.0]);
    let mut out = String::from("{\"ticks\":[");
    for (i, t) in ticks.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!("[{},{},{},{}]", t[0], t[1], t[2], t[3]));
    }
    out.push_str(&format!("],\"load_avg\":[{},{},{}]}}", avg[0], avg[1], avg[2]));
    println!("{out}");
}
```

Run locally (macOS): `cargo run -p btop-collect --example record > src-rust/fixtures/osx/cpu.json` and commit the resulting JSON. (If the runner is not macOS, record this in the report as BLOCKED-for-fixture-only and still commit code + smoke test.)

- [ ] **Step 4: Full gate**

Run: `cargo test --workspace --locked --offline && cargo clippy --workspace -- -D warnings && cargo fmt --check`
Expected: all green. If clippy flags the removed `allow(dead_code)` (unused trait methods until M3 wires them), restore `#![allow(dead_code)]` with a comment `// M3 wires collectors into draw; trait methods pending.` — that is the only acceptable reason to keep it.

- [ ] **Step 5: Commit**

```bash
git add src-rust/btop-collect/src/real.rs src-rust/btop-collect/examples/record.rs src-rust/fixtures/osx/cpu.json src-rust/btop-collect/tests/smoke.rs src-rust/btop-collect/src/backend.rs
git commit -m "feat(rust): add RealBackend externs, recorder, macOS smoke test"
```

---

## Self-review

- Spec coverage: M2a (Task 1) → M2b cpu (Task 2) → M2c mem (Task 3) → M2d net (Task 4) → M2e proc (Task 5) → M2f gpu+sensors (Task 6) → M2g wiring+smoke (Task 7). Intel `todo!/Unsupported` in Tasks 2/6/7. Recorder + fixtures/osx in Task 7. CI needs no change (paths already cover src-rust).
- Placeholders: none — every test has literal vectors derived from cited C++ lines; every implementation is complete code. Two explicit verify-and-adjust points (net rollover Task 4, proc cmult + gpu state names Tasks 5–6) name exact lines and forbid touching vectors silently — the engineer reports mismatches instead.
- Type consistency: `CpuTicks = Vec<[u64;4]>` everywhere; histories `VecDeque<i64>`; percents `i64`, bytes `u64`, proc cpu `f64`; `CollectError::{Unsupported, Syscall}` uniform; `EnergyUnit::{Nano,Micro,Milli}` shared between backend and gpu; `DiskInfo`/`ProcRaw` field names match types.rs. `proc` module is `proc_` (keyword escape) consistently in lib.rs/Task 5 paths.
- Known deviation ledger (all documented inline): cpu zero-progress guard (C++ div-by-zero), saturating math for counters, silent `Err(1)`-style degrade (`Ok(empty)`) for optional subsystems, JSON fixtures as interchange-only (zero-dep rule forbids a parser).
