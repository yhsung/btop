//! Theme parsing and color conversion mirroring src/btop_theme.cpp.
use std::collections::HashMap;
use std::path::Path;

/// Parse `theme[key]=#RRGGBB` lines into key → hex map.
pub fn parse_theme(path: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Ok(text) = std::fs::read_to_string(path) else {
        return map;
    };
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("theme[") {
            if let Some((key, value)) = rest.split_once(']') {
                if let Some(hex) = value.strip_prefix('=') {
                    let v = hex.trim();
                    // Mirror C++ loadFile (btop_theme.cpp:414-418): a leading
                    // `"` starts a quoted value read until the next `"`.
                    let v = match v.strip_prefix('"') {
                        Some(rest) => rest.split_once('"').map(|(inner, _)| inner).unwrap_or(rest),
                        None => v,
                    };
                    map.insert(key.to_string(), v.to_string());
                }
            }
        }
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
