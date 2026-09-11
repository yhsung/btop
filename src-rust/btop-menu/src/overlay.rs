//! Overlay byte assembly for the golden menu fixtures.
//!
//! Transcribed from `src/btop_menu.cpp` draw arms plus `msgBox`
//! (`:909-947`) using `btop-draw` primitives (`create_box`, `banner_gen`)
//! and the Default theme via `theme_grad::color`. Menus own LOGIC
//! (`menus.rs`); this module owns BYTES + mouse zones.
//!
//! Fixture setup (mirrors `tests/draw_golden.cpp` menu block + README):
//! 100x30, Default theme, `tty_mode=false`, `rounded_corners=true`,
//! `lowcolor=false`, `theme_background=true`; main `selected=0`, options
//! general tab page 0 selected 0, help page 0, msgboxes width 45 with
//! `{"Golden msgbox line one", "Golden msgbox line two"}` and titles
//! `"golden ok"` / `"golden yesno"`.
//!
//! P3 HANDOFF (options outside-click): C++ `optionsMenu` `:1417-1421`
//! closes the menu (`Closed`) when a `mouse_click` lands OUTSIDE the
//! options box (`mouse_x < x || mouse_x > x + 80 || mouse_y < y + 6 ||
//! mouse_y > y + 6 + height`). `MenuSystem::menu_options` has no geometry
//! and returns `NoChange` on `mouse_click` (documented deviation); P3 must
//! implement the outside-click → close rule in the sink using the `x`/`y`/
//! `height` geometry recomputed here (`options_geometry`) or the live
//! overlay box rect.

use std::collections::HashMap;

use btop_config::theme::hex_to_color;
use btop_draw::ansi::{mv_d, mv_l, mv_r, mv_to, mv_u, FX_B, FX_UB, MV_RESTORE, MV_SAVE};
use btop_draw::boxes::{banner_gen, create_box, BANNER_SRC};
use btop_draw::symbols::box_chars::{
    DIV_DOWN, DIV_LEFT, DIV_RIGHT, DIV_UP, DOWN, ENTER, H_LINE, LEFT, LEFT_DOWN, LEFT_UP, RIGHT,
    RIGHT_DOWN, RIGHT_UP, ROUND_LEFT_DOWN, ROUND_LEFT_UP, ROUND_RIGHT_DOWN, ROUND_RIGHT_UP,
    TITLE_LEFT_DOWN, TITLE_RIGHT_DOWN, UP, V_LINE,
};
use btop_draw::theme_grad::color;
use btop_tools::mouse::MouseMap;
use btop_tools::strtools::{cjust, s_replace, uresize};

use crate::msgbox::BoxKind;
use crate::options::{display_value, OptionsStore};
use crate::tables::{CATEGORIES, HELP_TEXT, MENU_BANNERS, P_SIGNALS};

/// `Fx::bl`/`Fx::ubl` (blink on/off, btop_tools.cpp:743-744) are used by the
/// signal/renice cursor (`Fx::bl + "█" + Fx::ubl`), out of golden scope.
/// Resolved theme escapes for overlay assembly (`Theme::c` under the
/// harness flags: `lowcolor=false`, `theme_background=true`).
#[derive(Debug, Clone)]
pub struct OverlayTheme {
    /// `main_fg` escape.
    pub main_fg: String,
    /// `main_bg` escape.
    pub main_bg: String,
    /// `title` escape.
    pub title: String,
    /// `hi_fg` escape.
    pub hi_fg: String,
    /// `div_line` escape.
    pub div_line: String,
    /// `selected_bg` escape.
    pub selected_bg: String,
    /// `selected_fg` escape.
    pub selected_fg: String,
    /// `Fx::reset` (`reset_base + main_fg + main_bg`).
    pub reset: String,
}

impl OverlayTheme {
    /// Resolve every overlay color from a complete theme map.
    pub fn resolve(theme: &HashMap<String, String>) -> Self {
        let main_fg = color("main_fg", theme, false, true);
        let main_bg = color("main_bg", theme, false, true);
        Self {
            main_fg: main_fg.clone(),
            main_bg: main_bg.clone(),
            title: color("title", theme, false, true),
            hi_fg: color("hi_fg", theme, false, true),
            div_line: color("div_line", theme, false, true),
            selected_bg: color("selected_bg", theme, false, true),
            selected_fg: color("selected_fg", theme, false, true),
            reset: format!("\x1b[0m{main_fg}{main_bg}"),
        }
    }
}

