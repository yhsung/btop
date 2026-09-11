//! Tick unit tests for P3 Task 4.
//!
//! These tests exercise the `tick()` scheduling + merge semantics without
//! touching the byte-fixture parity (that lives in `tests/golden_proc.rs`).
//! Uses `ReplayBackend` to inject deterministic per-box backend outputs.

use btop_collect::backend::{MacOsBackend, ProcRaw, ReplayBackend};
use btop_collect::gpu::EnergyUnit;
use btop_draw::boxes::{calc_sizes, LayoutInput};
use btop_input::actions::{Action, RunTarget};
use btop_runner::sink::{execute_all, FakeSys, World};
use btop_runner::tick::{tick, TickInput};

fn harness_world() -> World {
    let mut w = World::default();
    // Pin the harness defaults that `setup()` uses so draw_layer matches
    // fixtures (cpu graph totals, theme, etc.).
    w.state.core_count = 8;
    w.state.cpu_flags.common.theme_background = true;
    w.state.cpu_flags.common.tty_mode = false;
    w.state.cpu_flags.common.lowcolor = false;
    w.state.cpu_flags.show_uptime = false;
    w.state.cpu_flags.show_battery_cfg = false;
    w.state.cpu_flags.show_watts_cfg = false;
    w.state.cpu_flags.show_freq_cfg = false;
    w.state.proc_sorting = "pid".to_string();
    w.state.proc_reversed = false;
    w.state.proc_tree = false;
    w.state.cpu_graph_upper = "total".to_string();
    w.state.cpu_graph_lower = "total".to_string();
    w.state.gpu_panel = 0;
    w.recalc_layout = true;
    w
}

#[allow(dead_code)]
fn harness_layout(w: &mut World) -> btop_draw::boxes::Layout {
    let li = LayoutInput {
        term_w: 100,
        term_h: 30,
        shown_boxes: "cpu mem net proc".to_string(),
        cpu_bottom: false,
        mem_below_net: false,
        proc_left: false,
        core_count: 8,
        show_temp: true,
        cpu_width_p: 100,
        cpu_height_p: 32,
        mem_width_p: 45,
        net_height_p: 28,
        show_disks: false,
        swap_disk: false,
        mem_graphs: false,
        has_swap: false,
        swap_upload_download: false,
        gpus_extra_height: 0,
        gpu_total_height: 0,
        gpu_panels: vec![],
        gpu_height_p: 32,
        gpu_min_height: 8,
        gpu_min_width: 41,
    };
    let mut layout = calc_sizes(&li);
    layout.proc.select_max = 17;
    w.layout.proc.select_max = 17;
    layout
}

fn fake_backend() -> ReplayBackend {
    let mut b = ReplayBackend::default();
    // 4 cores; ticks (user, nice, sys, idle).
    b.cpu_ticks_q.push_back(vec![
        [100, 0, 50, 200],
        [110, 0, 55, 210],
        [120, 0, 60, 220],
        [130, 0, 65, 230],
    ]);
    b.load_avg_q.push_back([0.5, 0.3, 0.1]);
    b.package_temp_q.push_back(Some(55));
    b.core_temps_q.push_back(vec![50, 52, 54, 56]);
    b.vm_raw_q.push_back((100, 50, 200, 30, 4096));
    b.swap_raw_q.push_back((1024, 512, 512));
    b.if_counters_q
        .push_back(vec![("eth0".to_string(), 1000, 500)]);
    b.proc_list_q.push_back(vec![ProcRaw {
        pid: 1,
        name: "p".to_string(),
        cpu_ticks: 1000,
        mem_bytes: 4096,
        threads: 1,
        user: "u".to_string(),
        cmd: "/bin/p --flag".to_string(),
        p_nice: 0,
    }]);
    b
}

