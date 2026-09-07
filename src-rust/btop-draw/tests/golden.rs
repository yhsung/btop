//! Structural golden tests for Meter/Graph.
//!
//! Pattern mirrors btop-config/tests/golden.rs: fixtures live in
//! `../fixtures/draw/` via `CARGO_MANIFEST_DIR`.
//!
//! The marker-gradient tests below prove structure and color-index
//! mapping with EXACT full-string equality. The `byte_parity_*` tests at
//! the end (Task 3) build TRUE Default gradients via
//! [`btop_draw::theme_grad`] and assert BYTE equality against the C++
//! harness fixtures (`meter_50.ans` / `graph_default.ans` /
//! `graph_tty.ans`).

use btop_draw::boxes::{banner_gen, calc_sizes, create_box, Layout, LayoutInput};
use btop_draw::meter_graph::{meter, Graph, GraphOpts};
use btop_draw::theme_grad::{color, default_theme, gradient};
use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/draw")
}

/// 101-slot marker gradient: slot i renders as `G{i:03}`.
fn marker_gradient() -> Vec<String> {
    (0..=100).map(|i| format!("G{i:03}")).collect()
}

fn opts(
    g: Vec<String>,
    width: usize,
    height: usize,
    symbol: &str,
    invert: bool,
    no_zero: bool,
    max_value: i64,
    offset: i64,
) -> GraphOpts {
    GraphOpts {
        width,
        height,
        gradient: g,
        symbol: symbol.to_string(),
        invert,
        no_zero,
        max_value,
        offset,
    }
}

#[test]
fn meter_50_shape_with_markers() {
    // Same geometry as the meter_50 fixture (width 50, value 75):
    // cells y=2,4,...,74 on (37 cells), then meter_bg + 13-wide remainder.
    let g = marker_gradient();
    let out = meter(50, &g, "BG", "RST", 75, false);
    let mut expected = String::new();
    for i in 1..=50 {
        let y = ((i as f64) * 100.0 / 50.0).round() as usize;
        if 75 >= y {
            expected.push_str(&format!("G{y:03}■"));
        } else {
            expected.push_str("BG");
            expected.push_str(&"■".repeat(50 + 1 - i));
            break;
        }
    }
    expected.push_str("RST");
    assert_eq!(out, expected);
    assert!(out.starts_with("G002■G004■"));
    assert!(out.ends_with("BG■■■■■■■■■■■■■RST")); // 13-wide remainder
}

#[test]
fn graph_height1_two_samples_per_cell() {
    // Hand-derived (see meter_graph.rs): width 2, data [0, 100].
    // i=0 -> buffers flip to false, pair (0,0) sums 0 -> cursor skip;
    // i=1 -> flips back to true, pair (0,100) -> G100 + braille_up[4].
    let g = marker_gradient();
    let graph = Graph::new(
        opts(g, 2, 1, "braille", false, false, 0, 0),
        "RST",
        &[0, 100],
    );
    assert_eq!(graph.render(), "\x1b[1CG100⢸RST");
}

#[test]
fn graph_multi_height_gradient_rows() {
    // Hand-derived: height 2, width 1, data [50] (offset -1 pads one
    // zero pair). Top row color G100, bottom row G050 + braille_up[4].
    let g = marker_gradient();
    let graph = Graph::new(opts(g, 1, 2, "braille", false, false, 0, 0), "RST", &[50]);
    assert_eq!(graph.render(), "G100 \x1b[1B\x1b[1DG050⢸RST");
}

#[test]
fn graph_invert_selects_down_table() {
    // data [100, 100] inverted: pair (0,100) -> braille_down[4],
    // then (100,100) -> braille_down[24].
    let g = marker_gradient();
    let graph = Graph::new(
        opts(g, 2, 1, "braille", true, false, 0, 0),
        "RST",
        &[100, 100],
    );
    assert_eq!(graph.render(), "\x1b[1CG100⣿RST");
}

#[test]
fn graph_tty_symbols() {
    let g = marker_gradient();
    let graph = Graph::new(opts(g, 2, 1, "tty", false, false, 0, 0), "RST", &[0, 100]);
    assert_eq!(graph.render(), "\x1b[1CG100▒RST");
}

#[test]
fn graph_max_value_rescales() {
    // max_value=200 maps 100 -> 50: pair (0,50) -> round(50*4/100+0.3)=2.
    let g = marker_gradient();
    let graph = Graph::new(
        opts(g, 1, 1, "braille", false, false, 200, 0),
        "RST",
        &[100],
    );
    assert_eq!(graph.render(), "G050⢠RST");
}

#[test]
fn graph_no_zero_floors_bottom_row() {
    // data [0, 0] with no_zero: bottom-row pairs floor at 1 instead of
    // collapsing to cursor skips.
    let g = marker_gradient();
    let graph = Graph::new(opts(g, 2, 1, "braille", false, true, 0, 0), "RST", &[0, 0]);
    assert_eq!(graph.render(), "\x1b[1CG000⣀RST");
}

#[test]
fn graph_push_appends_one_cell() {
    let g = marker_gradient();
    let mut graph = Graph::new(
        opts(g, 2, 1, "braille", false, false, 0, 0),
        "RST",
        &[0, 100],
    );
    assert_eq!(graph.render(), "\x1b[1CG100⢸RST");
    // Grow the dataset as C++ callers do; push() drops the oldest cell of
    // the flipped buffer and appends one cell for the new tail pair.
    let out = graph.push(&[0, 100, 100]).to_string();
    assert!(out.ends_with("RST"));
    assert!(out.contains('⣿')); // pair (100,100) -> full block
}

#[test]
fn draw_fixtures_present() {
    // Byte-parity targets for the tests below.
    for name in [
        "meter_50.ans",
        "graph_default.ans",
        "graph_tty.ans",
        "createBox.ans",
        "banner_gen.ans",
        "calcSizes_S0.ans",
    ] {
        let path = fixture_dir().join(name);
        assert!(path.exists(), "missing fixture: {}", path.display());
        assert!(!std::fs::read(&path).unwrap().is_empty());
    }
}

// ── Task 3 byte parity ────────────────────────────────────────────────────
// The harness (tests/draw_golden.cpp:296-300) runs headless with Config
// defaults: color_theme="Default", tty_mode=false, lowcolor=false,
// theme_background=true, graph_symbol="braille". So:
// - gradient = theme_grad::gradient("cpu", &default_theme(), false),
// - meter_bg = Theme::c("meter_bg"), reset = Fx::reset
//   = reset_base + main_fg + main_bg,
// - Graph symbol "default" resolves to "braille"; "tty" stays tty.
// Fixture files carry one trailing "\n" from the harness printf, stripped
// before comparing.

/// Harness `Theme::c` / `Fx::reset` values under the defaults above.
fn harness_colors() -> (String, String) {
    let theme = default_theme();
    let meter_bg = color("meter_bg", &theme, false, true);
    let reset = format!(
        "{}{}{}",
        "\x1b[0m",                             // Fx::reset_base (btop_tools.cpp:749)
        color("main_fg", &theme, false, true), // Term::fg (btop_theme.cpp:468)
        color("main_bg", &theme, false, true), // Term::bg
    );
    (meter_bg, reset)
}

