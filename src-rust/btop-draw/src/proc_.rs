//! Proc box (`Proc::draw`, src/btop_draw.cpp:1707-2246).
//!
//! Stateless port of the force-redraw path plus the per-tick value path.
//! C++ keeps `Graph` maps (`p_graphs`/`p_counters`/`p_wide_cmd`), the
//! `box` string, `redraw`/selection statics and the `filter` TextEdit
//! across calls; this port rebuilds them from explicit inputs each call
//! (same bytes for `force_redraw=true, data_same=false`, which is what
//! the golden fixtures use).
//!
//! `data_same=true` returns `prev` unchanged (precedent: cpu/mem/net/gpu;
//! C++ threads `data_same` through the per-graph `operator()` calls, which
//! the stateless port cannot replay without graph state).
//! `force_redraw=false` omits the frame + titles + header (incremental
//! path); detail values and rows still render fully (stateless
//! approximation — byte parity is only tested for `force_redraw=true`).
//!
//! Stateless-Graph contract: per-row CPU sparks are single-sample graphs
//! (`Graph{5,1}` built empty then pushed once on first draw). The push
//! path and `Graph::new` over the one mapped sample produce identical
//! bytes (both end as 4 cursor-skips + 1 braille cell — verified against
//! the fixture), so rows render `Graph::new(.., &[mapped])` directly.
//! Detail graphs are built from the explicit history vecs in
//! [`ProcDetail`] (same contract as every other box).
//!
//! Seams (M4 owns the stateful caller side):
//! - `Draw::TextEdit` filter widget (:179-277): only its TEXT state is
//!   carried here (`filtering` + [`ProcDrawInput::filter`]); cursor,
//!   editing and mouse handling stay in M4.
//! - `Input::mouse_mappings` writes become [`MouseMap`] records in the
//!   caller-provided `&mut Vec<MouseMap>`; M4 wires them to real input.
//! - Follow/restore Config mutation (`:1740-1771` follow scan,
//!   `proc_followed`/`followed_pid`/`should_selection_return_to_followed`
//!   writes) is transcribed as the pure [`resolve_follow`] helper; the
//!   caller persists the returned values.
//! - The `p_counters` 10-decay that retires dead-process graphs and the
//!   `counter >= 100` map purge (:2058-2070, :2508-2522) need cross-call
//!   state: the stateless rule renders a spark iff `cpu_p > 0` (exact on
//!   first draw, which is what the fixtures capture).
//! - `selected_pid`/`selected_name` statics (:2051-2055, :2536-2539) and
//!   the `Runner::stopping` guard are caller concerns (no bytes).
//! - `"!"`-prefixed regex filters need `std::regex`; with zero external
//!   crates only plain-substring filtering is transcribed (see
//!   [`matches_filter`]). Tree-mode filter distribution (`_tree_gen`,
//!   btop_shared.cpp) is collect-side: in tree mode the caller passes
//!   final `tree_index` values and `filter` only drives the title row.
//!
//! `total_mem == 0` (the headless harness: `Shared::totalMem` is never
//! initialised, so `Mem::get_totalMem()` returns 0 and mem fixtures read
//! `Total: 0 Byte`): the C++ `(int)round(p.mem * 100 / totalMem)` integer
//! division traps on x86 but yields 0 on ARM (UDIV by zero), which is what
//! the checked-in fixtures encode (row-0 `m_color` is exactly
//! `process[0]`). This port pins the ARM/harness semantics explicitly:
//! `total_mem == 0` forces the mem-gradient value to 0 (and the
//! percent-mode readout, which the harness never takes, to the
//! `clamp(inf) = 100%` the C++ float path produces).

use crate::ansi::{mv_l, mv_to, FX_B, FX_UB};
use crate::boxes::{create_box, human_bytes, ljust_b, rjust_b, CommonFlags, Palette, ProcGeom};
use crate::meter_graph::{Graph, GraphOpts};
use crate::symbols::box_chars::{
    DIV_DOWN, DIV_LEFT, DIV_RIGHT, DIV_UP, DOWN, ENTER, H_LINE, LEFT, RIGHT, TITLE_LEFT,
    TITLE_LEFT_DOWN, TITLE_RIGHT, TITLE_RIGHT_DOWN, UP, V_LINE,
};
use crate::symbols::graph_table;
use crate::symbols::SUPERSCRIPT;
use crate::theme_grad::{color, gradient};
use btop_tools::strtools::{cjust, ljust, luresize, rjust, rtrim, uresize, wide_ulen};
use std::collections::HashMap;

pub use btop_tools::mouse::MouseMap;

/// One process row: the `Proc::proc_info` fields `draw` reads
/// (src/btop_shared.hpp:382-404). Ordering/sorting, tree prefixes and
/// `tree_index` assignment are collect-side; the vec order is rendered
/// as given (harness: pid order, 3 procs).
#[derive(Debug, Clone)]
pub struct ProcInfo {
    pub pid: u64,
    pub name: String,
    pub cmd: String,
    pub short_cmd: String,
    pub threads: u64,
    pub user: String,
    pub mem: u64, // bytes
    pub cpu_p: f64,
    pub cpu_c: f64,
    pub p_nice: i64,
    pub ppid: u64,
    /// Tree depth from `_tree_gen` (0 for roots / filter-surfaced rows).
    pub depth: usize,
    /// Collapsed by the user (`space`/`+/-`/`C`, `toggle_tree_collapse`).
    /// Persisted across ticks via the previous view; drives `[-]`/`[+]`.
    pub collapsed: bool,
    /// Filtered out or inside a collapsed subtree (`_tree_gen`); hidden
    /// via the `tree_index == len` sentinel (see `is_hidden`).
    pub filtered: bool,
    /// Tree prefix built collect-side (`_tree_gen`); empty in list mode.
    pub prefix: String,
    /// Collect-side tree order; `== procs.len()` hides the row in tree
    /// mode (`:1746/:2049`). Ignored when `proc_tree=false`.
    pub tree_index: usize,
}

/// Detail pane state (`Proc::detailed`, btop_shared.hpp:407-416).
/// `None` in the input = list view (`show_detailed=false`).
#[derive(Debug, Clone)]
pub struct ProcDetail {
    pub entry: ProcInfo,
    /// `detailed.status` (`"Running"`/`"Sleeping"`/`"Dead"`/...).
    pub status: String,
    pub elapsed: String,
    pub parent: String,
    pub io_read: String,
    pub io_write: String,
    /// Pre-humanized `detailed.memory` (`:2033` renders it verbatim).
    pub memory: String,
    /// `detailed.first_mem` (mem-graph `max_value`; -1 when unknown).
    pub first_mem: i64,
    /// `detailed.cpu_percent` full history (cpu-graph data).
    pub cpu_history: Vec<i64>,
    /// `detailed.mem_bytes` full history (mem-graph data).
    pub mem_history: Vec<i64>,
}

/// Every Config key `Proc::draw` reads, grouped (GraphOpts precedent).
/// Each field cites its cpp line.
#[derive(Debug, Clone)]
pub struct ProcFlags {
    pub proc_tree: bool,       // :1709 Config proc_tree
    pub proc_colors: bool,     // :1712 Config proc_colors
    pub proc_gradient: bool,   // :1711 Config proc_gradient (&& !lowcolor)
    pub mem_bytes: bool,       // :1716 Config proc_mem_bytes
    pub vim_keys: bool,        // :1717 Config vim_keys
    pub show_graphs: bool,     // :1718 Config proc_cpu_graphs
    pub pause_proc_list: bool, // :1719 Config pause_proc_list
    pub follow_process: bool,  // :1720 Config follow_process
    pub per_core: bool,        // :1928 Config proc_per_core (button bold)
    pub reversed: bool,        // :1933 Config proc_reversed (button bold)
    pub filtering: bool,       // :1900 Config proc_filtering (TextEdit active)
    /// Resolved `base_10_sizes` (harness false; the `base_10_bitrate`
    /// Auto/True/False override only affects bit+per_second calls, which
    /// proc never makes).
    pub base_10: bool,
    /// Shared box-independent flags (see [`CommonFlags`]).
    pub common: CommonFlags,
}

