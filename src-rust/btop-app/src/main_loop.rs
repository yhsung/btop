//! Main loop (P4 T7).
//!
//! C++ truth: `src/btop.cpp:1105-1180` (loop body), `:1103-1106`
//! (`update_ms` / `future_time` seed — owned by [`boot::init_tick_clock`]),
//! `:264-274` (`_sleep` / `_resume`, folded into [`SuspendOps`]).
//!
//! [`main_loop`] never blocks on real stdin in tests: input arrives via
//! the [`box_labels::InputPoll`] seam (scripted doubles), ticks via
//! [`TickDrive`], time/clock via [`ClockOps`], config reload via
//! [`Reloader`], and SIGTSTP suspension via [`SuspendOps`]. Signal flags
//! need no mock — tests script the real [`SignalFlags`] atomics directly.
//! Production `fn main` wires the `Real*` impls.

use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use btop_collect::backend::MacOsBackend;
use btop_input::actions::{handle_key, Action, InputState, ViewState};
use btop_input::textedit::TextEdit;
use btop_menu::menus::MenuCtx;
use btop_runner::sink::{execute_all, execute_single, Sys, World};
use btop_runner::tick::{tick, TickInput};

use crate::boot::init_config_dirs;
use crate::box_labels::{min_size_loop, InputPoll, MinSizeOutcome};
use crate::term::TermWrapper;

// ── LoopExit ────────────────────────────────────────────────────────────────

/// Why [`main_loop`] returned. C++ never returns (it only leaves via
/// `clean_quit`, which `_Exit`s); the enum is the testable stand-in — `fn
/// main` maps [`LoopExit::Quit`] to the real [`crate::clean_quit`].
/// C++'s try/catch around the loop (:1184-1187 → `clean_quit(1)`) has no
/// port: a Rust panic unwinds past the loop to the runtime, where the
/// atexit fallback restores the term and marks `quitting` — skipping
/// `exit_error_msg`, `Config::write`, and the runtime print. Acceptable:
/// single-threaded, and panics are bugs, not control flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopExit {
    /// `should_quit`, `quit_requested` (the `q` key), or min-size `q`.
    /// Carries the `clean_quit` signal (`0` on every path the loop
    /// produces; C++ calls `clean_quit(0)` for all three).
    Quit(i32),
    /// `max_iters` bound hit (tests only — production passes `None`).
    IterationsExhausted,
}

// ── ClockOps ────────────────────────────────────────────────────────────────

/// Time + clock-change seam. `now_ms` is the `time_ms()` equivalent;
/// `clock_changed` mirrors `Draw::update_clock()`'s `bool` (without its
/// render side effect — the tick renders the clock from `state.clock`).
pub trait ClockOps: Send + Sync {
    fn now_ms(&mut self) -> u64;
    fn clock_changed(&mut self, world: &World) -> bool;
}

/// Production [`ClockOps`] over [`SystemTime`].
pub struct RealClock {
    last_sec: i64,
}

impl RealClock {
    pub fn new() -> Self {
        Self { last_sec: -1 }
    }
}

impl Default for RealClock {
    fn default() -> Self {
        Self::new()
    }
}

fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl ClockOps for RealClock {
    fn now_ms(&mut self) -> u64 {
        // DEVIATION: C++ `time_ms()` reads `steady_clock` (monotonic);
        // `SystemTime` can step on NTP adjustments. The loop only uses it
        // for tick scheduling and poll bounds, where a wall step just
        // shifts one deadline — acceptable for zero new deps.
        wall_ms()
    }

    fn clock_changed(&mut self, world: &World) -> bool {
        // `Draw::update_clock` (src/btop_draw.cpp:333-358): false when the
        // cpu box is hidden or no clock format is set; otherwise true when
        // the rendered second is new. The format-compare refinement
        // (`strf_time` has no port yet) is folded into the 1-second
        // granularity: any format shows a new string at most once per
        // second, so second-change implies string-change for every
        // shipped format.
        let shown = world
            .config
            .get_s("shown_boxes")
            .is_some_and(|s| s.split_whitespace().any(|b| b == "cpu"));
        let fmt_empty = world.config.get_s("clock_format").is_none_or(str::is_empty);
        if !shown || fmt_empty {
            return false;
        }
        let sec = (wall_ms() / 1000) as i64;
        if sec == self.last_sec {
            return false;
        }
        self.last_sec = sec;
        true
    }
}