fn fixture_bytes(name: &str) -> Vec<u8> {
    let mut bytes = std::fs::read(fixture_dir().join(name)).unwrap();
    assert_eq!(bytes.pop(), Some(b'\n'), "{name}: harness printf newline");
    bytes
}

#[test]
fn byte_parity_meter_50() {
    // Draw::Meter(50, "cpu", false)(75).
    let theme = default_theme();
    let g = gradient("cpu", &theme, false);
    let (meter_bg, reset) = harness_colors();
    let out = meter(50, &g, &meter_bg, &reset, 75, false);
    assert_eq!(out.as_bytes(), fixture_bytes("meter_50.ans"));
}

#[test]
fn byte_parity_graph_default() {
    // Draw::Graph(20, 5, "cpu", {10..80 step 10}, "default").
    let theme = default_theme();
    let g = gradient("cpu", &theme, false);
    let (_, reset) = harness_colors();
    let data: Vec<i64> = (1..=8).map(|i| i * 10).collect();
    let graph = Graph::new(opts(g, 20, 5, "braille", false, false, 0, 0), &reset, &data);
    assert_eq!(
        graph.render().as_bytes(),
        fixture_bytes("graph_default.ans")
    );
}

#[test]
fn byte_parity_graph_tty() {
    // Draw::Graph(20, 5, "cpu", {10..80 step 10}, "tty").
    let theme = default_theme();
    let g = gradient("cpu", &theme, false);
    let (_, reset) = harness_colors();
    let data: Vec<i64> = (1..=8).map(|i| i * 10).collect();
    let graph = Graph::new(opts(g, 20, 5, "tty", false, false, 0, 0), &reset, &data);
    assert_eq!(graph.render().as_bytes(), fixture_bytes("graph_tty.ans"));
}

// ── Task 3 review: box/banner/calcSizes byte parity ───────────────────────
// Harness emits (tests/draw_golden.cpp): Draw::createBox(2, 3, 10, 5, "",
// false, "t", "b", 1) under Config defaults (tty_mode=false,
// rounded_corners=true); Draw::banner_gen(2, 3, false, false) at S0
// (100x30, centered=false); and the calcSizes S0 geometry dump.

#[test]
fn byte_parity_createBox() {
    // Fixed small box with div_line fallback, square titles, superscript 1.
    let theme = default_theme();
    let (_, reset) = harness_colors();
    let out = create_box(
        2,
        3,
        10,
        5,
        "",
        false,
        "t",
        "b",
        1,
        &color("div_line", &theme, false, true),
        &color("hi_fg", &theme, false, true),
        &color("title", &theme, false, true),
        &reset,
        false,
        true,
    );
    assert_eq!(out.as_bytes(), fixture_bytes("createBox.ans"));
}

#[test]
fn byte_parity_banner_gen() {
    // banner_gen(y=2, x=3, centered=false) at term_width=100.
    let theme = default_theme();
    let (_, reset) = harness_colors();
    let out = banner_gen(
        2,
        3,
        false,
        100,
        false,
        false,
        &color("main_fg", &theme, false, true),
        &reset,
    );
    assert_eq!(out.as_bytes(), fixture_bytes("banner_gen.ans"));
}

/// Stable S0 geometry dump, mirroring the harness format documented in
/// tests/draw_golden.cpp (one `box k=v ...` line per box).
fn layout_dump(l: &Layout) -> String {
    format!(
        "cpu x={} y={} w={} h={} bx={} by={} bw={} bh={} bcols={} bcolsz={}\n\
         mem x={} y={} w={} h={} memw={} disksw={} div={} itemh={} memsz={} meterm={} graphh={} diskm={}\n\
         net x={} y={} w={} h={} bx={} by={} bw={} bh={} dgraph={} ugraph={}\n\
         proc x={} y={} w={} h={} selmax={}\n\
         gputotal h={}",
        l.cpu.base.x,
        l.cpu.base.y,
        l.cpu.base.width,
        l.cpu.base.height,
        l.cpu.b_x,
        l.cpu.b_y,
        l.cpu.b_width,
        l.cpu.b_height,
        l.cpu.b_columns,
        l.cpu.b_column_size,
        l.mem.base.x,
        l.mem.base.y,
        l.mem.base.width,
        l.mem.base.height,
        l.mem.mem_width,
        l.mem.disks_width,
        l.mem.divider,
        l.mem.item_height,
        l.mem.mem_size,
        l.mem.mem_meter,
        l.mem.graph_height,
        l.mem.disk_meter,
        l.net.base.x,
        l.net.base.y,
        l.net.base.width,
        l.net.base.height,
        l.net.b_x,
        l.net.b_y,
        l.net.b_width,
        l.net.b_height,
        l.net.d_graph_height,
        l.net.u_graph_height,
        l.proc.base.x,
        l.proc.base.y,
        l.proc.base.width,
        l.proc.base.height,
        l.proc.select_max,
        l.gpu_total_height,
    )
}

// ── Task 4 byte parity: cpu box ──────────────────────────────────────────
// Harness (tests/draw_golden.cpp:157-187 fixed_cpu + :349-353/:369-373):
// Cpu::draw(cpu, no_gpus, true, false) at S0 (100x30) and S1 (160x48)
// under setup() determinism overrides (clock_format="", show_uptime=false,
// show_battery=false, show_cpu_watts=false, show_cpu_freq=false,
// cpu_graph_upper/lower="total", show_gpu_info="Off", coreCount=8,
// cpuName="Golden Test CPU 8-Core", cpuHz empty, available_fields
// {"Auto","total"}, got_sensors=true, cpu_temp_only=false, has_battery=false,
// supports_watts=false). Geometry = LayoutInput::defaults(w,h) cpu part.
// Theme = Default, tty=false, lowcolor=false, theme_background=true.

use btop_draw::cpu::{draw_cpu, BatteryState, CpuDrawInput, CpuFlags};
use std::collections::{HashMap, VecDeque};

fn fixed_cpu_percent() -> HashMap<String, VecDeque<i64>> {
    let mut m = HashMap::new();
    m.insert(
        "total".to_string(),
        VecDeque::from([
            12, 18, 25, 31, 27, 35, 42, 38, 45, 52, 48, 55, 61, 58, 64, 70, 66, 72, 78, 75,
        ]),
    );
    m
}

fn fixed_cpu_cores() -> Vec<VecDeque<i64>> {
    [
        [8, 12, 15, 18, 22, 25, 28, 30, 33, 35],
        [20, 25, 30, 35, 40, 45, 50, 55, 60, 65],
        [5, 10, 8, 12, 15, 10, 14, 18, 16, 20],
        [70, 65, 72, 68, 75, 71, 78, 74, 80, 77],
        [30, 32, 34, 36, 38, 40, 42, 44, 46, 48],
        [50, 48, 52, 55, 53, 57, 60, 58, 62, 65],
        [15, 18, 22, 20, 25, 28, 26, 30, 33, 31],
        [40, 45, 42, 48, 50, 47, 52, 55, 53, 58],
    ]
    .iter()
    .map(|a| VecDeque::from(*a))
    .collect()
}

