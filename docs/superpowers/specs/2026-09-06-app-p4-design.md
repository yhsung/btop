# P4 app + boot + signals — Design (Route A: single-threaded)

Date: 2026-09-06
Status: approved S1–S3, pending spec review
Route: A — single `btop-app` crate owns the init chain, Term wrapper, signals, main loop, and clean_quit; single-threaded (matches P3).

## 1. Background

C++ `btop.cpp` (~362 init + ~74 main loop + ~17 signal install + ~51 clean_quit + ~70 Term) + `btop_tools.cpp` (Term impl, locale, suid) + `btop_shared.cpp` (Shared::init maps). P3 already provides `tick()` (consumes MacOsBackend, no signals), sink executes 50 actions, World carries all tick state. Missing: SUID drop, Cli::parse (P4-1 own? or P1 CLI reused — reused), get_config_dir + Logger::init + Theme dir scan, locale hunt, Term wrapper, configure_tty_mode, Shared::init (cpuName/sensors/core_mapping/AppleSilicon wiring), set_boxes fallback, Theme::updateThemes/setTheme, atexit + 10 signal() installs (no separate input thread), pthread_sigmask for input poll (N/A — single thread), pthread_create runner (N/A — single thread), Config::presetsValid, apply_preset default branch, min-size gate loop, cached box-label strings, clean_quit (AppleSilicon shutdown + Config::write + Input::clear + Term::restore + exit_error_msg + Logger info + atexit). Plus leftover globals: `start_time`, `quitting`, `should_sleep`, `reload_conf`, `resized`, `should_quit`, `exit_error_msg`, `Banner_src`.

## 2. Decisions (user-confirmed)

- Single `btop-app` crate, one plan covers all (≈900 lines, risk accepted).
- Single-threaded throughout: signal handlers set atomic flags; main loop calls P3 `tick()` + `Input::process`; SIGTERM → atexit→clean_quit.
- Term: tiny singleton `Term` struct (state stored in-process); `init` is best-effort (returns bool false → exit "No tty detected").
- Headless smoke test path (no real termios) for the testable init sequence.

## 3. Architecture (S1 approved)

- New workspace member `btop-app` with `src/bin/btop.rs` (thin entry) + `src/lib.rs` (testable internals).
- Deps: `btop-cli`, `btop-config`, `btop-draw`, `btop-tools`, `btop-collect`, `btop-runner`, `btop-menu`, `btop-input`. Zero third-party.
- Init chain: `parse_cli` → `setuid_drop` → `init_config_dirs` (Config::load + theme dirs) → `init_locale` → `Term::init` → `configure_tty_mode` → `Shared::init` (writes AppState.tick_factor from machTck/clkTck; sets cpuName, sensors, core_count) → `Config::set_boxes` (fallback) → `Theme::updateThemes/setTheme` → `install_signals` → `presetsValid+apply_preset` → `min_size_loop` → `Draw::calcSizes` → `print_box_outlines` → enter main loop.

## 4. Signals + main loop (S2 approved)

- Signal handlers set `Arc<AtomicBool>` flags (no real pthread — single thread): `SIGINT/SIGTERM→should_quit`, `SIGTSTP→should_sleep` (or call `_sleep` which raises SIGSTOP; but spec says handler only sets flag + atexit), `SIGCONT→do_continue` flag (re-Term::init), `SIGWINCH→resized`, `SIGUSR1→interrupt_input` (re-poll wakeup), `SIGUSR2→reload_conf`, crash handlers restore Term+re-raise. `atexit` calls `clean_quit(-1)`.
- Main loop: `pselect(STDIN_FILENO+1, fds, NULL, NULL, timeout, &sig_mask)` blocks with signal_mask unblocking SIGUSR1/SIGTERM/SIGWINCH; loop body checks atomic flags, calls `tick()`, polls `Input::get()` + `process_key` via P1's `handle_key`, runs `Menu::process` if Menu active, processes `Runner::run` for re-renders; advances `next_tick_ms = now_ms + update_ms`.

## 5. clean_quit + tests (S3 approved)

- `clean_quit(sig: i32)` exact sequence per C++ :211-261: `quitting=true`; `Runner::stop` (no-op); `pthread_join` (no-op); `Gpu::AppleSilicon::shutdown` (no-op stub for macOS); `Config::write()` if `save_config_on_exit`; `Input::clear` + `Term::restore`; exit_error_msg printed in red; runtime printed; `_Exit(excode)`. Pure-Rust path: `std::process::exit(excode)`.
- Tests: `init_config_dirs` (theme dir scan), `init_locale` (UTF-8 detection — fake env vars), `Shared::init` (writes AppState fields — fake sysctl), `set_boxes` fallback, `presetsValid+apply_preset` default branch, `clean_quit` sequence (mock all side-effects), `print_box_outlines` (string exactness vs cpp:1095-1097), full boot path integration test (headless: `init_chain_returns_configured_state_when_term_init_fails_cleanly`).
- Signal handlers unit-tested via direct invocation (set flags, verify atomic values).

## 6. Non-goals

- No real pthread (single-threaded by decision); no actual tty enter/exit on smoke tests; no GPU panels wiring (P3 stub → P4 hardcoded to match harness `gpus_slice` config; full wiring is post-default-binary); no Theme::themes FS read (AppleSilicon builds use builtins only); no menu wiring for non-Apple OSes.

## 7. Self-review

- Placeholders: none; modules, function signatures, and test shapes concrete.
- Consistency: single-threaded matches P3; atomic-flag signals match the no-pthread decision; clean_quit matches C++ :211-261 verbatim; Term singleton lives alongside btop-tools.
- Scope: one spec, one crate; leftover C++ globals named (start_time, quitting, should_sleep, reload_conf, resized, should_quit, exit_error_msg, Banner_src) — each gets a Rust home (World field, atomic flag, or World string) documented in spec.
- Ambiguity resolved: "headless smoke" = no termios/POSIX signal install; uses fake env + helper mocks; "full boot path" = init chain runs to first-tick readiness with `Term::init` deliberately returning false.
