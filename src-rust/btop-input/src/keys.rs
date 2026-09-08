//! Key/mouse decode mirroring Input::get() (btop_input.cpp:123-199).
use btop_config::cli::stoi_prefix;
use btop_tools::mouse::MouseMap;

static KEY_ESCAPES: &[(&str, &str)] = &[
    ("\x1b", "escape"), ("\x12", "ctrl_r"), ("\n", "enter"), (" ", "space"),
    ("\x7f", "backspace"), ("\x08", "backspace"),
    ("[A", "up"), ("OA", "up"), ("[B", "down"), ("OB", "down"),
    ("[D", "left"), ("OD", "left"), ("[C", "right"), ("OC", "right"),
    ("[2~", "insert"), ("[4h", "insert"), ("[3~", "delete"), ("[P", "delete"),
    ("[H", "home"), ("[1~", "home"), ("[F", "end"), ("[4~", "end"),
    ("[5~", "page_up"), ("[6~", "page_down"), ("\t", "tab"), ("[Z", "shift_tab"),
    ("OP", "f1"), ("OQ", "f2"), ("OR", "f3"), ("OS", "f4"),
    ("[15~", "f5"), ("[17~", "f6"), ("[18~", "f7"), ("[19~", "f8"),
    ("[20~", "f9"), ("[21~", "f10"), ("[23~", "f11"), ("[24~", "f12"),
];

pub fn decode_key(raw: &str, input_maps: &[MouseMap], menu_maps: &[MouseMap], filtering: bool, menu_active: bool) -> String {
    let mut key = raw.to_string();
    if key.is_empty() { return key; }
    if key.len() > 1 && key.as_bytes()[0] == 0x1b { key.remove(0); }
    if key.starts_with("[<") {
        let mut view = key.as_str();
        let mouse_event: Option<&str>;
        if view.starts_with("[<0;") && view.contains('M') { mouse_event = Some("mouse_click"); view = &view[4..]; }
        else if view.starts_with("[<32;") { mouse_event = Some("mouse_drag"); view = &view[5..]; }
        else if view.starts_with("[<0;") && view.ends_with('m') { mouse_event = Some("mouse_release"); view = &view[4..]; }
        else if view.starts_with("[<64;") { mouse_event = Some("mouse_scroll_up"); view = &view[5..]; }
        else if view.starts_with("[<65;") { mouse_event = Some("mouse_scroll_down"); view = &view[5..]; }
        else { return String::new(); }
        let mouse_event = mouse_event.expect("set in every branch above");
        if filtering {
            if mouse_event == "mouse_click" { return mouse_event.to_string(); }
            else { return String::new(); }
        }
        // Position parse: col/line are i32 (stoi_prefix). Compare in i64 (lossless;
        // narrowing the rect to i32 could truncate absurd coords). For release
        // events (trailing lowercase 'm'), find('M') misses and mpos falls back
        // to view.len() — tolerated because stoi_prefix stops at non-digits.
        let Some(semi) = view.find(';') else { return String::new(); };
        let mpos = view.find('M').unwrap_or(view.len());
        let (Ok(col), Ok(line)) = (stoi_prefix(&view[..semi]), stoi_prefix(&view[semi + 1..mpos])) else { return String::new(); };
        let (col, line) = (col as i64, line as i64);
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

#[cfg(test)]
mod tests {
    use super::*;
    use btop_tools::mouse::MouseMap;

    fn maps() -> (Vec<MouseMap>, Vec<MouseMap>) {
        (vec![MouseMap { x: 10, y: 5, w: 4, h: 2, action: "proc_sort".into() }], vec![])
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
        assert_eq!(decode_key("\x1b[<0;12;6M", &input, &menu, false, false), "proc_sort");
        assert_eq!(decode_key("\x1b[<0;1;1M", &input, &menu, false, false), "mouse_click");
    }

    #[test]
    fn mouse_variants_decode() {
        let (input, menu) = maps();
        assert_eq!(decode_key("\x1b[<32;5;5M", &input, &menu, false, false), "mouse_drag");
        assert_eq!(decode_key("\x1b[<0;5;5m", &input, &menu, false, false), "mouse_release");
        assert_eq!(decode_key("\x1b[<64;5;5M", &input, &menu, false, false), "mouse_scroll_up");
        assert_eq!(decode_key("\x1b[<65;5;5M", &input, &menu, false, false), "mouse_scroll_down");
    }

    #[test]
    fn filtering_passes_only_click() {
        let (input, menu) = maps();
        assert_eq!(decode_key("\x1b[<0;5;5M", &input, &menu, true, false), "mouse_click");
        assert_eq!(decode_key("\x1b[<64;5;5M", &input, &menu, true, false), "");
        assert_eq!(decode_key("a", &input, &menu, true, false), "a");
    }

    #[test]
    fn menu_maps_win_when_active() {
        let input = vec![MouseMap { x: 1, y: 1, w: 80, h: 24, action: "input_hit".into() }];
        let menu = vec![MouseMap { x: 1, y: 1, w: 80, h: 24, action: "menu_hit".into() }];
        assert_eq!(decode_key("\x1b[<0;5;5M", &input, &menu, false, true), "menu_hit");
        assert_eq!(decode_key("\x1b[<0;5;5M", &input, &menu, false, false), "input_hit");
    }
}