fn fixed_cpu_temp() -> Vec<VecDeque<i64>> {
    [
        [55, 56, 55, 57, 56, 58, 57, 58, 59, 58],
        [50, 51, 50, 52, 51, 53, 52, 53, 54, 53],
        [52, 52, 53, 53, 54, 54, 55, 55, 56, 56],
        [48, 49, 48, 50, 49, 51, 50, 52, 51, 53],
        [60, 61, 60, 62, 61, 63, 62, 64, 63, 65],
        [54, 55, 54, 56, 55, 57, 56, 58, 57, 59],
        [58, 58, 59, 59, 60, 60, 61, 61, 62, 62],
        [51, 52, 51, 53, 52, 54, 53, 55, 54, 56],
        [57, 57, 58, 58, 59, 59, 60, 60, 61, 61],
    ]
    .iter()
    .map(|a| VecDeque::from(*a))
    .collect()
}

fn cpu_test_input<'a>(
    percent: &'a HashMap<String, VecDeque<i64>>,
    cores: &'a [VecDeque<i64>],
    temp: &'a [VecDeque<i64>],
) -> CpuDrawInput<'a> {
    static AVAILABLE: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let available = AVAILABLE.get_or_init(|| vec!["Auto".to_string(), "total".to_string()]);
    CpuDrawInput {
        percent,
        cores,
        temp,
        temp_max: 95,
        load_avg: [1.5, 1.2, 1.0],
        usage_watts: 0.0,
        active_cpus: None,
        core_count: 8,
        cpu_name: "Golden Test CPU 8-Core",
        custom_cpu_name: "",
        cpu_hz: "",
        container_engine: None,
        graph_up_cfg: "total",
        graph_lo_cfg: "total",
        available_fields: available,
        graph_symbol_cfg: "braille",
        graph_symbol_cpu_cfg: "default",
        flags: CpuFlags::harness_defaults(),
        battery: None,
        uptime_secs: 0,
        term_width: 100,
        force_redraw: true,
        data_same: false,
        prev: None,
    }
}

#[test]
fn byte_parity_cpu_S0() {
    let percent = fixed_cpu_percent();
    let cores = fixed_cpu_cores();
    let temp = fixed_cpu_temp();
    let input = cpu_test_input(&percent, &cores, &temp);
    let theme = default_theme();
    let layout = calc_sizes(&LayoutInput::defaults(100, 30));
    let out = draw_cpu(&input, &layout.cpu, &theme);
    assert_eq!(out.as_bytes(), fixture_bytes("cpu_S0.ans"));
}

#[test]
fn byte_parity_cpu_S1() {
    let percent = fixed_cpu_percent();
    let cores = fixed_cpu_cores();
    let temp = fixed_cpu_temp();
    let mut input = cpu_test_input(&percent, &cores, &temp);
    input.term_width = 160;
    let theme = default_theme();
    let layout = calc_sizes(&LayoutInput::defaults(160, 48));
    let out = draw_cpu(&input, &layout.cpu, &theme);
    assert_eq!(out.as_bytes(), fixture_bytes("cpu_S1.ans"));
}

#[test]
fn cpu_data_same_returns_prev() {
    // data_same → cached out (btop_draw.cpp Graph::operator(data_same=true)
    // returns `out`; Rust returns `prev` unchanged).
    let percent = fixed_cpu_percent();
    let cores = fixed_cpu_cores();
    let temp = fixed_cpu_temp();
    let mut input = cpu_test_input(&percent, &cores, &temp);
    input.data_same = true;
    input.prev = Some("CACHED");
    let theme = default_theme();
    let layout = calc_sizes(&LayoutInput::defaults(100, 30));
    assert_eq!(draw_cpu(&input, &layout.cpu, &theme), "CACHED");
}

// ── Fix 3: smoke tests for the previously no-op branches ─────────────────
// Each branch below had a `let _ = ...` body; now it runs transcribed code.
// No byte fixtures exist for these configs, so each test asserts non-panic
// plus ONE structural property — never byte equality.

#[test]
fn cpu_smoke_battery() {
    let percent = fixed_cpu_percent();
    let cores = fixed_cpu_cores();
    let temp = fixed_cpu_temp();
    let mut input = cpu_test_input(&percent, &cores, &temp);
    input.flags.show_battery_cfg = true;
    input.flags.has_battery = true;
    input.flags.show_battery_watts = true;
    input.battery = Some(BatteryState {
        percent: 85,
        watts: 12.5,
        seconds: 3660,
        status: "discharging".to_string(),
    });
    let theme = default_theme();
    let layout = calc_sizes(&LayoutInput::defaults(100, 30));
    let out = draw_cpu(&input, &layout.cpu, &theme);
    assert!(out.contains("BAT"), "battery title missing");
    assert!(out.contains("85%"), "battery percent missing");
}

#[test]
fn cpu_smoke_watts() {
    let percent = fixed_cpu_percent();
    let cores = fixed_cpu_cores();
    let temp = fixed_cpu_temp();
    let mut input = cpu_test_input(&percent, &cores, &temp);
    input.flags.show_watts_cfg = true;
    input.flags.supports_watts = true;
    input.usage_watts = 65.5;
    let theme = default_theme();
    let layout = calc_sizes(&LayoutInput::defaults(100, 30));
    let out = draw_cpu(&input, &layout.cpu, &theme);
    assert!(out.contains('W'), "watts suffix missing");
}

#[test]
fn cpu_smoke_freq() {
    let percent = fixed_cpu_percent();
    let cores = fixed_cpu_cores();
    let temp = fixed_cpu_temp();
    let mut input = cpu_test_input(&percent, &cores, &temp);
    input.flags.show_freq_cfg = true;
    input.flags.has_cpu_hz = true;
    input.cpu_hz = "3.40GHz";
    let theme = default_theme();
    let layout = calc_sizes(&LayoutInput::defaults(100, 30));
    let out = draw_cpu(&input, &layout.cpu, &theme);
    assert!(out.contains("GHz"), "cpu freq readout missing");
}

#[test]
fn cpu_smoke_uptime() {
    let percent = fixed_cpu_percent();
    let cores = fixed_cpu_cores();
    let temp = fixed_cpu_temp();
    let mut input = cpu_test_input(&percent, &cores, &temp);
    input.flags.show_uptime = true;
    input.uptime_secs = 3661;
    let theme = default_theme();
    let layout = calc_sizes(&LayoutInput::defaults(100, 30));
    let out = draw_cpu(&input, &layout.cpu, &theme);
    assert!(out.contains("up"), "uptime readout missing");
}

