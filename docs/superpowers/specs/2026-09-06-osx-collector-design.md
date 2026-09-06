# M2 macOS Collector (AppleSi) — Design (Route A replay harness)

Date: 2026-09-06
Status: approved S1–S3, pending spec review
Route: A — trait 隔離 + 錄製回放（零依賴、手寫 extern）

## 1. Background

`src/osx/btop_collect.cpp` (2067 lines) + `smc.{cpp,hpp}` + `sensors.{cpp,hpp}` fill
`btop_shared.hpp` structs via: Cpu `host_processor_info(PROCESSOR_CPU_LOAD_INFO)` ticks deltas,
`sysctlbyname` brand/freq/memsize, `getloadavg`, battery IOPowerSources; Mem
`host_statistics64(HOST_VM_INFO64)`, `sysctl VM_SWAPUSAGE`, `getmntinfo` + async `statvfs`,
IOMediaBSDClient IO deltas; Net `getifaddrs` + `NET_RT_IFLIST2` `if_msghdr2` byte counters with
Δ/Δt speed, rollover/offset; Proc `KERN_PROC_ALL` + `proc_pidinfo(PROC_PIDTASKINFO)` +
`proc_pidpath` + rusage, cpu% vs host totals; Gpu arm64-only `IOReport` residency/energy +
`pmgr voltage-states9` + `IOHIDEventSystemClient` temps; sensors AppleSi thermal + Intel SMC
(`TC0P/TCxC`, `sp78`) via `IOConnectCallStructMethod`. Linked: CoreFoundation, IOKit,
IOReport (arm64 only), Threads; libproc/Mach private symbols hand-declared.
`collect(no_update)` returns cached values except proc re-sorts/filters.

## 2. Decisions (user-confirmed)

- Zero third-party dependencies: stdlib + hand-written `extern "C"` only (Plan 1 rule extended).
- Apple Silicon complete, verified locally on M2 Max (arm64); Intel paths are `todo!("intel later")` stubs.
- GPU (IOReport, arm64-only) included in M2, not split out.
- Route A: `trait MacOsBackend` isolation + record/replay harness (deterministic, offline).

## 3. Architecture (S1 approved)

- New workspace member `btop-collect`: `types.rs`, `backend.rs`, `cpu.rs`, `mem.rs`,
  `net.rs`, `proc.rs`, `gpu.rs` — one responsibility each, independently testable.
- `types.rs`: Rust mirrors of shared structs (cpu_percent map, core_percent/temp vec-deques,
  mem stats/percent/disks, bandwidth/stat maps, proc vec + detail deques, gpu percent/clocks/power).
  History in `VecDeque`.
- `backend.rs`: `trait MacOsBackend` per-FFI-call methods; `RealBackend` = thin hand-written
  `extern "C"` + minimal `unsafe` (call → raw values, no logic); `#[repr(C)]` structs declare
  only used fields; `ReplayBackend` serves recorded fixtures.
- All math (ticks delta→percent, Δbytes/Δt, history trim to width*2, swap/used derivation) is
  pure functions over backend-returned plain structs — the testable core.

## 4. Data flow & degradation (S2 approved)

- Flow: `Collector::collect(backend, prev_state, no_update, dt) -> SubsystemInfo`.
  `no_update` + cache → return cache (proc re-sorts/filters as in C++); else backend raw →
  pure math → push + trim history. `dt` injected by caller (mockable clock), never hardcoded
  `mach_absolute_time`.
- Errors: each backend call `Result<T, CollectError>`; failure → empty fields + preserved
  history, never panic. Intel-only paths return `Unsupported` (TODO); missing IOReport on
  AppleSi → empty GPU + warning.
- Concurrency: C++ iokit/interface mutexes collapse into `Collector`-internal `Mutex<Cache>`;
  `std::async` sensor/statvfs fans out to caller-spawned work; collector itself stays
  synchronous and deterministic.

## 5. Testing — record/replay harness (S3 approved)

- `ReplayBackend` replays `src-rust/fixtures/osx/*.json` (two consecutive raw samples + fixed
  dt per subsystem) into pure functions; outputs asserted against expected JSON.
- Recorder: `btop-collect/examples/record.rs`, runs locally only (never in CI), captures real
  M2 Max values into fixtures. All later tests offline-deterministic.
- Gates: unit TDD (delta/percent/trim edges) → replay golden → local smoke (RealBackend 3
  rounds, no panic, non-empty fields; CI runs first two layers only, `--offline` retained).

## 6. Milestones (each mergeable)

- M2a: trait + types + harness skeleton.
- M2b: cpu. M2c: mem. M2d: net. M2e: proc. M2f: gpu + sensors. M2g: local full smoke.
- Intel paths `todo!` throughout.

## 7. Non-goals

- No Intel paths, no Linux/BSD collectors, no draw/input integration, no external crates
  (including libc/bindgen), no C++ FFI interlink.

## 8. Self-review

- Placeholders: none; APIs named, fixture paths fixed, recorder location fixed.
- Consistency: S1 modules map 1:1 to milestones M2b–M2f; S2 `dt` injection serves S3 determinism.
- Scope: single plan-sized spec (one crate, AppleSi-only); M3+ untouched.
- Ambiguity resolved: "parity" = replay-golden equality + local smoke non-empty (not C++
  dual-run, which is timing-flaky at collector level); "zero deps" includes build-deps.