/// Scripted [`ClockOps`]: `now_ms` pops the script (or advances the last
/// value by `step_ms` when exhausted); `clock_changed` pops its script
/// (`false` when exhausted).
pub struct ScriptClock {
    now_script: VecDeque<u64>,
    step_ms: u64,
    last: u64,
    changed_script: VecDeque<bool>,
}

impl ScriptClock {
    pub fn new(now_script: Vec<u64>, step_ms: u64, changed_script: Vec<bool>) -> Self {
        let last = now_script.first().copied().unwrap_or(0);
        Self {
            now_script: now_script.into(),
            step_ms,
            last,
            changed_script: changed_script.into(),
        }
    }
}

impl ClockOps for ScriptClock {
    fn now_ms(&mut self) -> u64 {
        if let Some(t) = self.now_script.pop_front() {
            self.last = t;
        } else {
            self.last += self.step_ms;
        }
        self.last
    }

    fn clock_changed(&mut self, _world: &World) -> bool {
        self.changed_script.pop_front().unwrap_or(false)
    }
}

// ── TickDrive ───────────────────────────────────────────────────────────────

/// Scheduled/forced draw seam. Production boot passes [`RealTick`]; tests
/// pass [`ScriptTick`].
///
/// NOTE: no `Send + Sync` bound (unlike the other T7 seams) — the live
/// collect backend (`RealBackend`) holds raw OS handles and is `!Send`.
/// The port is single-threaded; nothing crosses threads.
pub trait TickDrive {
    /// Run one tick (`force_redraw` = C++ `Runner::run(..., force=true)`).
    /// The impl owns output emission (production prints to stdout).
    fn tick_once(&mut self, world: &mut World, force_redraw: bool, now_ms: u64);
}

/// Production [`TickDrive`]: live collect backend + [`tick`] + stdout.
pub struct RealTick {
    backend: Box<dyn MacOsBackend>,
    sys: RealSys,
}

/// Live collect source: `RealBackend` on macOS (the M2 target);
/// degraded [`ReplayBackend`] (empty queues → skipped boxes) elsewhere —
/// the collect backends for other OSes have no port yet.
#[cfg(target_os = "macos")]
fn prod_backend() -> Box<dyn MacOsBackend> {
    Box::new(btop_collect::real::RealBackend::new())
}

/// See [`prod_backend`] (macOS arm).
#[cfg(not(target_os = "macos"))]
fn prod_backend() -> Box<dyn MacOsBackend> {
    Box::new(btop_collect::backend::ReplayBackend::default())
}

impl RealTick {
    pub fn new() -> Self {
        Self {
            backend: prod_backend(),
            sys: RealSys,
        }
    }
}

impl Default for RealTick {
    fn default() -> Self {
        Self::new()
    }
}

impl TickDrive for RealTick {
    fn tick_once(&mut self, world: &mut World, force_redraw: bool, now_ms: u64) {
        let overlay = if world.menu.active {
            world.menu.overlay.clone()
        } else {
            String::new()
        };
        let should_quit = world.signal_flags.should_quit.load(Ordering::SeqCst);
        let pending_resize = world.recalc_layout;
        let out = tick(
            world,
            &mut self.sys,
            TickInput {
                backend: &mut *self.backend,
                now_ms,
                pending_resize,
                should_quit,
                force_redraw_in: force_redraw,
                overlay,
            },
        );
        if !out.out.is_empty() {
            print!("{}", out.out);
            use std::io::Write as _;
            let _ = std::io::stdout().flush();
        }
    }
}

/// Recording [`TickDrive`]: logs `(force_redraw, now_ms)` per call,
/// advances `world.next_tick_ms` like the real schedule gate
/// (`now + update_ms`, mirroring tick.rs:423) so schedule tests observe
/// deadlines without running collect.
pub struct ScriptTick {
    pub calls: Vec<(bool, u64)>,
}

impl ScriptTick {
    pub fn new() -> Self {
        Self { calls: Vec::new() }
    }
}

impl Default for ScriptTick {
    fn default() -> Self {
        Self::new()
    }
}

impl TickDrive for ScriptTick {
    fn tick_once(&mut self, world: &mut World, force_redraw: bool, now_ms: u64) {
        self.calls.push((force_redraw, now_ms));
        let update_ms = world.config.get_i("update_ms").unwrap_or(2000).max(0) as u64;
        world.next_tick_ms = now_ms + update_ms;
        world.recalc_layout = false;
    }
}

// ── Sys ─────────────────────────────────────────────────────────────────────