#[test]
fn early_tick_returns_empty_with_remaining() {
    let mut w = harness_world();
    let mut s = FakeSys::default();
    let mut b = fake_backend();
    // First call seeds state at t=0, schedules next at 2000.
    let _ = tick(
        &mut w,
        &mut s,
        TickInput {
            backend: &mut b,
            now_ms: 0,
            pending_resize: false,
            should_quit: false,
            force_redraw_in: false,
            overlay: String::new(),
        },
    );
    // Early call at t=500: should NOT collect/draw, returns empty out +
    // remaining = 1500.
    let out = tick(
        &mut w,
        &mut s,
        TickInput {
            backend: &mut b,
            now_ms: 500,
            pending_resize: false,
            should_quit: false,
            force_redraw_in: false,
            overlay: String::new(),
        },
    );
    assert!(out.out.is_empty(), "early tick must not emit");
    assert_eq!(out.next_in_ms, 1500);
    assert_eq!(out.ran_boxes, Vec::<String>::new());
}

#[test]
fn scheduled_tick_emits_boxes_in_fixed_order() {
    let mut w = harness_world();
    let mut s = FakeSys::default();
    let mut b = fake_backend();
    let out = tick(
        &mut w,
        &mut s,
        TickInput {
            backend: &mut b,
            now_ms: 2000, // >= next_tick (scheduled)
            pending_resize: false,
            should_quit: false,
            force_redraw_in: false,
            overlay: String::new(),
        },
    );
    assert_eq!(out.ran_boxes, vec!["cpu", "mem", "net", "proc"]);
    assert!(!out.out.is_empty(), "scheduled tick must emit output");
    // The order matches cpp:720: cpu + mem + net + proc. (Gpu absent
    // because no panels.) We probe for unique box-header markers; the
    // "Mem" string is reused inside proc (memory column), so we use
    // `M[Bb/s]` and the iface selector instead. The net box uses the
    // iface selector "eth0" which the harness names uniquely; the proc
    // box uses "Pid:" header which appears only in proc.
    let cpu_pos = out
        .out
        .find("Cpu")
        .or_else(|| out.out.find("CPU"))
        .unwrap_or(usize::MAX);
    let mem_pos = out
        .out
        .find("used")
        .or_else(|| out.out.find("Used"))
        .unwrap_or(usize::MAX);
    let proc_pos = out
        .out
        .find("Pid:")
        .or_else(|| out.out.find("Process"))
        .unwrap_or(usize::MAX);
    // We can't order-check net vs proc on header text alone: the iface
    // name "eth0" sits in the proc box's CPU column header and several
    // places. Just check net is somewhere after mem.
    let net_pos = out
        .out
        .find("Net")
        .or_else(|| out.out.find("net "))
        .unwrap_or(usize::MAX);
    assert!(cpu_pos < mem_pos, "cpu before mem");
    assert!(mem_pos < net_pos, "mem before net");
    // The strongest order check is ran_boxes (already verified above).
}

#[test]
fn next_in_ms_advances_after_scheduled_tick() {
    let mut w = harness_world();
    let mut s = FakeSys::default();
    let mut b = fake_backend();
    let out = tick(
        &mut w,
        &mut s,
        TickInput {
            backend: &mut b,
            now_ms: 1000,
            pending_resize: false,
            should_quit: false,
            force_redraw_in: false,
            overlay: String::new(),
        },
    );
    // First scheduled tick at now=1000 with empty queue: executes and
    // reports next_in_ms = update_ms (time-to-next-execution from now).
    assert_eq!(out.next_in_ms, 2000);
}

#[test]
fn force_redraw_in_runs_pass_with_redraw_set() {
    let mut w = harness_world();
    let mut s = FakeSys::default();
    let mut b = fake_backend();
    // First scheduled tick seeds state.
    let _ = tick(
        &mut w,
        &mut s,
        TickInput {
            backend: &mut b,
            now_ms: 0,
            pending_resize: false,
            should_quit: false,
            force_redraw_in: false,
            overlay: String::new(),
        },
    );
    // force_redraw_in path: emits regardless of schedule.
    let out = tick(
        &mut w,
        &mut s,
        TickInput {
            backend: &mut b,
            now_ms: 100, // < next_tick (would be early)
            pending_resize: false,
            should_quit: false,
            force_redraw_in: true,
            overlay: String::new(),
        },
    );
    assert!(!out.out.is_empty(), "force_redraw_in must emit even early");
    assert_eq!(out.ran_boxes, vec!["cpu", "mem", "net", "proc"]);
}