impl ProcFlags {
    /// Harness defaults (tests/draw_golden.cpp setup() + btop_config.cpp
    /// compiled-in defaults): tree/reversed/per_core/vim_keys/pause/
    /// follow/filtering false; colors/gradient/graphs/mem_bytes true;
    /// base_10 false.
    pub fn harness_defaults() -> Self {
        Self {
            proc_tree: false,
            proc_colors: true,
            proc_gradient: true,
            mem_bytes: true,
            vim_keys: false,
            show_graphs: true,
            pause_proc_list: false,
            follow_process: false,
            per_core: false,
            reversed: false,
            filtering: false,
            base_10: false,
            common: CommonFlags::harness_defaults(),
        }
    }
}

/// All inputs `Proc::draw` reads, as explicit params.
/// Mirrors btop_draw.cpp:1707-2246.
///
/// Borrowed (CpuDrawInput/MemDrawInput precedent): the caller owns the
/// per-frame vecs/strings; this struct only borrows them so `draw_proc`
/// performs no per-frame clone on the hottest path.
#[derive(Debug, Clone)]
pub struct ProcDrawInput<'a> {
    /// `plist` in draw order (collect-side sorted; harness: pid order).
    pub procs: &'a [ProcInfo],
    /// `Proc::numpids` (:1733; harness 3).
    pub numpids: i64,
    /// `Mem::get_totalMem()` (:1732; harness 0 — see module docs).
    pub total_mem: u64,
    /// Config `proc_sorting` title string (:1916; harness "pid").
    pub sorting: &'a str,
    pub start: i64,    // :1726 Config proc_start (resolved view offset)
    pub selected: i64, // :1727 Config proc_selected (0 = none)
    /// `proc_followed` (:1722; list-middle anchor when following).
    pub followed: i64,
    pub followed_pid: i64, // :1721 Config followed_pid (0 = none)
    /// Config `detailed_pid` (follow-restore comparison, :1762).
    pub detailed_pid: i64,
    /// Config `restore_detailed_pid` (:1740; 0 = no restore).
    pub restore_pid: i64,
    /// Config `update_following` (:1741; follow-scan gate).
    pub update_following: bool,
    /// Config `should_selection_return_to_followed` (:1723).
    pub should_return: bool,
    /// Config `proc_last_selected` (:1779; last-row highlight tracking).
    pub last_selected: i64,
    /// Previous `is_last_process_in_list` static (:1780-1788).
    pub was_last: bool,
    /// Previous `previous_proc_banner_state` static (:1770-1776).
    pub prev_banner: bool,
    /// Stored `proc_filter` text (title row when `!filtering`; `None` =
    /// empty). The active-edit text while `filtering` (TextEdit seam).
    pub filter: Option<&'a str>,
    /// Detail pane (`show_detailed && last_pid == detailed_pid`);
    /// `None` = list view (harness).
    pub detailed: Option<&'a ProcDetail>,
    pub graph_symbol_cfg: &'a str,      // :1714 Config graph_symbol
    pub graph_symbol_proc_cfg: &'a str, // :1714 Config graph_symbol_proc
    pub flags: ProcFlags,
    pub force_redraw: bool,    // :1734 force_redraw → redraw
    pub data_same: bool,       // per-graph data_same
    pub prev: Option<&'a str>, // cached out for data_same
}

/// Plain-substring filter match (`Proc::matches_filter` non-`!` arm,
/// btop_shared.cpp:176-193): pid digits or case-insensitive
/// name/cmd/user hit. `"!"` regex filters are M4 (zero external crates):
/// bare `"!"` matches everything (C++), longer `"!"` patterns match
/// nothing (documented approximation — C++ tries `std::regex`).
pub fn matches_filter(p: &ProcInfo, filter: &str) -> bool {
    if filter.is_empty() {
        return true;
    }
    if let Some(pat) = filter.strip_prefix('!') {
        // M4 seam: regex search over pid/name/cmd/user.
        return pat.is_empty();
    }
    let lower = filter.to_lowercase();
    p.pid.to_string().contains(filter)
        || p.name.to_lowercase().contains(&lower)
        || p.cmd.to_lowercase().contains(&lower)
        || p.user.to_lowercase().contains(&lower)
}

/// Row hidden from the list (`:1746` follow-scan skip + `:2049` row skip).
/// In tree mode filtering is collect-side (`_tree_gen`); rows hide via
/// the `filtered` flag OR the `tree_index` sentinel (both arms mirror
/// `:2049` — the sentinel alone is not sufficient: collapsed/filtered
/// descendants can carry small indices from the `tree_sort` quirk).
fn is_hidden(p: &ProcInfo, filter: Option<&str>, tree: bool, list_len: usize) -> bool {
    if tree {
        return p.filtered || p.tree_index == list_len;
    }
    match filter {
        Some(f) if !f.is_empty() => !matches_filter(p, f),
        _ => false,
    }
}

/// ASCII-control sanitizer (`Tools::replace_ascii_control`, default
/// replacement `' '`). Fixture commands are clean ASCII so this is the
/// identity there; multibyte wide-char handling passes non-ASCII through
/// (documented approximation of the `mbstowcs` path).
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_control() { ' ' } else { c })
        .collect()
}

/// `{:.2}` CPU value with C++ trimming (`:2139-2146`): 3 chars below 10
/// and from 100 (inclusive) below 1000, `k`-suffixed thousands at/above
/// 10000, full 2-decimal form otherwise (the later `rjust(.,4)` crops it).
fn cpu_str(cpu_p: f64) -> String {
    if cpu_p >= 10_000.0 {
        let mut s = format!("{:.2}", cpu_p / 1000.0);
        s.truncate(3);
        if s.ends_with('.') {
            s.pop();
        }
        s.push('k');
        return s;
    }
    let mut s = format!("{cpu_p:.2}");
    if cpu_p < 10.0 || (100.0..1000.0).contains(&cpu_p) {
        s.truncate(3);
    }
    s
}

