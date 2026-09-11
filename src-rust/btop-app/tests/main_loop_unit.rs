//! T7 headless tests: `clean_quit_impl` + `main_loop` (P4 Task 7).
//!
//! C++ truth: `src/btop.cpp:211-261` (`clean_quit`), `:1105-1180` (loop).
//!
//! No test touches a real terminal, stdin, or signal handlers: the term
//! runs on [`MockOps`], input on a scripted [`InputPoll`] queue, ticks on
//! [`ScriptTick`], time on [`ScriptClock`], reload on [`ScriptReloader`],
//! suspend on [`ScriptSuspend`], and `Sys` on the sink's [`FakeSys`].
//! Signal flags need no mock — tests script the real atomics directly.

use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use btop_app::box_labels::InputPoll;
use btop_app::clean_quit::{clean_quit_impl, QuitEnv, FG_RED, FG_WHITE, FX_RESET};
use btop_app::main_loop::{
    LoopEnv, LoopExit, RealClock, RealReloader, RealSuspend, ScriptClock, ScriptReloader,
    ScriptSuspend, ScriptTick,
};
use btop_app::term::{MockOps, TermWrapper};
use btop_runner::sink::{FakeSys, World};

// ── QuitEnv double ──────────────────────────────────────────────────────────

#[derive(Default)]
struct RecQuit {
    clears: u32,
    saves: u32,
    stderr: String,
    stdout: Vec<String>,
}

impl QuitEnv for RecQuit {
    fn clear_input(&mut self) {
        self.clears += 1;
    }
    fn save_config(&mut self, _world: &mut World) {
        self.saves += 1;
    }
    fn emit_stderr(&mut self, s: &str) {
        self.stderr.push_str(s);
    }
    fn emit_stdout(&mut self, s: &str) {
        self.stdout.push(s.to_string());
    }
}

fn quit_world() -> World {
    let mut w = World::default();
    w.start_time = Some(Instant::now());
    w
}

fn init_term(w: u16, h: u16) -> (Arc<MockOps>, TermWrapper) {
    let ops = Arc::new(MockOps::new().with_size(w, h));
    let term = TermWrapper::new(ops.clone());
    assert!(term.init());
    (ops, term)
}

// ── clean_quit_impl ─────────────────────────────────────────────────────────

#[test]
fn quit_guard_second_call_returns_none() {
    let mut w = quit_world();
    let (_ops, term) = init_term(120, 40);
    let mut env = RecQuit::default();
    assert!(clean_quit_impl(-1, &mut w, &term, &mut env).is_some());
    assert_eq!(clean_quit_impl(-1, &mut w, &term, &mut env), None);
}

#[test]
fn quit_saves_config_only_when_flag_set() {
    for save in [false, true] {
        let mut w = quit_world();
        let (_ops, term) = init_term(120, 40);
        let _ = w.config.set_b("save_config_on_exit", save);
        let mut env = RecQuit::default();
        let out = clean_quit_impl(-1, &mut w, &term, &mut env).expect("first call");
        assert_eq!(out.excode, 0);
        assert_eq!(env.saves, u32::from(save));
    }
}

#[test]
fn quit_error_msg_forces_sig1_and_red_line() {
    let mut w = quit_world();
    let (_ops, term) = init_term(120, 40);
    w.exit_error_msg = Some("boom".to_string());
    let mut env = RecQuit::default();
    let out = clean_quit_impl(0, &mut w, &term, &mut env).expect("first call");
    assert_eq!(out.excode, 1);
    let line = out.error_line.expect("error line");
    assert_eq!(line, format!("{FG_RED}ERROR: {FG_WHITE}boom{FX_RESET}\n"));
    assert_eq!(env.stderr, line);
}

#[test]
fn quit_empty_error_msg_is_ignored() {
    let mut w = quit_world();
    let (_ops, term) = init_term(120, 40);
    w.exit_error_msg = Some(String::new());
    let mut env = RecQuit::default();
    let out = clean_quit_impl(0, &mut w, &term, &mut env).expect("first call");
    assert_eq!(out.excode, 0);
    assert_eq!(out.error_line, None);
    assert!(env.stderr.is_empty());
}

#[test]
fn quit_signal_passthrough() {
    let mut w = quit_world();
    let (_ops, term) = init_term(120, 40);
    let mut env = RecQuit::default();
    let out = clean_quit_impl(15, &mut w, &term, &mut env).expect("first call");
    assert_eq!(out.excode, 15);
}

