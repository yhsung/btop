//! Theme parsing and color conversion mirroring src/btop_theme.cpp.
use std::collections::HashMap;
use std::path::Path;

/// Built-in "Default" theme hexes, transcribed from `Default_theme`
/// (src/btop_theme.cpp:52-101). There is no `themes/Default.theme` file;
/// the map below is the single source (used headless by `setTheme` when
/// the theme name is `"Default"`).
///
/// WARNING: hand copy — bump with btop_theme.cpp `Default_theme`.
/// (Moved here from btop-draw's theme_grad.rs so `parse_theme` can filter
/// unknown keys like C++ `loadFile`; btop-draw re-exports it.)
pub fn default_theme() -> HashMap<String, String> {
    [
        ("main_bg", "#00"),
        ("main_fg", "#cc"),
        ("title", "#ee"),
        ("hi_fg", "#b54040"),
        ("selected_bg", "#6a2f2f"),
        ("selected_fg", "#ee"),
        ("inactive_fg", "#40"),
        ("graph_text", "#60"),
        ("meter_bg", "#40"),
        ("proc_misc", "#0de756"),
        ("cpu_box", "#556d59"),
        ("mem_box", "#6c6c4b"),
        ("net_box", "#5c588d"),
        ("proc_box", "#805252"),
        ("div_line", "#30"),
        ("temp_start", "#4897d4"),
        ("temp_mid", "#5474e8"),
        ("temp_end", "#ff40b6"),
        ("cpu_start", "#77ca9b"),
        ("cpu_mid", "#cbc06c"),
        ("cpu_end", "#dc4c4c"),
        ("free_start", "#384f21"),
        ("free_mid", "#b5e685"),
        ("free_end", "#dcff85"),
        ("cached_start", "#163350"),
        ("cached_mid", "#74e6fc"),
        ("cached_end", "#26c5ff"),
        ("available_start", "#4e3f0e"),
        ("available_mid", "#ffd77a"),
        ("available_end", "#ffb814"),
        ("used_start", "#592b26"),
        ("used_mid", "#d9626d"),
        ("used_end", "#ff4769"),
        ("download_start", "#291f75"),
        ("download_mid", "#4f43a3"),
        ("download_end", "#b0a9de"),
        ("upload_start", "#620665"),
        ("upload_mid", "#7d4180"),
        ("upload_end", "#dcafde"),
        ("process_start", "#80d0a3"),
        ("process_mid", "#dcd179"),
        ("process_end", "#d45454"),
        ("proc_pause_bg", "#b54040"),
        ("proc_follow_bg", "#4040b5"),
        ("proc_banner_bg", "#7b407b"),
        ("proc_banner_fg", "#ee"),
        ("followed_bg", "#4040b5"),
        ("followed_fg", "#ee"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

/// Parse `theme[key]=#RRGGBB` lines into key → hex map, mirroring C++
/// `loadFile` (src/btop_theme.cpp:389-427):
/// - keys not in [`default_theme`] are skipped (`:407-410`),
/// - whitespace around `theme`/`[`/`]`/`=` is tolerated (`:403`
///   ignore-to-`'['`, `:411` ignore-to-`'='`, `:412` whitespace skip), so
///   `theme [ key ] = "#..."` resolves to `key`,
/// - a leading `"` starts a quoted value read until the next `"`
///   (`:414-418`); one surrounding pair is stripped, the rest of the line
///   ignored,
/// - `#` comment lines and non-`theme` lines are ignored (`:399-402`).
///
/// Deliberate deviations: the key is trimmed before the known-key check
/// (C++ would keep inner spaces, making the spaced form an unknown key and
/// dropping it); each line is pre-trimmed, so trailing whitespace on
/// unquoted values is dropped (C++ keeps it via `getline(..., '\n')`);
/// an unreadable path yields an empty map and a bad hex value yields `""`
/// from [`hex_to_color`] (no diagnostics), while C++ falls back to
/// `Default_theme` (later plan).
pub fn parse_theme(path: &Path) -> HashMap<String, String> {
    let defaults = default_theme();
    let mut map = HashMap::new();
    let Ok(text) = std::fs::read_to_string(path) else {
        return map;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(after_theme) = line.strip_prefix("theme") else {
            continue;
        };
        // Tolerate whitespace between `theme` and `[` (:403 skips to '[').
        let Some((_, after_open)) = after_theme.split_once('[') else {
            continue;
        };
        let Some((key, after_close)) = after_open.split_once(']') else {
            continue;
        };
        let key = key.trim();
        if !defaults.contains_key(key) {
            continue;
        }
        // Tolerate anything between `]` and `=` (:411 skips to '=').
        let Some((_, value)) = after_close.split_once('=') else {
            continue;
        };
        let v = value.trim_start();
        // Mirror C++ loadFile (btop_theme.cpp:414-418): a leading `"`
        // starts a quoted value read until the next `"`.
        let v = match v.strip_prefix('"') {
            Some(rest) => rest.split_once('"').map(|(inner, _)| inner).unwrap_or(rest),
            None => v,
        };
        map.insert(key.to_string(), v.to_string());
    }
    map
}

fn hex_pair(s: &str) -> u8 {
    u8::from_str_radix(s, 16).unwrap_or(0)
}

/// Truecolor `(r,g,b)` → 256-color index. Mirrors `truecolor_to_256` in
/// src/btop_theme.cpp: greyscale ramp first (all three `round(x/11)` equal
/// → `232 + red`), else the 6×6×6 cube with `round(x/51)`.
fn truecolor_to_256(r: u8, g: u8, b: u8) -> u8 {
    let red = (r as f64 / 11.0).round() as i32;
    let green = (g as f64 / 11.0).round() as i32;
    let blue = (b as f64 / 11.0).round() as i32;
    if red == green && red == blue {
        (232 + red) as u8
    } else {
        let rc = (r as f64 / 51.0).round() as i32;
        let gc = (g as f64 / 51.0).round() as i32;
        let bc = (b as f64 / 51.0).round() as i32;
        (16 + rc * 36 + gc * 6 + bc) as u8
    }
}

/// `#RRGGBB` → ANSI escape. `depth` is `"fg"` or `"bg"`.
/// Mirrors Theme::hex_to_color: strips exactly one leading char (like C++
/// `erase(0,1)`), requires remaining chars to be ASCII hexdigits, supports
/// 2-char greyscale shorthand `#CC` and 6-char `#RRGGBB`; anything else
/// returns an empty string.
pub fn hex_to_color(hex: &str, to_256: bool, depth: &str) -> String {
    let s = hex.trim();
    let Some(h) = s.get(1..) else {
        return String::new();
    };
    if h.is_empty() || !h.bytes().all(|c| c.is_ascii_hexdigit()) {
        return String::new();
    }
    let layer = if depth == "fg" { 38 } else { 48 };
    match h.len() {
        2 => {
            let n = hex_pair(h);
            if to_256 {
                let idx = truecolor_to_256(n, n, n);
                format!("\x1b[{layer};5;{idx}m")
            } else {
                format!("\x1b[{layer};2;{n};{n};{n}m")
            }
        }
        6 => {
            let (r, g, b) = (hex_pair(&h[0..2]), hex_pair(&h[2..4]), hex_pair(&h[4..6]));
            dec_to_color(r, g, b, to_256, depth)
        }
        _ => String::new(),
    }
}

/// `(r,g,b)` → ANSI escape. Mirrors Theme::dec_to_color.
/// Note: C++ clamps inputs to 0–255; no-op here by construction since the
/// signature takes `u8` (already in range).
pub fn dec_to_color(r: u8, g: u8, b: u8, to_256: bool, depth: &str) -> String {
    let layer = if depth == "fg" { 38 } else { 48 };
    if to_256 {
        let n = truecolor_to_256(r, g, b);
        format!("\x1b[{layer};5;{n}m")
    } else {
        format!("\x1b[{layer};2;{r};{g};{b}m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_theme_entry_count_and_spots() {
        // 48 entries in btop_theme.cpp:52-101; spot-check two hexes.
        let theme = default_theme();
        assert_eq!(theme.len(), 48);
        assert_eq!(theme["main_fg"], "#cc");
        assert_eq!(theme["cpu_start"], "#77ca9b");
    }

    fn write_temp(name: &str, body: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "btop_theme_unit_{}_{}.theme",
            std::process::id(),
            name
        ));
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn parse_theme_drops_unknown_keys() {
        let path = write_temp(
            "unknown",
            "theme[main_bg]=#112233\ntheme[nope_xyz]=#445566\n",
        );
        let map = parse_theme(&path);
        let _ = std::fs::remove_file(&path);
        assert_eq!(map.get("main_bg").map(String::as_str), Some("#112233"));
        assert!(!map.contains_key("nope_xyz"));
    }

    #[test]
    fn parse_theme_tolerates_whitespace_and_quotes() {
        let path = write_temp(
            "ws",
            "theme [ title ] = \"#aabbcc\"\ntheme[cpu_start] = #77ca9b\n",
        );
        let map = parse_theme(&path);
        let _ = std::fs::remove_file(&path);
        assert_eq!(map.get("title").map(String::as_str), Some("#aabbcc"));
        assert_eq!(map.get("cpu_start").map(String::as_str), Some("#77ca9b"));
    }

    #[test]
    fn parse_theme_ignores_comments_and_other_lines() {
        let path = write_temp("misc", "# comment\n  \notherkey=5\ntheme[title]#oops\n");
        let map = parse_theme(&path);
        let _ = std::fs::remove_file(&path);
        assert!(map.is_empty());
    }
}