/// Follow/restore view resolution (`:1740-1776`), pure transcription with
/// Config writes returned instead of stored. Returns
/// `(start, selected, followed, follow_process, followed_pid,
/// should_return, select_max, banner)`.
#[allow(clippy::too_many_arguments)]
pub fn resolve_follow(
    procs: &[ProcInfo],
    filter: Option<&str>,
    tree: bool,
    follow_process: bool,
    pause_proc_list: bool,
    update_following: bool,
    followed_pid: i64,
    followed: i64,
    restore_pid: i64,
    detailed_pid: i64,
    start: i64,
    selected: i64,
    select_max: i64,
    numpids: i64,
    should_return: bool,
    prev_banner: bool,
) -> (i64, i64, i64, bool, i64, bool, i64, bool) {
    let mut start = start;
    let mut selected = selected;
    let mut followed = followed;
    let mut follow_process = follow_process;
    let mut followed_pid = followed_pid;
    let mut should_return = should_return;
    let mut banner = pause_proc_list || follow_process;
    let mut select_max = select_max;
    if (follow_process && (!pause_proc_list || update_following)) || restore_pid > 0 {
        // :1736 update_following reset (caller persists `false`).
        let list_len = procs.len();
        let mut loc = 1i64;
        let mut can_follow = false;
        let target = if restore_pid > 0 {
            restore_pid
        } else {
            followed_pid
        };
        for p in procs {
            if is_hidden(p, filter, tree, list_len) {
                continue;
            }
            if p.pid as i64 == target {
                can_follow = true;
                break;
            }
            loc += 1;
        }
        if can_follow {
            let list_middle = if select_max % 2 == 0 {
                select_max / 2
            } else {
                select_max / 2 + 1
            };
            start = (loc - list_middle).max(0);
            followed = if loc < list_middle {
                loc
            } else if start > numpids - select_max {
                select_max - numpids + loc
            } else {
                list_middle
            };
            if restore_pid == 0 {
                // :1753-1754 proc_followed + should-return writes.
                should_return = true;
            }
            selected = if followed_pid != detailed_pid || restore_pid > 0 {
                followed
            } else {
                0
            };
        } else if restore_pid == 0 {
            followed_pid = 0;
            follow_process = false;
            banner = pause_proc_list;
            followed = 0;
            if !banner {
                select_max += 1;
            }
        }
        // :1765 restore_detailed_pid reset (caller persists 0).
    }
    // :1770-1776 banner-change / pause adjustments.
    if banner != prev_banner && !banner && start + select_max - 1 == numpids {
        selected += 1;
    } else if pause_proc_list && selected > select_max {
        start += 1;
    }
    (
        start,
        selected,
        followed,
        follow_process,
        followed_pid,
        should_return,
        select_max,
        banner,
    )
}

/// Last-row highlight tracking (`:1779-1788`). Only the returned flag
/// affects bytes (the `↓` button color); `redraw` is a no-op statelessly
/// (the caller always rebuilds).
pub fn resolve_is_last(
    selected: i64,
    last_selected: i64,
    start: i64,
    select_max: i64,
    numpids: i64,
    was_last: bool,
) -> bool {
    if selected != last_selected {
        selected >= select_max && start >= numpids - select_max
    } else {
        was_last
    }
}

/// Field widths (`:1806-1815`). Non-negative clamp (C++ would reinterpret
/// negatives as huge `size_t`; real layouts keep them non-negative —
/// harness S0: prog 18, cmd hidden, tree 28).
struct FieldWidths {
    user: i64,
    thread: i64,
    prog: i64,
    cmd: i64,
    tree: i64,
}

fn field_widths(width: i64, show_graphs: bool) -> FieldWidths {
    let user = if width < 75 { 5 } else { 10 };
    let thread = if width < 75 { -1 } else { 4 };
    let prog = if width > 70 {
        16
    } else if width > 55 {
        8
    } else {
        width - user - thread - 33
    };
    let mut cmd = if width > 55 {
        width - prog - user - thread - 33
    } else {
        -1
    };
    let mut tree = width - user - thread - 23;
    if !show_graphs {
        cmd += 5;
        tree += 5;
    }
    // `cmd == 0` still counts as hidden (`cmd_size > 0` gates use).
    FieldWidths {
        user: user.max(0),
        thread,
        prog: prog.max(0),
        cmd,
        tree: tree.max(0),
    }
}

/// Detail header (redraw block `:1918-1990`): divider + pid/name titles,
/// action buttons with mouse maps, labels, command lines.
#[allow(clippy::too_many_arguments)]
fn render_detail_header(
    y: i64,
    x: i64,
    width: i64,
    d_y: i64,
    d_x: i64,
    d_width: i64,
    dgraph_x: i64,
    dgraph_width: i64,
    detail: &ProcDetail,
    vim_keys: bool,
    follow_process: bool,
    selected: i64,
    pal: &Palette,
    title_left: &str,
    title_right: &str,
    maps: &mut Vec<MouseMap>,
) -> String {
    let mut out = String::new();
    let alive = detail.status != "Dead";
    let pid_str = detail.entry.pid.to_string();
    // :1927-1932 divider + titles.
    out += &mv_to(y, x);
    out += &pal.box_color;
    out += DIV_LEFT;
    out += H_LINE;
    out += title_left;
    out += &pal.hi_fg;
    out += FX_B;
    out += SUPERSCRIPT[4];
    out += &pal.title;
    out += "proc";
    out += FX_UB;
    out += title_right;
    out += &H_LINE.repeat((width - 10).max(0) as usize);
    out += DIV_RIGHT;
    out += &mv_to(d_y, dgraph_x + 2);
    out += title_left;
    out += FX_B;
    out += &pal.title;
    out += &pid_str;
    out += FX_UB;
    out += title_right;
    out += title_left;
    out += FX_B;
    out += &pal.title;
    out += &uresize(
        &detail.entry.name,
        (dgraph_width - pid_str.len() as i64 - 7).max(0) as usize,
        true,
    );
    out += FX_UB;
    out += title_right;
    // :1934 divider column.
    out += &mv_to(d_y, d_x - 1);
    out += &pal.box_color;
    out += DIV_UP;
    out += &mv_to(y, d_x - 1);
    out += DIV_DOWN;
    out += &pal.div_line;
    for i in 1..8 {
        out += &mv_to(d_y + i, d_x - 1);
        out += V_LINE;
    }
    // :1937-1957 action buttons.
    let t_color = if !alive || selected > 0 {
        pal.inactive.clone()
    } else {
        pal.title.clone()
    };
    let hi_color = if !alive || selected > 0 {
        t_color.clone()
    } else {
        pal.hi_fg.clone()
    };
    let mut mouse_x = d_x + 2;
    out += &mv_to(d_y, d_x + 1);
    if width > 55 {
        out += FX_UB;
        out += title_left;
        out += &hi_color;
        out += FX_B;
        out += "t";
        out += &t_color;
        out += "erminate";
        out += FX_UB;
        out += title_right;
        if alive && selected == 0 {
            maps.push(MouseMap {
                x: mouse_x,
                y: d_y,
                w: 9,
                h: 1,
                action: "t".to_string(),
            });
        }
        mouse_x += 11;
    }
    let kill_key = if vim_keys { "K" } else { "k" };
    out += title_left;
    out += &hi_color;
    out += FX_B;
    out += kill_key;
    out += &t_color;
    out += "ill";
    out += FX_UB;
    out += title_right;
    out += title_left;
    out += &hi_color;
    out += FX_B;
    out += "s";
    out += &t_color;
    out += "ignals";
    out += FX_UB;
    out += title_right;
    out += title_left;
    out += &hi_color;
    out += FX_B;
    out += "N";
    out += &t_color;
    out += "ice";
    out += FX_UB;
    out += title_right;
    if alive && selected == 0 {
        maps.push(MouseMap {
            x: mouse_x,
            y: d_y,
            w: 4,
            h: 1,
            action: kill_key.to_string(),
        });
        mouse_x += 6;
        maps.push(MouseMap {
            x: mouse_x,
            y: d_y,
            w: 7,
            h: 1,
            action: "s".to_string(),
        });
        mouse_x += 9;
        maps.push(MouseMap {
            x: mouse_x,
            y: d_y,
            w: 5,
            h: 1,
            action: "N".to_string(),
        });
        mouse_x += 7;
    }
    if width > 77 {
        out += title_left;
        if follow_process {
            out += FX_B;
        }
        out += &hi_color;
        out += "F";
        out += &t_color;
        out += "ollow";
        out += FX_UB;
        out += title_right;
        if selected == 0 {
            maps.push(MouseMap {
                x: mouse_x,
                y: d_y,
                w: 6,
                h: 1,
                action: "F".to_string(),
            });
        }
    }
    // :1959-1970 labels.
    let item_fit = ((d_width - 2) as f64 / 10.0).floor() as i64;
    let item_width = if item_fit.min(8) > 0 {
        ((d_width - 2) as f64 / item_fit.min(8) as f64).floor() as i64
    } else {
        0
    };
    let iw = item_width.max(0) as usize;
    out += &mv_to(d_y + 1, d_x + 1);
    out += FX_B;
    out += &pal.title;
    out += &cjust("Status:", iw, false, true);
    out += &cjust("Elapsed:", iw, false, true);
    if item_fit >= 3 {
        out += &cjust("IO/R:", iw, false, true);
    }
    if item_fit >= 4 {
        out += &cjust("IO/W:", iw, false, true);
    }
    if item_fit >= 5 {
        out += &cjust("Parent:", iw, false, true);
    }
    if item_fit >= 6 {
        out += &cjust("User:", iw, false, true);
    }
    if item_fit >= 7 {
        out += &cjust("Threads:", iw, false, true);
    }
    if item_fit >= 8 {
        out += &cjust("Nice:", iw, false, true);
    }
    // :1973-1983 command lines.
    for (i, l) in ['C', 'M', 'D'].iter().enumerate() {
        out += &mv_to(d_y + 5 + i as i64, d_x + 1);
        out += &l.to_string();
    }
    out += &pal.main_fg;
    out += FX_UB;
    let san_cmd = sanitize(&detail.entry.cmd);
    let cmd_size = san_cmd.chars().count() as i64;
    let denom = (d_width - 5).max(1);
    let num_lines = (3).min((cmd_size as f64 / denom as f64).ceil() as i64);
    for i in 0..num_lines {
        let take = (cmd_size - denom * i).max(0) as usize;
        out += &mv_to(d_y + 5 + if num_lines == 1 { 1 } else { i }, d_x + 3);
        out += &cjust(
            &luresize(&san_cmd, take, true),
            (d_width - 5).max(0) as usize,
            true,
            true,
        );
    }
    out
}