#[test]
fn cpu_smoke_container() {
    let percent = fixed_cpu_percent();
    let cores = fixed_cpu_cores();
    let temp = fixed_cpu_temp();
    let mut input = cpu_test_input(&percent, &cores, &temp);
    input.container_engine = Some("docker");
    let theme = default_theme();
    let layout = calc_sizes(&LayoutInput::defaults(100, 30));
    let out = draw_cpu(&input, &layout.cpu, &theme);
    assert!(out.contains("docker"), "container engine name missing");
}

#[test]
fn cpu_smoke_gpu_brief_omitted() {
    // The :975-1021 brief needs per-GPU data the stateless input does not
    // carry, so the branch was deleted (not stubbed): show_gpu=true must
    // still render the full box without panic.
    let percent = fixed_cpu_percent();
    let cores = fixed_cpu_cores();
    let temp = fixed_cpu_temp();
    let mut input = cpu_test_input(&percent, &cores, &temp);
    input.flags.show_gpu = true;
    let theme = default_theme();
    let layout = calc_sizes(&LayoutInput::defaults(100, 30));
    let out = draw_cpu(&input, &layout.cpu, &theme);
    assert!(!out.is_empty(), "gpu-flagged draw is empty");
    assert!(out.contains("CPU "), "meter line missing");
}
// ── Task 6 byte parity: gpu box ──────────────────────────────────────────
// Harness (tests/draw_golden.cpp:289-308 fixed_gpu + :389-398): Gpu::draw
// at S0 (100x30) and S1 (160x48) under OWN size passes
// (setup(w,h,"gpu0") with Gpu::count=1, names={"Golden GPU"}, offsets={7}).
// Geometry = LayoutInput::defaults(w,h) + shown_boxes="gpu0" + one
// GpuPanel{panel:0, b_offset:7}. All supported_functions default true, so
// every section renders; pwr_state is pinned to 8 (pwr_state=8 pin).
// Theme = Default, tty=false, lowcolor=false, theme_background=true,
// graph_symbol_gpu="default" → "braille", gpu_mirror_graph=true (mirrored),
// cpu_invert_lower=true, check_temp=true.

use btop_draw::boxes::{GpuGeom, GpuPanel};
use btop_draw::gpu::{draw_gpu, GpuDrawInput, GpuFlags, GpuSupported};

fn gpu_layout(w: usize, h: usize) -> GpuGeom {
    let mut li = LayoutInput::defaults(w, h);
    li.shown_boxes = "gpu0".to_string();
    li.gpu_panels = vec![GpuPanel {
        panel: 0,
        b_offset: 7,
    }];
    calc_sizes(&li).gpu_panels.pop().unwrap()
}

fn fixed_gpu_percent() -> HashMap<String, Vec<i64>> {
    [
        (
            "gpu-totals",
            vec![20, 25, 30, 35, 40, 45, 50, 55, 60, 55],
        ),
        (
            "gpu-vram-totals",
            vec![40, 41, 42, 43, 44, 45, 46, 47, 48, 49],
        ),
        (
            "gpu-pwr-totals",
            vec![50, 52, 54, 56, 58, 60, 58, 56, 54, 52],
        ),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.clone()))
    .collect()
}

fn gpu_test_input<'a>(
    percent: &'a HashMap<String, Vec<i64>>,
    temp: &'a [i64],
    mem_util: &'a [i64],
) -> GpuDrawInput<'a> {
    GpuDrawInput {
        percent,
        gpu_clock_speed: 1800,
        pwr_usage: 125000,
        pwr_state: 8,
        temp,
        temp_max: 95,
        mem_total: 17179869184,
        mem_used: 8589934592,
        mem_utilization: mem_util,
        mem_clock_speed: 8000,
        pcie_tx: 102400,
        pcie_rx: 204800,
        encoder_utilization: 25,
        decoder_utilization: 10,
        supported: GpuSupported::all(),
        gpu_name: "Golden GPU",
        panel: 0,
        graph_symbol_cfg: "braille",
        graph_symbol_gpu_cfg: "default",
        flags: GpuFlags::harness_defaults(),
        force_redraw: true,
        data_same: false,
        prev: None,
    }
}

#[test]
fn byte_parity_gpu_S0() {
    let percent = fixed_gpu_percent();
    let temp = [55, 56, 57, 58, 59, 60];
    let mem_util = [45, 46, 47, 48, 49, 50, 51, 52, 53, 54];
    let input = gpu_test_input(&percent, &temp, &mem_util);
    let theme = default_theme();
    let out = draw_gpu(&input, &gpu_layout(100, 30), &theme);
    assert_eq!(out.as_bytes(), fixture_bytes("gpu_S0.ans"));
}

#[test]
fn byte_parity_gpu_S1() {
    let percent = fixed_gpu_percent();
    let temp = [55, 56, 57, 58, 59, 60];
    let mem_util = [45, 46, 47, 48, 49, 50, 51, 52, 53, 54];
    let input = gpu_test_input(&percent, &temp, &mem_util);
    let theme = default_theme();
    let out = draw_gpu(&input, &gpu_layout(160, 48), &theme);
    assert_eq!(out.as_bytes(), fixture_bytes("gpu_S1.ans"));
}

#[test]
fn gpu_data_same_returns_prev() {
    let percent = fixed_gpu_percent();
    let temp = [55, 56, 57, 58, 59, 60];
    let mem_util = [45, 46, 47, 48, 49, 50, 51, 52, 53, 54];
    let mut input = gpu_test_input(&percent, &temp, &mem_util);
    input.data_same = true;
    input.prev = Some("CACHED");
    let theme = default_theme();
    assert_eq!(draw_gpu(&input, &gpu_layout(100, 30), &theme), "CACHED");
}

#[test]
fn gpu_smoke_no_redraw() {
    // !force_redraw: frame (outer "gpu0" + inner "Golden GPU" boxes)
    // omitted, values still render.
    let percent = fixed_gpu_percent();
    let temp = [55, 56, 57, 58, 59, 60];
    let mem_util = [45, 46, 47, 48, 49, 50, 51, 52, 53, 54];
    let mut input = gpu_test_input(&percent, &temp, &mem_util);
    input.force_redraw = false;
    let theme = default_theme();
    let out = draw_gpu(&input, &gpu_layout(100, 30), &theme);
    assert!(!out.contains("gpu0"), "frame should be omitted");
    assert!(out.contains("GPU "), "meter line missing");
    assert!(out.contains("PWR "), "pwr line missing");
}

#[test]
fn gpu_smoke_no_temp() {
    // temp_info=false AND check_temp=false both hide the readout
    // (:1068/:1123); the GPU meter line still renders.
    let percent = fixed_gpu_percent();
    let temp = [55, 56, 57, 58, 59, 60];
    let mem_util = [45, 46, 47, 48, 49, 50, 51, 52, 53, 54];
    let theme = default_theme();
    let layout = gpu_layout(100, 30);
    let mut no_info = gpu_test_input(&percent, &temp, &mem_util);
    no_info.supported.temp_info = false;
    let out = draw_gpu(&no_info, &layout, &theme);
    assert!(!out.contains("°C"), "temp readout should be hidden");
    assert!(out.contains("GPU "), "meter line missing");
    let mut no_check = gpu_test_input(&percent, &temp, &mem_util);
    no_check.flags.check_temp = false;
    let out2 = draw_gpu(&no_check, &layout, &theme);
    assert!(!out2.contains("°C"), "temp readout should be hidden");
    assert!(out2.contains("GPU "), "meter line missing");
}

