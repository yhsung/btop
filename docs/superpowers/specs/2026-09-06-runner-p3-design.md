# P3 runner + wiring + sink — Design (Route A: single-threaded tick)

Date: 2026-09-06
Status: approved S1–S3, pending spec review
Route: A — wiring first, full sink, single-threaded tick, detail/tree/filter upgrade included.

## 1. Background

C++ `btop.cpp` (~450 runner + ~360 main/loop lines): main thread (scheduler, input,
menu, signals) + one `_runner` pthread synced by `binary_semaphore do_work` and
`atomic_waiting_lock active`. Ticks: `update_ms`/`future_time` scheduling, per-box
`collect(no_update)` → `draw(...)`, output merge (boxes + clock + overlay +
dim-behind + terminal_sync wrap, single flush), `no_update` = cached redraw,
`force_redraw` = full re-render. Signals INT/TSTP/CONT/WINCH/USR1/USR2 + crash
handlers; `clean_quit` joins thread, writes config, restores terminal. Rust port
already has: collect (incl. cmult-ready math, offset-returning counters), draw
(all boxes + MouseMap + Clock + Layout), input (Action enum + process_key),
menu (overlay + menu Actions). Missing: assembly, execution, loop.

## 2. Decisions (user-confirmed)

- Single-threaded tick (not dual-thread faithful port).
- Detail/tree/filter byte-parity upgrade included (3 new harness scenarios).
- Route A over B (complexity without benefit) and C (loop unclosed).

## 3. Architecture (S1 approved)

- New workspace member `btop-runner`, zero third-party deps; deps: collect, draw,
  input, menu, config.
- `wiring.rs`: collector outputs → `AppState` → per-box draw inputs.
- `sink.rs`: executes every `Action` against the world.
- `tick.rs`: single-threaded schedule + collect/draw/merge/print.

## 4. Wiring assembly (S2 approved)

- `AppState` centrally owns: proc list/sort/tree/selection/detail histories,
  net offsets/iface/graph_max, cpu cmult/per_core/core_mapping, mem disks order,
  gpu panel map, merged mouse maps (boxes + menus), clock value.
- One `assemble_*` pure fn per box (headless-testable); per-tick state updates
  (offset persist, history push/trim, selection) live here, not in draw/collect.

## 5. Sink + tick + upgrade (S3 approved)

- Sink executes ALL Actions incl. OS calls (`kill(2)`, `set_priority`,
  theme rebuild/FS reads, term escape writes); tested against a fake world
  (recording FS/process/term doubles, never real syscalls in tests).
- `tick()` single fn: schedule check → collect → draw → merge (boxes + clock +
  overlay + dim-behind + empty-bg) → print; `no_update`/`force_redraw`/overlay/
  clock semantics per C++; tested with fake collect/draw backends.
- Upgrade: 3 harness scenarios (detail-open, tree, filtered) + Rust byte parity,
  closing the M3 quality follow-up.

## 6. Non-goals

- No threads/semaphores (single-threaded by decision), no signal installation
  (P4 owns signals; tick takes `pending_resize`/`should_quit` flags as inputs),
  no CLI/locale/Term init (P4), no fd reading (P4 feeds `handle_key`).

## 7. Self-review

- Placeholders: none; modules, state ownership, and test shapes concrete.
- Consistency: single-Action language flows P1→P2→P3 sink; stateless draw/collect
  precedent preserved (all tick state in AppState); golden upgrade matches M3/P2
  harness pattern.
- Scope: single plan-sized spec (one crate + 3 harness blocks); P4 seams named
  (flags in, terminal/threads/signals out).
- Ambiguity resolved: "single-threaded" = one `tick()` fn, no pthreads, P4 loop
  calls it; "fake world" = trait doubles, zero real syscalls under `cargo test`.