/// Detail values (per-frame `:1993-2027`): cpu graph + readout, info
/// values, memory line with mem graph.
#[allow(clippy::too_many_arguments)]
fn render_detail_values(
    d_y: i64,
    d_x: i64,
    d_width: i64,
    dgraph_x: i64,
    dgraph_width: i64,
    detail: &ProcDetail,
    pause: bool,
    total_mem: u64,
    base_symbol: &str,
    theme: &HashMap<String, String>,
    lowcolor: bool,
    tbg: bool,
    pal: &Palette,
    graph_bg: &str,
) -> String {
    let mut out = String::new();
    let alive = detail.status != "Dead";
    let item_fit = ((d_width - 2) as f64 / 10.0).floor() as i64;
    let item_width = if item_fit.min(8) > 0 {
        ((d_width - 2) as f64 / item_fit.min(8) as f64).floor() as i64
    } else {
        0
    };
    let iw = item_width.max(0) as usize;
    // :1999-2005 cpu graph + readout.
    let cpu_graph = Graph::new(
        GraphOpts {
            width: (dgraph_width - 1).max(0) as usize,
            height: 7,
            gradient: gradient("cpu", theme, false),
            symbol: base_symbol.to_string(),
            invert: false,
            no_zero: true,
            max_value: 0,
            offset: 0,
        },
        &pal.reset,
        &detail.cpu_history,
    );
    let cpu_s = if alive || pause {
        let v = detail.entry.cpu_p;
        let prec = if v < 9.995 {
            2
        } else if v < 99.95 {
            1
        } else {
            0
        };
        format!("{v:>4.prec$}", prec = prec)
    } else {
        String::new()
    };
    out += &mv_to(d_y + 1, dgraph_x + 1);
    out += FX_UB;
    out += cpu_graph.render();
    out += &mv_to(d_y + 1, dgraph_x + 1);
    out += &pal.title;
    out += FX_B;
    out += &cpu_s;
    out += "%";
    for (i, l) in ['C', 'P', 'U'].iter().enumerate() {
        out += &mv_to(d_y + 3 + i as i64, dgraph_x + 1);
        out += &l.to_string();
    }
    // :2007-2017 info values.
    let stat_color = if !alive {
        pal.inactive.clone()
    } else if detail.status == "Running" {
        color("proc_misc", theme, lowcolor, tbg)
    } else {
        pal.main_fg.clone()
    };
    out += &mv_to(d_y + 2, d_x + 1);
    out += &stat_color;
    out += FX_UB;
    out += &cjust(&detail.status, iw, false, true);
    out += &pal.main_fg;
    out += &cjust(&detail.elapsed, iw, false, true);
    if item_fit >= 3 {
        out += &cjust(&detail.io_read, iw, false, true);
    }
    if item_fit >= 4 {
        out += &cjust(&detail.io_write, iw, false, true);
    }
    if item_fit >= 5 {
        out += &cjust(&detail.parent, iw, false, true);
    }
    if item_fit >= 6 {
        out += &cjust(&detail.entry.user, iw, false, true);
    }
    if item_fit >= 7 {
        out += &cjust(&detail.entry.threads.to_string(), iw, false, true);
    }
    if item_fit >= 8 {
        out += &cjust(&detail.entry.p_nice.to_string(), iw, false, true);
    }
    // :2019-2027 memory line. DEVIATION from a clean Rust port: when
    // `total_mem == 0`, the C++ btop_draw.cpp:2026 divides by zero and
    // `fmt::format("{:.2f}", inf)` → "inf", then `mem_str.resize(4)` pads
    // with a NUL byte. The harness fixture captures this NUL faithfully
    // (it lives between "M:" and "%" in the memory line). To match
    // byte-for-byte, the Rust port reproduces the divide-by-zero path
    // (f64 → "inf"/"nan") and pads with NUL on truncation. Real-world
    // btop never hits this path (total_mem is always nonzero after
    // `Shared::init`), so the deviation is fixture-only.
    let mem_back = detail.mem_history.last().copied().unwrap_or(0) as u64;
    let mem_p = if total_mem == 0 {
        if mem_back == 0 {
            0.0
        } else {
            f64::INFINITY
        }
    } else {
        (mem_back as f64 * 100.0 / total_mem as f64).clamp(0.0, 100.0)
    };
    let mut mem_s = format!("{mem_p:.2}");
    mem_s.truncate(4);
    if mem_s.ends_with('.') {
        mem_s.pop();
    }
    // Pad to 4 bytes with NUL (matches C++ `string::resize(4)` on a 3-byte
    // "inf" / "nan"). The NUL is what produces the embedded 0 in the
    // fixture at the "M:inf\0%" position.
    while mem_s.len() < 4 {
        mem_s.push('\0');
    }
    let mem_graph = Graph::new(
        GraphOpts {
            width: (d_width / 3).max(0) as usize,
            height: 1,
            gradient: Vec::new(),
            symbol: base_symbol.to_string(),
            invert: false,
            no_zero: false,
            max_value: detail.first_mem,
            offset: 0,
        },
        &pal.reset,
        &detail.mem_history,
    );
    out += &mv_to(d_y + 4, d_x + 1);
    out += &pal.title;
    out += FX_B;
    let mem_label =
        (if item_fit > 4 { "Memory: " } else { "M:" }.to_string()) + &rjust(&mem_s, 4, true) + "% ";
    out += &rjust(&mem_label, (d_width / 3 - 2).max(0) as usize, true);
    out += &pal.inactive;
    out += FX_UB;
    out += &graph_bg.repeat((d_width / 3).max(0) as usize);
    out += &mv_l(d_width / 3);
    out += &color("proc_misc", theme, lowcolor, tbg);
    out += mem_graph.render();
    out += " ";
    out += &pal.title;
    out += FX_B;
    out += &detail.memory;
    out
}

