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
                    map.insert(key.to_string(), hex.trim().to_string());
                }
            }
        }
    }
    map
}

fn hex_pair(s: &str) -> u8 {
    u8::from_str_radix(s, 16).unwrap_or(0)
}

/// `#RRGGBB` → ANSI escape. `depth` is `"fg"` or `"bg"`.
/// Mirrors Theme::hex_to_color: truecolor `\x1b[38;2;r;g;bm`, 256-mode via
/// the 6×6×6 cube `16 + 36*(r/51) + 6*(g/51) + (b/51)`.
pub fn hex_to_color(hex: &str, to_256: bool, depth: &str) -> String {
    let h = hex.trim_start_matches('#');
    let (r, g, b) = (
        hex_pair(&h[0..2]),
        hex_pair(&h[2..4]),
        hex_pair(&h[4..6]),
    );
    dec_to_color(r, g, b, to_256, depth)
}

/// `(r,g,b)` → ANSI escape. Mirrors Theme::dec_to_color.
pub fn dec_to_color(r: u8, g: u8, b: u8, to_256: bool, depth: &str) -> String {
    let layer = if depth == "bg" { 48 } else { 38 };
    if to_256 {
        let n = 16 + 36 * (r as u16 / 51) + 6 * (g as u16 / 51) + (b as u16 / 51);
        format!("\x1b[{layer};5;{n}m")
    } else {
        format!("\x1b[{layer};2;{r};{g};{b}m")
    }
}