/// Production [`Sys`]: real `kill(2)` / `setpriority(2)` / stdout escapes.
/// Theme listing rides with `Theme::updateThemes` (deferred past P4 per the
/// plan), so `read_theme_names` reports the compiled-in default only.
pub struct RealSys;

impl Sys for RealSys {
    fn kill(&mut self, pid: u64, sig: i32) -> Result<(), i32> {
        use nix::sys::signal::{kill, Signal};
        use nix::unistd::Pid;
        let sig = Signal::try_from(sig).map_err(|_| nix::errno::Errno::EINVAL as i32)?;
        kill(Pid::from_raw(pid as i32), sig).map_err(|e| e as i32)
    }

    fn set_priority(&mut self, pid: u64, nice: i64) -> bool {
        // SAFETY: `setpriority` with explicit args performs no ownership
        // transfer; the return/errno check is the whole contract.
        unsafe {
            nix::libc::setpriority(
                nix::libc::PRIO_PROCESS,
                pid as nix::libc::id_t,
                nice.clamp(-20, 19) as i32,
            ) == 0
        }
    }

    fn write_term(&mut self, esc: &str) {
        print!("{esc}");
        use std::io::Write as _;
        let _ = std::io::stdout().flush();
    }

    fn read_theme_names(&mut self) -> Vec<String> {
        vec!["Default".to_string()]
    }
}

// ── Reloader ────────────────────────────────────────────────────────────────

/// Hot-reload seam (btop.cpp:1124-1133, SIGUSR2 / Ctrl+R). Production boot
/// passes [`RealReloader`]; tests pass [`ScriptReloader`].
pub trait Reloader: Send + Sync {
    fn reload(&mut self, world: &mut World);
}

/// Production [`Reloader`]: `Config::unlock` + [`init_config_dirs`] with
/// the boot-time CLI bits, warnings to stderr. Theme re-update +
/// `banner_gen` ride with the deferred Theme port (plan known-deferred);
/// the loop sets `resized` after, which re-runs layout + redraw.
pub struct RealReloader {
    low_color: bool,
    filter: Option<String>,
}

impl RealReloader {
    pub fn new(low_color: bool, filter: Option<String>) -> Self {
        Self { low_color, filter }
    }
}

impl Reloader for RealReloader {
    fn reload(&mut self, world: &mut World) {
        use crate::boot::RealFs;
        use btop_config::cli::Cli;
        world.config.unlock();
        let cli = Cli {
            low_color: self.low_color,
            filter: self.filter.clone(),
            ..Cli::default()
        };
        match init_config_dirs(world, &cli, &RealFs) {
            Ok(warnings) => {
                for w in warnings {
                    eprintln!("{w}");
                }
            }
            Err(e) => eprintln!("Config reload failed: {e}"),
        }
    }
}

/// Counting [`Reloader`] test double.
pub struct ScriptReloader {
    pub calls: u32,
}

impl ScriptReloader {
    pub fn new() -> Self {
        Self { calls: 0 }
    }
}

impl Default for ScriptReloader {
    fn default() -> Self {
        Self::new()
    }
}

impl Reloader for ScriptReloader {
    fn reload(&mut self, _world: &mut World) {
        self.calls += 1;
    }
}

// ── SuspendOps ──────────────────────────────────────────────────────────────

/// SIGTSTP suspension seam. Folds `_sleep` (btop.cpp:264-269: stop +
/// restore + `raise(SIGSTOP)`) with the `_resume` re-init (btop.cpp:271-274)
/// that the SIGCONT delivery triggers while stopped — the loop sets
/// `resized` after `suspend` returns, mirroring `term_resize(true)`.
pub trait SuspendOps: Send + Sync {
    fn suspend(&mut self, term: &TermWrapper);
}

/// Production [`SuspendOps`].
pub struct RealSuspend;

impl SuspendOps for RealSuspend {
    fn suspend(&mut self, term: &TermWrapper) {
        use nix::sys::signal::{raise, Signal};
        term.restore();
        // Best-effort like C++: ignore the raise result; on return (after
        // the SIGCONT handler ran) re-init the terminal.
        let _ = raise(Signal::SIGSTOP);
        term.init();
    }
}

/// Counting [`SuspendOps`] test double (never raises).
pub struct ScriptSuspend {
    pub calls: u32,
}

impl ScriptSuspend {
    pub fn new() -> Self {
        Self { calls: 0 }
    }
}

impl Default for ScriptSuspend {
    fn default() -> Self {
        Self::new()
    }
}

