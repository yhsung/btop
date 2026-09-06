use btop_config::config::Config;
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
