//! `btop` binary entry (P4 T7).
//!
//! C++ truth: `btop_main` (src/btop.cpp:824+), boot order
//! `:862-1097`, main loop `:1105-1180` (via [`btop_app::main_loop`]).
//!
//! Order (with cpp lines):
//! `start_time` (:824) → SUID drop (:828-836) → `Cli::parse`
//! (:838-851) → `init_locale` (:933-1000) → `init_config_dirs`
//! (:862-925 + `init_config` :325-353) → signals + atexit (:1048-1065)
//! → `Shared::init` (:1029-1035) → `set_boxes` fallback (:1039-1042)
//! → theme (DEFERRED, see below) → `tty_mode` (:798-815)
//! → preset (:1076-1080) → `Term::init` (:1003-1006) → valid-dim wait
//! (:1015-1027) → boot `term_resize(true)` (:1082-1089, via
//! `min_size_loop`) → `calcSizes` (:1092, via `boot_layout`) → outlines
//! (:1093-1097) → `update_ms`/`future_time` (:1099-1106, via
//! `init_tick_clock`) → `main_loop` → `clean_quit`.
//!
//! DEVIATIONS (all plan-blessed deferreds): `Theme::updateThemes`/
//! `setTheme` (+ `banner_gen`) have no port yet (P4 known-deferred — the
//! per-tick boxes still get color from the compiled-in `Default_theme`
//! the P3 tick builds itself; only the boot outline pre-print below
//! renders uncolored until `World::theme` is filled);
//! `Logger::init` has no port (warnings go to stderr); usage/help/version
//! printers are M4 CLI wiring (`--help`/`--version` exit `0` silently,
//! parse errors exit non-silently with the parser code).

use std::io::Write as _;
use std::sync::Arc;
use std::time::Instant;

use btop_app::boot::{
    apply_preset_default, boot_layout, ensure_boxes, init_config_dirs, init_tick_clock,
    shared_init, RealFs, RealProbe,
};
use btop_app::box_labels::{min_size_loop, print_box_outlines, MinSizeOutcome, RealInputPoll};
use btop_app::clean_quit::clean_quit;
use btop_app::locale::init_locale;
use btop_app::main_loop::{
    main_loop, ClockOps, LoopEnv, LoopExit, RealClock, RealReloader, RealSuspend, RealSys, RealTick,
};
use btop_app::signals::{install_signals, register_atexit, RealAtExit, RealInstaller};
use btop_app::term::TermWrapper;
use btop_config::cli;
use btop_runner::sink::World;

/// `Global::exit_error_msg = …; clean_quit(1)` — the fatal-boot shape used
/// at btop.cpp:1025 (`Failed to get size`), :1032 (`Shared::init`
/// exception), :1005 (`No tty`), :832 (SUID).
fn boot_fail(world: &mut World, term: &TermWrapper, msg: String) -> ! {
    world.exit_error_msg = Some(msg);
    clean_quit(1, world, term);
}