impl SuspendOps for ScriptSuspend {
    fn suspend(&mut self, _term: &TermWrapper) {
        self.calls += 1;
    }
}

// ── LoopEnv ─────────────────────────────────────────────────────────────────

/// Everything [`main_loop`] needs beyond `World`. Lifetimes are
/// stack-bound on purpose: no `Arc`, no globals (unlike C++).
pub struct LoopEnv<'a> {
    pub term: &'a TermWrapper,
    pub input: &'a dyn InputPoll,
    pub clock: &'a mut dyn ClockOps,
    pub ticker: &'a mut dyn TickDrive,
    pub sys: &'a mut dyn Sys,
    pub reloader: &'a mut dyn Reloader,
    pub sleeper: &'a mut dyn SuspendOps,
    /// `Some(n)` bounds outer iterations (tests). `None` loops until
    /// [`LoopExit::Quit`] (production).
    pub max_iters: Option<u64>,
}

// ── ViewState ───────────────────────────────────────────────────────────────

/// Build the per-keypress [`ViewState`] from [`World`]. Every field mirrors
/// the Config key / Proc global of the same name (see `ViewState` docs);
/// box-gate flags come from `shown_boxes`, selection state from the
/// sink-owned fields (`proc_selected`, `proc_scroll_pos`, …).
///
/// DEVIATION (documented gap): box click-maps (`Input::mouse_mappings`)
/// are not yet published by the draw side, so `handle_key` receives an
/// empty `input_maps` — mouse clicks decode position only, never box
/// actions. Menu maps ride from `world.menu.mouse_maps` (real). Keyboard
/// is fully wired.
pub fn view_from_world(world: &World) -> ViewState {
    let shown = world.config.get_s("shown_boxes").unwrap_or("").to_string();
    let has = |b: &str| shown.split_whitespace().any(|x| x == b);
    ViewState {
        proc_shown: has("proc"),
        cpu_shown: has("cpu"),
        mem_shown: has("mem"),
        net_shown: has("net"),
        update_ms: world.config.get_i("update_ms").unwrap_or(2000),
        net_interfaces: world.state.net_interfaces.clone(),
        net_selected: world.state.selected_iface.clone(),
        sorting: world.config.get_s("proc_sorting").unwrap_or("").to_string(),
        tree: world.config.get_b("proc_tree").unwrap_or(false),
        selected: world.state.proc_selected,
        filter_text: world.state.proc_filter.clone(),
        scroll_pos: world.state.proc_scroll_pos,
        follow_process: world.config.get_b("follow_process").unwrap_or(false),
        pause_proc_list: world.config.get_b("pause_proc_list").unwrap_or(false),
        proc_last_selected: world.state.last_selected,
        ..ViewState::default()
    }
}

// ── Dispatch ────────────────────────────────────────────────────────────────

/// Run one decoded key through `handle_key` + [`execute_all`], returning
/// the sink's follow-up run requests (transcribes the `Input::process`
/// half of btop.cpp:1170-1171).
fn dispatch_key(
    world: &mut World,
    sys: &mut dyn Sys,
    dims: (usize, usize),
    key: &str,
    now_ms: u64,
    input_state: &mut InputState,
    editor: &mut TextEdit,
) -> Vec<btop_runner::sink::RunRequest> {
    input_state.menu_active = world.menu.active;
    input_state.filtering = world.config.get_b("proc_filtering").unwrap_or(false);
    input_state.vim_keys = world.config.get_b("vim_keys").unwrap_or(false);
    let view = view_from_world(world);
    // Borrow the menu maps before the mutable `execute_all` below
    // (disjoint from the `&mut World` only across statements).
    let actions = {
        let menu_maps = &world.menu.mouse_maps;
        let empty: &[btop_tools::mouse::MouseMap] = &[];
        handle_key(key, empty, menu_maps, input_state, &view, editor, now_ms)
    };
    // `Config::unlock` when the runner is idle (btop.cpp:1168).
    world.config.unlock();
    execute_all(world, sys, dims.0, dims.1, &actions)
}