#[test]
fn quit_restores_term_only_when_initialized() {
    // Initialized: clear + restore run.
    let mut w = quit_world();
    let (ops, term) = init_term(120, 40);
    let mut env = RecQuit::default();
    clean_quit_impl(-1, &mut w, &term, &mut env).expect("first call");
    assert_eq!(env.clears, 1);
    assert_eq!(ops.restore_calls.load(Ordering::Relaxed), 1);
    // Never initialized: neither runs.
    let mut w = quit_world();
    let ops = Arc::new(MockOps::new().with_size(120, 40));
    let term = TermWrapper::new(ops.clone());
    let mut env = RecQuit::default();
    clean_quit_impl(-1, &mut w, &term, &mut env).expect("first call");
    assert_eq!(env.clears, 0);
    assert_eq!(ops.restore_calls.load(Ordering::Relaxed), 0);
}

#[test]
fn quit_runtime_line_shape() {
    let mut w = quit_world();
    let (_ops, term) = init_term(120, 40);
    let mut env = RecQuit::default();
    let out = clean_quit_impl(-1, &mut w, &term, &mut env).expect("first call");
    assert!(
        out.runtime_line.starts_with("Quitting! Runtime: "),
        "{}",
        out.runtime_line
    );
    assert_eq!(env.stdout, vec![out.runtime_line.clone()]);
}

// ── main_loop doubles ───────────────────────────────────────────────────────

/// Scripted [`InputPoll`]: `Some(key)` = hit, `None` = timeout/miss.
struct ScriptInput {
    steps: Mutex<VecDeque<Option<String>>>,
    pending: Mutex<String>,
}

impl ScriptInput {
    fn new(steps: Vec<Option<String>>) -> Self {
        Self {
            steps: Mutex::new(steps.into_iter().collect()),
            pending: Mutex::new(String::new()),
        }
    }
}

impl InputPoll for ScriptInput {
    fn poll(&self, _timeout_ms: u64) -> bool {
        match self.steps.lock().unwrap().pop_front().unwrap_or(None) {
            Some(key) => {
                *self.pending.lock().unwrap() = key;
                true
            }
            None => false,
        }
    }
    fn get(&self) -> String {
        std::mem::take(&mut *self.pending.lock().unwrap())
    }
}

fn key(s: &str) -> Option<String> {
    Some(s.to_string())
}

/// Fitting 120×40 term (above the `(100, 24)` stub minima, so
/// `min_size_loop` returns `Ready` without polling).
fn loop_term() -> (Arc<MockOps>, TermWrapper) {
    init_term(120, 40)
}

#[allow(clippy::too_many_arguments)]
fn run_loop(
    world: &mut World,
    term: &TermWrapper,
    input: &dyn InputPoll,
    clock: &mut ScriptClock,
    ticker: &mut ScriptTick,
    sys: &mut FakeSys,
    reloader: &mut ScriptReloader,
    sleeper: &mut ScriptSuspend,
    update_ms: u64,
    future_time: u64,
    max_iters: u64,
) -> LoopExit {
    let env = LoopEnv {
        term,
        input,
        clock,
        ticker,
        sys,
        reloader,
        sleeper,
        max_iters: Some(max_iters),
    };
    btop_app::main_loop::main_loop(world, env, update_ms, future_time)
}

fn quiet_world() -> World {
    let mut w = World::default();
    w.start_time = Some(Instant::now());
    // No scheduled tick unless a test moves the deadline up.
    w.next_tick_ms = u64::MAX;
    w
}

// ── main_loop: flags ────────────────────────────────────────────────────────

#[test]
fn loop_should_quit_exits_zero_without_tick() {
    let mut w = quiet_world();
    let (_ops, term) = loop_term();
    let input = ScriptInput::new(vec![]);
    let mut clock = ScriptClock::new(vec![1000], 1000, vec![]);
    let mut ticker = ScriptTick::new();
    let mut sys = FakeSys::default();
    let mut reloader = ScriptReloader::new();
    let mut sleeper = ScriptSuspend::new();
    w.signal_flags.should_quit.store(true, Ordering::SeqCst);
    let exit = run_loop(
        &mut w,
        &term,
        &input,
        &mut clock,
        &mut ticker,
        &mut sys,
        &mut reloader,
        &mut sleeper,
        2000,
        3000,
        10,
    );
    assert_eq!(exit, LoopExit::Quit(0));
    assert!(ticker.calls.is_empty());
}

#[test]
fn loop_should_sleep_suspends_and_marks_resized() {
    let mut w = quiet_world();
    let (_ops, term) = loop_term();
    let input = ScriptInput::new(vec![None]);
    let mut clock = ScriptClock::new(vec![1000, 1000, 1000], 1000, vec![]);
    let mut ticker = ScriptTick::new();
    let mut sys = FakeSys::default();
    let mut reloader = ScriptReloader::new();
    let mut sleeper = ScriptSuspend::new();
    w.signal_flags.should_sleep.store(true, Ordering::SeqCst);
    let exit = run_loop(
        &mut w,
        &term,
        &input,
        &mut clock,
        &mut ticker,
        &mut sys,
        &mut reloader,
        &mut sleeper,
        2000,
        3000,
        1,
    );
    assert_eq!(exit, LoopExit::IterationsExhausted);
    assert_eq!(sleeper.calls, 1);
    // The suspend path is followed by the resize arm, which consumes the
    // flag and forces a tick (term fits, so no small-screen loop).
    assert_eq!(ticker.calls.len(), 1);
    assert!(ticker.calls[0].0, "resize arm forces the tick");
}