/// Uppercase the first byte (option names are ASCII; mirrors
/// `capitalize`, btop_tools.hpp:194).
fn capitalize(s: &str) -> String {
    let mut out = s.to_string();
    if let Some(b) = out.as_bytes().first() {
        out.replace_range(0..1, &(*b as char).to_ascii_uppercase().to_string());
    }
    out
}

/// Scalar count used for the msgbox centering (`Fx::uncolor(line).size()`
/// on plain-ASCII content).
fn plain_len(s: &str) -> usize {
    s.chars().count()
}

/// `msgBox` overlay (`btop_menu.cpp:909-947`): constructor body plus
/// `operator()`. `boxtype`: 0 = OK, 1 = YES/NO (Yes focused), 2 = YES/NO
/// (No focused). Returns the full overlay string plus the button mouse
/// zones (`button1`, and `button2` for two-button boxes).
#[allow(clippy::too_many_arguments)]
pub fn msgbox_overlay(
    width: i64,
    boxtype: i32,
    content: &[String],
    title: &str,
    selected: i32,
    ot: &OverlayTheme,
    tty_mode: bool,
    rounded: bool,
    term_w: i64,
    term_h: i64,
) -> (String, Vec<MouseMap>) {
    let height = content.len() as i64 + 7;
    let x = term_w / 2 - width / 2;
    let y = term_h / 2 - height / 2;
    let (left_up, right_up, left_down, right_down) = if tty_mode || !rounded {
        (LEFT_UP, RIGHT_UP, LEFT_DOWN, RIGHT_DOWN)
    } else {
        (
            ROUND_LEFT_UP,
            ROUND_RIGHT_UP,
            ROUND_LEFT_DOWN,
            ROUND_RIGHT_DOWN,
        )
    };
    // `:924-925`.
    let button_left = [
        left_up.to_string(),
        H_LINE.repeat(6),
        mv_l(7),
        mv_d(2),
        left_down.to_string(),
        H_LINE.repeat(6),
        mv_l(7),
        mv_u(1),
        V_LINE.to_string(),
    ]
    .concat();
    let button_right = [
        V_LINE.to_string(),
        mv_l(7),
        mv_u(1),
        H_LINE.repeat(6),
        right_up.to_string(),
        mv_l(7),
        mv_d(2),
        H_LINE.repeat(6),
        right_down.to_string(),
        mv_u(2),
    ]
    .concat();
    // `:927-931`.
    let mut box_contents = create_box(
        x,
        y,
        width,
        height,
        &ot.hi_fg,
        true,
        title,
        "",
        0,
        &ot.div_line,
        &ot.hi_fg,
        &ot.title,
        &ot.reset,
        tty_mode,
        rounded,
    ) + &mv_d(1);
    for line in content {
        let pad = (width / 2 - plain_len(line) as i64 / 2 - 1).max(0);
        box_contents += &format!("{MV_SAVE}{}{line}{MV_RESTORE}{}", mv_r(pad), mv_d(1));
    }
    // `:934-947`.
    let mut maps = Vec::new();
    let pos = width / 2 - if boxtype == 0 { 6 } else { 14 };
    let first_color = if selected == 0 {
        ot.hi_fg.clone()
    } else {
        ot.div_line.clone()
    };
    let first_label = if boxtype == 0 {
        "    Ok    "
    } else {
        "    Yes    "
    };
    let first_focus = if selected == 0 {
        ot.title.clone()
    } else {
        format!("{}{FX_UB}", ot.main_fg)
    };
    let mut out = format!(
        "{}{}{FX_B}{first_color}{button_left}{first_focus}{first_label}{first_color}{button_right}",
        mv_d(1),
        mv_r(pos),
    );
    maps.push(MouseMap {
        x: x + pos + 1,
        y: y + height - 4,
        w: 12 + if boxtype > 0 { 1 } else { 0 },
        h: 3,
        action: "button1".to_string(),
    });
    if boxtype > 0 {
        let second_color = if selected == 1 {
            ot.hi_fg.clone()
        } else {
            ot.div_line.clone()
        };
        let second_focus = if selected == 1 {
            ot.title.clone()
        } else {
            format!("{}{FX_UB}", ot.main_fg)
        };
        out += &format!(
            "{}{second_color}{button_left}{second_focus}    No    {second_color}{button_right}",
            mv_r(2),
        );
        maps.push(MouseMap {
            x: x + pos + 15 + 1,
            y: y + height - 4,
            w: 12,
            h: 3,
            action: "button2".to_string(),
        });
    }
    (format!("{box_contents}{out}{}", ot.reset), maps)
}