/// Filter title row (`:1995-1999`): `f[ilter] [text] [del]/[↵]`.
#[allow(clippy::too_many_arguments)]
fn render_filter_row(
    input: &ProcDrawInput,
    y: i64,
    x: i64,
    width: i64,
    pal: &Palette,
    title_left: &str,
    title_right: &str,
    maps: &mut Vec<MouseMap>,
) -> String {
    let mut out = String::new();
    let filtering = input.flags.filtering;
    let filter_text = input.filter.unwrap_or("");
    let filter_shown = uresize(filter_text, (6).max(width - 66) as usize, false);
    // :1995-1999 filter title.
    out += &mv_to(y, x + 9);
    out += title_left;
    if !filter_shown.is_empty() {
        out += FX_B;
    }
    out += &pal.hi_fg;
    out += "f";
    out += &pal.title;
    if !filter_shown.is_empty() {
        out += " ";
        out += &filter_shown;
    } else {
        out += "ilter";
    }
    if !filtering && !filter_shown.is_empty() {
        out += &pal.hi_fg;
        out += " del";
    }
    if filtering {
        out += &pal.hi_fg;
        out += " ";
        out += ENTER;
    }
    out += FX_UB;
    out += title_right;
    if !filtering {
        let f_len = if filter_shown.is_empty() {
            6
        } else {
            filter_shown.chars().count() as i64 + 2
        };
        maps.push(MouseMap {
            x: x + 10,
            y,
            w: f_len,
            h: 1,
            action: "f".to_string(),
        });
        if !filter_shown.is_empty() {
            maps.push(MouseMap {
                x: x + 11 + f_len,
                y,
                w: 3,
                h: 1,
                action: "delete".to_string(),
            });
        }
    }
    out
}

/// Pause / per-core / reverse / tree / sorting arrows (`:2009-2039`).
#[allow(clippy::too_many_arguments)]
fn render_sort_buttons(
    input: &ProcDrawInput,
    y: i64,
    width: i64,
    sort_pos: i64,
    pal: &Palette,
    title_left: &str,
    title_right: &str,
    maps: &mut Vec<MouseMap>,
) -> String {
    let mut out = String::new();
    let f = &input.flags;
    let sorting = input.sorting;
    // :2009-2039 pause / per-core / reverse / tree / sorting.
    let sort_len = sorting.len() as i64;
    if width > 60 + sort_len {
        out += &mv_to(y, sort_pos - 32);
        out += title_left;
        if f.pause_proc_list {
            out += FX_B;
        }
        out += &pal.title;
        out += "pa";
        out += &pal.hi_fg;
        out += "u";
        out += &pal.title;
        out += "se";
        out += FX_UB;
        out += title_right;
        maps.push(MouseMap {
            x: sort_pos - 31,
            y,
            w: 5,
            h: 1,
            action: "u".to_string(),
        });
    }
    if width > 55 + sort_len {
        out += &mv_to(y, sort_pos - 25);
        out += title_left;
        if f.per_core {
            out += FX_B;
        }
        out += &pal.title;
        out += "per-";
        out += &pal.hi_fg;
        out += "c";
        out += &pal.title;
        out += "ore";
        out += FX_UB;
        out += title_right;
        maps.push(MouseMap {
            x: sort_pos - 24,
            y,
            w: 8,
            h: 1,
            action: "c".to_string(),
        });
    }
    if width > 45 + sort_len {
        out += &mv_to(y, sort_pos - 15);
        out += title_left;
        if f.reversed {
            out += FX_B;
        }
        out += &pal.hi_fg;
        out += "r";
        out += &pal.title;
        out += "everse";
        out += FX_UB;
        out += title_right;
        maps.push(MouseMap {
            x: sort_pos - 14,
            y,
            w: 7,
            h: 1,
            action: "r".to_string(),
        });
    }
    if width > 35 + sort_len {
        out += &mv_to(y, sort_pos - 6);
        out += title_left;
        if f.proc_tree {
            out += FX_B;
        }
        out += &pal.title;
        out += "tre";
        out += &pal.hi_fg;
        out += "e";
        out += FX_UB;
        out += title_right;
        maps.push(MouseMap {
            x: sort_pos - 5,
            y,
            w: 4,
            h: 1,
            action: "e".to_string(),
        });
    }
    out += &mv_to(y, sort_pos);
    out += title_left;
    out += FX_B;
    out += &pal.hi_fg;
    out += LEFT;
    out += " ";
    out += &pal.title;
    out += sorting;
    out += " ";
    out += &pal.hi_fg;
    out += RIGHT;
    out += FX_UB;
    out += title_right;
    maps.push(MouseMap {
        x: sort_pos + 1,
        y,
        w: 2,
        h: 1,
        action: "left".to_string(),
    });
    maps.push(MouseMap {
        x: sort_pos + sort_len + 3,
        y,
        w: 2,
        h: 1,
        action: "right".to_string(),
    });
    out
}

/// Column field labels (`:2076-2089`): Pid/Program/Command or Tree plus
/// Threads/User/Mem/Cpu headers.
fn render_header(input: &ProcDrawInput, y: i64, x: i64, fw: &FieldWidths, pal: &Palette) -> String {
    let mut out = String::new();
    let f = &input.flags;
    // :2076-2089 field labels.
    out += &mv_to(y + 1, x + 1);
    out += &pal.title;
    out += FX_B;
    if !f.proc_tree {
        out += &rjust("Pid:", 8, true);
        out += " ";
        out += &ljust("Program:", fw.prog.max(0) as usize, true);
        out += " ";
        if fw.cmd > 0 {
            out += &ljust("Command:", fw.cmd as usize, true);
        }
        out += " ";
    } else {
        out += &ljust("Tree:", fw.tree.max(0) as usize, true);
        out += " ";
    }
    if fw.thread > 0 {
        out += &mv_l(4);
        out += "Threads: ";
    }
    out += &ljust("User:", fw.user.max(0) as usize, true);
    out += " ";
    out += &rjust(if f.mem_bytes { "MemB" } else { "Mem%" }, 5, true);
    out += " ";
    out += &rjust("Cpu%", if f.show_graphs { 10 } else { 5 }, true);
    out += FX_UB;
    out
}

