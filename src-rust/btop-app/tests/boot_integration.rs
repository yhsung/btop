//! T6 headless boot-path tests (P4 Task 6).
//!
//! No test touches a real terminal, stdin, signal handlers, or the home
//! directory: filesystem/env probing goes through local `MockFs`/`MockProbe`
//! doubles, `TermWrapper` runs on scripted [`TermOps`] impls, and the input
//! poll seam runs on scripted [`InputPoll`] queues. The one real sleep is
//! C++'s `sleep_ms(100)` per too-small iteration (btop.cpp:159) — every
//! script below terminates in ≤ 2 iterations.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use btop_app::boot::{apply_preset_default, init_config_dirs, init_tick_clock, shared_init};
use btop_app::box_labels::{
    box_number, gpu_box_label, min_size_loop, print_box_outlines, InputPoll, MinSizeOutcome,
    BOX_LABELS,
};
use btop_app::locale::{hunt_locale, HuntLog, LocaleOps};
use btop_app::signals::{install_signals, MockInstaller, SignalFlags};
use btop_app::term::{TermOps, TermWrapper};
use btop_config::cli::Cli;
use btop_runner::sink::World;

// ── MockFs ──────────────────────────────────────────────────────────────────

struct MockFs {
    config_base: Option<PathBuf>,
    exe_dir: Option<PathBuf>,
    readable: HashSet<PathBuf>,
}

impl btop_app::boot::FsOps for MockFs {
    fn config_base(&self) -> Option<PathBuf> {
        self.config_base.clone()
    }
    fn create_dir_all(&self, _path: &Path) -> Result<(), String> {
        Ok(())
    }
    fn is_readable_dir(&self, path: &Path) -> bool {
        self.readable.contains(path)
    }
    fn exe_dir(&self) -> Option<PathBuf> {
        self.exe_dir.clone()
    }
}

// ── MockProbe ───────────────────────────────────────────────────────────────

struct MockProbe;

impl btop_app::boot::SysProbe for MockProbe {
    fn core_count(&self) -> i64 {
        8
    }
    fn page_size(&self) -> i64 {
        16384
    }
    fn mach_tick(&self) -> f64 {
        41.666
    }
    fn clk_tick(&self) -> i64 {
        100
    }
    fn total_mem(&self) -> u64 {
        16 * 1024 * 1024 * 1024
    }
    fn cpu_name_raw(&self) -> String {
        "Apple M2".to_string()
    }
}

// ── ScriptTerm ──────────────────────────────────────────────────────────────

/// Scripted [`TermOps`]: `width`/`height` are live cells (the wrapper caches
/// them on `init`/changed-`refresh`); each `refresh` pops a
/// `(return, dims-to-install)` step, installing dims *before* returning so a
/// `true` makes the wrapper resync to the new size (mirrors C++ globals
/// updated by `Term::refresh`).
struct RefreshStep {
    ret: bool,
    dims: Option<(u16, u16)>,
}

struct ScriptTerm {
    width: AtomicU16,
    height: AtomicU16,
    min: (u16, u16),
    init_ret: AtomicBool,
    initialized: AtomicBool,
    refresh_calls: AtomicU32,
    steps: Mutex<VecDeque<RefreshStep>>,
}

impl ScriptTerm {
    fn new(w: u16, h: u16, min: (u16, u16)) -> Self {
        Self {
            width: AtomicU16::new(w),
            height: AtomicU16::new(h),
            min,
            init_ret: AtomicBool::new(true),
            initialized: AtomicBool::new(false),
            refresh_calls: AtomicU32::new(0),
            steps: Mutex::new(VecDeque::new()),
        }
    }

    fn push_refresh(self, ret: bool, dims: Option<(u16, u16)>) -> Self {
        self.steps
            .lock()
            .unwrap()
            .push_back(RefreshStep { ret, dims });
        self
    }
}

impl TermOps for ScriptTerm {
    fn init(&self) -> bool {
        let ok = self.init_ret.load(Ordering::Relaxed);
        self.initialized.store(ok, Ordering::Relaxed);
        ok
    }
    fn refresh(&self, _only_check: bool) -> bool {
        self.refresh_calls.fetch_add(1, Ordering::Relaxed);
        let step = self.steps.lock().unwrap().pop_front();
        match step {
            Some(s) => {
                if let Some((w, h)) = s.dims {
                    self.width.store(w, Ordering::Relaxed);
                    self.height.store(h, Ordering::Relaxed);
                }
                s.ret
            }
            None => false,
        }
    }
    fn restore(&self) {
        self.initialized.store(false, Ordering::Relaxed);
    }
    fn get_min_size(&self) -> (u16, u16) {
        self.min
    }
    fn width(&self) -> u16 {
        self.width.load(Ordering::Relaxed)
    }
    fn height(&self) -> u16 {
        self.height.load(Ordering::Relaxed)
    }
    fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Relaxed)
    }
}

