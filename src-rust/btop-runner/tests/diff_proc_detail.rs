//! Quick diagnostic to find byte differences in proc_detail.

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
            p_nice: 0,
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
            p_nice: 0,
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
            p_nice: 0,
            prefix: String::new(),
            tree_index: 0,
        },
    ]
}

#[test]
fn diff_proc_detail() {
    let procs = fixed_procs();
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
        mem_history: vec![67108864; 8],
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
    let want = fixture_bytes("proc_detail.ans");
    let got = out.as_bytes();
    if got != &want[..] {
        // Write the actual bytes to disk for diff inspection.
        std::fs::write("/tmp/proc_detail_got.ans", got).unwrap();
        std::fs::write("/tmp/proc_detail_want.ans", &want).unwrap();
        // Find first diff byte.
        let mut i = 0;
        while i < got.len() && i < want.len() && got[i] == want[i] {
            i += 1;
        }
        eprintln!(
            "first diff at byte {i}; got={:02x} want={:02x}",
            got.get(i).copied().unwrap_or(0),
            want.get(i).copied().unwrap_or(0)
        );
        eprintln!("context bytes [{}..{}] got:", i.saturating_sub(20), i + 20);
        let lo = i.saturating_sub(20);
        let hi = (i + 20).min(got.len());
        for j in lo..hi {
            eprint!("{:02x} ", got[j]);
        }
        eprintln!();
        eprintln!("context bytes [{}..{}] want:", lo, (i + 20).min(want.len()));
        let hi = (i + 20).min(want.len());
        for j in lo..hi {
            eprint!("{:02x} ", want[j]);
        }
        eprintln!();
        eprintln!("got len {}, want len {}", got.len(), want.len());
    }
    assert_eq!(got, &want[..]);
}
