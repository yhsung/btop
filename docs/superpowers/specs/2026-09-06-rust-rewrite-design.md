# btop Rust Rewrite — Design (Route A)

Date: 2026-09-06
Status: approved S1–S3, pending spec review
Route: A — 分層絞殺 + 雙跑比對（同 repo 漸進，不做 FFI 混編）

## 1. Background

btop (C++23, ~20k lines) structure: `X/btop_collect::Cpu/Mem/Net/Proc/Gpu::collect(no_update)`
polling → global state in `btop_shared.hpp` (`box/width/redraw` + `*_info` deques) →
`draw(info, force_redraw, data_same)` returning ANSI strings → `Runner::run()` merges output,
with double-buffer `redraw/data_same` to avoid repaint. Platform switch is compile-time
`#ifdef` + `CMake/PLATFORM_DIR`: linux (/proc + dlopen NVML/RSMI + intel_gpu_top),
osx (IOKit/SMC/IOReport), freebsd (kvm/devstat), openbsd (sysctlbyname+kvm), netbsd (kvm/proplib).
Deps: fmt (header-only) + pthreads (+ kvm/devstat, IOKit). Config `~/.config/btop/btop.conf`
(key=value), themes `themes/*.theme`. Risk spots: terminal escape/wide-char/braille graphs,
cross-platform parity, atomic+mutex timing, GPU dynamic loading.

## 2. Decisions (user-confirmed)

- Milestone 1: macOS-first (`src/osx/btop_collect.cpp`: IOKit/SMC/IOReport first, Linux /proc second).
- UI: pixel/behavior-compatible, keep layout, themes, mouse/keyboard interactions.
- Compat: `btop.conf` + `.theme` + CLI flags fully compatible, no breaking change.
- Location: same repo gradual replacement; every step has BTDD/TDD guardrails proving
  Rust output identical to C++.

## 3. Architecture (S1 approved)

- New cargo workspace under `src-rust/`; C++ tree untouched. CMake gains opt-in `USE_RUST`
  to build `btop-rs` side-by-side for comparison. No C++/Rust FFI interlink.
- Crates (one purpose each, independently testable):
  - `btop-tools` ← `btop_tools/Shared/Log`: strings, numbers, wide-char, locks. No syscalls.
  - `btop-config` ← `btop_config/cli/theme`: `btop.conf` key=value parse+write, `.theme` parse,
    CLI flags. Byte-compatible behavior.
  - `btop-collect` ← per-platform `btop_collect`: `trait Collector { Cpu/Mem/Net/Proc/Gpu }`.
    Implement `osx` first, then `linux`, other BSDs as stubs returning empty+logged.
  - `btop-draw` ← `btop_draw/menu`: pure `draw(state) -> String`. No syscalls.
  - `btop-input` ← `btop_input`: key/mouse event → action enum.
  - `btop-app` ← `btop.cpp Runner/main`: scheduling + output merge.
- Targeted cleanup (serves the goal only): collapse global `shared` mutable state into explicit
  `AppState` passed by reference; split `draw` (2551 lines) + `menu` (1963 lines) by box
  submodules. No unrelated refactoring.

## 4. Data flow & error handling (S2 approved)

- Flow: `Collector::collect(snapshot) → AppState { cpu/mem/net/proc/gpu + history deques +
  box/width/redraw } → draw(state, force_redraw, data_same) -> String → Runner merges output`.
  `collect` never renders; `draw` never touches the system; both communicate via immutable
  snapshots, enabling dual-run comparison.
- Scheduling: single main scheduler, per-collector intervals preserving current `no_update`
  semantics. macOS IOKit/SMC failure and Linux /proc parse failure degrade to empty values
  with history preserved, never crash the frame.
- Errors: `thiserror` layered errors (`Collect::Io/Parse/Platform`). `draw` returns `String`,
  never `Err` (bad data renders as blank cells, never panics). Fatal exit only at startup
  (config parse, terminal init) reusing current message formats.

## 5. Testing — BTDD/TDD guardrails (S3 approved)

Every crate merges only when all three gates are green:
1. Unit TDD: pure logic (`tools/config/theme`) — failing test first, then implementation.
2. Golden BTDD: `config/.theme/CLI` fixtures taken from current release files;
   `draw` compares ANSI output against recorded `AppState` JSON snapshots
   (wide-char/braille graphs explicitly covered).
3. Dual-run parity: `collect` replays recorded macOS stubs (IOKit replay) + Linux `/proc`
   snapshots through both C++ and Rust binaries; output diff must be empty.
   CI adds a `parity-gate` job running all three.

## 6. Milestones (each a shippable parity point)

- M0: workspace skeleton + harness (`src-rust/`, fixtures, `parity-gate` CI).
- M1: `tools/config/theme/cli` parity.
- M2: `osx` collector parity (first priority).
- M3: `draw` parity.
- M4: `input/menu/app` integration.
- M5: `linux` collector.
- M6: remaining BSDs + switch default binary to Rust.

## 7. Non-goals

- No new UI design, no config format change, no FFI-mixed binary, no unrelated refactors.

## 8. Self-review

- Placeholders: none; all gates and milestones concrete.
- Consistency: architecture (Sec.3) matches flow (Sec.4) and gates (Sec.5); M-order follows
  dependency order (pure → collect → draw → app).
- Scope: single spec covering full rewrite via decomposable milestones M0–M6; each milestone
  gets its own implementation plan cycle.
- Ambiguity resolved: "identical" means byte-identical ANSI/golden diff + empty dual-run diff;
  "macOS-first" means M2 before M5; compat means existing user files work unmodified.
