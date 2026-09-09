use btop_config::config::Config;
use btop_config::theme::{dec_to_color, hex_to_color, parse_theme};
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures")
        .join(name);
    assert!(path.exists(), "missing fixture: {}", path.display());
    path
}

#[test]
fn load_sample_conf() {
    let mut c = Config::new();
    c.strings.insert("color_theme".into(), "Default".into());
    c.bools.insert("theme_background".into(), false);
    c.ints.insert("update_ms".into(), 1000);
    let warnings = c.load(&fixture("sample.conf"));
    assert_eq!(c.get_s("color_theme"), Some("Dracula"));
    assert_eq!(c.get_b("theme_background"), Some(true));
    assert_eq!(c.get_i("update_ms"), Some(2000));
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("typo_key"));
}

#[test]
fn load_invalid_conf_warns() {
    let mut c = Config::new();
    c.strings.insert("color_theme".into(), "Default".into());
    c.bools.insert("theme_background".into(), false);
    c.ints.insert("update_ms".into(), 1000);
    let warnings = c.load(&fixture("sample_invalid.conf"));
    assert_eq!(warnings.len(), 4);
    assert!(warnings[0].contains("broken_line_without_equals"));
    assert!(warnings[1].contains("theme_background"));
    assert!(warnings[2].contains("update_ms"));
    assert!(warnings[3].contains("unknown_thing"));
}

#[test]
fn theme_parse_strict_drops_unknown_and_tolerates_ws() {
    // Strict loadFile behavior (src/btop_theme.cpp:389-427): keys not in
    // Default_theme are dropped (:407-410); whitespace around `[`/`]`/`=`
    // is tolerated (:403 ignore-to-'[', :411 ignore-to-'=', :412 skip-ws).
    // One-pair quote strip mirrors :414-418.
    let path = std::env::temp_dir().join(format!(
        "btop_theme_strict_{}_{}.theme",
        std::process::id(),
        "unknown_keys"
    ));
    std::fs::write(
        &path,
        "theme[main_bg]=#112233\ntheme[some_unknown_key_xyz]=#445566\ntheme [ title ] = \"#aabbcc\"\ntheme[cpu_start]=#77ca9b\notherkey=5\n# a comment\n",
    )
    .unwrap();
    let map = parse_theme(&path);
    let _ = std::fs::remove_file(&path);
    assert_eq!(map.get("main_bg").map(String::as_str), Some("#112233"));
    assert!(!map.contains_key("some_unknown_key_xyz"));
    assert_eq!(map.get("title").map(String::as_str), Some("#aabbcc"));
    assert_eq!(map.get("cpu_start").map(String::as_str), Some("#77ca9b"));
    assert!(!map.contains_key("otherkey"));
}

#[test]
fn theme_golden() {
    let map = parse_theme(&fixture("sample.theme"));
    assert_eq!(map.get("main_bg").map(String::as_str), Some("#1e1e2e"));
    assert_eq!(
        hex_to_color("#cdd6f4", false, "fg"),
        "\x1b[38;2;205;214;244m"
    );
    assert_eq!(
        dec_to_color(205, 214, 244, false, "bg"),
        "\x1b[48;2;205;214;244m"
    );
    assert_eq!(hex_to_color("#cdd6f4", true, "fg"), "\x1b[38;5;189m");
    assert_eq!(hex_to_color("#808080", true, "fg"), "\x1b[38;5;244m");
}