#[test]
fn gpu_smoke_single_graph() {
    // gpu_mirror_graph=false: one tall graph, no lower half.
    let percent = fixed_gpu_percent();
    let temp = [55, 56, 57, 58, 59, 60];
    let mem_util = [45, 46, 47, 48, 49, 50, 51, 52, 53, 54];
    let mut input = gpu_test_input(&percent, &temp, &mem_util);
    input.flags.mirror_graph = false;
    let theme = default_theme();
    let out = draw_gpu(&input, &gpu_layout(100, 30), &theme);
    assert!(out.contains("GPU "), "meter line missing");
    assert!(out.contains("P-state:"), "p-state line missing");
}

#[test]
fn gpu_smoke_no_pwr_clock() {
    // pwr_usage=false kills meter + P-state; gpu_clock=false kills title.
    let percent = fixed_gpu_percent();
    let temp = [55, 56, 57, 58, 59, 60];
    let mem_util = [45, 46, 47, 48, 49, 50, 51, 52, 53, 54];
    let mut input = gpu_test_input(&percent, &temp, &mem_util);
    input.supported.pwr_usage = false;
    input.supported.gpu_clock = false;
    let theme = default_theme();
    let out = draw_gpu(&input, &gpu_layout(100, 30), &theme);
    assert!(!out.contains("PWR "), "pwr line should be hidden");
    assert!(!out.contains("P-state:"), "p-state should be hidden");
    assert!(!out.contains("1800 MHz"), "clock title should be hidden");
    assert!(out.contains("GPU "), "meter line missing");
}

#[test]
fn gpu_smoke_pwr_hidden_state() {
    // pwr_state=false and pwr_state==32 (NVML_PSTATE_UNKNOWN) both hide
    // the suffix while the PWR meter stays.
    let percent = fixed_gpu_percent();
    let temp = [55, 56, 57, 58, 59, 60];
    let mem_util = [45, 46, 47, 48, 49, 50, 51, 52, 53, 54];
    let theme = default_theme();
    let layout = gpu_layout(100, 30);
    let mut no_state = gpu_test_input(&percent, &temp, &mem_util);
    no_state.supported.pwr_state = false;
    let out = draw_gpu(&no_state, &layout, &theme);
    assert!(!out.contains("P-state:"), "p-state should be hidden");
    assert!(out.contains("PWR "), "pwr meter missing");
    let mut unknown = gpu_test_input(&percent, &temp, &mem_util);
    unknown.pwr_state = 32;
    let out2 = draw_gpu(&unknown, &layout, &theme);
    assert!(!out2.contains("P-state:"), "p-state 32 should be hidden");
    assert!(out2.contains("PWR "), "pwr meter missing");
}

#[test]
fn gpu_smoke_no_encdec() {
    // Either gate off hides the whole ENC/DEC row (:1150).
    let percent = fixed_gpu_percent();
    let temp = [55, 56, 57, 58, 59, 60];
    let mem_util = [45, 46, 47, 48, 49, 50, 51, 52, 53, 54];
    let mut input = gpu_test_input(&percent, &temp, &mem_util);
    input.supported.decoder_utilization = false;
    let theme = default_theme();
    let out = draw_gpu(&input, &gpu_layout(100, 30), &theme);
    assert!(!out.contains("ENC "), "enc/dec row should be hidden");
    assert!(!out.contains("DEC "), "enc/dec row should be hidden");
    assert!(out.contains("vram"), "vram section missing");
}

#[test]
fn gpu_smoke_vram_variants() {
    // mem_total-only / mem_used-only take the single-count line (:1188);
    // neither hides the section; mem_util=false and mem_clock=false drop
    // their runs from the combined header.
    let percent = fixed_gpu_percent();
    let temp = [55, 56, 57, 58, 59, 60];
    let mem_util = [45, 46, 47, 48, 49, 50, 51, 52, 53, 54];
    let theme = default_theme();
    let layout = gpu_layout(100, 30);
    let mut only_total = gpu_test_input(&percent, &temp, &mem_util);
    only_total.supported.mem_used = false;
    let out = draw_gpu(&only_total, &layout, &theme);
    assert!(out.contains("VRAM total:"), "single total line missing");
    assert!(!out.contains("Utilization:"), "util graph should be hidden");
    let mut only_used = gpu_test_input(&percent, &temp, &mem_util);
    only_used.supported.mem_total = false;
    let out2 = draw_gpu(&only_used, &layout, &theme);
    assert!(out2.contains("VRAM usage:"), "single usage line missing");
    assert!(out2.contains("VRAM clock:"), "single-line mem clock missing");
    let mut neither = gpu_test_input(&percent, &temp, &mem_util);
    neither.supported.mem_total = false;
    neither.supported.mem_used = false;
    let out3 = draw_gpu(&neither, &layout, &theme);
    assert!(!out3.contains("vram"), "vram section should be hidden");
    assert!(!out3.contains("VRAM"), "vram lines should be hidden");
    let mut no_sub = gpu_test_input(&percent, &temp, &mem_util);
    no_sub.supported.mem_utilization = false;
    no_sub.supported.mem_clock = false;
    let out4 = draw_gpu(&no_sub, &layout, &theme);
    assert!(out4.contains("vram"), "vram header missing");
    assert!(!out4.contains("Utilization:"), "util should be hidden");
    assert!(!out4.contains("8000 MHz"), "mem clock should be hidden");
}

#[test]
fn gpu_smoke_no_pcie() {
    // pcie_txrx=false and negative tx (manually disabled) both hide it.
    let percent = fixed_gpu_percent();
    let temp = [55, 56, 57, 58, 59, 60];
    let mem_util = [45, 46, 47, 48, 49, 50, 51, 52, 53, 54];
    let theme = default_theme();
    let layout = gpu_layout(100, 30);
    let mut off = gpu_test_input(&percent, &temp, &mem_util);
    off.supported.pcie_txrx = false;
    assert!(!draw_gpu(&off, &layout, &theme).contains("TX:"));
    let mut neg = gpu_test_input(&percent, &temp, &mem_util);
    neg.pcie_tx = -1;
    assert!(!draw_gpu(&neg, &layout, &theme).contains("TX:"));
    let full = gpu_test_input(&percent, &temp, &mem_util);
    let out = draw_gpu(&full, &layout, &theme);
    assert!(out.contains("TX:") && out.contains("RX:"), "pcie footer missing");
}

