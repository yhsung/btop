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