#[test]
fn loop_reload_conf_calls_reloader() {
    let mut w = quiet_world();
    let (_ops, term) = loop_term();
    let input = ScriptInput::new(vec![None]);
    let mut clock = ScriptClock::new(vec![1000, 1000, 1000], 1000, vec![]);
    let mut ticker = ScriptTick::new();
    let mut sys = FakeSys::default();
    let mut reloader = ScriptReloader::new();
    let mut sleeper = ScriptSuspend::new();
    w.signal_flags.reload_conf.store(true, Ordering::SeqCst);
    let exit = run_loop(
        &mut w,
        &term,
        &input,
        &mut clock,
        &mut ticker,
        &mut sys,
        &mut reloader,
        &mut sleeper,
        2000,
        3000,
        1,
    );
    assert_eq!(exit, LoopExit::IterationsExhausted);
    assert_eq!(reloader.calls, 1);
}

#[test]
fn loop_do_continue_reinits_term() {
    let mut w = quiet_world();
    let (ops, term) = loop_term();
    let inits_before = ops.init_calls.load(Ordering::Relaxed);
    let input = ScriptInput::new(vec![None]);
    let mut clock = ScriptClock::new(vec![1000, 1000], 1000, vec![]);
    let mut ticker = ScriptTick::new();
    let mut sys = FakeSys::default();
    let mut reloader = ScriptReloader::new();
    let mut sleeper = ScriptSuspend::new();
    w.signal_flags.do_continue.store(true, Ordering::SeqCst);
    let exit = run_loop(
        &mut w,
        &term,
        &input,
        &mut clock,
        &mut ticker,
        &mut sys,
        &mut reloader,
        &mut sleeper,
        2000,
        3000,
        1,
    );
    assert_eq!(exit, LoopExit::IterationsExhausted);
    assert_eq!(ops.init_calls.load(Ordering::Relaxed), inits_before + 1);
}

// ── main_loop: tick arms ────────────────────────────────────────────────────

#[test]
fn loop_scheduled_tick_fires_and_advances_deadline() {
    let mut w = quiet_world();
    w.next_tick_ms = 1000;
    let (_ops, term) = loop_term();
    let input = ScriptInput::new(vec![None]);
    let mut clock = ScriptClock::new(vec![1000, 1000, 1000], 1000, vec![]);
    let mut ticker = ScriptTick::new();
    let mut sys = FakeSys::default();
    let mut reloader = ScriptReloader::new();
    let mut sleeper = ScriptSuspend::new();
    let exit = run_loop(
        &mut w,
        &term,
        &input,
        &mut clock,
        &mut ticker,
        &mut sys,
        &mut reloader,
        &mut sleeper,
        2000,
        1000,
        1,
    );
    assert_eq!(exit, LoopExit::IterationsExhausted);
    assert_eq!(ticker.calls.len(), 1);
    assert!(!ticker.calls[0].0, "scheduled tick is not forced");
    assert_eq!(w.next_tick_ms, 3000, "deadline advances by update_ms");
}

#[test]
fn loop_resized_arm_forces_tick() {
    let mut w = quiet_world();
    let (_ops, term) = loop_term();
    let input = ScriptInput::new(vec![None]);
    let mut clock = ScriptClock::new(vec![1000, 1000, 1000], 1000, vec![]);
    let mut ticker = ScriptTick::new();
    let mut sys = FakeSys::default();
    let mut reloader = ScriptReloader::new();
    let mut sleeper = ScriptSuspend::new();
    w.signal_flags.resized.store(true, Ordering::SeqCst);
    let exit = run_loop(
        &mut w,
        &term,
        &input,
        &mut clock,
        &mut ticker,
        &mut sys,
        &mut reloader,
        &mut sleeper,
        2000,
        3000,
        1,
    );
    assert_eq!(exit, LoopExit::IterationsExhausted);
    assert_eq!(ticker.calls.len(), 1);
    assert!(ticker.calls[0].0, "resize arm forces the tick");
}

