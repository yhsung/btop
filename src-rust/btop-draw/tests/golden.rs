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

use btop_draw::cpu::{draw_cpu, CpuDrawInput, CpuFlags};
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
        temp_scale: "celsius",
        flags: CpuFlags::harness_defaults(),
        battery: None,
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