#[test]
fn gpu_smoke_no_util() {
    // gpu_utilization=false: whole graph/meter/temp/clock block gone,
    // later sections still render.
    let percent = fixed_gpu_percent();
    let temp = [55, 56, 57, 58, 59, 60];
    let mem_util = [45, 46, 47, 48, 49, 50, 51, 52, 53, 54];
    let mut input = gpu_test_input(&percent, &temp, &mem_util);
    input.supported.gpu_utilization = false;
    let theme = default_theme();
    let out = draw_gpu(&input, &gpu_layout(100, 30), &theme);
    assert!(!out.contains("GPU "), "util block should be hidden");
    assert!(!out.contains("1800 MHz"), "clock title should be hidden");
    assert!(out.contains("PWR "), "pwr line missing");
    assert!(out.contains("vram"), "vram section missing");
}
// ── Task 5 byte parity: mem + net boxes ──────────────────────────────────
// Harness (tests/draw_golden.cpp:189-244 fixed_mem/fixed_net + :354-361,
// :374-381): Mem::draw / Net::draw at S0 (100x30) and S1 (160x48) under
// setup() determinism overrides (show_swap=true pinned via has_swap,
// swap_disk=false, io_mode=false, show_gpu_info="Off", net_auto=true,
// net_sync=true, selected_iface="eth0", graph_max {dl 10M, ul 5M}).
// Geometry = LayoutInput::defaults(w,h) mem/net parts. Theme = Default.
// NOTE: fixtures embed totalMem=0 ("Total: ... 0 Byte") — capture.sh notes
// Mem::get_totalMem() is host-specific; the checked-in .ans was captured
// where it returned 0, so the tests below pass total_mem=0 explicitly.

use btop_draw::mem::{DiskDraw, MemDrawInput, MemFlags};
use btop_draw::net::{NetDrawInput, NetFlags, NetStat};

fn fixed_mem_stats() -> HashMap<String, u64> {
    [
        ("used", 8589934592u64),
        ("available", 3221225472u64),
        ("cached", 2147483648u64),
        ("free", 5368709120u64),
        ("swap_total", 4294967296u64),
        ("swap_used", 1073741824u64),
        ("swap_free", 3221225472u64),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), *v))
    .collect()
}

fn fixed_mem_percent() -> HashMap<String, Vec<i64>> {
    [
        ("used", vec![62, 63, 64, 63, 65, 66, 65, 67, 68, 67]),
        ("available", vec![30, 31, 30, 32, 31, 33, 32, 34, 33, 35]),
        ("cached", vec![15, 15, 16, 16, 15, 17, 16, 18, 17, 18]),
        ("free", vec![25, 24, 25, 23, 24, 22, 23, 21, 22, 20]),
        ("swap_total", vec![25, 25, 25, 25, 25, 25, 25, 25, 25, 25]),
        ("swap_used", vec![20, 20, 21, 21, 22, 22, 23, 23, 24, 25]),
        ("swap_free", vec![80, 80, 79, 79, 78, 78, 77, 77, 76, 75]),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.clone()))
    .collect()
}

fn fixed_mem_disks() -> (HashMap<String, DiskDraw>, Vec<String>) {
    let root = DiskDraw {
        name: "/".to_string(),
        total: 100000000000,
        used: 40000000000,
        free: 60000000000,
        used_percent: 40,
        free_percent: 60,
        io_read: (0..10).map(|i| 1000000 + 500000 * i).collect(),
        io_write: (0..10).map(|i| 500000 + 250000 * i).collect(),
        io_activity: vec![10, 15, 20, 25, 30, 35, 30, 25, 20, 15],
    };
    (
        [("/".to_string(), root)].into_iter().collect(),
        vec!["/".to_string()],
    )
}

fn mem_test_input<'a>(
    stats: &'a HashMap<String, u64>,
    percent: &'a HashMap<String, Vec<i64>>,
    disks: &'a HashMap<String, DiskDraw>,
    order: &'a [String],
) -> MemDrawInput<'a> {
    MemDrawInput {
        stats,
        percent,
        disks,
        disks_order: order,
        total_mem: 0, // checked-in fixture captured with totalMem=0 (see note above)
        has_swap: true,
        disk_ios: 1,
        io_graph_speeds: "",
        graph_symbol_cfg: "braille",
        graph_symbol_mem_cfg: "default",
        flags: MemFlags::harness_defaults(),
        force_redraw: true,
        data_same: false,
        prev: None,
    }
}

#[test]
fn byte_parity_mem_S0() {
    let stats = fixed_mem_stats();
    let percent = fixed_mem_percent();
    let (disks, order) = fixed_mem_disks();
    let input = mem_test_input(&stats, &percent, &disks, &order);
    let theme = default_theme();
    let layout = calc_sizes(&LayoutInput::defaults(100, 30));
    let out = btop_draw::mem::draw_mem(&input, &layout.mem, &theme);
    assert_eq!(out.as_bytes(), fixture_bytes("mem_S0.ans"));
}

#[test]
fn byte_parity_mem_S1() {
    let stats = fixed_mem_stats();
    let percent = fixed_mem_percent();
    let (disks, order) = fixed_mem_disks();
    let input = mem_test_input(&stats, &percent, &disks, &order);
    let theme = default_theme();
    let layout = calc_sizes(&LayoutInput::defaults(160, 48));
    let out = btop_draw::mem::draw_mem(&input, &layout.mem, &theme);
    assert_eq!(out.as_bytes(), fixture_bytes("mem_S1.ans"));
}

#[test]
fn mem_data_same_returns_prev() {
    let stats = fixed_mem_stats();
    let percent = fixed_mem_percent();
    let (disks, order) = fixed_mem_disks();
    let mut input = mem_test_input(&stats, &percent, &disks, &order);
    input.data_same = true;
    input.prev = Some("CACHED");
    let theme = default_theme();
    let layout = calc_sizes(&LayoutInput::defaults(100, 30));
    assert_eq!(
        btop_draw::mem::draw_mem(&input, &layout.mem, &theme),
        "CACHED"
    );
}

#[test]
fn mem_smoke_no_swap() {
    // Swap hidden three ways (:1354): show_swap=false, has_swap=false,
    // swap_disk=true (swap becomes a disk in collect, not a meter block).
    let stats = fixed_mem_stats();
    let percent = fixed_mem_percent();
    let (disks, order) = fixed_mem_disks();
    let theme = default_theme();
    let layout = calc_sizes(&LayoutInput::defaults(100, 30));
    let mut input = mem_test_input(&stats, &percent, &disks, &order);
    input.flags.show_swap = false;
    let out = btop_draw::mem::draw_mem(&input, &layout.mem, &theme);
    assert!(!out.contains("Swap:"), "swap block should be hidden");
    assert!(out.contains("Total:"), "mem section missing");
    let mut input2 = mem_test_input(&stats, &percent, &disks, &order);
    input2.has_swap = false;
    let out2 = btop_draw::mem::draw_mem(&input2, &layout.mem, &theme);
    assert!(
        !out2.contains("Swap:"),
        "swap block should be hidden (no swap)"
    );
    let mut input3 = mem_test_input(&stats, &percent, &disks, &order);
    input3.flags.swap_disk = true;
    let out3 = btop_draw::mem::draw_mem(&input3, &layout.mem, &theme);
    assert!(
        !out3.contains("Swap:"),
        "swap block should be hidden (swap_disk)"
    );
}