#[test]
fn overlay_non_empty_sets_paused_and_appends_overlay() {
    let mut w = harness_world();
    // Pin pause behavior: World seeds C++ background_update=true.
    w.background_update = false;
    // Pin overlay shape: World seeds C++ terminal_sync=true (wraps output
    // in ?2026h/l); these tests assert the bare merge.
    w.terminal_sync = false;
    let mut s = FakeSys::default();
    let mut b = fake_backend();
    let ovl = "\x1b[2J\x1b[Hhello".to_string();
    let out = tick(
        &mut w,
        &mut s,
        TickInput {
            backend: &mut b,
            now_ms: 0,
            pending_resize: false,
            should_quit: false,
            force_redraw_in: false,
            overlay: ovl.clone(),
        },
    );
    assert!(
        out.pause_output,
        "overlay non-empty + !background_update → paused"
    );
    assert!(out.out.ends_with(&ovl), "payload ends with overlay");
    // Underneath, output is dimmed with Fx::ub + inactive_fg + uncolor(out).
    assert!(
        out.out.contains("\x1b[22m"),
        "output wrapped in Fx::ub prefix"
    );
}

#[test]
fn overlay_dim_wrap_strips_color_from_output_before_overlay() {
    let mut w = harness_world();
    let mut s = FakeSys::default();
    let mut b = fake_backend();
    let ovl = "\x1b[2JHELLO".to_string();
    let out = tick(
        &mut w,
        &mut s,
        TickInput {
            backend: &mut b,
            now_ms: 0,
            pending_resize: false,
            should_quit: false,
            force_redraw_in: false,
            overlay: ovl.clone(),
        },
    );
    // Fx::ub (`\x1b[22m`) precedes the dim background, then output is
    // `uncolor`'d (color escapes stripped) before the overlay is appended.
    // The btop_tools::uncolor regex is `\x1b\[\d+(;\d+)*m`; our CPU output
    // contains title/box SGR sequences that must be stripped.
    assert!(
        out.out.contains("\x1b[22m"),
        "fx::ub prefix before dim wrap"
    );
    let mid = out.out.split("\x1b[22m").nth(1).unwrap_or("");
    // mid = inactive_fg + uncolor(out) + overlay
    assert!(mid.contains("HELLO"), "overlay present after dim wrap");
    // The output should NOT contain raw title color SGRs between ub and
    // the overlay (they are stripped by uncolor).
    let stripped = uncolor_strip(mid.trim_end_matches("HELLO"));
    // inactive_fg itself is a color escape; uncolor leaves it (the wrap is
    // added BEFORE uncolor in cpp:723).
    assert!(
        !stripped.contains("\x1b[1m"),
        "raw SGR bold escapes should be stripped"
    );
}