fn main() {
    // btop.cpp:824.
    let mut world = World {
        start_time: Some(Instant::now()),
        ..World::default()
    };

    // The term exists from here on so every `boot_fail` restores it;
    // `clean_quit` skips the restore while it is uninitialized.
    let term = Arc::new(TermWrapper::with_real_ops());

    // btop.cpp:828-836 — drop SUID privileges; skip gracefully when the
    // uids already match (the common case).
    if nix::unistd::getuid() != nix::unistd::geteuid()
        && nix::unistd::seteuid(nix::unistd::getuid()).is_err()
    {
        boot_fail(
            &mut world,
            &term,
            "Failed to change effective user ID. Unset btop SUID bit to ensure security on this system. Quitting!"
                .to_string(),
        );
    }

    // btop.cpp:838-851. Printers are M4 wiring — exit with the parser
    // code (`0` for `--help`/`--version`/`--default-config`).
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = match cli::parse(&args) {
        Ok(cli) => cli,
        Err(code) => std::process::exit(code),
    };

    // btop.cpp:933-1000. `init_locale` owns the hunt; the caller owns
    // `exit_error_msg` + the quit (the `Err` already carries the verbatim
    // message).
    if let Err(msg) = init_locale(&mut world, cli.force_utf) {
        boot_fail(&mut world, &term, msg);
    }

    // btop.cpp:862-925 + `init_config` (:325-353). No logger port —
    // warnings go to stderr.
    match init_config_dirs(&mut world, &cli, &RealFs) {
        Ok(warnings) => {
            for w in warnings {
                eprintln!("{w}");
            }
        }
        Err(e) => boot_fail(&mut world, &term, e),
    }

    // btop.cpp:1048-1065. The installed `Arc` clones `World::signal_flags`
    // (all `SignalFlags` fields are `Arc` atomics, so the clone shares
    // every flag with the loop's copy); the installer pins the clones in
    // process-lifetime statics.
    let flags = Arc::new(world.signal_flags.clone());
    let installer = RealInstaller::with_term(term.clone());
    if let Err(e) = install_signals(&installer, flags.clone()) {
        boot_fail(&mut world, &term, e);
    }
    // `std::atexit(_exit_handler)` (:1048): the hook only restores the
    // term + marks `quitting` (abnormal-exit fallback — every normal exit
    // goes through the real `clean_quit`, which `_exit`s past `atexit`).
    if let Err(e) = register_atexit(&RealAtExit, flags.clone(), term.clone()) {
        eprintln!("atexit registration failed: {e}");
    }

    // btop.cpp:1029-1035 (`Shared::init` is inside try/catch).
    if let Err(e) = shared_init(&mut world, &RealProbe) {
        boot_fail(
            &mut world,
            &term,
            format!("Exception in Shared::init() -> {e}"),
        );
    }

    // btop.cpp:1039-1042.
    ensure_boxes(&mut world);

    // btop.cpp:1045-1046 (`Theme::updateThemes` + `setTheme`) — DEFERRED
    // (no theme-file port yet; the tick builds the compiled-in Default
    // theme itself, so only the outline pre-print below loses color).

    // `configure_tty_mode` (btop.cpp:798-815). The `/dev/tty` autodetect
    // is Linux-only upstream (`#if !defined(__APPLE__) …`), so macOS skips
    // it here too.
    if let Some(force) = cli.force_tty {
        let _ = world.config.set_b("tty_mode", force);
    } else if world.config.get_b("force_tty").unwrap_or(false) {
        let _ = world.config.set_b("tty_mode", true);
    }

    // btop.cpp:1076-1080.
    apply_preset_default(&mut world, cli.preset);

    // btop.cpp:1003-1006.
    if !term.init() {
        boot_fail(
            &mut world,
            &term,
            "No tty detected!\nbtop++ needs an interactive shell to run.".to_string(),
        );
    }

    // btop.cpp:1015-1027. DEVIATION (brief-mandated): C++ sleeps 10ms ×
    // up to 100 retries; the brief prescribes 100ms × 10 here.
    let mut t_count = 0;
    while term.width() == 0
        || u32::from(term.width()) > 10000
        || term.height() == 0
        || u32::from(term.height()) > 10000
    {
        std::thread::sleep(std::time::Duration::from_millis(100));
        term.refresh(false);
        t_count += 1;
        if t_count == 10 {
            boot_fail(
                &mut world,
                &term,
                "Failed to get size of terminal!".to_string(),
            );
        }
    }

    // btop.cpp:1082-1089 — the boot-time `term_resize(true)`: run the
    // too-small screen before geometry is computed.
    {
        let input = RealInputPoll::new();
        let mut screen = String::new();
        match min_size_loop(&mut world, &term, &input, &mut screen) {
            MinSizeOutcome::Ready => {}
            // btop.cpp:190 (`q` → `clean_quit(0)`).
            MinSizeOutcome::QuitRequested => clean_quit(0, &mut world, &term),
        }
        if !screen.is_empty() {
            print!("{screen}");
            let _ = std::io::stdout().flush();
        }
    }

    // btop.cpp:1092.
    boot_layout(
        &mut world,
        i64::from(term.width()),
        i64::from(term.height()),
    );

    // btop.cpp:1093-1097.
    {
        let mut out = String::new();
        print_box_outlines(&world, &mut out);
        print!("{out}");
        let _ = std::io::stdout().flush();
    }

    // btop.cpp:1099-1106.
    let mut clock = RealClock::new();
    let (update_ms, future_time) = init_tick_clock(&mut world, cli.updates, clock.now_ms());

    // btop.cpp:1105-1180. `main_loop` returns (C++ never does); map the
    // outcome to the real `clean_quit`, which `_exit`s.
    let input = RealInputPoll::new();
    let mut ticker = RealTick::new();
    let mut sys = RealSys;
    let mut reloader = RealReloader::new(cli.low_color, cli.filter);
    let mut sleeper = RealSuspend;
    let env = LoopEnv {
        term: &term,
        input: &input,
        clock: &mut clock,
        ticker: &mut ticker,
        sys: &mut sys,
        reloader: &mut reloader,
        sleeper: &mut sleeper,
        max_iters: None,
    };
    match main_loop(&mut world, env, update_ms, future_time) {
        // C++ calls `clean_quit(0)` on every loop-produced quit path.
        LoopExit::Quit(sig) => clean_quit(sig, &mut world, &term),
        // Unreachable in production (`max_iters` is `None`) — same shape
        // as the atexit path.
        LoopExit::IterationsExhausted => clean_quit(-1, &mut world, &term),
    }
}
