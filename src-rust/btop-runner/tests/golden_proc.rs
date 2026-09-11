//! Proc detail/tree/filter byte parity tests (P3 Task 4).
//!
//! Mirrors the harness block in `tests/draw_golden.cpp` proc-detail/tree/
//! filter scenarios. We do NOT go through `tick()` here — the byte
//! fixtures capture `Proc::draw` output in isolation. The proc-detail
//! header costs 8 rows; tree/filter are list-only. All scenarios run at
//! S0 (100x30).

use std::path::PathBuf;

use btop_config::theme::default_theme;
use btop_draw::boxes::{calc_sizes, LayoutInput, ProcGeom};
use btop_draw::proc_::{draw_proc, ProcDetail, ProcDrawInput, ProcFlags, ProcInfo};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/draw")
}

fn fixture_bytes(name: &str) -> Vec<u8> {
    let mut bytes = std::fs::read(fixture_dir().join(name)).unwrap();
    assert_eq!(bytes.pop(), Some(b'\n'), "{name}: harness printf newline");
    bytes
}

/// Layout for S0 100x30 with all four boxes shown (matches the harness
/// setup()). Returns the `ProcGeom` for the proc box.
fn proc_geom_s0() -> ProcGeom {
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
    let layout = calc_sizes(&li);
    layout.proc
}

/// Base 3-proc fixture (matches C++ `fixed_procs` at proc_S0).
fn fixed_procs() -> Vec<ProcInfo> {
    vec![
        ProcInfo {
            pid: 1,
            name: "launchd".to_string(),
            cmd: "/sbin/launchd".to_string(),
            short_cmd: "launchd".to_string(),
            threads: 4,
            user: "root".to_string(),
            mem: 12582912,
            cpu_p: 0.5,
            cpu_c: 0.0,
            p_nice: 0,
            ppid: 0,
            depth: 0,
            collapsed: false,
            filtered: false,
            prefix: String::new(),
            tree_index: 0,
        },
        ProcInfo {
            pid: 777,
            name: "kernel_task".to_string(),
            cmd: "kernel_task".to_string(),
            short_cmd: "kernel_task".to_string(),
            threads: 128,
            user: "root".to_string(),
            mem: 134217728,
            cpu_p: 3.2,
            cpu_c: 0.0,
            p_nice: 0,
            ppid: 0,
            depth: 0,
            collapsed: false,
            filtered: false,
            prefix: String::new(),
            tree_index: 0,
        },
        ProcInfo {
            pid: 4242,
            name: "btop".to_string(),
            cmd: "btop --utf-force".to_string(),
            short_cmd: "btop".to_string(),
            threads: 3,
            user: "tester".to_string(),
            mem: 67108864,
            cpu_p: 12.5,
            cpu_c: 0.0,
            p_nice: 0,
            ppid: 0,
            depth: 0,
            collapsed: false,
            filtered: false,
            prefix: String::new(),
            tree_index: 0,
        },
    ]
}