/// Convenience wrapper taking [`BoxKind`] (`Ok = 0`, `YesNo = 1`).
#[allow(clippy::too_many_arguments)]
pub fn msgbox_kind_overlay(
    width: i64,
    kind: BoxKind,
    content: &[String],
    title: &str,
    ot: &OverlayTheme,
    tty_mode: bool,
    rounded: bool,
    term_w: i64,
    term_h: i64,
) -> (String, Vec<MouseMap>) {
    let (boxtype, selected) = match kind {
        BoxKind::Ok => (0, 0),
        BoxKind::YesNo => (1, 0),
        BoxKind::NoYes => (2, 1),
    };
    msgbox_overlay(
        width, boxtype, content, title, selected, ot, tty_mode, rounded, term_w, term_h,
    )
}

/// Main-menu overlay (`mainMenu` `:1229-1293`). `selected` is 0..2
/// (Options/Help/Quit).
pub fn main_overlay(
    selected: usize,
    term_w: i64,
    term_h: i64,
    ot: &OverlayTheme,
    tty_mode: bool,
    lowcolor: bool,
) -> (String, Vec<MouseMap>) {
    let y = term_h / 2 - 10;
    let bg = banner_gen(
        y,
        0,
        true,
        term_w,
        tty_mode,
        lowcolor,
        &ot.main_fg,
        &ot.reset,
    );
    let selected_colors = [
        hex_to_color(BANNER_SRC[0].0, lowcolor, "fg"),
        hex_to_color(BANNER_SRC[2].0, lowcolor, "fg"),
        hex_to_color(BANNER_SRC[4].0, lowcolor, "fg"),
    ];
    let normal_colors = [
        hex_to_color("#CC", lowcolor, "fg"),
        hex_to_color("#AA", lowcolor, "fg"),
        hex_to_color("#80", lowcolor, "fg"),
    ];
    let widths = [19i64, 12, 12];
    let mut out = format!("{bg}{}{FX_B}", ot.reset);
    let mut maps = Vec::new();
    let mut cy = y + 7;
    for i in 0..3 {
        let lines: &[&str] = if !tty_mode && i == selected {
            &MENU_BANNERS[9 + i * 3..12 + i * 3]
        } else {
            &MENU_BANNERS[i * 3..i * 3 + 3]
        };
        let colors = if i == selected {
            &selected_colors
        } else {
            &normal_colors
        };
        maps.push(MouseMap {
            x: term_w / 2 - widths[i] / 2,
            y: cy,
            w: widths[i],
            h: 3,
            action: format!("button_{i}"),
        });
        for (ic, line) in lines.iter().enumerate() {
            let col = term_w / 2 - widths[i] / 2;
            let prefix = if tty_mode {
                String::new()
            } else {
                colors[ic].clone()
            };
            out += &format!("{}{prefix}{line}", mv_to(cy, col));
            cy += 1;
        }
    }
    out += &ot.reset;
    (out, maps)
}

/// Options-menu geometry (`optionsMenu` `:1360-1363`): `(x, y, height)`.
/// Exported for the P3 outside-click rule (see the module-level handoff).
pub fn options_geometry(term_w: i64, term_h: i64) -> (i64, i64, i64) {
    let max_items = CATEGORIES.iter().map(|c| c.len()).max().unwrap_or(0) as i64;
    let y = (term_h / 2 - 3 - max_items).max(1);
    let x = term_w / 2 - 39;
    let mut height = (term_h - 7).min(max_items * 2 + 4);
    if height % 2 != 0 {
        height -= 1;
    }
    (x, y, height)
}

