//! Net box (`Net::draw`, src/btop_draw.cpp:1499-1602).
//!
//! Stateless port of the force-redraw path plus the per-tick value path.
//! C++ keeps `Graph` maps, the `box` string, `redraw` and the `old_ip`
//! watcher across calls; this port rebuilds them from explicit inputs each
//! call (same bytes for `force_redraw=true, data_same=false`, which is
//! what the golden fixtures use).
//!
//! `data_same=true` returns `prev` unchanged (documents
//! `Graph::operator(data_same=true)` returning cached `out`).
//! `force_redraw=false` omits the frame (outer/inner boxes + selector
//! buttons) unless the IP changed (`old_ip != ip`, btop_draw.cpp:1508-1511
//! forces `redraw` — transcribed via the explicit `old_ip` input).
//! Graphs are still fully rebuilt (stateless approximation — byte parity is
//! only tested for `force_redraw=true`).
//!
//! Stateless-Graph contract: callers must pass full histories, never deltas
//! (every `Graph::new` below receives the complete `Vec` contents;
//! `Graph::push` incrementalism is not used).
//!
//! Reuse: `floating_humanizer` with `HumanOpts` (Task 2) for every speed /
//! total; `base_10_sizes` (plus the `base_10_bitrate` Auto rule for
//! bit+per_second) is resolved by the caller into `NetFlags::base_10`.

use crate::ansi::{mv_to, FX_B, FX_UB};
use crate::boxes::{create_box, human_bytes, ljust_b, rjust_b, CommonFlags, NetGeom, Palette};
use crate::meter_graph::{Graph, GraphOpts};
use crate::theme_grad::gradient;
use btop_tools::mouse::MouseMap;
use btop_tools::strtools::{floating_humanizer, uresize, HumanOpts};
use std::collections::HashMap;

/// One direction's counters, mirroring `Net::net_stat`
/// (src/btop_shared.hpp:323-328). Byte counts are `u64` (humanizer input).
#[derive(Debug, Clone)]
pub struct NetStat {
    pub speed: u64,
    pub top: u64,
    pub total: u64,
    pub offset: u64,
}

/// Every Config key `Net::draw` reads, grouped (GraphOpts precedent).
/// Each field cites its cpp line.
#[derive(Debug, Clone)]
pub struct NetFlags {
    pub net_sync: bool,             // :1502 Config net_sync
    pub net_auto: bool,             // :1502 Config net_auto
    pub swap_upload_download: bool, // :1505 Config swap_upload_download
    /// Resolved base-10 flag: `base_10_sizes`, or `base_10_bitrate`
    /// True/False when it overrides Auto for bit+per_second calls
    /// (btop_tools.cpp:426-434).
    pub base_10: bool,
    /// Shared box-independent flags (see [`CommonFlags`]).
    pub common: CommonFlags,
}

impl NetFlags {
    /// Harness defaults (tests/draw_golden.cpp setup() + btop_config.cpp
    /// compiled-in defaults): net_auto/net_sync true, swap false,
    /// base_10 false (`base_10_sizes=false`, `base_10_bitrate="Auto"`).
    pub fn harness_defaults() -> Self {
        Self {
            net_sync: true,
            net_auto: true,
            swap_upload_download: false,
            base_10: false,
            common: CommonFlags::harness_defaults(),
        }
    }
}

/// All inputs `Net::draw` reads, as explicit params.
/// Mirrors btop_draw.cpp:1499-1602.
pub struct NetDrawInput<'a> {
    /// `net.bandwidth` (:1525/:1528-1534 — full histories per direction).
    pub bandwidth: &'a HashMap<String, Vec<i64>>,
    /// `net.stat` (:1540/:1575-1578).
    pub stat: &'a HashMap<String, NetStat>,
    pub ipv4: &'a str,           // :1507 net.ipv4
    pub ipv6: &'a str,           // :1507 net.ipv6
    pub connected: bool,         // :1572 net.connected
    pub selected_iface: &'a str, // :1516 Net::selected_iface
    /// `Net::graph_max` (:1517-1518, auto-scale ceilings).
    pub graph_max: &'a HashMap<String, u64>,
    /// Config `net_download`/`net_upload` fixed MiB values (:1517-1518,
    /// used when `net_auto=false`).
    pub net_download_cfg: i64,
    pub net_upload_cfg: i64,
    pub graph_symbol_cfg: &'a str,     // :1506 Config graph_symbol
    pub graph_symbol_net_cfg: &'a str, // :1506 Config graph_symbol_net
    /// Previous-draw IP (`Net::old_ip`, :1495/:1508-1511). Differs from the
    /// current `ipv4/ipv6` pick ⟹ frame re-emitted even when
    /// `!force_redraw`.
    pub old_ip: &'a str,
    pub flags: NetFlags,
    pub force_redraw: bool,    // :1501 force_redraw → redraw
    pub data_same: bool,       // :1572 Graph data_same
    pub prev: Option<&'a str>, // cached out for data_same
}

