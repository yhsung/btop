//! Structural golden tests for Meter/Graph.
//!
//! Pattern mirrors btop-config/tests/golden.rs: fixtures live in
//! `../fixtures/draw/` via `CARGO_MANIFEST_DIR`.
//!
//! Byte-parity vs `meter_50.ans` / `graph_default.ans` / `graph_tty.ans` is
//! DEFERRED to Task 3: those blobs embody Default-theme gradients
//! (101-step interpolations built by `Theme::updateTheme()` in
//! btop_theme.cpp:305-363), and gradient construction from theme files is
//! Task 3's scope. Here, tests build 101-slot marker gradients
//! (`G000`-`G100`) and assert EXACT full-string equality — proving the
//! structure and color-index mapping. The `draw_fixtures_present` test pins
//! the fixture files this task's successors must byte-match.

use btop_draw::meter_graph::{meter, Graph, GraphOpts};
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
    // Task 3 byte-parity targets (see module docs for the deferral reason).
    for name in ["meter_50.ans", "graph_default.ans", "graph_tty.ans"] {
        let path = fixture_dir().join(name);
        assert!(path.exists(), "missing fixture: {}", path.display());
        assert!(!std::fs::read(&path).unwrap().is_empty());
    }
}
