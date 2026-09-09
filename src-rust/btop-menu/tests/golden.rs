//! Overlay golden tests (P2 Task 6).
//!
//! Byte parity against the C++ harness fixtures in
//! `../fixtures/draw/` (100x30, Default theme, `tests/draw_golden.cpp`
//! menu block): `menu_main.ans`, `menu_options.ans`, `menu_help.ans`,
//! `msgbox_ok.ans`, `msgbox_yesno.ans`.
//!
//! Pattern mirrors `btop-draw/tests/golden.rs`: the splitter appends one
//! trailing newline, popped before comparison.

use std::collections::HashMap;
use std::path::PathBuf;

use btop_config::config::Config;
use btop_config::theme::default_theme;
use btop_menu::menus::{MenuCtx, MenuSystem, Menus};
use btop_menu::msgbox::BoxKind;
use btop_menu::options::OptionsStore;
use btop_menu::overlay::{msgbox_kind_overlay, OverlayTheme};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/draw")
}

fn fixture_bytes(name: &str) -> Vec<u8> {
    let mut bytes = std::fs::read(fixture_dir().join(name)).unwrap();
    assert_eq!(bytes.pop(), Some(b'\n'), "{name}: harness printf newline");
    bytes
}

/// Options values matching the C++ harness: compiled-in defaults plus the
/// `setup()` overrides. Only the general-tab first page (rows 0-8) is
/// rendered, but every general option is pinned for clarity.
fn harness_store() -> OptionsStore {
    let mut s = OptionsStore::default();
    s.strings.insert("color_theme".into(), "Default".into());
    s.strings.insert(
        "presets".into(),
        "cpu:1:default,proc:0:default cpu:0:default,mem:0:default,net:0:default cpu:0:block,net:0:tty".into(),
    );
    s.strings
        .insert("shown_boxes".into(), "cpu mem net proc".into());
    s.strings.insert("disable_presets".into(), "Off".into());
    s.strings.insert("clock_format".into(), String::new());
    s.strings.insert("proc_sorting".into(), "pid".into());
    s.strings.insert("cpu_graph_upper".into(), "total".into());
    s.strings.insert("cpu_graph_lower".into(), "total".into());
    s.strings.insert("show_gpu_info".into(), "Off".into());
    s.bools.insert("theme_background".into(), true);
    s.bools.insert("truecolor".into(), true);
    s.bools.insert("force_tty".into(), false);
    s.bools.insert("vim_keys".into(), false);
    s.bools.insert("disable_mouse".into(), false);
    s.bools.insert("rounded_corners".into(), true);
    s.bools.insert("tty_mode".into(), false);
    s.bools.insert("lowcolor".into(), false);
    s.bools.insert("show_uptime".into(), false);
    s.bools.insert("show_battery".into(), false);
    s.bools.insert("show_cpu_watts".into(), false);
    s.bools.insert("show_cpu_freq".into(), false);
    s.bools.insert("proc_reversed".into(), false);
    s.bools.insert("proc_tree".into(), false);
    s.bools.insert("swap_disk".into(), false);
    s.bools.insert("io_mode".into(), false);
    s.ints.insert("update_ms".into(), 2000);
    s
}

/// Browsable value lists matching the headless harness: `Theme::themes`
/// is `[Default, TTY]` (theme dirs unset, no host files picked up).
fn harness_lists() -> HashMap<String, Vec<String>> {
    let mut l = HashMap::new();
    l.insert("color_theme".into(), vec!["Default".into(), "TTY".into()]);
    l
}

fn ctx() -> MenuCtx {
    MenuCtx {
        term_w: 100,
        term_h: 30,
        target_pid: 0,
    }
}

/// Open `menu` on a fresh system (activation draw only — logic state is
/// what the overlay renders), then rebuild the overlay bytes.
fn rendered(menu: Menus, store: &OptionsStore, lists: &HashMap<String, Vec<String>>) -> MenuSystem {
    let mut sys = MenuSystem::default();
    let c = ctx();
    let mut s = harness_store();
    let mut cfg = Config::new();
    sys.show(menu, 0, &c, &mut s, &mut cfg, lists);
    sys.render_overlay(store, lists, &default_theme(), 100, 30);
    sys
}

#[test]
fn byte_parity_menu_main() {
    let store = harness_store();
    let lists = harness_lists();
    let sys = rendered(Menus::Main, &store, &lists);
    assert_eq!(sys.overlay.as_bytes(), fixture_bytes("menu_main.ans"));
}

#[test]
fn byte_parity_menu_options() {
    let store = harness_store();
    let lists = harness_lists();
    let sys = rendered(Menus::Options, &store, &lists);
    assert_eq!(sys.overlay.as_bytes(), fixture_bytes("menu_options.ans"));
}