// (Shared just/humanizer/Palette helpers live in boxes.rs — Step 0 dedupe.
// Net's bit/per_second humanizer calls stay explicit: mem's `human_bytes`
// shape only covers bit=false/per_second=false, which is just the max-label
// call below.)

/// Draw the net box. `geom` is the net part of `Layout` (calcSizes);
/// `theme` is the Default-keyed map (`default_theme()`).
pub fn draw_net(
    input: &NetDrawInput,
    geom: &NetGeom,
    theme: &HashMap<String, String>,
    maps: &mut Vec<MouseMap>,
) -> String {
    // data_same → cached out (Graph::operator(data_same=true) returns `out`).
    if input.data_same {
        return input.prev.unwrap_or("").to_string();
    }
    let f = &input.flags;
    let lowcolor = f.common.lowcolor;
    let tbg = f.common.theme_background;

    let pal = Palette::new("net_box", theme, lowcolor, tbg);

    // Symbol resolution (:1506 + Graph ctor :498-501).
    let base_symbol: &str = if f.common.tty_mode || input.graph_symbol_net_cfg == "tty" {
        "tty"
    } else if input.graph_symbol_net_cfg != "default" {
        input.graph_symbol_net_cfg
    } else {
        input.graph_symbol_cfg
    };

    let (x, y, width, height) = (geom.base.x, geom.base.y, geom.base.width, geom.base.height);
    let (b_x, b_y, b_width, b_height) = (geom.b_x, geom.b_y, geom.b_width, geom.b_height);

    // Title glyphs (:1514-1515; note the embedded Fx::ub).
    let title_left = format!(
        "{}{}{}",
        pal.box_color,
        FX_UB,
        crate::symbols::box_chars::TITLE_LEFT
    );
    let title_right = format!(
        "{}{}{}",
        pal.box_color,
        FX_UB,
        crate::symbols::box_chars::TITLE_RIGHT
    );

    // IP pick + change watcher (:1507-1511).
    let ip_addr = if input.ipv4.is_empty() {
        input.ipv6
    } else {
        input.ipv4
    };
    let ip_changed = input.old_ip != ip_addr;

    // Graph ceilings (:1517-1518).
    let down_max = if f.net_auto {
        input.graph_max.get("download").copied().unwrap_or(0)
    } else {
        ((input.net_download_cfg as u64) << 20) / 8
    };
    let up_max = if f.net_auto {
        input.graph_max.get("upload").copied().unwrap_or(0)
    } else {
        ((input.net_upload_cfg as u64) << 20) / 8
    };

    // i_size (:1516; byte size, as C++ string::size — iface names ASCII).
    let i_size = (input.selected_iface.len() as i64).min(15);

    let mut out = String::new();
    // NOTE: the inner create_box needs div_line, not box_color.
    // Rebuild the frame here instead of delegating the inner box color.
    let frame = if input.force_redraw || ip_changed {
        let mut fr = String::new();
        fr += &create_box(
            x,
            y,
            width,
            height,
            &pal.box_color,
            true,
            "net",
            "",
            3,
            &pal.div_line,
            &pal.hi_fg,
            &pal.title,
            &pal.reset,
            f.common.tty_mode,
            f.common.rounded,
        );
        let (up_title, down_title) = if f.swap_upload_download {
            ("upload", "download")
        } else {
            ("download", "upload")
        };
        fr += &create_box(
            b_x,
            b_y,
            b_width,
            b_height,
            "",
            false,
            up_title,
            down_title,
            0,
            &pal.div_line,
            &pal.hi_fg,
            &pal.title,
            &pal.reset,
            f.common.tty_mode,
            f.common.rounded,
        );
        fr += &render_frame_buttons(input, geom, &pal, &title_left, &title_right, i_size, maps);
        fr
    } else {
        String::new()
    };
    out += &frame;

    // Empty bandwidth → frame + reset only (:1525-1526). (C++ only reaches
    // this under `redraw` — under `!redraw` it would throw on the empty
    // `graphs` map; the stateless port returns whatever frame was emitted
    // instead. The smoke test uses force_redraw=true.)
    let empty_dl = input
        .bandwidth
        .get("download")
        .map(|v| v.is_empty())
        .unwrap_or(true);
    let empty_ul = input
        .bandwidth
        .get("upload")
        .map(|v| v.is_empty())
        .unwrap_or(true);
    if empty_dl || empty_ul {
        return out + &pal.reset;
    }

    // IP or device address (:1557-1559; always rendered, not just on frame).
    if !ip_addr.is_empty() && width - i_size - 36 > ip_addr.len() as i64 {
        out += &mv_to(y, x + 8);
        out += &title_left;
        out += &pal.title;
        out += FX_B;
        out += ip_addr;
        out += &title_right;
    }

    // Graphs (:1528-1534). Note the height swap: download uses
    // u_graph_height, upload uses d_graph_height.
    let empty_vec: Vec<i64> = Vec::new();
    let dl_data = input
        .bandwidth
        .get("download")
        .map(|v| v.as_slice())
        .unwrap_or(&empty_vec);
    let ul_data = input
        .bandwidth
        .get("upload")
        .map(|v| v.as_slice())
        .unwrap_or(&empty_vec);
    let dl_graph = Graph::new(
        GraphOpts {
            width: (width - b_width - 2).max(0) as usize,
            height: geom.u_graph_height.max(0) as usize,
            gradient: gradient("download", theme, lowcolor),
            symbol: base_symbol.to_string(),
            invert: f.swap_upload_download,
            no_zero: true,
            max_value: down_max as i64,
            offset: 0,
        },
        &pal.reset,
        dl_data,
    );
    let ul_graph = Graph::new(
        GraphOpts {
            width: (width - b_width - 2).max(0) as usize,
            height: geom.d_graph_height.max(0) as usize,
            gradient: gradient("upload", theme, lowcolor),
            symbol: base_symbol.to_string(),
            invert: !f.swap_upload_download,
            no_zero: true,
            max_value: up_max as i64,
            offset: 0,
        },
        &pal.reset,
        ul_data,
    );

    // Graphs + stats (:1562-1595; `{"download", "upload"}` order).
    for dir in ["download", "upload"] {
        // Graph position (:1567-1571; XNOR table in the comment).
        if (!f.swap_upload_download && dir == "download")
            || (f.swap_upload_download && dir == "upload")
        {
            out += &mv_to(y + 1, x + 1);
        } else {
            out += &mv_to(
                y + geom.u_graph_height + 1 + ((height * f.swap_upload_download as i64) % 2),
                x + 1,
            );
        }
        let graph = if dir == "download" {
            &dl_graph
        } else {
            &ul_graph
        };
        // `redraw or data_same or not connected` (:1572): on this path the
        // flag is always true, so C++ returns the cached `out`, which equals
        // the construction render emitted below. No branch is stubbed out —
        // a disconnected redraw tick renders full graphs (proven by
        // `net_smoke_disconnected`); the read below keeps the `connected`
        // dependency explicit for future incremental ports (same
        // documentation-read pattern as cpu.rs:839 / boxes.rs:875).
        let _ = input.connected;
        out += graph.render();
        // Max label (:1573-1574).
        out += &mv_to(
            y + 1 + ((dir == "upload") != f.swap_upload_download) as i64 * (height - 3),
            x + 1,
        );
        out += FX_UB;
        out += &pal.graph_text;
        out += &human_bytes(
            if dir == "upload" { up_max } else { down_max },
            true,
            f.base_10,
        );
        let stat = input.stat.get(dir);
        let speed = floating_humanizer(
            stat.map(|s| s.speed).unwrap_or(0),
            0,
            HumanOpts {
                shorten: false,
                bit: false,
                per_second: true,
                base_10: f.base_10,
            },
        ); // :1575
        let speed_bits = if b_width >= 20 {
            floating_humanizer(
                stat.map(|s| s.speed).unwrap_or(0),
                0,
                HumanOpts {
                    shorten: false,
                    bit: true,
                    per_second: true,
                    base_10: f.base_10,
                },
            )
        } else {
            String::new()
        }; // :1576
        let top = floating_humanizer(
            stat.map(|s| s.top).unwrap_or(0),
            0,
            HumanOpts {
                shorten: false,
                bit: true,
                per_second: true,
                base_10: f.base_10,
            },
        ); // :1577
        let total = human_bytes(stat.map(|s| s.total).unwrap_or(0), false, f.base_10); // :1578
        let symbol = if dir == "upload" { "▲" } else { "▼" }; // :1579
        if (f.swap_upload_download && dir == "upload")
            || (!f.swap_upload_download && dir == "download")
        {
            // Top graph rows (:1580-1586).
            out += &mv_to(b_y + 1, b_x + 1);
            out += FX_UB;
            out += &pal.main_fg;
            out += symbol;
            out += " ";
            out += &ljust_b(&speed, 10);
            if b_width >= 20 {
                out += &rjust_b(&format!("({speed_bits})"), 13);
            }
            if b_height >= 8 {
                out += &mv_to(b_y + 2, b_x + 1);
                out += symbol;
                out += " ";
                out += "Top: ";
                out += &rjust_b(&format!("({top}"), if b_width >= 20 { 17 } else { 9 });
                out += ")";
            }
            if b_height >= 6 {
                out += &mv_to(b_y + 2 + (b_height >= 8) as i64, b_x + 1);
                out += symbol;
                out += " ";
                out += "Total: ";
                out += &rjust_b(&total, if b_width >= 20 { 16 } else { 8 });
            }
        } else {
            // Bottom graph rows (:1587-1593).
            out += &mv_to(b_y + b_height - (b_height / 2), b_x + 1);
            out += FX_UB;
            out += &pal.main_fg;
            out += symbol;
            out += " ";
            out += &ljust_b(&speed, 10);
            if b_width >= 20 {
                out += &rjust_b(&format!("({speed_bits})"), 13);
            }
            if b_height >= 8 {
                out += &mv_to(b_y + b_height - (b_height / 2) + 1, b_x + 1);
                out += symbol;
                out += " ";
                out += "Top: ";
                out += &rjust_b(&format!("({top}"), if b_width >= 20 { 17 } else { 9 });
                out += ")";
            }
            if b_height >= 6 {
                out += &mv_to(
                    b_y + b_height - (b_height / 2) + 1 + (b_height >= 8) as i64,
                    b_x + 1,
                );
                out += symbol;
                out += " ";
                out += "Total: ";
                out += &rjust_b(&total, if b_width >= 20 { 16 } else { 8 });
            }
        }
    }

    out += &pal.reset; // :1598 (+Fx::reset)
    out
}

