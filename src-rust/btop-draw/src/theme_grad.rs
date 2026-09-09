//! Theme color gradients (src/btop_theme.cpp:305-363).
//!
//! Ports `generateColors` (depth rule only) + `generateGradients`
//! (`:306-363`): every `*_start` rgb triple seeds a 101-entry (0-100)
//! gradient interpolated toward `_mid` then `_end` with C++ integer
//! arithmetic (`start + (i-offset)*(end-start)/range`, truncating
//! division, range 50+50 with a mid color else 100), rendered with
//! `dec_to_color(..., "fg")`.
//!
//! Gradient names are the `_start` bases found in the theme map plus the
//! two synthetic `generateGradients` entries (`:311-318`): for the
//! built-in Default theme that is `temp cpu free cached available used
//! download upload process proc proc_color` (11 names — note there is no
//! `mem`/`net` gradient; `Meter`/`Graph` callers use `cpu`, `temp`,
//! `used`/`free`/`cached`/`available`, `download`/`upload`,
//! `process`/`proc`/`proc_color` directly).
//!
//! Endpoint conversion reuses [`btop_config::theme`]; the hex→rgb triple
//! (`hex_to_dec`, btop_theme.cpp:218-239) is local (no such public API).
//! Callers must supply a complete Default-keyed map ([`default_theme`]);
//! like C++ `rgbs[name]` lookups, missing keys degrade to black, which is
//! only documented, not emulated key-by-key.
//!
//! The TTY theme path (`generateTTYColors`, :366-386) is NOT ported: the
//! golden harness runs with `tty_mode=false`, so byte parity only needs
//! the Default interpolation.

use btop_config::theme::{dec_to_color, hex_to_color};
use std::collections::HashMap;

/// Re-exported from `btop_config::theme` (moved there so `parse_theme` can
/// filter unknown keys like C++ `loadFile`); kept here for existing callers.
pub use btop_config::theme::default_theme;

/// `#RRGGBB` / `#CC` → `[r, g, b]`, mirroring `hex_to_dec`
/// (src/btop_theme.cpp:218-239): strips one leading char, requires hex
/// digits, greyscale shorthand expands to a triple; anything else is
/// `[-1, -1, -1]` (the "undefined" sentinel the gradient code tests).
fn hex_to_rgb(hex: &str) -> [i32; 3] {
    let Some(h) = hex.get(1..) else {
        return [-1, -1, -1];
    };
    if h.is_empty() || !h.bytes().all(|c| c.is_ascii_hexdigit()) {
        return [-1, -1, -1];
    }
    let pair = |s: &str| i32::from_str_radix(s, 16).unwrap_or(-1);
    match h.len() {
        2 => {
            let n = pair(h);
            [n, n, n]
        }
        6 => [pair(&h[0..2]), pair(&h[2..4]), pair(&h[4..6])],
        _ => [-1, -1, -1],
    }
}

/// Depth rule from `generateColors` (src/btop_theme.cpp:253): background
/// layer only for `*bg` names except `meter_bg`.
fn depth(name: &str) -> &'static str {
    if name.ends_with("bg") && name != "meter_bg" {
        "bg"
    } else {
        "fg"
    }
}

/// Single theme color escape for `name`, mirroring `generateColors`
/// (src/btop_theme.cpp:242-303) for the complete-map case: `#...` hexes
/// convert with the depth rule above; `main_bg` with
/// `theme_background=false` is `\x1b[49m`. Missing keys yield `""`
/// (C++ would fall back to Default values / `inactive_fg` — only the
/// complete-map path is ported).
pub fn color(
    name: &str,
    theme: &HashMap<String, String>,
    to_256: bool,
    theme_background: bool,
) -> String {
    if name == "main_bg" && !theme_background {
        return "\x1b[49m".to_string();
    }
    match theme.get(name) {
        Some(hex) if hex.starts_with('#') => hex_to_color(hex, to_256, depth(name)),
        _ => String::new(),
    }
}

/// Gradient base names derivable from `theme`: every `*_start` key's base
/// plus the synthetic `proc` / `proc_color` entries `generateGradients`
/// always injects (src/btop_theme.cpp:311-318). Sorted.
pub fn gradient_names(theme: &HashMap<String, String>) -> Vec<String> {
    let mut names: Vec<String> = theme
        .keys()
        .filter(|k| k.ends_with("_start"))
        .map(|k| k.trim_end_matches("_start").to_string())
        .collect();
    for synth in ["proc", "proc_color"] {
        if !names.iter().any(|n| n == synth) {
            names.push(synth.to_string());
        }
    }
    names.sort();
    names
}

