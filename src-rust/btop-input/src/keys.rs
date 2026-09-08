//! Key/mouse decode mirroring Input::get() (btop_input.cpp:123-199).
use btop_config::cli::stoi_prefix;
use btop_tools::mouse::MouseMap;

static KEY_ESCAPES: &[(&str, &str)] = &[
    ("\x1b", "escape"),
    ("\x12", "ctrl_r"),
    ("\n", "enter"),
    (" ", "space"),
    ("\x7f", "backspace"),
    ("\x08", "backspace"),
    ("[A", "up"),
    ("OA", "up"),
    ("[B", "down"),
    ("OB", "down"),
    ("[D", "left"),
    ("OD", "left"),
    ("[C", "right"),
    ("OC", "right"),
    ("[2~", "insert"),
    ("[4h", "insert"),
    ("[3~", "delete"),
    ("[P", "delete"),
    ("[H", "home"),
    ("[1~", "home"),
    ("[F", "end"),
    ("[4~", "end"),
    ("[5~", "page_up"),
    ("[6~", "page_down"),
    ("\t", "tab"),
    ("[Z", "shift_tab"),
    ("OP", "f1"),
    ("OQ", "f2"),
    ("OR", "f3"),
    ("OS", "f4"),
    ("[15~", "f5"),
    ("[17~", "f6"),
    ("[18~", "f7"),
    ("[19~", "f8"),
    ("[20~", "f9"),
    ("[21~", "f10"),
    ("[23~", "f11"),
    ("[24~", "f12"),
];

pub fn decode_key(
    raw: &str,
    input_maps: &[MouseMap],
    menu_maps: &[MouseMap],
    filtering: bool,
    menu_active: bool,
) -> String {
    let mut key = raw.to_string();
    if key.is_empty() {
        return key;
    }
    if key.len() > 1 && key.as_bytes()[0] == 0x1b {
        key.remove(0);
    }
    if key.starts_with("[<") {
        let Some((mouse_event, rest)) = mouse_event_and_rest(&key) else {
            return String::new();
        };
        if filtering {
            if mouse_event == "mouse_click" {
                return mouse_event.to_string();
            } else {
                return String::new();
            }
        }
        // Position parse: col/line are i32 (stoi_prefix). Compare in i64 (lossless;
        // narrowing the rect to i32 could truncate absurd coords). For release
        // events (trailing lowercase 'm'), find('M') misses and mpos falls back
        // to rest.len() — tolerated because stoi_prefix stops at non-digits.
        let Some((col, line)) = mouse_coords(rest) else {
            return String::new();
        };
        let mut key = mouse_event.to_string();
        if key == "mouse_click" || key == "mouse_drag" {
            let maps = if menu_active { menu_maps } else { input_maps };
            for m in maps {
                if col >= m.x && col < m.x + m.w && line >= m.y && line < m.y + m.h {
                    key = m.action.clone();
                    break;
                }
            }
        }
        key
    } else if let Some((_, name)) = KEY_ESCAPES.iter().find(|(k, _)| *k == key) {
        name.to_string()
    } else if key.chars().count() > 1 {
        String::new()
    } else {
        key
    }
}

/// SGR mouse event classifier: mirrors the `key_view.starts_with` chain in
/// `Input::get()` (btop_input.cpp:135-156). Returns the event name plus the
/// remainder after the event prefix (`"col;lineM"`), or `None` when the shape
/// does not match (C++ `key.clear()` path). Branch order is load-bearing:
/// click (`[<0;` + `M`) is tested before release (`[<0;` + trailing `m`).
fn mouse_event_and_rest(key: &str) -> Option<(&'static str, &str)> {
    if let Some(rest) = key.strip_prefix("[<0;") {
        if rest.contains('M') {
            return Some(("mouse_click", rest));
        }
        if key.ends_with('m') {
            return Some(("mouse_release", rest));
        }
    }
    if let Some(rest) = key.strip_prefix("[<32;") {
        return Some(("mouse_drag", rest));
    }
    if let Some(rest) = key.strip_prefix("[<64;") {
        return Some(("mouse_scroll_up", rest));
    }
    if let Some(rest) = key.strip_prefix("[<65;") {
        return Some(("mouse_scroll_down", rest));
    }
    None
}

/// `(col, line)` from the post-prefix remainder (`"col;lineM"`).
/// Mirrors btop_input.cpp:166-168 (`stoi` halves; `invalid_argument` /
/// `out_of_range` → event cleared). Here failure is `None`.
fn mouse_coords(rest: &str) -> Option<(i64, i64)> {
    let semi = rest.find(';')?;
    let mpos = rest.find('M').unwrap_or(rest.len());
    let col = stoi_prefix(&rest[..semi]).ok()?;
    let line = stoi_prefix(&rest[semi + 1..mpos]).ok()?;
    Some((col as i64, line as i64))
}