// ── ScriptPoll / PanicPoll ──────────────────────────────────────────────────

struct PollStep {
    hit: bool,
    key: String,
}

struct ScriptPoll {
    steps: Mutex<VecDeque<PollStep>>,
    calls: AtomicU32,
    pending: Mutex<String>,
}

impl ScriptPoll {
    fn new(steps: Vec<PollStep>) -> Self {
        Self {
            steps: Mutex::new(steps.into()),
            calls: AtomicU32::new(0),
            pending: Mutex::new(String::new()),
        }
    }
}

impl InputPoll for ScriptPoll {
    fn poll(&self, _timeout_ms: u64) -> bool {
        self.calls.fetch_add(1, Ordering::Relaxed);
        // Exhausted queue degrades to `q` so a mis-scripted loop quits
        // instead of spinning forever (tests asserting `Ready` then fail).
        let step = self.steps.lock().unwrap().pop_front().unwrap_or(PollStep {
            hit: true,
            key: "q".to_string(),
        });
        if step.hit {
            *self.pending.lock().unwrap() = step.key;
        }
        step.hit
    }
    fn get(&self) -> String {
        std::mem::take(&mut *self.pending.lock().unwrap())
    }
}

/// Proves the size-ok path never touches input (zero polls).
struct PanicPoll;

impl InputPoll for PanicPoll {
    fn poll(&self, _timeout_ms: u64) -> bool {
        panic!("min_size_loop polled stdin on a fitting terminal")
    }
    fn get(&self) -> String {
        panic!("min_size_loop read a key on a fitting terminal")
    }
}

// ── Label unit coverage ─────────────────────────────────────────────────────

#[test]
fn labels_match_draw_titles_and_gpu_is_dynamic() {
    assert_eq!(
        BOX_LABELS,
        &[
            ("cpu", "cpu"),
            ("mem", "mem"),
            ("net", "net"),
            ("proc", "proc")
        ]
    );
    assert_eq!(box_number("cpu"), 1);
    assert_eq!(box_number("mem"), 2);
    assert_eq!(box_number("net"), 3);
    assert_eq!(box_number("proc"), 4);
    assert_eq!(box_number("gpu0"), 5);
    assert_eq!(box_number("gpu4"), 9);
    assert_eq!(box_number("gpu5"), 0);
    assert_eq!(box_number("nope"), 0);
    assert_eq!(gpu_box_label(2), "gpu2");
}

// ── Outline goldens ─────────────────────────────────────────────────────────

fn cpu_only_world() -> World {
    let mut w = World::default();
    w.config.strings.insert("shown_boxes".into(), "cpu".into());
    w.config.bools.insert("tty_mode".into(), false);
    w.config.bools.insert("rounded_corners".into(), true);
    w.config.bools.insert("lowcolor".into(), false);
    w.config.bools.insert("theme_background".into(), true);
    w.config.bools.insert("cpu_bottom".into(), false);
    w.layout.cpu.base.x = 1;
    w.layout.cpu.base.y = 1;
    w.layout.cpu.base.width = 30;
    w.layout.cpu.base.height = 10;
    w.layout.cpu.base.shown = true;
    w
}

#[test]
fn outlines_hidden_by_default_emit_nothing() {
    let w = World::default();
    let mut out = String::new();
    print_box_outlines(&w, &mut out);
    assert!(out.is_empty(), "unexpected outline bytes: {out:?}");
}

#[test]
fn outlines_sync_pair_only_when_terminal_sync() {
    let w = World {
        terminal_sync: true,
        ..World::default()
    };
    let mut out = String::new();
    print_box_outlines(&w, &mut out);
    assert_eq!(out, "\x1b[?2026h\x1b[?2026l");
}