/// Run one menu key through `MenuSystem::process` + [`execute_all`]
/// (btop.cpp:1170, `Menu::active` arm), returning the sink's follow-up run
/// requests for the caller to serve (a demanded redraw becomes a forced
/// tick — C++ draws the menu inline in `Menu::process`).
fn dispatch_menu(
    world: &mut World,
    sys: &mut dyn Sys,
    term_w: usize,
    term_h: usize,
    key: &str,
) -> Vec<btop_runner::sink::RunRequest> {
    let ctx = MenuCtx {
        term_w,
        term_h,
        target_pid: 0,
    };
    // Disjoint-field borrows (`menu`/`store`/`config`/`lists`) in one
    // expression — the borrow checker accepts these as separate fields.
    let actions = world
        .menu
        .process(key, &ctx, &mut world.store, &mut world.config, &world.lists);
    // Same logic/render split as the ShowMenu arm in sink.rs: rebuild the
    // overlay bytes after every menu keypress (theme falls back to Default
    // until the Theme-file port lands).
    let theme = if world.theme.is_empty() {
        btop_config::theme::default_theme()
    } else {
        world.theme.clone()
    };
    world.menu.render_overlay(
        &world.store,
        &world.lists,
        &theme,
        term_w as i64,
        term_h as i64,
    );
    execute_all(world, sys, term_w, term_h, &actions)
}

/// Serve follow-up run requests: a demanded redraw becomes an immediate
/// forced tick (C++ `Runner::run` per action side effect; same screen,
/// heavier collect — see the clock-arm note on [`main_loop`]).
fn serve_runs(
    world: &mut World,
    ticker: &mut dyn TickDrive,
    clock: &mut dyn ClockOps,
    runs: &[btop_runner::sink::RunRequest],
) {
    if runs.iter().any(|r| r.force_redraw) {
        ticker.tick_once(world, true, clock.now_ms());
    }
}

// ── main_loop ───────────────────────────────────────────────────────────────