/// Bottom select / info / terminate / kill / signals / nice / follow
/// buttons (`:2041-2074`).
#[allow(clippy::too_many_arguments)]
fn render_action_buttons(
    input: &ProcDrawInput,
    y: i64,
    x: i64,
    width: i64,
    height: i64,
    selected: i64,
    is_last: bool,
    pal: &Palette,
    title_left_down: &str,
    title_right_down: &str,
    maps: &mut Vec<MouseMap>,
) -> String {
    let mut out = String::new();
    let f = &input.flags;
    // :2041-2074 select / info / signal / follow buttons.
    let down_button = (if is_last {
        pal.inactive.clone()
    } else {
        pal.hi_fg.clone()
    }) + DOWN;
    let up_lit = selected != 0
        || (f.follow_process && input.followed_pid == input.detailed_pid && input.should_return);
    let up_button = (if up_lit {
        pal.hi_fg.clone()
    } else {
        pal.inactive.clone()
    }) + UP;
    let t_color = if selected == 0 {
        pal.inactive.clone()
    } else {
        pal.title.clone()
    };
    let hi_color = if selected == 0 {
        pal.inactive.clone()
    } else {
        pal.hi_fg.clone()
    };
    let mut mouse_x = x + 14;
    out += &mv_to(y + height - 1, x + 1);
    out += title_left_down;
    out += FX_B;
    out += &hi_color;
    out += &up_button;
    out += &pal.title;
    out += " select ";
    out += &down_button;
    out += FX_UB;
    out += title_right_down;
    out += title_left_down;
    out += FX_B;
    out += &t_color;
    out += "info ";
    out += &hi_color;
    out += ENTER;
    out += FX_UB;
    out += title_right_down;
    if selected > 0 {
        maps.push(MouseMap {
            x: mouse_x,
            y: y + height - 1,
            w: 6,
            h: 1,
            action: "info_enter".to_string(),
        });
    }
    mouse_x += 8;
    if width > 60 {
        out += title_left_down;
        out += FX_B;
        out += &hi_color;
        out += "t";
        out += &t_color;
        out += "erminate";
        out += FX_UB;
        out += title_right_down;
        if selected > 0 {
            maps.push(MouseMap {
                x: mouse_x,
                y: y + height - 1,
                w: 9,
                h: 1,
                action: "t".to_string(),
            });
        }
        mouse_x += 11;
    }
    if width > 55 {
        out += title_left_down;
        out += FX_B;
        out += &hi_color;
        out += if f.vim_keys { "K" } else { "k" };
        out += &t_color;
        out += "ill";
        out += FX_UB;
        out += title_right_down;
        if selected > 0 {
            maps.push(MouseMap {
                x: mouse_x,
                y: y + height - 1,
                w: 4,
                h: 1,
                action: if f.vim_keys {
                    "K".to_string()
                } else {
                    "k".to_string()
                },
            });
        }
        mouse_x += 6;
    }
    out += title_left_down;
    out += FX_B;
    out += &hi_color;
    out += "s";
    out += &t_color;
    out += "ignals";
    out += FX_UB;
    out += title_right_down;
    if selected > 0 {
        maps.push(MouseMap {
            x: mouse_x,
            y: y + height - 1,
            w: 7,
            h: 1,
            action: "s".to_string(),
        });
    }
    mouse_x += 9;
    out += title_left_down;
    out += FX_B;
    out += &hi_color;
    out += "N";
    out += &t_color;
    out += "ice";
    out += FX_UB;
    out += title_right_down;
    if selected > 0 {
        maps.push(MouseMap {
            x: mouse_x,
            y: y + height - 1,
            w: 5,
            h: 1,
            action: "N".to_string(),
        });
    }
    mouse_x += 6;
    if width > 72 {
        out += title_left_down;
        if f.follow_process {
            out += FX_B;
        }
        out += &hi_color;
        out += "F";
        out += &t_color;
        out += "ollow";
        out += FX_UB;
        out += title_right_down;
        if selected > 0 {
            maps.push(MouseMap {
                x: mouse_x,
                y: y + height - 1,
                w: 6,
                h: 1,
                action: "F".to_string(),
            });
        }
    }
    out
}

/// Filter + sort + bottom buttons + header (redraw block `:1993-2090`).
/// Orchestration only: delegates to the four section helpers
/// (`render_filter_row` / `render_sort_buttons` / `render_action_buttons` /
/// `render_header`), each owning one contiguous C++ range.
#[allow(clippy::too_many_arguments)]
fn render_titles(
    input: &ProcDrawInput,
    y: i64,
    x: i64,
    width: i64,
    height: i64,
    selected: i64,
    sort_pos: i64,
    is_last: bool,
    fw: &FieldWidths,
    pal: &Palette,
    title_left: &str,
    title_right: &str,
    title_left_down: &str,
    title_right_down: &str,
    maps: &mut Vec<MouseMap>,
) -> String {
    let mut out = String::new();
    out += &render_filter_row(input, y, x, width, pal, title_left, title_right, maps);
    out += &render_sort_buttons(
        input,
        y,
        width,
        sort_pos,
        pal,
        title_left,
        title_right,
        maps,
    );
    out += &render_action_buttons(
        input,
        y,
        x,
        width,
        height,
        selected,
        is_last,
        pal,
        title_left_down,
        title_right_down,
        maps,
    );
    out += &render_header(input, y, x, fw, pal);
    out
}