#[test]
fn outlines_cpu_golden() {
    let w = cpu_only_world();
    let mut out = String::new();
    print_box_outlines(&w, &mut out);
    assert!(!out.is_empty());
    assert!(out.contains("cpu"), "outline must carry its title: {out:?}");
    // Recorded golden: `create_box(1, 1, 30, 10, "", fill, "cpu", "", 1,
    // "", "", "", "\x1b[0m", tty=false, rounded=true)` with an empty theme
    // (all palette entries resolve to `""`). Re-record by hand if
    // `create_box` changes — this test pins the exact boot-print bytes.
    const EXPECTED: &str = "\x1b[0m\x1b[1;1f─────────────────────────────\x1b[10;1f─────────────────────────────\x1b[2;1f│                            │\x1b[3;1f│                            │\x1b[4;1f│                            │\x1b[5;1f│                            │\x1b[6;1f│                            │\x1b[7;1f│                            │\x1b[8;1f│                            │\x1b[9;1f│                            │\x1b[1;1f╭\x1b[1;30f╮\x1b[10;1f╰\x1b[10;30f╯\x1b[1;3f┐\x1b[1m¹cpu\x1b[22m┌\x1b[0m\x1b[2;2f";
    assert_eq!(out, EXPECTED);
}

// ── min_size_loop scripts ───────────────────────────────────────────────────

fn world_with_boxes(shown: &str) -> World {
    let mut w = World::default();
    w.config.strings.insert("shown_boxes".into(), shown.into());
    w
}

#[test]
fn min_size_ready_immediately_without_polling() {
    let mut w = world_with_boxes("cpu mem net proc");
    let ops = Arc::new(ScriptTerm::new(200, 50, (100, 24)));
    let term = TermWrapper::new(ops);
    assert!(term.init());
    let mut out = String::new();
    let outcome = min_size_loop(&mut w, &term, &PanicPoll, &mut out);
    assert_eq!(outcome, MinSizeOutcome::Ready);
    assert!(out.is_empty());
}

#[test]
fn min_size_q_requests_quit() {
    let mut w = world_with_boxes("cpu mem net proc");
    let ops = Arc::new(ScriptTerm::new(80, 24, (100, 24)).push_refresh(false, None));
    let term = TermWrapper::new(ops);
    assert!(term.init());
    let poll = ScriptPoll::new(vec![PollStep {
        hit: true,
        key: "q".to_string(),
    }]);
    let mut out = String::new();
    let outcome = min_size_loop(&mut w, &term, &poll, &mut out);
    assert_eq!(outcome, MinSizeOutcome::QuitRequested);
    assert!(out.contains("Terminal size too small:"));
    assert!(out.contains("Needed for current config:"));
    assert_eq!(poll.calls.load(Ordering::Relaxed), 1);
}

#[test]
fn min_size_digit_toggles_box_then_ready_on_resize() {
    let mut w = world_with_boxes("cpu mem net proc");
    w.current_preset = Some(0);
    let ops = Arc::new(
        ScriptTerm::new(80, 24, (100, 24))
            .push_refresh(false, None)
            .push_refresh(true, Some((200, 50))),
    );
    let term = TermWrapper::new(ops);
    assert!(term.init());
    // "2" → ALL_BOXES[2] ("mem"); the resize arrives with the 2nd refresh.
    let poll = ScriptPoll::new(vec![PollStep {
        hit: true,
        key: "2".to_string(),
    }]);
    let mut out = String::new();
    let outcome = min_size_loop(&mut w, &term, &poll, &mut out);
    assert_eq!(outcome, MinSizeOutcome::Ready);
    assert_eq!(w.config.get_s("shown_boxes"), Some("cpu net proc"));
    assert_eq!(w.current_preset, None);
    assert_eq!(term.width(), 200);
    assert_eq!(term.height(), 50);
}

#[test]
fn min_size_resize_without_key_gets_ready() {
    let mut w = world_with_boxes("cpu mem net proc");
    let ops = Arc::new(
        ScriptTerm::new(80, 24, (100, 24))
            .push_refresh(false, None)
            .push_refresh(true, Some((120, 30))),
    );
    let term = TermWrapper::new(ops);
    assert!(term.init());
    let poll = ScriptPoll::new(vec![PollStep {
        hit: false,
        key: String::new(),
    }]);
    let mut out = String::new();
    let outcome = min_size_loop(&mut w, &term, &poll, &mut out);
    assert_eq!(outcome, MinSizeOutcome::Ready);
    assert_eq!(poll.calls.load(Ordering::Relaxed), 1);
    assert_eq!(w.config.get_s("shown_boxes"), Some("cpu mem net proc"));
}