/// Main loop transcribing btop.cpp:1105-1180. Iteration order (with cpp
/// lines):
///
/// | C++ | Step |
/// |---|---|
/// | :1111-1113 | `thread_exception` → `clean_quit(1)` — DROPPED: no second thread exists; single-threaded panics unwind instead |
/// | :1114-1116 | `should_quit` → `clean_quit(0)` → [`LoopExit::Quit`] |
/// | — | `quit_requested` (the `q` key → `Action::Quit`) → [`LoopExit::Quit`] — DEVIATION: C++ calls `clean_quit(0)` inline in `Input::process`; deferred here to the top of the next iteration (same observable shutdown) |
/// | :1117-1120 | `should_sleep` → suspend (folded `_sleep`+`_resume`, see [`SuspendOps`]) |
/// | :1122-1133 | `reload_conf` → [`Reloader::reload`] + `resized = true` |
/// | :307-309 | `do_continue` (SIGCONT) → `Term::init` + `resized = true` (the `_resume` the handler defers) |
/// | :1136 | `term_resize(resized)`: refresh dims, too-small screen via [`min_size_loop`] (`QuitRequested` → [`LoopExit::Quit`]) |
/// | :1138-1146 | `resized` → layout + forced clock + `resized = false` + menu-process or forced tick |
/// | :1149-1151 | clock changed and no menu → `UpdateClock` + clock tick |
/// | :1154-1157 | scheduled tick + `update_ms` reread + `future_time` advance |
/// | :1159-1178 | inner poll loop: `update_ms` reread / `future_time` clamp / `poll(min(1000, remaining))` → dispatch / `break` on timeout |
pub fn main_loop(
    world: &mut World,
    env: LoopEnv<'_>,
    mut update_ms: u64,
    mut future_time: u64,
) -> LoopExit {
    let mut input_state = InputState::default();
    let mut editor = TextEdit::new(String::new(), false);
    let mut iters: u64 = 0;

    loop {
        if let Some(max) = env.max_iters {
            if iters >= max {
                return LoopExit::IterationsExhausted;
            }
            iters += 1;
        }

        // ── Flags ──
        // Independent `if`s, not C++'s else-if chain (:1111-1131): every
        // set flag is serviced in the same iteration; sleep/reload/continue
        // each set `resized`, which the arm below coalesces — benign.
        if world.quit_requested || world.signal_flags.should_quit.load(Ordering::SeqCst) {
            return LoopExit::Quit(0); // :1114-1116 (+ deferred `q`).
        }
        if world
            .signal_flags
            .should_sleep
            .swap(false, Ordering::SeqCst)
        {
            // :1117-1120 (`should_sleep = false; _sleep()`).
            env.sleeper.suspend(env.term);
            world.signal_flags.resized.store(true, Ordering::SeqCst);
        }
        if world.signal_flags.reload_conf.swap(false, Ordering::SeqCst) {
            // :1122-1133.
            env.reloader.reload(world);
            world.signal_flags.resized.store(true, Ordering::SeqCst);
        }
        if world.signal_flags.do_continue.swap(false, Ordering::SeqCst) {
            // `_resume` (:271-274), deferred from the SIGCONT handler.
            env.term.init();
            world.signal_flags.resized.store(true, Ordering::SeqCst);
        }

        // ── term_resize(resized) (:1136) ──
        // Keep the tick's width honest every iteration (cheap atomic load).
        world.state.term_width = env.term.width() as i64;
        let resized = world.signal_flags.resized.swap(false, Ordering::SeqCst);
        let changed = env.term.refresh(false);
        if resized || changed {
            world.state.term_width = env.term.width() as i64;
            let mut screen = String::new();
            match min_size_loop(world, env.term, env.input, &mut screen) {
                MinSizeOutcome::Ready => {}
                // btop.cpp:190 (`q` → `clean_quit(0)`).
                MinSizeOutcome::QuitRequested => return LoopExit::Quit(0),
            }
            if !screen.is_empty() {
                env.sys.write_term(&screen);
            }
        }

        // ── Resized → layout + clock + run (:1138-1146) ──
        if resized {
            world.recalc_layout = true; // `Draw::calcSizes`.
            let (tw, th) = (env.term.width() as usize, env.term.height() as usize);
            // `Draw::update_clock(true)` (forced) routes through the real
            // `UpdateClock` action so the sink stays the single writer of
            // `clock_refresh`.
            let _ = execute_single(world, env.sys, tw, th, &Action::UpdateClock);
            if world.menu.active {
                let runs = dispatch_menu(world, env.sys, tw, th, "");
                serve_runs(world, env.ticker, env.clock, &runs);
            } else {
                env.ticker.tick_once(world, true, env.clock.now_ms());
            }
        }

        // ── Clock (:1149-1151) ──
        if env.clock.clock_changed(world) && !world.menu.active {
            let (tw, th) = (env.term.width() as usize, env.term.height() as usize);
            let _ = execute_single(world, env.sys, tw, th, &Action::UpdateClock);
            // DEVIATION: C++ `Runner::run("clock")` redraws the clock box
            // only; the P3 tick has no clock-only path, so a forced full
            // tick covers it (same bytes on screen, heavier collect).
            env.ticker.tick_once(world, true, env.clock.now_ms());
        }

        // ── Scheduled tick (:1154-1157) ──
        let now = env.clock.now_ms();
        if now >= world.next_tick_ms && !world.signal_flags.resized.load(Ordering::SeqCst) {
            env.ticker.tick_once(world, false, now);
            update_ms = world.config.get_i("update_ms").unwrap_or(2000).max(0) as u64;
            future_time = now + update_ms;
        }

        // ── Inner poll loop (:1159-1178) ──
        let mut current = env.clock.now_ms();
        while current < future_time {
            // :1162-1167 — reread the interval, clamp runaway deadlines.
            let cfg_ms = world.config.get_i("update_ms").unwrap_or(2000).max(0) as u64;
            if update_ms != cfg_ms {
                update_ms = cfg_ms;
                future_time = env.clock.now_ms() + update_ms;
            } else if future_time - current > update_ms {
                future_time = current;
            // :1169 — wait on input OR signal (plain `poll`, no masked
            // `pselect`; the SIGUSR1 process-wide block is documented on
            // `RealInstaller`). `false` covers both timeout and an EINTR-broken
            // wait; with `SA_RESTART` (see the install site) the kernel may
            // instead restart the wait, so a pending flag is serviced at
            // most one quantum late (≤1000ms here) — the outer flag checks
            // make that bound exact.
            } else if env.input.poll((future_time - current).min(1000)) {
                let (tw, th) = (env.term.width() as usize, env.term.height() as usize);
                let key = env.input.get();
                let runs = if world.menu.active {
                    dispatch_menu(world, env.sys, tw, th, &key)
                } else {
                    dispatch_key(
                        world,
                        env.sys,
                        (tw, th),
                        &key,
                        current,
                        &mut input_state,
                        &mut editor,
                    )
                };
                // C++ `Runner::run` per action side effect: a demanded
                // redraw becomes an immediate forced tick (same screen,
                // heavier collect — see the clock arm note).
                serve_runs(world, env.ticker, env.clock, &runs);
                if world.quit_requested || world.signal_flags.should_quit.load(Ordering::SeqCst) {
                    return LoopExit::Quit(0);
                }
            } else {
                // Timeout with no input (or an interrupted wait) — back to
                // the outer flag checks (:1176-1177 `else break`).
                break;
            }
            current = env.clock.now_ms();
        }
    }
}