/// Options-menu overlay (`optionsMenu` `:1357-1371` background +
/// `:1592-1718` rows). Covers the non-editing, warning-free golden path;
/// `editing` swaps the selected value cell for `editor_text` (approximation
/// of `TextEdit::operator()(24)` — golden fixtures never edit), and
/// `warnings` appends a nested warning box.
#[allow(clippy::too_many_arguments)]
pub fn options_overlay(
    tab: usize,
    page: usize,
    selected: usize,
    store: &OptionsStore,
    lists: &HashMap<String, Vec<String>>,
    term_w: i64,
    term_h: i64,
    ot: &OverlayTheme,
    tty_mode: bool,
    lowcolor: bool,
    editing: bool,
    editor_text: &str,
    warnings: Option<&str>,
) -> (String, Vec<MouseMap>) {
    let (x, y, height) = options_geometry(term_w, term_h);
    let title_str = format!("{}tab{}{}", ot.hi_fg, ot.main_fg, RIGHT);
    let mut bg = banner_gen(
        y,
        0,
        true,
        term_w,
        tty_mode,
        lowcolor,
        &ot.main_fg,
        &ot.reset,
    );
    bg += &create_box(
        x,
        y + 6,
        78,
        height,
        &ot.hi_fg,
        true,
        &title_str,
        "",
        0,
        &ot.div_line,
        &ot.hi_fg,
        &ot.title,
        &ot.reset,
        tty_mode,
        true,
    );
    bg += &[
        mv_to(y + 8, x),
        ot.hi_fg.clone(),
        DIV_LEFT.to_string(),
        ot.div_line.clone(),
        H_LINE.repeat(29),
        DIV_UP.to_string(),
        H_LINE.repeat(78 - 32),
        ot.hi_fg.clone(),
        DIV_RIGHT.to_string(),
    ]
    .concat();
    bg += &format!(
        "{}{DIV_DOWN}{}",
        mv_to(y + 6 + height - 1, x + 30),
        ot.div_line
    );
    for i in 0..height - 4 {
        bg += &format!("{}{V_LINE}", mv_to(y + 9 + i, x + 30));
    }
    let mut out = bg;
    let mut maps = Vec::new();

    let tab = tab.min(CATEGORIES.len().saturating_sub(1));
    let n = CATEGORIES[tab].len();
    let item_height = (n as i64).min((height - 4) / 2) as usize;
    let pages = n.div_ceil(item_height.max(1)).max(1);
    let page = page.min(pages - 1);
    let select_max =
        (item_height.saturating_sub(1)).min(n.saturating_sub(1).saturating_sub(item_height * page));
    let selected = selected.min(select_max);

    let sel_name = CATEGORIES[tab][item_height * page + selected][0];
    let sel_kind = crate::options::classify_option(sel_name, store);
    let sel_browsable = matches!(sel_kind, crate::options::OptKind::Browsable);
    let sel_2d = crate::options::is_2d(sel_kind);
    let sel_editable = matches!(
        sel_kind,
        crate::options::OptKind::Editable
            | crate::options::OptKind::Int
            | crate::options::OptKind::Str
    ) && !sel_browsable;

    out += &mv_to(y + 7, x + 4);
    for (i, m) in ["general", "cpu", "gpu", "mem", "net", "proc"]
        .iter()
        .enumerate()
    {
        if i == tab {
            out += &format!("{FX_B}{}[{}{m}{}]", ot.hi_fg, ot.title, ot.hi_fg);
        } else {
            out += &format!("{FX_B}{}{}{}{m} ", ot.hi_fg, i + 1, ot.title);
        }
        out += &mv_r(7);
        maps.push(MouseMap {
            x: x + 2 + 12 * i as i64,
            y: y + 6,
            w: 12,
            h: 3,
            action: format!("select_cat_{}", i + 1),
        });
    }
    if pages > 1 {
        out += &[
            mv_to(y + 6 + height - 1, x + 2),
            ot.hi_fg.clone(),
            TITLE_LEFT_DOWN.to_string(),
            FX_B.to_string(),
            UP.to_string(),
            ot.title.clone(),
            format!(" page {}/{} ", page + 1, pages),
            ot.hi_fg.clone(),
            DOWN.to_string(),
            FX_UB.to_string(),
            TITLE_RIGHT_DOWN.to_string(),
        ]
        .concat();
    }
    let mut cy = y + 9;
    for (row, entry) in CATEGORIES[tab]
        .iter()
        .skip(item_height * page)
        .take(item_height)
        .enumerate()
    {
        let option = entry[0];
        let value = display_value(option, store);
        let mut idx_str = String::new();
        if row == selected && sel_browsable {
            if let Some(list) = lists.get(option) {
                let raw = store.strings.get(option).cloned().unwrap_or_default();
                let mut idx = list.len();
                for (k, p) in list.iter().enumerate() {
                    // Render-path matching (`:1668-1680`): full path,
                    // filename, stem, or legacy absolute filename.
                    let file = p.rsplit('/').next().unwrap_or(p);
                    let stem = match file.rfind('.') {
                        Some(j) => &file[..j],
                        None => file,
                    };
                    let cur_file = raw.rsplit('/').next().unwrap_or(raw.as_str());
                    if p == &raw
                        || file == raw
                        || stem == raw
                        || (raw.starts_with('/') && cur_file == file)
                    {
                        idx = k;
                        break;
                    }
                }
                idx_str = format!(" {}/{}", idx + 1, list.len());
            }
        }
        let name_cell = cjust(
            &(capitalize(&s_replace(option, "_", " ")) + &idx_str),
            29,
            false,
            true,
        );
        if row == selected {
            let sel_colors = format!("{}{}", ot.selected_bg, ot.selected_fg);
            out += &format!("{}{}{FX_B}{name_cell}", mv_to(cy, x + 1), sel_colors,);
            cy += 1;
            let shown = if editing {
                cjust(editor_text, 34, false, true)
            } else {
                cjust(&value, 25, false, true)
            };
            out += &format!("{}{FX_UB}  {shown}  ", mv_to(cy, x + 1));
            cy += 1;
            if !editing && (sel_2d || sel_browsable) {
                out += &format!(
                    "{FX_B}{}{LEFT}{}{RIGHT}",
                    mv_to(cy - 1, x + 2),
                    mv_to(cy - 1, x + 28),
                );
                maps.push(MouseMap {
                    x,
                    y: cy - 2,
                    w: 5,
                    h: 2,
                    action: "left".to_string(),
                });
                maps.push(MouseMap {
                    x: x + 25,
                    y: cy - 2,
                    w: 5,
                    h: 2,
                    action: "right".to_string(),
                });
            }
            if sel_editable {
                let shift = if !editing && matches!(sel_kind, crate::options::OptKind::Int) {
                    2
                } else {
                    0
                };
                let glyph = if tty_mode { "E" } else { ENTER };
                out += &format!("{FX_B}{}{glyph}", mv_to(cy - 1, x + 28 - shift));
            }
            out += &format!("{}{}{FX_B}", ot.reset, ot.title);
            let mut cyy = y + 7;
            for desc in entry.iter() {
                let check = cyy;
                cyy += 1;
                if check == y + 7 {
                    continue;
                }
                if cyy == y + 10 {
                    out += &format!("{}{FX_UB}", ot.main_fg);
                }
                if cyy > y + height + 4 {
                    break;
                }
                out += &format!("{}{desc}", mv_to(cyy, x + 32));
            }
        } else {
            out += &format!("{}{}{FX_B}{name_cell}", mv_to(cy, x + 1), ot.title);
            cy += 1;
            out += &format!(
                "{}{}{FX_UB}  {}  ",
                mv_to(cy, x + 1),
                ot.main_fg,
                cjust(&value, 25, false, true),
            );
            cy += 1;
        }
    }
    if let Some(w) = warnings {
        let uw = uresize(w, 74, false);
        let ww = (plain_len(w) as i64 + 10).min(78);
        let (mout, _) = msgbox_overlay(
            ww,
            0,
            &[uw],
            "warning",
            0,
            ot,
            tty_mode,
            true,
            term_w,
            term_h,
        );
        out += &mout;
    }
    out += &ot.reset;
    (out, maps)
}