#[test]
fn min_size_illegal_digit_is_ignored() {
    // gpu_count 0: index 0 ("gpu5") is illegal (needs 5+ GPUs) and index 9
    // ("gpu4") is illegal (needs count ≥ 5); both must leave config alone.
    // The queue-exhaustion `q` fallback then quits the loop deterministically.
    let mut w = world_with_boxes("cpu mem net proc");
    w.gpu_count = 0;
    let ops = Arc::new(ScriptTerm::new(80, 24, (100, 24)).push_refresh(false, None));
    let term = TermWrapper::new(ops);
    assert!(term.init());
    let poll = ScriptPoll::new(vec![
        PollStep {
            hit: true,
            key: "0".to_string(),
        },
        PollStep {
            hit: true,
            key: "9".to_string(),
        },
    ]);
    let mut out = String::new();
    let outcome = min_size_loop(&mut w, &term, &poll, &mut out);
    assert_eq!(outcome, MinSizeOutcome::QuitRequested);
    assert_eq!(w.config.get_s("shown_boxes"), Some("cpu mem net proc"));
}

// ── Full headless boot path ─────────────────────────────────────────────────

/// `setlocale` stub stuck on UTF-8: branch 1 of the hunt adopts it with no
/// env scan.
struct Utf8Ops;

impl LocaleOps for Utf8Ops {
    fn set_all(&self, locale: &str) -> Option<String> {
        if locale.is_empty() {
            Some("en_US.UTF-8".to_string())
        } else {
            Some(locale.to_string())
        }
    }
    fn std_locale_name(&self) -> Option<String> {
        None
    }
    fn macos_locale_id(&self) -> Option<String> {
        None
    }
}

#[test]
fn boot_integration_headless_full_path() {
    let mut w = World::default();

    // 1. Config dirs (mock fs; exe dir points nowhere so no theme fallback).
    let fs = MockFs {
        config_base: Some(PathBuf::from("/cfg")),
        exe_dir: Some(PathBuf::from("/nonexistent-bin")),
        readable: HashSet::new(),
    };
    let cli = Cli::default();
    let warnings = init_config_dirs(&mut w, &cli, &fs).expect("mock fs never fails");
    assert_eq!(w.conf_file, PathBuf::from("/cfg/btop.conf"));
    assert_eq!(w.conf_dir, PathBuf::from("/cfg"));
    assert!(w.theme_dir.as_os_str().is_empty());
    let _ = warnings;

    // 2. Platform probe.
    shared_init(&mut w, &MockProbe).expect("probes always fall back");
    assert_eq!(w.state.core_count, 8);
    assert_eq!(w.state.page_size, 16384);
    assert!(w.state.tick_factor > 0.0);
    assert_eq!(w.state.total_mem, 16 * 1024 * 1024 * 1024);
    assert!(!w.state.cpu_name.is_empty());

    // 3. No `--preset` requested: untouched.
    assert!(!apply_preset_default(&mut w, None));
    assert_eq!(w.current_preset, None);

    // 4. Locale hunt (fake env + UTF-8 stub ops).
    let env = [("LANG".to_string(), "C".to_string())];
    let get = |k: &str| env.iter().find(|(ek, _)| ek == k).map(|(_, v)| v.clone());
    let mut set = |_k: &str, _v: &str| true;
    let mut log = HuntLog::default();
    let found = hunt_locale(&get, &mut set, &Utf8Ops, false, false, &mut log)
        .expect("stub locale always adopts");
    assert_eq!(found, "en_US.UTF-8");

    // 5. Signal install (mock).
    let installer = MockInstaller::new();
    let flags = Arc::new(SignalFlags::default());
    install_signals(&installer, flags).expect("mock installer succeeds");
    assert_eq!(installer.calls.load(Ordering::SeqCst), 1);

    // 6. Term init (mock, already big enough).
    let ops = Arc::new(ScriptTerm::new(200, 50, (100, 24)));
    let term = TermWrapper::new(ops);
    assert!(term.init());
    assert!(term.is_initialized());

    // 7. Size gate passes without touching input.
    let mut screen = String::new();
    assert_eq!(
        min_size_loop(&mut w, &term, &PanicPoll, &mut screen),
        MinSizeOutcome::Ready
    );

    // 8. Tick clock seed (cpp:1099-1106): `--updates 500` lands in config,
    //    `future_time` pins `next_tick_ms`.
    let (update_ms, future_time) = init_tick_clock(&mut w, Some(500), 42_000);
    assert_eq!((update_ms, future_time), (500, 42_000));
    assert_eq!(w.config.get_i("update_ms"), Some(500));
    assert_eq!(w.next_tick_ms, 42_000);

    // 9. Ready-to-tick: outlines render for a shown box.
    w.layout.cpu.base.width = 30;
    w.layout.cpu.base.height = 10;
    w.layout.cpu.base.shown = true;
    let mut outlines = String::new();
    print_box_outlines(&w, &mut outlines);
    assert!(outlines.contains("cpu"));
    assert!(!w.quit_requested);
}