#[test]
fn mem_smoke_tall_graph() {
    // 100x60 → mem height 23, graph_height=2: exercises the `up` cursor-up
    // (:1349) and the mem_size=3 two-line items at taller graphs. The
    // meters variant covers the divider.empty() arm (:1381, graph_height=0
    // with mem_size>2).
    let stats = fixed_mem_stats();
    let percent = fixed_mem_percent();
    let (disks, order) = fixed_mem_disks();
    let theme = default_theme();
    let layout = calc_sizes(&LayoutInput::defaults(100, 60));
    assert_eq!(layout.mem.graph_height, 2);
    assert_eq!(layout.mem.mem_size, 3);
    let input = mem_test_input(&stats, &percent, &disks, &order);
    let out = btop_draw::mem::draw_mem(&input, &layout.mem, &theme);
    assert!(out.contains("Total:"), "mem section missing");
    assert!(out.contains('▲') || out.contains('▼') || out.contains('%'));
    let mut meters = mem_test_input(&stats, &percent, &disks, &order);
    meters.flags.use_graphs = false;
    let out2 = btop_draw::mem::draw_mem(&meters, &layout.mem, &theme);
    assert!(out2.contains("Total:"), "meter variant missing");
    assert!(out2.contains('■'), "meter blocks missing");
}

#[test]
fn mem_smoke_tiny_height() {
    // 100x24 → mem height 9: the item loop hits `cy > height-4` and the
    // swap guard `cy > height-5` (:1356/:1359), so no swap block fits.
    let stats = fixed_mem_stats();
    let percent = fixed_mem_percent();
    let (disks, order) = fixed_mem_disks();
    let theme = default_theme();
    let layout = calc_sizes(&LayoutInput::defaults(100, 24));
    assert_eq!(layout.mem.base.height, 9);
    let input = mem_test_input(&stats, &percent, &disks, &order);
    let out = btop_draw::mem::draw_mem(&input, &layout.mem, &theme);
    assert!(!out.contains("Swap:"), "swap should not fit at height 9");
    assert!(out.contains("Total:"), "mem section missing");
}

#[test]
fn mem_smoke_narrow() {
    // 60x48 → mem_width 12 (big_mem=false, :1350/:1380 take-5 arm) and
    // small disks (disks_width 13, disk_meter via the max(-14,·) arm).
    let stats = fixed_mem_stats();
    let percent = fixed_mem_percent();
    let (disks, order) = fixed_mem_disks();
    let theme = default_theme();
    let layout = calc_sizes(&LayoutInput::defaults(60, 48));
    assert!(layout.mem.mem_width <= 21);
    assert_eq!(layout.mem.mem_size, 3);
    let input = mem_test_input(&stats, &percent, &disks, &order);
    let out = btop_draw::mem::draw_mem(&input, &layout.mem, &theme);
    assert!(out.contains("Total:"), "mem section missing");
    assert!(out.contains("Avail"), "narrow titles missing");
}

#[test]
fn mem_smoke_no_disks() {
    // show_disks=false: disk rows + io title omitted (the `disks` toggle
    // title itself is unconditional in C++, :2488-2489, so assert on the
    // disk content `93G` instead). Geometry recomputed with show_disks off
    // (mem spans the full width, mem_size=2).
    let stats = fixed_mem_stats();
    let percent = fixed_mem_percent();
    let (disks, order) = fixed_mem_disks();
    let mut input = mem_test_input(&stats, &percent, &disks, &order);
    input.flags.show_disks = false;
    let theme = default_theme();
    let mut li = LayoutInput::defaults(100, 30);
    li.show_disks = false;
    let layout = calc_sizes(&li);
    let out = btop_draw::mem::draw_mem(&input, &layout.mem, &theme);
    assert!(!out.contains("93G"), "disk rows should be hidden");
    assert!(!out.contains(" IO"), "io title should be hidden");
    assert!(out.contains("Total:"), "mem section missing");
    assert!(out.contains("Avail"), "full-width titles missing");
}

#[test]
fn mem_smoke_meters() {
    // mem_graphs=false: meters instead of graphs.
    let stats = fixed_mem_stats();
    let percent = fixed_mem_percent();
    let (disks, order) = fixed_mem_disks();
    let mut input = mem_test_input(&stats, &percent, &disks, &order);
    input.flags.use_graphs = false;
    let theme = default_theme();
    let layout = calc_sizes(&LayoutInput::defaults(100, 30));
    let out = btop_draw::mem::draw_mem(&input, &layout.mem, &theme);
    assert!(out.contains("Total:"), "mem section missing");
    assert!(out.contains('■'), "meter blocks missing");
}

#[test]
fn mem_smoke_io_mode() {
    // io_mode=true (split read/write graphs) + combined variant.
    let stats = fixed_mem_stats();
    let percent = fixed_mem_percent();
    let (disks, order) = fixed_mem_disks();
    let theme = default_theme();
    let layout = calc_sizes(&LayoutInput::defaults(100, 30));
    let mut input = mem_test_input(&stats, &percent, &disks, &order);
    input.flags.io_mode = true;
    let out = btop_draw::mem::draw_mem(&input, &layout.mem, &theme);
    assert!(out.contains('▲'), "io read marker missing");
    assert!(out.contains('▼'), "io write marker missing");
    let mut combined = mem_test_input(&stats, &percent, &disks, &order);
    combined.flags.io_mode = true;
    combined.flags.io_graph_combined = true;
    let out2 = btop_draw::mem::draw_mem(&combined, &layout.mem, &theme);
    assert!(out2.contains("RW") || out2.contains('▲') || out2.contains('▼'));
}

#[test]
fn mem_smoke_no_io_stat() {
    // show_io_stat=false: activity row omitted in normal disk view.
    let stats = fixed_mem_stats();
    let percent = fixed_mem_percent();
    let (disks, order) = fixed_mem_disks();
    let mut input = mem_test_input(&stats, &percent, &disks, &order);
    input.flags.show_io_stat = false;
    let theme = default_theme();
    let layout = calc_sizes(&LayoutInput::defaults(100, 30));
    let out = btop_draw::mem::draw_mem(&input, &layout.mem, &theme);
    assert!(!out.contains("IO"), "activity row should be hidden");
    assert!(out.contains('■'), "disk meter blocks missing");
}

fn fixed_net_bandwidth() -> HashMap<String, Vec<i64>> {
    [
        (
            "download",
            vec![
                1000000, 1500000, 2000000, 2500000, 3000000, 3500000, 4000000, 4500000, 5000000,
                4500000, 4000000, 3500000, 3000000, 2500000, 2000000, 2500000, 3000000, 3500000,
                4000000, 4500000,
            ],
        ),
        (
            "upload",
            vec![
                500000, 600000, 700000, 800000, 900000, 1000000, 1100000, 1200000, 1300000,
                1200000, 1100000, 1000000, 900000, 800000, 700000, 800000, 900000, 1000000,
                1100000, 1200000,
            ],
        ),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.clone()))
    .collect()
}