#[test]
fn byte_parity_menu_help() {
    let store = harness_store();
    let lists = harness_lists();
    let sys = rendered(Menus::Help, &store, &lists);
    assert_eq!(sys.overlay.as_bytes(), fixture_bytes("menu_help.ans"));
}

fn golden_msgbox_content() -> Vec<String> {
    vec![
        "Golden msgbox line one".to_string(),
        "Golden msgbox line two".to_string(),
    ]
}

#[test]
fn byte_parity_msgbox_ok() {
    let ot = OverlayTheme::resolve(&default_theme());
    let (out, _) = msgbox_kind_overlay(
        45,
        BoxKind::Ok,
        &golden_msgbox_content(),
        "golden ok",
        &ot,
        false,
        true,
        100,
        30,
    );
    assert_eq!(out.as_bytes(), fixture_bytes("msgbox_ok.ans"));
}

#[test]
fn byte_parity_msgbox_yesno() {
    let ot = OverlayTheme::resolve(&default_theme());
    let (out, _) = msgbox_kind_overlay(
        45,
        BoxKind::YesNo,
        &golden_msgbox_content(),
        "golden yesno",
        &ot,
        false,
        true,
        100,
        30,
    );
    assert_eq!(out.as_bytes(), fixture_bytes("msgbox_yesno.ans"));
}

/// Mouse zones mirror the C++ `mouse_mappings` registrations
/// (hand-derived from the 100x30 geometry):
/// - main: `button_0` at (41,12) 19x3, `button_1` at (44,15) 12x3,
///   `button_2` at (44,18) 12x3.
/// - options: six `select_cat_N` tabs at row 7 (`x = 13 + 12*(N-1)`,
///   12x3) plus `left` at (11,10) and `right` at (36,10), both 5x2.
/// - help registers no zones.
/// - msgboxes register `button1` at (45,16) 12x3 (OK) / 13x3 (YES_NO)
///   and `button2` at (52,16) 12x3 for YES_NO.
#[test]
fn mouse_maps_match_cpp_registrations() {
    let store = harness_store();
    let lists = harness_lists();

    let sys = rendered(Menus::Main, &store, &lists);
    assert_eq!(sys.mouse_maps.len(), 3);
    let b0 = sys
        .mouse_maps
        .iter()
        .find(|m| m.action == "button_0")
        .expect("button_0");
    assert_eq!((b0.x, b0.y, b0.w, b0.h), (41, 12, 19, 3));
    let b1 = sys
        .mouse_maps
        .iter()
        .find(|m| m.action == "button_1")
        .expect("button_1");
    assert_eq!((b1.x, b1.y, b1.w, b1.h), (44, 15, 12, 3));
    let b2 = sys
        .mouse_maps
        .iter()
        .find(|m| m.action == "button_2")
        .expect("button_2");
    assert_eq!((b2.x, b2.y, b2.w, b2.h), (44, 18, 12, 3));

    let sys = rendered(Menus::Options, &store, &lists);
    assert_eq!(sys.mouse_maps.len(), 8);
    for (n, x) in [1, 2, 3, 4, 5, 6].iter().zip([13, 25, 37, 49, 61, 73]) {
        let m = sys
            .mouse_maps
            .iter()
            .find(|m| m.action == format!("select_cat_{n}"))
            .unwrap_or_else(|| panic!("select_cat_{n}"));
        assert_eq!((m.x, m.y, m.w, m.h), (x, 7, 12, 3));
    }
    let left = sys
        .mouse_maps
        .iter()
        .find(|m| m.action == "left")
        .expect("left");
    assert_eq!((left.x, left.y, left.w, left.h), (11, 10, 5, 2));
    let right = sys
        .mouse_maps
        .iter()
        .find(|m| m.action == "right")
        .expect("right");
    assert_eq!((right.x, right.y, right.w, right.h), (36, 10, 5, 2));

    let sys = rendered(Menus::Help, &store, &lists);
    assert!(sys.mouse_maps.is_empty(), "help registers no zones");

    let ot = OverlayTheme::resolve(&default_theme());
    let (_, maps) = msgbox_kind_overlay(
        45,
        BoxKind::Ok,
        &golden_msgbox_content(),
        "golden ok",
        &ot,
        false,
        true,
        100,
        30,
    );
    assert_eq!(maps.len(), 1);
    assert_eq!(
        (maps[0].x, maps[0].y, maps[0].w, maps[0].h),
        (45, 16, 12, 3)
    );
    assert_eq!(maps[0].action, "button1");

    let (_, maps) = msgbox_kind_overlay(
        45,
        BoxKind::YesNo,
        &golden_msgbox_content(),
        "golden yesno",
        &ot,
        false,
        true,
        100,
        30,
    );
    assert_eq!(maps.len(), 2);
    assert_eq!(
        (maps[1].x, maps[1].y, maps[1].w, maps[1].h),
        (52, 16, 12, 3)
    );
    assert_eq!(maps[1].action, "button2");
}
