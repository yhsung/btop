//! `clean_quit` (P4 T7).
//!
//! C++ truth: `src/btop.cpp:211-261` (`clean_quit`), `:264-274` (`_sleep` /
//! `_resume`, folded into the main-loop suspend path — see
//! [`crate::main_loop`]), `:276-278` (`_exit_handler` → `clean_quit(-1)`).
//!
//! [`clean_quit_impl`] is the testable body: it performs every C++ step and
//! returns the exit code instead of exiting, so tests assert on effects
//! (config write, term restore, stderr text, exit code) without dying. The
//! production [`clean_quit`] wrapper calls it and ends in `libc::_exit`
//! (mirroring C++ `:256-260`, which deliberately bypasses `atexit` via
//! `_Exit`/`quick_exit` — `std::process::exit` would re-run `atexit`
//! handlers and recurse, since the T7 atexit hook is registered with real
//! `atexit`).

use btop_runner::sink::World;

use crate::term::TermWrapper;

// ── QuitEnv ─────────────────────────────────────────────────────────────────

/// Mockable side-effect seam for [`clean_quit_impl`]. Production boot passes
/// [`RealQuitEnv`]; tests pass a recording double.
pub trait QuitEnv: Send + Sync {
    /// `Input::clear` (btop.cpp:242): drain queued input.
    fn clear_input(&mut self);
    /// `Config::write` (btop.cpp:237-239): persist config. The caller
    /// already checked `save_config_on_exit` — this runs unconditionally.
    fn save_config(&mut self, world: &mut World);
    /// Sink for the red `ERROR:` line (btop.cpp:250, `cerr`).
    fn emit_stderr(&mut self, s: &str);
    /// Sink for the runtime line (btop.cpp:252, `Logger::info`; no logger
    /// port exists, so the production impl prints it after `Term::restore`
    /// when the terminal is back in cooked mode).
    fn emit_stdout(&mut self, s: &str);
}

/// Production [`QuitEnv`]: drains stdin, writes `World::conf_file`, prints
/// to the real stdio.
pub struct RealQuitEnv;

impl QuitEnv for RealQuitEnv {
    fn clear_input(&mut self) {
        // `Input::clear` (src/btop_input.cpp) drops the queued key bytes.
        // The port keeps no global input queue (`RealInputPoll` owns a
        // per-instance buffer), so drain whatever stdin already holds with
        // non-blocking-shaped zero-timeout polls (a single pass suffices —
        // quit is not a steady state).
        use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
        use std::os::fd::BorrowedFd;
        // SAFETY: fd 0 is stdin for process lifetime; `poll`/`read` take
        // no ownership.
        let fd = unsafe { BorrowedFd::borrow_raw(0) };
        let mut fds = [PollFd::new(fd, PollFlags::POLLIN)];
        while poll(&mut fds, PollTimeout::ZERO).unwrap_or(0) > 0 {
            let mut buf = [0u8; 1024];
            let n = unsafe { nix::libc::read(0, buf.as_mut_ptr().cast(), buf.len()) };
            if n <= 0 {
                break;
            }
            if let Some(revents) = fds[0].revents() {
                if !revents.contains(PollFlags::POLLIN) {
                    break;
                }
            }
        }
    }

    fn save_config(&mut self, world: &mut World) {
        // `Config::write` no-ops on an empty path (btop_config.cpp:837);
        // surface real I/O failures on stderr (C++ has a `TODO` there and
        // reports nothing — printing is arguably a fix, kept to one line).
        if let Err(e) = world.config.write(&world.conf_file) {
            eprintln!("Failed to write config: {e}");
        }
    }

    fn emit_stderr(&mut self, s: &str) {
        eprint!("{s}");
    }

    fn emit_stdout(&mut self, s: &str) {
        println!("{s}");
    }
}

// ── Outcome ─────────────────────────────────────────────────────────────────

/// What [`clean_quit_impl`] did. `None` (the return, not this struct) means
/// the `quitting` guard was already set — C++ `:212` early return.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuitOutcome {
    /// `(sig != -1 ? sig : 0)` (btop.cpp:254), with `exit_error_msg`
    /// forcing `sig = 1` (:247).
    pub excode: i32,
    /// The red error line, if `exit_error_msg` was set.
    pub error_line: Option<String>,
    /// `Quitting! Runtime: <dhms>` (btop.cpp:252).
    pub runtime_line: String,
}