fn fixed_net_stat() -> HashMap<String, NetStat> {
    [
        (
            "download",
            NetStat {
                speed: 2500000,
                top: 8000000,
                total: 123456789012,
                offset: 0,
            },
        ),
        (
            "upload",
            NetStat {
                speed: 1200000,
                top: 3000000,
                total: 45678901234,
                offset: 0,
            },
        ),
    ]
    .iter()
    .map(|(k, v)| {
        (
            k.to_string(),
            NetStat {
                speed: v.speed,
                top: v.top,
                total: v.total,
                offset: v.offset,
            },
        )
    })
    .collect()
}

fn fixed_net_graph_max() -> HashMap<String, u64> {
    [("download", 10000000u64), ("upload", 5000000u64)]
        .iter()
        .map(|(k, v)| (k.to_string(), *v))
        .collect()
}

fn net_test_input<'a>(
    bandwidth: &'a HashMap<String, Vec<i64>>,
    stat: &'a HashMap<String, NetStat>,
    graph_max: &'a HashMap<String, u64>,
) -> NetDrawInput<'a> {
    NetDrawInput {
        bandwidth,
        stat,
        ipv4: "192.0.2.1",
        ipv6: "",
        connected: true,
        selected_iface: "eth0",
        graph_max,
        net_download_cfg: 100,
        net_upload_cfg: 100,
        graph_symbol_cfg: "braille",
        graph_symbol_net_cfg: "default",
        old_ip: "",
        flags: NetFlags::harness_defaults(),
        force_redraw: true,
        data_same: false,
        prev: None,
    }
}

#[test]
fn byte_parity_net_S0() {
    let bw = fixed_net_bandwidth();
    let stat = fixed_net_stat();
    let gm = fixed_net_graph_max();
    let input = net_test_input(&bw, &stat, &gm);
    let theme = default_theme();
    let layout = calc_sizes(&LayoutInput::defaults(100, 30));
    let out = btop_draw::net::draw_net(&input, &layout.net, &theme);
    assert_eq!(out.as_bytes(), fixture_bytes("net_S0.ans"));
}

#[test]
fn byte_parity_net_S1() {
    let bw = fixed_net_bandwidth();
    let stat = fixed_net_stat();
    let gm = fixed_net_graph_max();
    let input = net_test_input(&bw, &stat, &gm);
    let theme = default_theme();
    let layout = calc_sizes(&LayoutInput::defaults(160, 48));
    let out = btop_draw::net::draw_net(&input, &layout.net, &theme);
    assert_eq!(out.as_bytes(), fixture_bytes("net_S1.ans"));
}

#[test]
fn net_data_same_returns_prev() {
    let bw = fixed_net_bandwidth();
    let stat = fixed_net_stat();
    let gm = fixed_net_graph_max();
    let mut input = net_test_input(&bw, &stat, &gm);
    input.data_same = true;
    input.prev = Some("CACHED");
    let theme = default_theme();
    let layout = calc_sizes(&LayoutInput::defaults(100, 30));
    assert_eq!(
        btop_draw::net::draw_net(&input, &layout.net, &theme),
        "CACHED"
    );
}

#[test]
fn net_smoke_disconnected() {
    // connected=false: graphs still render (redraw path), speeds shown.
    let bw = fixed_net_bandwidth();
    let stat = fixed_net_stat();
    let gm = fixed_net_graph_max();
    let mut input = net_test_input(&bw, &stat, &gm);
    input.connected = false;
    let theme = default_theme();
    let layout = calc_sizes(&LayoutInput::defaults(100, 30));
    let out = btop_draw::net::draw_net(&input, &layout.net, &theme);
    assert!(
        out.contains("Total:"),
        "stat rows missing when disconnected"
    );
}

#[test]
fn net_smoke_swapped() {
    // swap_upload_download=true: upload on top.
    let bw = fixed_net_bandwidth();
    let stat = fixed_net_stat();
    let gm = fixed_net_graph_max();
    let mut input = net_test_input(&bw, &stat, &gm);
    input.flags.swap_upload_download = true;
    let theme = default_theme();
    let layout = calc_sizes(&LayoutInput::defaults(100, 30));
    let out = btop_draw::net::draw_net(&input, &layout.net, &theme);
    assert!(out.contains('▲'), "upload marker missing");
    assert!(out.contains('▼'), "download marker missing");
}

#[test]
fn net_smoke_no_ip() {
    // empty ipv4+ipv6: no address title run.
    let bw = fixed_net_bandwidth();
    let stat = fixed_net_stat();
    let gm = fixed_net_graph_max();
    let mut input = net_test_input(&bw, &stat, &gm);
    input.ipv4 = "";
    input.ipv6 = "";
    let theme = default_theme();
    let layout = calc_sizes(&LayoutInput::defaults(100, 30));
    let out = btop_draw::net::draw_net(&input, &layout.net, &theme);
    assert!(out.contains("eth0"), "iface selector missing");
}

#[test]
fn net_smoke_empty_bandwidth() {
    // empty bandwidth: frame + reset only (btop_draw.cpp:1525-1526).
    let bw: HashMap<String, Vec<i64>> = HashMap::new();
    let stat = fixed_net_stat();
    let gm = fixed_net_graph_max();
    let input = net_test_input(&bw, &stat, &gm);
    let theme = default_theme();
    let layout = calc_sizes(&LayoutInput::defaults(100, 30));
    let out = btop_draw::net::draw_net(&input, &layout.net, &theme);
    assert!(!out.is_empty(), "frame should still render");
    assert!(!out.contains("Total:"), "no stats without bandwidth");
}

#[test]
fn byte_parity_calcSizes_S0() {
    let dump = layout_dump(&calc_sizes(&LayoutInput::defaults(100, 30)));
    let text = std::fs::read_to_string(fixture_dir().join("calcSizes_S0.ans")).unwrap();
    let fixture = text.strip_suffix('\n').unwrap();
    // Cross-check: Rust output contains each dumped key=value line.
    for line in fixture.lines() {
        assert!(dump.contains(line), "missing dump line: {line}");
    }
    // Hand-derived S0 lines (same values as boxes.rs s0_all_boxes_geometry).
    for expected in [
        "cpu x=1 y=1 w=100 h=10 bx=35 by=2 bw=65 bh=8 bcols=2 bcolsz=2",
        "mem x=1 y=11 w=45 h=11 memw=22 disksw=21 div=23 itemh=6 memsz=1 meterm=11 graphh=1 diskm=14",
        "net x=1 y=22 w=45 h=9 bx=26 by=23 bw=19 bh=7 dgraph=4 ugraph=3",
        "proc x=46 y=11 w=55 h=20 selmax=17",
        "gputotal h=0",
    ] {
        assert!(dump.contains(expected), "hand-derivation mismatch: {expected}");
    }
    assert_eq!(dump, fixture);
}