/// Help-menu overlay (`helpMenu` `:1753-1793`). `page` is 0-based.
pub fn help_overlay(
    page: usize,
    term_h: i64,
    term_w: i64,
    ot: &OverlayTheme,
    tty_mode: bool,
    lowcolor: bool,
) -> (String, Vec<MouseMap>) {
    let len = HELP_TEXT.len() as i64;
    let y = (term_h / 2 - 4 - len / 2).max(1);
    let x = term_w / 2 - 39;
    let height = (term_h - 6).min(len + 3);
    let per = height - 3;
    let pages = (len + per - 1) / per.max(1);
    let page = page.min(pages as usize - 1);
    let mut out = banner_gen(
        y,
        0,
        true,
        term_w,
        tty_mode,
        lowcolor,
        &ot.main_fg,
        &ot.reset,
    );
    out += &create_box(
        x,
        y + 6,
        78,
        height,
        &ot.hi_fg,
        true,
        "help",
        "",
        0,
        &ot.div_line,
        &ot.hi_fg,
        &ot.title,
        &ot.reset,
        tty_mode,
        true,
    );
    if pages > 1 {
        out += &[
            mv_to(y + height + 6, x + 2),
            ot.hi_fg.clone(),
            TITLE_LEFT_DOWN.to_string(),
            FX_B.to_string(),
            UP.to_string(),
            ot.title.clone(),
            format!(" page {}/{} ", page + 1, pages),
            ot.hi_fg.clone(),
            DOWN.to_string(),
            FX_UB.to_string(),
            TITLE_RIGHT_DOWN.to_string(),
        ]
        .concat();
    }
    let mut cy = y + 7;
    out += &format!(
        "{}{}{FX_B}{}Description:",
        mv_to(cy, x + 1),
        ot.title,
        cjust("Key:", 20, false, true),
    );
    cy += 1;
    let per = (height - 3) as usize;
    for (key, desc) in HELP_TEXT.iter().skip(per * page).take(per) {
        out += &format!(
            "{}{}{FX_B}{}{}{FX_UB}{desc}",
            mv_to(cy, x + 1),
            ot.hi_fg,
            cjust(key, 20, false, true),
            ot.main_fg,
        );
        cy += 1;
    }
    out += &ot.reset;
    (out, Vec::new())
}