/// `Global::fg_red / fg_white` (btop.cpp:102-103) + `Fx::reset` for the
/// error line (btop.cpp:250).
pub const FG_RED: &str = "\x1b[1;91m";
/// See [`FG_RED`].
pub const FG_WHITE: &str = "\x1b[1;97m";
/// See [`FG_RED`].
pub const FX_RESET: &str = "\x1b[0m";

// ── Body ────────────────────────────────────────────────────────────────────

/// Testable `clean_quit` body, transcribing btop.cpp:211-261 step by step:
///
/// | C++ lines | Step | Port |
/// |---|---|---|
/// | :212-213 | `quitting` guard | `None` when already set (C++ returns `void`) |
/// | :214 | `Runner::stop()` | no-op: single-threaded, the loop is always "stopped" once it returns the outcome |
/// | :215-224 | `pthread_join` | no-op: no `_runner` thread (`_runner_started` never set) |
/// | :226-234 | `Gpu::*::shutdown` | no-op stub: AppleSilicon sensor backend not yet ported (plan P4 known-deferred) |
/// | :237-239 | `Config::write` iff `save_config_on_exit` | `env.save_config` under the same gate |
/// | :241-244 | `Input::clear` + `Term::restore` iff initialized | `env.clear_input` + `term.restore` under `is_initialized` |
/// | :246-251 | `exit_error_msg` → `sig = 1`, red `ERROR:` on `cerr` | same, via `env.emit_stderr` |
/// | :252 | runtime `Logger::info` | `runtime_line` via `env.emit_stdout` (no logger port) |
/// | :254 | `excode = (sig != -1 ? sig : 0)` | returned in [`QuitOutcome`] (the caller exits) |
/// | :256-260 | `_Exit`/`quick_exit` | [`clean_quit`], deliberately NOT here (tests must survive) |
pub fn clean_quit_impl(
    sig: i32,
    world: &mut World,
    term: &TermWrapper,
    env: &mut dyn QuitEnv,
) -> Option<QuitOutcome> {
    // btop.cpp:212-213.
    if world
        .signal_flags
        .quitting
        .swap(true, std::sync::atomic::Ordering::SeqCst)
    {
        return None;
    }
    // btop.cpp:214 (`Runner::stop`), :215-224 (`pthread_join`),
    // :226-234 (`Gpu::*::shutdown`): all no-ops — see the table above.

    // btop.cpp:237-239.
    if world.config.get_b("save_config_on_exit").unwrap_or(false) {
        env.save_config(world);
    }

    // btop.cpp:241-244.
    if term.is_initialized() {
        env.clear_input();
        term.restore();
    }

    // btop.cpp:246-251.
    let mut sig = sig;
    let error_line = world
        .exit_error_msg
        .clone()
        .filter(|m| !m.is_empty())
        .map(|m| {
            sig = 1;
            format!("{FG_RED}ERROR: {FG_WHITE}{m}{FX_RESET}\n")
        });
    if let Some(line) = &error_line {
        env.emit_stderr(line);
    }

    // btop.cpp:252 (`time_s() - start_time`, seconds granularity).
    let runtime_secs = world.start_time.map(|t| t.elapsed().as_secs()).unwrap_or(0);
    let runtime_line = format!(
        "Quitting! Runtime: {}",
        btop_tools::strtools::sec_to_dhms(runtime_secs, false, false)
    );
    env.emit_stdout(&runtime_line);

    // btop.cpp:254.
    Some(QuitOutcome {
        excode: if sig != -1 { sig } else { 0 },
        error_line,
        runtime_line,
    })
}

/// Production entry: run [`clean_quit_impl`] with [`RealQuitEnv`] and
/// `_exit` with the outcome code (btop.cpp:256-260). Never returns; a
/// `None` (re-entrant call) exits `0` — the first call already performed
/// cleanup and owns the real code.
pub fn clean_quit(sig: i32, world: &mut World, term: &TermWrapper) -> ! {
    let mut env = RealQuitEnv;
    let excode = clean_quit_impl(sig, world, term, &mut env).map_or(0, |o| o.excode);
    // SAFETY: `_exit` neither runs `atexit` handlers nor returns — exactly
    // C++ `:257` (`_Exit` on Apple). `std::process::exit` is WRONG here:
    // it runs `atexit`, and the T7 atexit hook is a real `atexit` entry,
    // so exiting through it would re-enter the quit path.
    unsafe { nix::libc::_exit(excode) }
}