/// Interface selector + zero/auto/sync buttons (:1538-1553), split from
/// [`render_frame`] so the inner-box color fix above stays local.
fn render_frame_buttons(
    input: &NetDrawInput,
    geom: &NetGeom,
    pal: &Palette,
    title_left: &str,
    title_right: &str,
    i_size: i64,
    maps: &mut Vec<MouseMap>,
) -> String {
    let f = &input.flags;
    let (x, y, width) = (geom.base.x, geom.base.y, geom.base.width);
    let mut out = String::new();
    let dl_off = input.stat.get("download").map(|s| s.offset).unwrap_or(0);
    let ul_off = input.stat.get("upload").map(|s| s.offset).unwrap_or(0);
    out += &mv_to(y, x + width - i_size - 9);
    out += title_left;
    out += FX_B;
    out += &pal.hi_fg;
    out += crate::symbols::box_chars::LEFT;
    out += "b ";
    out += &pal.title;
    out += &uresize(input.selected_iface, 15, false);
    out += &pal.hi_fg;
    out += " n";
    out += crate::symbols::box_chars::RIGHT;
    out += title_right;
    // Interface selector + zero (:1544-1550) ride with the pixels.
    maps.push(MouseMap {
        x: x + width - i_size - 8,
        y,
        w: 3,
        h: 1,
        action: "b".to_string(),
    });
    maps.push(MouseMap {
        x: x + width - 6,
        y,
        w: 3,
        h: 1,
        action: "n".to_string(),
    });
    out += &mv_to(y, x + width - i_size - 15);
    out += title_left;
    out += &pal.hi_fg;
    if dl_off + ul_off > 0 {
        out += FX_B;
    }
    out += "z";
    out += &pal.title;
    out += "ero";
    out += title_right;
    maps.push(MouseMap {
        x: x + width - i_size - 14,
        y,
        w: 4,
        h: 1,
        action: "z".to_string(),
    });
    if width - i_size - 20 > 6 {
        out += &mv_to(y, x + width - i_size - 21);
        out += title_left;
        out += &pal.hi_fg;
        if f.net_auto {
            out += FX_B;
        }
        out += "a";
        out += &pal.title;
        out += "uto";
        out += title_right;
        maps.push(MouseMap {
            x: x + width - i_size - 20,
            y,
            w: 4,
            h: 1,
            action: "a".to_string(),
        });
    }
    if width - i_size - 20 > 13 {
        out += &mv_to(y, x + width - i_size - 27);
        out += title_left;
        out += &pal.title;
        if f.net_sync {
            out += FX_B;
        }
        out += "s";
        out += &pal.hi_fg;
        out += "y";
        out += &pal.title;
        out += "nc";
        out += title_right;
        maps.push(MouseMap {
            x: x + width - i_size - 26,
            y,
            w: 4,
            h: 1,
            action: "y".to_string(),
        });
    }
    out
}