/// `signalSend` confirmation box (btop_menu.cpp:1146-1158): `MsgBox{50,
/// 1, content, "signal"}`. Content lines carry their colors inline (as in
/// C++); the title names the signal except for 0/1/17/out-of-range.
/// `msg_selected`: 0 = Yes focused, 1 = No.
pub fn signal_send_overlay(
    pid: u64,
    pname: &str,
    signum: i32,
    msg_selected: i32,
    ot: &OverlayTheme,
    term_w: i64,
    term_h: i64,
) -> (String, Vec<MouseMap>) {
    // C++ indexes `P_Signals` 1-based and guards `> 1 && <= 32 && != 17`
    // for the title (the `<= 32` end is an upstream landmine on a
    // 32-entry table — the port clamps to the valid range).
    let named = (signum > 1 && signum != 17)
        .then(|| P_SIGNALS.get(signum as usize).copied())
        .flatten();
    let sig_name = named.unwrap_or("signal");
    // `hi_fg + N + (valid ? main_fg + " (NAME)" : "")` (:1150-1151).
    let sig_num = if (1..=31).contains(&signum) {
        format!("{signum}{} ({})", ot.main_fg, P_SIGNALS[signum as usize])
    } else {
        signum.to_string()
    };
    let content = vec![
        format!(
            "{FX_B}{}Send signal: {FX_UB}{}{sig_num}",
            ot.main_fg, ot.hi_fg
        ),
        format!(
            "{FX_B}{}To PID: {FX_UB}{}{pid}{} ({}){}",
            ot.main_fg,
            ot.hi_fg,
            ot.main_fg,
            uresize(pname, 16, false),
            ot.reset,
        ),
    ];
    msgbox_overlay(
        50,
        1,
        &content,
        sig_name,
        msg_selected,
        ot,
        false,
        false,
        term_w,
        term_h,
    )
}

/// Rebuild [`crate::menus::MenuSystem::overlay`] + `mouse_maps` for the
/// `SignalReturn`/`Renice` menus render nothing here (out of golden scope
/// — P3 adds them if needed); `SignalSend` renders its confirmation box.
#[allow(clippy::too_many_arguments)]
pub fn render_into(
    current: crate::menus::Menus,
    main_selected: usize,
    tab: usize,
    page: usize,
    selected: usize,
    help_page: usize,
    signal_pid: u64,
    signal_pname: &str,
    signal_to_send: i32,
    msg_selected: i32,
    store: &OptionsStore,
    lists: &HashMap<String, Vec<String>>,
    theme: &HashMap<String, String>,
    term_w: i64,
    term_h: i64,
) -> (String, Vec<MouseMap>) {
    let ot = OverlayTheme::resolve(theme);
    match current {
        crate::menus::Menus::Main => main_overlay(main_selected, term_w, term_h, &ot, false, false),
        crate::menus::Menus::Options => options_overlay(
            tab, page, selected, store, lists, term_w, term_h, &ot, false, false, false, "", None,
        ),
        crate::menus::Menus::Help => help_overlay(help_page, term_h, term_w, &ot, false, false),
        crate::menus::Menus::SignalSend => signal_send_overlay(
            signal_pid,
            signal_pname,
            signal_to_send,
            msg_selected,
            &ot,
            term_w,
            term_h,
        ),
        _ => (String::new(), Vec::new()),
    }
}