/// Mouse (col, line) from raw SGR input. Shares parse core with decode_key.
/// Returns None when not parseable (mirrors C++ catch→clear).
/// Parses whenever the shape matches, independent of event name: C++ sets
/// `mouse_pos` for any parsed event (btop_input.cpp:163-184), so click, drag,
/// release and both scroll variants all yield positions. The filtering gate
/// (only clicks pass while filtering) lives in decode_key/handle_key, not here.
/// The caller threads the result into `InputState::mouse_pos`, mirroring the
/// C++ `Input::mouse_pos` global that `process()` reads at :395.
pub fn decode_mouse_pos(raw: &str) -> Option<(i64, i64)> {
    // Single-ESC strip mirrors decode_key (len>1 guard keeps "\x1b" itself).
    let key = if raw.len() > 1 {
        raw.strip_prefix('\x1b').unwrap_or(raw)
    } else {
        raw
    };
    if !key.starts_with("[<") {
        return None;
    }
    let (_, rest) = mouse_event_and_rest(key)?;
    mouse_coords(rest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use btop_tools::mouse::MouseMap;

    fn maps() -> (Vec<MouseMap>, Vec<MouseMap>) {
        (
            vec![MouseMap {
                x: 10,
                y: 5,
                w: 4,
                h: 2,
                action: "proc_sort".into(),
            }],
            vec![],
        )
    }

    #[test]
    fn escapes_map_verbatim() {
        let (input, menu) = maps();
        assert_eq!(decode_key("\x1b[A", &input, &menu, false, false), "up");
        assert_eq!(decode_key("\x1bOA", &input, &menu, false, false), "up");
        assert_eq!(decode_key("\n", &input, &menu, false, false), "enter");
        assert_eq!(decode_key("\x7f", &input, &menu, false, false), "backspace");
        assert_eq!(decode_key("\x12", &input, &menu, false, false), "ctrl_r");
        assert_eq!(decode_key("q", &input, &menu, false, false), "q");
    }

    #[test]
    fn multichar_unknown_clears() {
        let (input, menu) = maps();
        assert_eq!(decode_key("\x1b[9~", &input, &menu, false, false), "");
    }

    #[test]
    fn mouse_click_hit_tests_mapping() {
        let (input, menu) = maps();
        assert_eq!(
            decode_key("\x1b[<0;12;6M", &input, &menu, false, false),
            "proc_sort"
        );
        assert_eq!(
            decode_key("\x1b[<0;1;1M", &input, &menu, false, false),
            "mouse_click"
        );
    }

    #[test]
    fn mouse_variants_decode() {
        let (input, menu) = maps();
        assert_eq!(
            decode_key("\x1b[<32;5;5M", &input, &menu, false, false),
            "mouse_drag"
        );
        assert_eq!(
            decode_key("\x1b[<0;5;5m", &input, &menu, false, false),
            "mouse_release"
        );
        assert_eq!(
            decode_key("\x1b[<64;5;5M", &input, &menu, false, false),
            "mouse_scroll_up"
        );
        assert_eq!(
            decode_key("\x1b[<65;5;5M", &input, &menu, false, false),
            "mouse_scroll_down"
        );
    }

    #[test]
    fn filtering_passes_only_click() {
        let (input, menu) = maps();
        assert_eq!(
            decode_key("\x1b[<0;5;5M", &input, &menu, true, false),
            "mouse_click"
        );
        assert_eq!(decode_key("\x1b[<64;5;5M", &input, &menu, true, false), "");
        assert_eq!(decode_key("a", &input, &menu, true, false), "a");
    }

    #[test]
    fn mouse_pos_parses_all_variants() {
        // Every SGR variant carries coords (btop_input.cpp:163-184 parses the
        // position for any parsed event, independent of event name).
        assert_eq!(decode_mouse_pos("\x1b[<0;12;6M"), Some((12, 6)));
        assert_eq!(decode_mouse_pos("\x1b[<32;5;5M"), Some((5, 5)));
        assert_eq!(decode_mouse_pos("\x1b[<0;5;5m"), Some((5, 5)));
        assert_eq!(decode_mouse_pos("\x1b[<64;7;9M"), Some((7, 9)));
        assert_eq!(decode_mouse_pos("\x1b[<65;7;9M"), Some((7, 9)));
    }

    #[test]
    fn mouse_pos_rejects_garbage() {
        assert_eq!(decode_mouse_pos(""), None);
        assert_eq!(decode_mouse_pos("q"), None);
        assert_eq!(decode_mouse_pos("\x1b[A"), None);
        assert_eq!(decode_mouse_pos("\x1b[<9;5;5M"), None);
        assert_eq!(decode_mouse_pos("\x1b[<0;5M"), None);
    }

    #[test]
    fn menu_maps_win_when_active() {
        let input = vec![MouseMap {
            x: 1,
            y: 1,
            w: 80,
            h: 24,
            action: "input_hit".into(),
        }];
        let menu = vec![MouseMap {
            x: 1,
            y: 1,
            w: 80,
            h: 24,
            action: "menu_hit".into(),
        }];
        assert_eq!(
            decode_key("\x1b[<0;5;5M", &input, &menu, false, true),
            "menu_hit"
        );
        assert_eq!(
            decode_key("\x1b[<0;5;5M", &input, &menu, false, false),
            "input_hit"
        );
    }
}