/// 101-entry gradient for `name`, porting `generateGradients`
/// (src/btop_theme.cpp:306-363) with `to_256` = Config `lowcolor`.
///
/// Unknown names yield 101 empty strings (mirrors C++ `gradients[name]`
/// default-inserting a blank array). When only a start color is defined
/// the array is filled with that color's escape; otherwise each slot is
/// `dec_to_color` of the integer-interpolated triple.
pub fn gradient(name: &str, theme: &HashMap<String, String>, to_256: bool) -> Vec<String> {
    let mut rgbs: HashMap<String, [i32; 3]> = HashMap::new();
    for (k, v) in theme {
        rgbs.insert(k.clone(), hex_to_rgb(v));
    }
    // Synthetic process gradients (:311-318). Missing keys degrade to
    // [0,0,0] exactly like C++ `rgbs[...]` default insertion; the
    // complete-map caller (default_theme) never hits this.
    let get = |rgbs: &HashMap<String, [i32; 3]>, k: &str| *rgbs.get(k).unwrap_or(&[0, 0, 0]);
    rgbs.insert("proc_start".to_string(), get(&rgbs, "main_fg"));
    rgbs.insert("proc_mid".to_string(), [-1, -1, -1]);
    rgbs.insert("proc_end".to_string(), get(&rgbs, "inactive_fg"));
    rgbs.insert("proc_color_start".to_string(), get(&rgbs, "inactive_fg"));
    rgbs.insert("proc_color_mid".to_string(), [-1, -1, -1]);
    rgbs.insert("proc_color_end".to_string(), get(&rgbs, "process_start"));

    let start_key = format!("{name}_start");
    let Some(&start) = rgbs.get(&start_key) else {
        return vec![String::new(); 101];
    };
    let mid = *rgbs.get(&format!("{name}_mid")).unwrap_or(&[-1, -1, -1]);
    let end = *rgbs.get(&format!("{name}_end")).unwrap_or(&[-1, -1, -1]);

    // Only a start color: fill with its escape (:357-360).
    if end[0] < 0 {
        let fill = theme
            .get(&start_key)
            .map(|hex| hex_to_color(hex, to_256, depth(&start_key)))
            .unwrap_or_default();
        return vec![fill; 101];
    }

    // Two-pass (50+51) interpolation with a mid color, else one pass of
    // 100 (:338-349). `input_colors[start,mid,end]`, integer division
    // truncates toward zero in both languages; the source switch at
    // `i == range` happens AFTER slot `i` is computed.
    let input = [start, mid, end];
    let range: i32 = if mid[0] >= 0 { 50 } else { 100 };
    let mut out_rgb = [[0i32; 3]; 101];
    for c in 0..3 {
        let (mut s, mut e, mut offset) = (0usize, if range == 50 { 1 } else { 2 }, 0i32);
        for i in 0i32..=100 {
            out_rgb[i as usize][c] =
                input[s][c] + (i - offset) * (input[e][c] - input[s][c]) / range;
            if i == range {
                s += 1;
                e += 1;
                offset = 50;
            }
        }
    }
    out_rgb
        .iter()
        .map(|[r, g, b]| {
            dec_to_color(
                (*r).clamp(0, 255) as u8,
                (*g).clamp(0, 255) as u8,
                (*b).clamp(0, 255) as u8,
                to_256,
                "fg",
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_gradient_names() {
        assert_eq!(
            gradient_names(&default_theme()),
            [
                "available",
                "cached",
                "cpu",
                "download",
                "free",
                "proc",
                "proc_color",
                "process",
                "temp",
                "upload",
                "used"
            ]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>(),
        );
    }

    #[test]
    fn cpu_endpoints_and_mid_match_hex() {
        let theme = default_theme();
        let g = gradient("cpu", &theme, false);
        assert_eq!(g.len(), 101);
        // Endpoints are exact (i=0 → start; i=100 → mid + (end-mid)).
        assert_eq!(g[0], hex_to_color("#77ca9b", false, "fg"));
        assert_eq!(g[50], hex_to_color("#cbc06c", false, "fg"));
        assert_eq!(g[100], hex_to_color("#dc4c4c", false, "fg"));
    }

    #[test]
    fn cpu_midpoint_uses_truncating_division() {
        // Hand-derived from the C++ formula: r = 119 + 2*84/50 = 122,
        // g = 202 + 2*(-10)/50 = 202 + 0, b = 155 + 2*(-47)/50 = 154.
        // (Truncation matters: exact real division would give 121.7.)
        let g = gradient("cpu", &default_theme(), false);
        assert_eq!(g[2], dec_to_color(122, 202, 154, false, "fg"));
        assert_eq!(g[2], "\x1b[38;2;122;202;154m");
    }

    #[test]
    fn all_default_gradients_span_101_with_exact_endpoints() {
        let theme = default_theme();
        for name in gradient_names(&theme) {
            if name == "proc" || name == "proc_color" {
                continue;
            }
            let g = gradient(&name, &theme, false);
            assert_eq!(g.len(), 101, "{name}");
            assert_eq!(
                g[0],
                hex_to_color(&theme[&format!("{name}_start")], false, "fg"),
                "{name}[0]"
            );
            assert_eq!(
                g[100],
                hex_to_color(&theme[&format!("{name}_end")], false, "fg"),
                "{name}[100]"
            );
        }
        // Synthetic entries interpolate main_fg→inactive_fg and
        // inactive_fg→process_start over a single range of 100.
        let proc = gradient("proc", &theme, false);
        assert_eq!(proc[0], hex_to_color("#cc", false, "fg"));
        assert_eq!(proc[100], hex_to_color("#40", false, "fg"));
        let pc = gradient("proc_color", &theme, false);
        assert_eq!(pc[0], hex_to_color("#40", false, "fg"));
        assert_eq!(pc[100], hex_to_color("#80d0a3", false, "fg"));
    }

    #[test]
    fn unknown_gradient_is_blank() {
        assert_eq!(
            gradient("mem", &default_theme(), false),
            vec![String::new(); 101]
        );
    }

    #[test]
    fn colors_follow_depth_rule() {
        let theme = default_theme();
        assert_eq!(
            color("meter_bg", &theme, false, true),
            "\x1b[38;2;64;64;64m"
        );
        assert_eq!(
            color("main_fg", &theme, false, true),
            "\x1b[38;2;204;204;204m"
        );
        assert_eq!(color("main_bg", &theme, false, true), "\x1b[48;2;0;0;0m");
        assert_eq!(color("main_bg", &theme, false, false), "\x1b[49m");
        assert_eq!(
            color("div_line", &theme, false, true),
            "\x1b[38;2;48;48;48m"
        );
    }
}