/// One list row (`:2042-2167`): selection/follow highlight or the
/// `proc_colors`/`proc_gradient` fade, then the tree or normal line plus
/// the shared user/mem/graph/cpu tail.
#[allow(clippy::too_many_arguments)]
fn render_row(
    y: i64,
    x: i64,
    lc: i64,
    p: &ProcInfo,
    is_selected: bool,
    is_followed: bool,
    select_max: i64,
    calc: i64,
    fw: &FieldWidths,
    flags: &ProcFlags,
    total_mem: u64,
    base_symbol: &str,
    graph_bg: &str,
    process_grad: &[String],
    proc_color_grad: &[String],
    proc_fade: &[String],
    theme: &HashMap<String, String>,
    pal: &Palette,
) -> String {
    let lowcolor = flags.common.lowcolor;
    let tbg = flags.common.theme_background;
    let mut out = String::new();
    out += &pal.reset;
    // :2071-2105 highlight or gradient fade.
    let (c_color, m_color, t_color, g_color, end) = if is_selected || is_followed {
        let highlight = if is_followed {
            color("followed_bg", theme, lowcolor, tbg) + &color("followed_fg", theme, lowcolor, tbg)
        } else {
            color("selected_bg", theme, lowcolor, tbg) + &color("selected_fg", theme, lowcolor, tbg)
        };
        out += &highlight;
        out += FX_B;
        (
            FX_B.to_string(),
            FX_B.to_string(),
            FX_B.to_string(),
            String::new(),
            FX_UB.to_string(),
        )
    } else if flags.proc_colors {
        let mem_v: i64 = p
            .mem
            .saturating_mul(100)
            .checked_div(total_mem)
            .unwrap_or(0) as i64; // total_mem == 0 → 0: ARM/harness div-by-zero semantics (see module docs).
        let vs = [p.cpu_p.round() as i64, mem_v, (p.threads / 3) as i64];
        let faded = flags.proc_gradient && !lowcolor;
        let mut cols = [String::new(), String::new(), String::new()];
        for (i, &v) in vs.iter().enumerate() {
            if faded {
                let val = v.min(100) + 100 - calc * 100 / select_max.max(1);
                cols[i] = if val < 100 {
                    proc_color_grad[val.max(0) as usize].clone()
                } else {
                    process_grad[(val - 100).clamp(0, 100) as usize].clone()
                };
            } else {
                cols[i] = process_grad[v.clamp(0, 100) as usize].clone();
            }
        }
        let g = if faded {
            proc_fade[(calc * 100 / select_max.max(1)).clamp(0, 100) as usize].clone()
        } else {
            String::new()
        };
        (
            cols[0].clone(),
            cols[1].clone(),
            cols[2].clone(),
            g,
            pal.main_fg.clone() + FX_UB,
        )
    } else {
        (
            FX_B.to_string(),
            FX_B.to_string(),
            FX_B.to_string(),
            String::new(),
            FX_UB.to_string(),
        )
    };

    let san_cmd = sanitize(&p.cmd);
    // `p_wide_cmd` (:2108): scalar count vs column width differ.
    let wide = san_cmd.chars().count() != wide_ulen(&san_cmd);
    if !flags.proc_tree {
        // :2111-2115 normal line.
        out += &mv_to(y + 2 + lc, x + 1);
        out += &g_color;
        out += &rjust(&p.pid.to_string(), 8, true);
        out += " ";
        out += &c_color;
        out += &ljust(&p.name, fw.prog.max(0) as usize, true);
        out += " ";
        out += &end;
        if fw.cmd > 0 {
            out += &g_color;
            out += &ljust(&san_cmd, fw.cmd as usize, true);
            out += &mv_to(y + 2 + lc, x + 11 + fw.prog + fw.cmd);
            out += " ";
        }
    } else {
        // :2117-2135 tree line.
        let prefix_pid = format!("{}{}", p.prefix, p.pid);
        let mut width_left = fw.tree;
        out += &mv_to(y + 2 + lc, x + 1);
        out += &g_color;
        out += &uresize(&prefix_pid, width_left.max(0) as usize, false);
        out += " ";
        width_left -= prefix_pid.chars().count() as i64;
        if width_left > 0 {
            out += &c_color;
            out += &uresize(&p.name, (width_left - 1).max(0) as usize, false);
            out += &end;
            out += " ";
            width_left -= p.name.chars().count() as i64 + 1;
        }
        if width_left > 7 {
            let cmd_view = if width_left > 40 {
                rtrim(&san_cmd, " ").to_string()
            } else {
                p.short_cmd.clone()
            };
            if !cmd_view.is_empty() && cmd_view != p.name {
                out += &g_color;
                out += "(";
                out += &uresize(&cmd_view, (width_left - 3).max(0) as usize, wide);
                out += ") ";
                width_left -= wide_ulen(&cmd_view) as i64 + 3;
            }
        }
        out += &" ".repeat(width_left.max(0) as usize);
        out += &mv_to(y + 2 + lc, x + 2 + fw.tree);
    }
    // :2137-2167 common tail.
    let cpu_s = cpu_str(p.cpu_p);
    let mem_s = if flags.mem_bytes {
        human_bytes(p.mem, true, flags.base_10)
    } else {
        let mem_p = if total_mem == 0 {
            100.0 // C++ float path: clamp(inf) (harness never takes this arm).
        } else {
            (p.mem as f64 * 100.0 / total_mem as f64).clamp(0.0, 100.0)
        };
        let mut s = format!("{mem_p:.1}");
        if s.len() > 3 {
            s.truncate(3);
        }
        if s.ends_with('.') {
            s.pop();
        }
        s + "%"
    };
    let threads_s = if p.threads > 9999 {
        format!("{}K", p.threads / 1000)
    } else {
        p.threads.to_string()
    };
    if fw.thread > 0 {
        out += &t_color;
        out += &rjust(&threads_s, fw.thread as usize, true);
        out += " ";
        out += &end;
    }
    // Byte-exact user crop (`user.size() > user_size`, utf=false path):
    // ASCII byte cut + pad (harness users are ASCII).
    let user_cut = if fw.user > 0 && p.user.len() > fw.user as usize {
        let mut cut = (fw.user - 1).max(0) as usize;
        while cut > 0 && !p.user.is_char_boundary(cut) {
            cut -= 1;
        }
        format!("{}+", &p.user[..cut])
    } else {
        p.user.clone()
    };
    out += &g_color;
    out += &ljust_b(&user_cut, fw.user.max(0) as usize);
    out += " ";
    out += &m_color;
    out += &rjust_b(&mem_s, 5);
    out += &end;
    out += " ";
    if !(is_selected || is_followed) {
        out += &pal.inactive;
    }
    if flags.show_graphs {
        out += &graph_bg.repeat(5);
    }
    if flags.show_graphs && p.cpu_p > 0.0 {
        let mapped = if p.cpu_p >= 0.1 && p.cpu_p < 5.0 {
            5
        } else {
            p.cpu_p.round() as i64
        };
        let spark = Graph::new(
            GraphOpts {
                width: 5,
                height: 1,
                gradient: Vec::new(),
                symbol: base_symbol.to_string(),
                invert: false,
                no_zero: false,
                max_value: 0,
                offset: 0,
            },
            &pal.reset,
            &[mapped],
        );
        out += &mv_l(5);
        out += &c_color;
        out += spark.render();
    }
    out += &end;
    out += " ";
    out += &c_color;
    out += &rjust(&cpu_s, 4, true);
    out += "  ";
    out += &end;
    out
}