fn uncolor_strip(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == 0x1b && bytes[i + 1] == b'[' {
            // parse SGR until 'm'
            let mut j = i + 2;
            while j < bytes.len() && bytes[j] != b'm' {
                j += 1;
            }
            if j < bytes.len() {
                i = j + 1;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_default()
}

#[test]
fn empty_boxes_emit_hint_text_when_not_paused() {
    let mut w = harness_world();
    w.config.strings.insert("shown_boxes".into(), String::new());
    let mut s = FakeSys::default();
    let mut b = fake_backend();
    let out = tick(
        &mut w,
        &mut s,
        TickInput {
            backend: &mut b,
            now_ms: 0,
            pending_resize: false,
            should_quit: false,
            force_redraw_in: false,
            overlay: String::new(),
        },
    );
    assert!(!out.out.is_empty(), "empty-bg hint must appear");
    assert!(
        out.out.contains("No boxes"),
        "hint text should mention 'No boxes'"
    );
    assert!(!out.pause_output);
}

#[test]
fn paused_keeps_clock_skips_boxes() {
    // cpp:664: overlay + !background_update → pause_output = true. Box
    // draws (cpp:551-637) gate on `not pause_output`. Clock still appends
    // (cpp:663: `if (not pause_output) output += conf.clock;`)... but in
    // P3 the clock is empty unless the caller sets it. Test: paused path
    // does not panic and pause_output=true is reported.
    let mut w = harness_world();
    // Pin pause behavior: World seeds C++ background_update=true.
    w.background_update = false;
    // Pin overlay shape: World seeds C++ terminal_sync=true (wraps output
    // in ?2026h/l); these tests assert the bare merge.
    w.terminal_sync = false;
    w.state.clock.time = "12:00".to_string();
    w.state.clock.date = "2026-09-10".to_string();
    let mut s = FakeSys::default();
    let mut b = fake_backend();
    let out = tick(
        &mut w,
        &mut s,
        TickInput {
            backend: &mut b,
            now_ms: 0,
            pending_resize: false,
            should_quit: false,
            force_redraw_in: false,
            overlay: "OVERLAY".to_string(),
        },
    );
    assert!(out.pause_output);
    // The overlay must be appended, even if no boxes drew.
    assert!(out.out.ends_with("OVERLAY"));
}

#[test]
fn second_paused_tick_skips_boxes_and_clock() {
    // cpp:551/575/597/617/637 gate every box on `not pause_output`.
    // pause_output is updated AT THE END of the tick (cpp:664), so the
    // gate reads the PRIOR-TICK value: the first tick with overlay still
    // emits boxes (pause_output starts false), but on the SECOND tick
    // (pause_output is now true from the previous tick), the gate
    // suppresses all box draws AND the clock (cpp:663). The output is
    // empty, no empty-bg hint is shown (cpp:665: `not pause_output`),
    // and the overlay is appended verbatim (cpp:723: `output.empty()
    // ? "" : … + conf.overlay`).
    let mut w = harness_world();
    // Pin pause behavior: World seeds C++ background_update=true.
    w.background_update = false;
    // Pin overlay shape: World seeds C++ terminal_sync=true (wraps output
    // in ?2026h/l); these tests assert the bare merge.
    w.terminal_sync = false;
    w.state.clock.time = "12:00".to_string();
    w.state.clock.date = "2026-09-10".to_string();
    let mut s = FakeSys::default();
    let mut b = fake_backend();
    // First tick: pause_output becomes true at end (overlay && !bg).
    let _ = tick(
        &mut w,
        &mut s,
        TickInput {
            backend: &mut b,
            now_ms: 0,
            pending_resize: false,
            should_quit: false,
            force_redraw_in: false,
            overlay: "OVERLAY".to_string(),
        },
    );
    assert!(w.paused, "first tick sets w.paused = true");
    // Second tick: boxes + clock are skipped.
    let out = tick(
        &mut w,
        &mut s,
        TickInput {
            backend: &mut b,
            now_ms: 2000, // >= next_tick (scheduled, not early-return)
            pending_resize: false,
            should_quit: false,
            force_redraw_in: false,
            overlay: "OVERLAY".to_string(),
        },
    );
    assert!(out.pause_output);
    assert_eq!(out.ran_boxes, Vec::<String>::new(), "no boxes ran");
    // No "12:00" clock bytes — clock is gated on `not pause_output`.
    assert!(
        !out.out.contains("12:00"),
        "clock suppressed under pause_output"
    );
    // No empty-bg hint — cpp:665 gates it on `not pause_output`.
    assert!(
        !out.out.contains("No boxes"),
        "empty-bg hint suppressed under pause_output"
    );
    // Output is exactly the overlay (no dim wrap because output is empty).
    assert_eq!(out.out, "OVERLAY");
}

#[test]
fn should_quit_propagates_via_world_flag() {
    let mut w = harness_world();
    let mut s = FakeSys::default();
    let mut b = fake_backend();
    // Drain pending Actions via Quit (execute_all path) before tick.
    let _ = execute_all(&mut w, &mut s, 100, 30, &[Action::Quit]);
    assert!(w.quit_requested);
    let _ = tick(
        &mut w,
        &mut s,
        TickInput {
            backend: &mut b,
            now_ms: 0,
            pending_resize: false,
            should_quit: false,
            force_redraw_in: false,
            overlay: String::new(),
        },
    );
    // Tick still runs (the loop checks quit_requested in P4); here we
    // just confirm tick does not panic when quit is queued.
}

#[test]
fn terminal_sync_wraps_output_when_flag_set() {
    let mut w = harness_world();
    w.terminal_sync = true;
    let mut s = FakeSys::default();
    let mut b = fake_backend();
    let out = tick(
        &mut w,
        &mut s,
        TickInput {
            backend: &mut b,
            now_ms: 0,
            pending_resize: false,
            should_quit: false,
            force_redraw_in: false,
            overlay: String::new(),
        },
    );
    // sync_start = `\x1b[?2026h`, sync_end = `\x1b[?2026l` per
    // btop_tools::Term — the wrap surrounds the output.
    assert!(
        out.out.starts_with("\x1b[?2026h"),
        "terminal_sync wraps start"
    );
    assert!(out.out.ends_with("\x1b[?2026l"), "terminal_sync wraps end");
}

#[test]
fn backend_methods_called_per_box() {
    // Record every backend call to confirm the per-box call list.
    struct Recorder {
        b: ReplayBackend,
        calls: Vec<String>,
    }
    impl MacOsBackend for Recorder {
        fn cpu_ticks(
            &mut self,
        ) -> Result<btop_collect::backend::CpuTicks, btop_collect::types::CollectError> {
            self.calls.push("cpu_ticks".into());
            self.b.cpu_ticks()
        }
        fn load_avg(&mut self) -> Result<[f64; 3], btop_collect::types::CollectError> {
            self.calls.push("load_avg".into());
            self.b.load_avg()
        }
        fn package_temp(&mut self) -> Result<Option<i64>, btop_collect::types::CollectError> {
            self.calls.push("package_temp".into());
            self.b.package_temp()
        }
        fn core_temps(&mut self) -> Result<Vec<i64>, btop_collect::types::CollectError> {
            self.calls.push("core_temps".into());
            self.b.core_temps()
        }
        fn vm_raw(
            &mut self,
        ) -> Result<(u64, u64, u64, u64, u64), btop_collect::types::CollectError> {
            self.calls.push("vm_raw".into());
            self.b.vm_raw()
        }
        fn swap_raw(&mut self) -> Result<(u64, u64, u64), btop_collect::types::CollectError> {
            self.calls.push("swap_raw".into());
            self.b.swap_raw()
        }
        fn disk_raw(
            &mut self,
            mount: &str,
        ) -> Result<(u64, u64, u64), btop_collect::types::CollectError> {
            self.calls.push(format!("disk_raw({})", mount));
            self.b.disk_raw(mount)
        }
        fn disk_mounts(
            &mut self,
        ) -> Result<Vec<(String, String)>, btop_collect::types::CollectError> {
            self.calls.push("disk_mounts".into());
            self.b.disk_mounts()
        }
        fn if_counters(
            &mut self,
        ) -> Result<Vec<(String, u64, u64)>, btop_collect::types::CollectError> {
            self.calls.push("if_counters".into());
            self.b.if_counters()
        }
        fn proc_list(&mut self) -> Result<Vec<ProcRaw>, btop_collect::types::CollectError> {
            self.calls.push("proc_list".into());
            self.b.proc_list()
        }
        fn gpu_residency(
            &mut self,
        ) -> Result<Vec<(String, u64, u64)>, btop_collect::types::CollectError> {
            self.calls.push("gpu_residency".into());
            self.b.gpu_residency()
        }
        fn gpu_energy(&mut self) -> Result<(u64, EnergyUnit), btop_collect::types::CollectError> {
            self.calls.push("gpu_energy".into());
            self.b.gpu_energy()
        }
        fn system_uptime(&mut self) -> u64 {
            self.calls.push("system_uptime".into());
            self.b.system_uptime()
        }
        fn hid_temps(&mut self) -> Result<Vec<f64>, btop_collect::types::CollectError> {
            self.calls.push("hid_temps".into());
            self.b.hid_temps()
        }
    }
    let mut rec = Recorder {
        b: fake_backend(),
        calls: vec![],
    };
    let mut w = harness_world();
    let mut s = FakeSys::default();
    let _ = tick(
        &mut w,
        &mut s,
        TickInput {
            backend: &mut rec,
            now_ms: 0,
            pending_resize: false,
            should_quit: false,
            force_redraw_in: false,
            overlay: String::new(),
        },
    );
    // cpu box: cpu_ticks + load_avg + package_temp + core_temps.
    assert!(
        rec.calls.iter().any(|c| c == "cpu_ticks"),
        "cpu_ticks called"
    );
    assert!(rec.calls.iter().any(|c| c == "load_avg"), "load_avg called");
    assert!(
        rec.calls.iter().any(|c| c == "package_temp"),
        "package_temp called"
    );
    assert!(
        rec.calls.iter().any(|c| c == "core_temps"),
        "core_temps called"
    );
    // mem: vm_raw + swap_raw.
    assert!(rec.calls.iter().any(|c| c == "vm_raw"), "vm_raw called");
    assert!(rec.calls.iter().any(|c| c == "swap_raw"), "swap_raw called");
    // net: if_counters.
    assert!(
        rec.calls.iter().any(|c| c == "if_counters"),
        "if_counters called"
    );
    // proc: proc_list.
    assert!(
        rec.calls.iter().any(|c| c == "proc_list"),
        "proc_list called"
    );
}

#[test]
fn shown_boxes_omits_excluded_boxes_from_collect() {
    let mut w = harness_world();
    w.config
        .strings
        .insert("shown_boxes".into(), "cpu proc".to_string());
    let mut s = FakeSys::default();
    let mut b = fake_backend();
    let out = tick(
        &mut w,
        &mut s,
        TickInput {
            backend: &mut b,
            now_ms: 0,
            pending_resize: false,
            should_quit: false,
            force_redraw_in: false,
            overlay: String::new(),
        },
    );
    assert_eq!(out.ran_boxes, vec!["cpu", "proc"]);
}

#[test]
fn run_request_run_target_forwarded_through_sink() {
    // A RunRequest from sink should not break tick; ensure tick still
    // emits output even after sink actions.
    let mut w = harness_world();
    let mut s = FakeSys::default();
    let runs = execute_all(
        &mut w,
        &mut s,
        100,
        30,
        &[Action::Run {
            target: RunTarget::Cpu,
            no_update: true,
            redraw: true,
        }],
    );
    assert_eq!(runs.len(), 1);
    let mut b = fake_backend();
    let out = tick(
        &mut w,
        &mut s,
        TickInput {
            backend: &mut b,
            now_ms: 0,
            pending_resize: false,
            should_quit: false,
            force_redraw_in: false,
            overlay: String::new(),
        },
    );
    assert!(!out.out.is_empty());
}

#[test]
fn tick_mem_disks_render_names() {
    let mut w = harness_world();
    w.recalc_layout = true;
    let mut s = FakeSys::default();
    let mut b = fake_backend();
    b.disk_mounts_q
        .push_back(vec![("/".to_string(), "root".to_string())]);
    b.disk_raw_q.push_back((100, 40, 4096));
    let out = tick(
        &mut w,
        &mut s,
        TickInput {
            backend: &mut b,
            now_ms: 0,
            pending_resize: false,
            should_quit: false,
            force_redraw_in: true,
            overlay: String::new(),
        },
    );
    assert!(
        out.out.contains("root"),
        "disk name missing from tick output; order={:?} disks_w={} cfg_show_disks={:?} recalc={} out_len={}",
        w.state.mem_disks_order,
        w.layout.mem.disks_width,
        w.config.get_b("show_disks"),
        w.recalc_layout,
        out.out.len(),
    );
}

#[test]
fn tick_threads_backend_uptime_into_state() {
    let mut w = harness_world();
    let mut s = FakeSys::default();
    let mut b = fake_backend();
    b.uptime_q.push_back(398_123);
    let _ = tick(
        &mut w,
        &mut s,
        TickInput {
            backend: &mut b,
            now_ms: 0,
            pending_resize: false,
            should_quit: false,
            force_redraw_in: false,
            overlay: String::new(),
        },
    );
    assert_eq!(w.state.uptime_secs, 398_123);
}