#[test]
fn loop_clock_arm_ticks_on_change_without_menu() {
    let mut w = quiet_world();
    let (_ops, term) = loop_term();
    let input = ScriptInput::new(vec![None]);
    let mut clock = ScriptClock::new(vec![1000, 1000, 1000], 1000, vec![true]);
    let mut ticker = ScriptTick::new();
    let mut sys = FakeSys::default();
    let mut reloader = ScriptReloader::new();
    let mut sleeper = ScriptSuspend::new();
    let exit = run_loop(
        &mut w,
        &term,
        &input,
        &mut clock,
        &mut ticker,
        &mut sys,
        &mut reloader,
        &mut sleeper,
        2000,
        3000,
        1,
    );
    assert_eq!(exit, LoopExit::IterationsExhausted);
    assert_eq!(ticker.calls.len(), 1);
    assert!(ticker.calls[0].0, "clock arm forces the tick");
}

#[test]
fn loop_clock_arm_skipped_while_menu_active() {
    let mut w = quiet_world();
    w.menu.active = true;
    let (_ops, term) = loop_term();
    let input = ScriptInput::new(vec![None]);
    let mut clock = ScriptClock::new(vec![1000, 1000], 1000, vec![true]);
    let mut ticker = ScriptTick::new();
    let mut sys = FakeSys::default();
    let mut reloader = ScriptReloader::new();
    let mut sleeper = ScriptSuspend::new();
    let exit = run_loop(
        &mut w,
        &term,
        &input,
        &mut clock,
        &mut ticker,
        &mut sys,
        &mut reloader,
        &mut sleeper,
        2000,
        3000,
        1,
    );
    assert_eq!(exit, LoopExit::IterationsExhausted);
    assert!(ticker.calls.is_empty(), "menu suppresses the clock tick");
}

// ── main_loop: input ────────────────────────────────────────────────────────

#[test]
fn loop_q_key_quits() {
    let mut w = quiet_world();
    let (_ops, term) = loop_term();
    let input = ScriptInput::new(vec![key("q")]);
    let mut clock = ScriptClock::new(vec![1000, 1000, 1000], 1000, vec![]);
    let mut ticker = ScriptTick::new();
    let mut sys = FakeSys::default();
    let mut reloader = ScriptReloader::new();
    let mut sleeper = ScriptSuspend::new();
    let exit = run_loop(
        &mut w,
        &term,
        &input,
        &mut clock,
        &mut ticker,
        &mut sys,
        &mut reloader,
        &mut sleeper,
        2000,
        3000,
        10,
    );
    assert_eq!(exit, LoopExit::Quit(0));
}

#[test]
fn loop_box_toggle_key_drives_sink_to_forced_tick() {
    let mut w = quiet_world();
    let (_ops, term) = loop_term();
    // `2` → `ToggleBox{index: 2}` (mem): legal on the default world, so
    // the sink demands a redraw and `serve_runs` answers with a forced
    // tick — the full key→sink→tick path, still no quit.
    let input = ScriptInput::new(vec![key("2"), None]);
    let mut clock = ScriptClock::new(vec![1000, 1000, 1000, 1000, 1000], 1000, vec![]);
    let mut ticker = ScriptTick::new();
    let mut sys = FakeSys::default();
    let mut reloader = ScriptReloader::new();
    let mut sleeper = ScriptSuspend::new();
    let exit = run_loop(
        &mut w,
        &term,
        &input,
        &mut clock,
        &mut ticker,
        &mut sys,
        &mut reloader,
        &mut sleeper,
        2000,
        3000,
        1,
    );
    assert_eq!(exit, LoopExit::IterationsExhausted);
    assert!(!w.quit_requested);
    assert_eq!(ticker.calls.len(), 1);
    assert!(ticker.calls[0].0, "redraw demand becomes a forced tick");
    assert!(!w
        .config
        .get_s("shown_boxes")
        .unwrap_or("")
        .split_whitespace()
        .any(|b| b == "mem"));
}

#[test]
fn loop_poll_timeout_breaks_to_next_iteration() {
    let mut w = quiet_world();
    let (_ops, term) = loop_term();
    let input = ScriptInput::new(vec![None, None]);
    let mut clock = ScriptClock::new(vec![1000, 1000, 1000, 1000], 1000, vec![]);
    let mut ticker = ScriptTick::new();
    let mut sys = FakeSys::default();
    let mut reloader = ScriptReloader::new();
    let mut sleeper = ScriptSuspend::new();
    // Two bounded iterations, no flags, no input, no deadline: the loop
    // must return instead of spinning.
    let exit = run_loop(
        &mut w,
        &term,
        &input,
        &mut clock,
        &mut ticker,
        &mut sys,
        &mut reloader,
        &mut sleeper,
        2000,
        3000,
        2,
    );
    assert_eq!(exit, LoopExit::IterationsExhausted);
    assert!(ticker.calls.is_empty());
}

// ── Real* smoke (construction only — no TTY/loop here) ──────────────────────

#[test]
fn real_env_types_construct() {
    let _clock = RealClock::new();
    let _reloader = RealReloader::new(false, None);
    let _suspender = RealSuspend;
}