/// Draw the proc box. `geom` is the proc part of `Layout` (calcSizes);
/// `theme` is the Default-keyed map (`default_theme()`). Mouse mappings
/// accumulate into `maps` (M4 wiring).
pub fn draw_proc(
    input: &ProcDrawInput,
    geom: &ProcGeom,
    theme: &HashMap<String, String>,
    maps: &mut Vec<MouseMap>,
) -> String {
    // data_same → cached out.
    if input.data_same {
        return input.prev.unwrap_or("").to_string();
    }
    let f = &input.flags;
    let lowcolor = f.common.lowcolor;
    let tbg = f.common.theme_background;
    let pal = Palette::new("proc_box", theme, lowcolor, tbg);

    // Symbol resolution (:1714 + Graph ctor :498-501).
    let base_symbol: &str = if f.common.tty_mode || input.graph_symbol_proc_cfg == "tty" {
        "tty"
    } else if input.graph_symbol_proc_cfg != "default" {
        input.graph_symbol_proc_cfg
    } else {
        input.graph_symbol_cfg
    };
    let table_key = format!("{base_symbol}_up");
    let graph_bg = graph_table(&table_key)
        .map(|t| t[6])
        .unwrap_or(" ")
        .to_string();

    let title_left = format!("{}{}", pal.box_color, TITLE_LEFT);
    let title_right = format!("{}{}", pal.box_color, TITLE_RIGHT);
    let title_left_down = format!("{}{}", pal.box_color, TITLE_LEFT_DOWN);
    let title_right_down = format!("{}{}", pal.box_color, TITLE_RIGHT_DOWN);

    let detailed = input.detailed.is_some();
    // :1728-1731 y/height/select_max.
    let (x, width) = (geom.base.x, geom.base.width);
    let (y, height) = if detailed {
        (geom.base.y + 8, geom.base.height - 8)
    } else {
        (geom.base.y, geom.base.height)
    };
    let banner0 = f.pause_proc_list || f.follow_process;
    let mut select_max = if detailed {
        if banner0 {
            geom.select_max - 9
        } else {
            geom.select_max - 8
        }
    } else if banner0 {
        geom.select_max - 1
    } else {
        geom.select_max
    };

    // Follow/restore resolution (pure; caller persists the Config half).
    let (start0, selected0, _followed0, _fp0, _fpid0, _sr0, select_max0, banner) = resolve_follow(
        input.procs,
        input.filter,
        f.proc_tree,
        f.follow_process,
        f.pause_proc_list,
        input.update_following,
        input.followed_pid,
        input.followed,
        input.restore_pid,
        input.detailed_pid,
        input.start,
        input.selected,
        select_max,
        input.numpids,
        input.should_return,
        input.prev_banner,
    );
    select_max = select_max0;
    // :2030-2036 selection/view bounds.
    let mut start = start0;
    let mut selected = selected0;
    if start > 0 && input.numpids <= select_max {
        start = 0;
    }
    if start > input.numpids - select_max {
        start = (input.numpids - select_max).max(0);
    }
    if selected > select_max {
        selected = select_max;
    }
    if selected > input.numpids {
        selected = input.numpids;
    }
    let is_last = resolve_is_last(
        selected,
        input.last_selected,
        start,
        select_max,
        input.numpids,
        input.was_last,
    );

    let mut out = String::new();
    if input.force_redraw {
        // :1797 frame (`out = box`).
        out += &create_box(
            x,
            geom.base.y,
            width,
            geom.base.height,
            &pal.box_color,
            true,
            "proc",
            "",
            4,
            &pal.div_line,
            &pal.hi_fg,
            &pal.title,
            &pal.reset,
            f.common.tty_mode,
            f.common.rounded,
        );
        let fw = field_widths(width, f.show_graphs);
        if let Some(detail) = input.detailed {
            let dgraph_width = (width / 3).max(width - 121);
            let d_width = width - dgraph_width - 1;
            let d_x = x + dgraph_width + 1;
            let d_y = geom.base.y;
            out += &render_detail_header(
                y,
                x,
                width,
                d_y,
                d_x,
                d_width,
                x,
                dgraph_width,
                detail,
                f.vim_keys,
                f.follow_process,
                selected,
                &pal,
                &title_left,
                &title_right,
                maps,
            );
        }
        let sort_pos = x + width - input.sorting.len() as i64 - 8;
        out += &render_titles(
            input,
            y,
            x,
            width,
            height,
            selected,
            sort_pos,
            is_last,
            &fw,
            &pal,
            &title_left,
            &title_right,
            &title_left_down,
            &title_right_down,
            maps,
        );
    }

    // Detail values (per-frame, :1993+).
    if let Some(detail) = input.detailed {
        let dgraph_width = (width / 3).max(width - 121);
        let d_width = width - dgraph_width - 1;
        let d_x = x + dgraph_width + 1;
        let d_y = geom.base.y;
        out += &render_detail_values(
            d_y,
            d_x,
            d_width,
            x,
            dgraph_width,
            detail,
            f.pause_proc_list,
            input.total_mem,
            base_symbol,
            theme,
            lowcolor,
            tbg,
            &pal,
            &graph_bg,
        );
    }

    // :2042-2167 process rows.
    let process_grad = gradient("process", theme, lowcolor);
    let proc_color_grad = gradient("proc_color", theme, lowcolor);
    let proc_fade = gradient("proc", theme, lowcolor);
    let filter_ref = input.filter;
    let list_len = input.procs.len();
    let fw = field_widths(width, f.show_graphs);
    let mut lc = 0i64;
    let mut n = 0i64;
    // `selected_pid` static (:2051-2055): the selected row's pid, used by
    // the detail hide button below; reset when nothing is selected.
    let mut selected_pid = 0u64;
    for p in input.procs {
        if is_hidden(p, filter_ref, f.proc_tree, list_len) {
            continue;
        }
        if n < start {
            n += 1;
            continue;
        }
        n += 1;
        let is_selected = lc + 1 == selected;
        if is_selected {
            selected_pid = p.pid;
        }
        let is_followed = input.followed_pid == p.pid as i64;
        let calc = if selected > lc {
            selected - lc
        } else {
            lc - selected
        };
        out += &render_row(
            y,
            x,
            lc,
            p,
            is_selected,
            is_followed,
            select_max,
            calc,
            &fw,
            f,
            input.total_mem,
            base_symbol,
            &graph_bg,
            &process_grad,
            &proc_color_grad,
            &proc_fade,
            theme,
            &pal,
        );
        // :2169-2170 line bound (`lc++` post-increment, verbatim: the
        // first arm breaks on the pre-increment value, the second on the
        // post-increment value when the banner steals a row).
        let old = lc;
        lc += 1;
        if old > height - 5 || (lc > height - 5 && banner) {
            break;
        }
    }

    // :2172-2185 blanks + banner.
    out += &pal.reset;
    while lc < height - 3 {
        lc += 1;
        out += &mv_to(y + lc + 1, x + 1);
        out += &" ".repeat((width - 2).max(0) as usize);
    }
    if banner {
        let label = if f.pause_proc_list && f.follow_process {
            "Paused list and Following process"
        } else if f.pause_proc_list {
            "Process list paused"
        } else {
            "Following process"
        };
        let bg = if f.pause_proc_list && f.follow_process {
            color("proc_banner_bg", theme, lowcolor, tbg)
        } else if f.pause_proc_list {
            color("proc_pause_bg", theme, lowcolor, tbg)
        } else {
            color("proc_follow_bg", theme, lowcolor, tbg)
        };
        out += &mv_to(y + height - 2, x + 1);
        out += &bg;
        out += &color("proc_banner_fg", theme, lowcolor, tbg);
        out += FX_B;
        // Centered in width-2 (`{:^w$}`, no truncation — C++ fmt same).
        let w = (width - 2).max(0) as usize;
        let pad = w.saturating_sub(label.len());
        out += &" ".repeat(pad / 2);
        out += label;
        out += &" ".repeat(pad - pad / 2);
        out += &pal.reset;
    }

    // :2187-2195 scrollbar.
    if input.numpids > select_max {
        let scroll_pos = ((start as f64 * select_max as f64
            / (input.numpids - select_max).max(1) as f64)
            .round() as i64)
            .clamp(0, height - 5);
        out += &mv_to(y + 1, x + width - 2);
        out += FX_B;
        out += &pal.main_fg;
        out += UP;
        out += &mv_to(y + height - 2, x + width - 2);
        out += DOWN;
        for i in y + 2..y + height - 2 {
            out += &mv_to(i, x + width - 2);
            out += if i == y + 2 + scroll_pos { "█" } else { " " };
        }
    }

    // :2197-2199 location counter.
    let location = format!(
        "{}/{}",
        start
            + if f.follow_process {
                input.followed
            } else {
                selected
            },
        input.numpids
    );
    let loc_clear = H_LINE.repeat((9i64 - location.len() as i64).max(0) as usize);
    out += &mv_to(
        y + height - 1,
        x + width - 3 - (9).max(location.len() as i64),
    );
    out += FX_UB;
    out += &pal.box_color;
    out += &loc_clear;
    out += TITLE_LEFT_DOWN;
    out += &pal.title;
    out += FX_B;
    out += &location;
    out += FX_UB;
    out += &pal.box_color;
    out += TITLE_RIGHT_DOWN;

    // :2206-2215 hide button (detail view).
    if input.detailed.is_some() {
        let dgraph_width = (width / 3).max(width - 121);
        let d_width = width - dgraph_width - 1;
        let d_x = x + dgraph_width + 1;
        let d_y = geom.base.y;
        // `selected_pid != detailed_pid && selected > 0` (:2207).
        let greyed = selected_pid as i64 != input.detailed_pid && selected > 0;
        out += &mv_to(d_y, d_x + d_width - 10);
        out += &pal.box_color;
        out += TITLE_LEFT;
        out += FX_B;
        out += if greyed { &pal.inactive } else { &pal.title };
        out += "hide ";
        if !greyed {
            out += &pal.hi_fg;
        }
        out += ENTER;
        out += FX_UB;
        out += &pal.box_color;
        out += TITLE_RIGHT;
        if !greyed {
            maps.push(MouseMap {
                x: d_x + d_width - 9,
                y: d_y,
                w: 6,
                h: 1,
                action: "enter".to_string(),
            });
        }
    }

    out += &pal.reset; // :2228 (+Fx::reset)
    out
}