/// Mirror of the proc-detail harness block (tests/draw_golden.cpp:412-435):
/// `show_detailed=true` + `detailed_pid=4242` + populated detail pane.
/// The rust draw gate is implicit in `detailed: Some(...)`; the harness
/// pins `detailed.last_pid == detailed_pid` for the gate, which we mirror
/// by setting `detailed_pid = 4242` (the matching proc info's pid).
#[test]
fn byte_parity_proc_detail() {
    let mut procs = fixed_procs();
    let detail = ProcDetail {
        entry: procs[2].clone(),
        status: "Running".to_string(),
        elapsed: "12:34".to_string(),
        parent: "launchd".to_string(),
        io_read: "1.0M".to_string(),
        io_write: "512K".to_string(),
        memory: "64M".to_string(),
        first_mem: 134217728,
        cpu_history: vec![10, 20, 30, 40, 50, 60, 70, 80],
        mem_history: vec![
            67108864, 67108864, 67108864, 67108864, 67108864, 67108864, 67108864, 67108864,
        ],
    };
    let input = ProcDrawInput {
        procs: &procs,
        numpids: 3,
        total_mem: 0,
        sorting: "pid",
        start: 0,
        selected: 0,
        followed: 0,
        followed_pid: 0,
        detailed_pid: 4242,
        restore_pid: 0,
        update_following: false,
        should_return: false,
        last_selected: 0,
        was_last: false,
        prev_banner: false,
        filter: None,
        detailed: Some(&detail),
        graph_symbol_cfg: "braille",
        graph_symbol_proc_cfg: "default",
        flags: ProcFlags::harness_defaults(),
        force_redraw: true,
        data_same: false,
        prev: None,
    };
    let theme = default_theme();
    let geom = proc_geom_s0();
    let mut maps = Vec::new();
    let out = draw_proc(&input, &geom, &theme, &mut maps);
    let _ = procs.pop(); // silence unused-mut if compiler complains
    assert_eq!(out.as_bytes(), fixture_bytes("proc_detail.ans"));
}

/// Mirror of the proc-tree harness block (tests/draw_golden.cpp:436-461):
/// `proc_tree=true` + per-row prefix + tree_index. The rust `is_hidden`
/// hides rows whose `tree_index == list_len`; here all three are <3, so
/// all visible.
#[test]
fn byte_parity_proc_tree() {
    let mut procs = fixed_procs();
    procs[0].prefix = "[-]\u{2500}".to_string(); // [-]─
    procs[1].prefix = " \u{251c}\u{2500}".to_string(); //  ├─
    procs[2].prefix = " \u{2514}\u{2500}".to_string(); //  └─
    let mut flags = ProcFlags::harness_defaults();
    flags.proc_tree = true;
    let input = ProcDrawInput {
        procs: &procs,
        numpids: 3,
        total_mem: 0,
        sorting: "pid",
        start: 0,
        selected: 0,
        followed: 0,
        followed_pid: 0,
        detailed_pid: 0,
        restore_pid: 0,
        update_following: false,
        should_return: false,
        last_selected: 0,
        was_last: false,
        prev_banner: false,
        filter: None,
        detailed: None,
        graph_symbol_cfg: "braille",
        graph_symbol_proc_cfg: "default",
        flags,
        force_redraw: true,
        data_same: false,
        prev: None,
    };
    let theme = default_theme();
    let geom = proc_geom_s0();
    let mut maps = Vec::new();
    let out = draw_proc(&input, &geom, &theme, &mut maps);
    assert_eq!(out.as_bytes(), fixture_bytes("proc_tree.ans"));
}

/// Mirror of the proc-filtered harness block (tests/draw_golden.cpp:462-475):
/// committed filter "btop" → only pid 4242 visible, numpids=1.
#[test]
fn byte_parity_proc_filtered() {
    let procs = fixed_procs();
    let mut flags = ProcFlags::harness_defaults();
    // proc_filtering stays false (committed view, not TextEdit cursor).
    let input = ProcDrawInput {
        procs: &procs,
        numpids: 1,
        total_mem: 0,
        sorting: "pid",
        start: 0,
        selected: 0,
        followed: 0,
        followed_pid: 0,
        detailed_pid: 0,
        restore_pid: 0,
        update_following: false,
        should_return: false,
        last_selected: 0,
        was_last: false,
        prev_banner: false,
        filter: Some("btop"),
        detailed: None,
        graph_symbol_cfg: "braille",
        graph_symbol_proc_cfg: "default",
        flags,
        force_redraw: true,
        data_same: false,
        prev: None,
    };
    let theme = default_theme();
    let geom = proc_geom_s0();
    let mut maps = Vec::new();
    let out = draw_proc(&input, &geom, &theme, &mut maps);
    assert_eq!(out.as_bytes(), fixture_bytes("proc_filtered.ans"));
}
