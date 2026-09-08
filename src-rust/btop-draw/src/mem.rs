//! Mem box (`Mem::draw`, src/btop_draw.cpp:1237-1487).
//!
//! Stateless port of the force-redraw path plus the per-tick value path.
//! C++ keeps `Meter`/`Graph` maps, `redraw`/`disks_io_h` statics and the
//! `box` string across calls; this port rebuilds them from explicit inputs
//! each call (same bytes for `force_redraw=true, data_same=false`, which
//! is what the golden fixtures use).
//!
//! `data_same=true` returns `prev` unchanged (documents
//! `Graph::operator(data_same=true)` returning cached `out`; the rest of
//! the box would re-render identical bytes anyway when data is unchanged).
//! `force_redraw=false` omits the frame (outer box + disks/io titles,
//! incremental path); graphs are still fully rebuilt (stateless
//! approximation — byte parity is only tested for `force_redraw=true`).
//!
//! Stateless-Graph contract: callers must pass full histories, never deltas
//! (every `Graph::new` below receives the complete `Vec` contents;
//! `Graph::push` incrementalism is not used).
//!
//! Reuse decisions (see task notes):
//! - `btop-collect` `disk_usage` NOT reused: `Mem::draw` never recomputes
//!   disk usage — it renders the precomputed `disk.total/used/free` +
//!   `used_percent/free_percent` carried by `Mem::disk_info`
//!   (btop_draw.cpp:1405-1472 only reads them). The input struct below
//!   carries the same precomputed values, so there is nothing to recompute.
//! - `floating_humanizer` with `HumanOpts` (Task 2) reused for every byte
//!   count; `base_10_sizes` (plus the `base_10_bitrate` Auto rule, which
//!   only matters for bit+per_second — never used here) is resolved by the
//!   caller into `MemFlags::base_10`.

use crate::ansi::{mv_d, mv_l, mv_r, mv_to, mv_u, FX_B, FX_UB};
use crate::boxes::{create_box, human_bytes, ljust_b, rjust_b, CommonFlags, MemGeom, Palette};
use crate::meter_graph::{meter, Graph, GraphOpts};
use crate::symbols::graph_table;
use crate::theme_grad::gradient;
use btop_tools::strtools::{ssplit, trans, uresize};
use std::collections::HashMap;

/// One disk's precomputed values, mirroring `Mem::disk_info`
/// (src/btop_shared.hpp:273-288). Byte counts are `u64` (humanizer input);
/// io series are `i64` (`deque<long long>`).
#[derive(Debug, Clone)]
pub struct DiskDraw {
    pub name: String,
    pub total: u64,
    pub used: u64,
    pub free: u64,
    pub used_percent: i64,
    pub free_percent: i64,
    pub io_read: Vec<i64>,
    pub io_write: Vec<i64>,
    pub io_activity: Vec<i64>,
}

/// Every Config key / Mem:: global `Mem::draw` reads, grouped (GraphOpts
/// precedent). Each field cites its cpp line.
#[derive(Debug, Clone)]
pub struct MemFlags {
    pub show_swap: bool,         // :1240 Config show_swap
    pub swap_disk: bool,         // :1241 Config swap_disk
    pub show_disks: bool,        // :1242 Config show_disks
    pub show_io_stat: bool,      // :1243 Config show_io_stat
    pub io_mode: bool,           // :1244 Config io_mode
    pub io_graph_combined: bool, // :1245 Config io_graph_combined
    pub use_graphs: bool,        // :1246 Config mem_graphs
    /// Resolved `base_10_sizes` (harness false; the `base_10_bitrate`
    /// Auto/True/False override only affects bit+per_second calls, which
    /// mem never makes).
    pub base_10: bool,
    /// Shared box-independent flags (see [`CommonFlags`]).
    pub common: CommonFlags,
}

impl MemFlags {
    /// Harness defaults (tests/draw_golden.cpp setup() + btop_config.cpp
    /// compiled-in defaults): show_swap/show_disks/show_io_stat/
    /// mem_graphs true, swap_disk/io_mode/io_graph_combined false
    /// (setup() pins swap_disk + io_mode false; swap_disk's compiled-in
    /// default is true), base_10 false.
    pub fn harness_defaults() -> Self {
        Self {
            show_swap: true,
            swap_disk: false,
            show_disks: true,
            show_io_stat: true,
            io_mode: false,
            io_graph_combined: false,
            use_graphs: true,
            base_10: false,
            common: CommonFlags::harness_defaults(),
        }
    }
}

/// All inputs `Mem::draw` reads, as explicit params.
/// Mirrors btop_draw.cpp:1237-1487.
pub struct MemDrawInput<'a> {
    /// `mem.stats` (:1352/:1364/:1373 — used/available/cached/free +
    /// swap_total/swap_used/swap_free).
    pub stats: &'a HashMap<String, u64>,
    /// `mem.percent` (:1267/:1274/:1376 — full histories per name).
    pub percent: &'a HashMap<String, Vec<i64>>,
    /// `mem.disks` keyed by mountpoint (:1302/:1405).
    pub disks: &'a HashMap<String, DiskDraw>,
    /// `mem.disks_order` iteration order (:1402/:1442).
    pub disks_order: &'a [String],
    /// `Mem::get_totalMem()` (:1250/:1352). C++ reads it live (host RAM);
    /// the stateless port takes it as input (fixtures embed 0).
    pub total_mem: u64,
    pub has_swap: bool, // :1271/:1354 Mem::has_swap
    pub disk_ios: i64,  // :1286/:1470 Mem::disk_ios
    /// Config `io_graph_speeds` raw string (:1289-1299).
    pub io_graph_speeds: &'a str,
    pub graph_symbol_cfg: &'a str,     // :1248 Config graph_symbol
    pub graph_symbol_mem_cfg: &'a str, // :1248 Config graph_symbol_mem
    pub flags: MemFlags,
    pub force_redraw: bool,    // :1239 force_redraw → redraw
    pub data_same: bool,       // :1376/:1425 Graph data_same
    pub prev: Option<&'a str>, // cached out for data_same
}

/// `Tools::capitalize` (src/btop_tools.hpp:194): first byte uppercased.
/// All mem titles are ASCII (`used`, `swap_free`→`Free`), so byte indexing
/// matches C++ `str.at(0) = toupper(...)` exactly.
fn capitalize(s: &str) -> String {
    let mut out = s.to_string();
    if let Some(b) = out.as_bytes().first() {
        out.replace_range(0..1, &(*b as char).to_uppercase().to_string());
    }
    out
}

// (Shared just/humanizer/Palette helpers live in boxes.rs — Step 0 dedupe.)

/// Outer box + disks title + divider column (calcSizes :2487-2495) plus the
/// io title (draw :1338-1339). Returns empty when `!force_redraw`.
/// (`Input::mouse_mappings` writes are caller concerns, not bytes.)
fn render_frame(input: &MemDrawInput, geom: &MemGeom, pal: &Palette) -> String {
    if !input.force_redraw {
        return String::new();
    }
    let f = &input.flags;
    let (x, y, width, height) = (geom.base.x, geom.base.y, geom.base.width, geom.base.height);
    let mut out = String::new();
    out += &create_box(
        x,
        y,
        width,
        height,
        &pal.box_color,
        true,
        "mem",
        "",
        2,
        &pal.div_line,
        &pal.hi_fg,
        &pal.title,
        &pal.reset,
        f.common.tty_mode,
        f.common.rounded,
    );
    // Disks title (:2488-2489).
    out += &mv_to(
        y,
        if f.show_disks {
            geom.divider + 2
        } else {
            x + width - 9
        },
    );
    out += &pal.box_color;
    out += crate::symbols::box_chars::TITLE_LEFT;
    if f.show_disks {
        out += FX_B;
    }
    out += &pal.hi_fg;
    out += "d";
    out += &pal.title;
    out += "isks";
    out += FX_UB;
    out += &pal.box_color;
    out += crate::symbols::box_chars::TITLE_RIGHT;
    // Divider column (:2491-2494).
    if f.show_disks {
        out += &mv_to(y, geom.divider);
        out += crate::symbols::box_chars::DIV_UP;
        out += &mv_to(y + height - 1, geom.divider);
        out += crate::symbols::box_chars::DIV_DOWN;
        out += &pal.div_line;
        for i in 1..height - 1 {
            out += &mv_to(y + i, geom.divider);
            out += crate::symbols::box_chars::V_LINE;
        }
        // Io title (:1338-1339; part of the draw-redraw block, but frame
        // geometry — emitted here so `draw_mem` stays orchestration-only).
        out += &mv_to(y, x + width - 6);
        out += FX_UB;
        out += &pal.box_color;
        out += crate::symbols::box_chars::TITLE_LEFT;
        if f.io_mode {
            out += FX_B;
        }
        out += &pal.hi_fg;
        out += "i";
        out += &pal.title;
        out += "o";
        out += FX_UB;
        out += &pal.box_color;
        out += crate::symbols::box_chars::TITLE_RIGHT;
    }
    out
}

/// Graph/meter widget set for one mem name. `grad_name` is the `Theme::g`
/// key (`name`, or `name[5..]` for swap — btop_draw.cpp:1274/:1276).
#[allow(clippy::too_many_arguments)]
fn widget_for(
    grad_name: &str,
    data: &[i64],
    mem_meter: i64,
    graph_height: i64,
    use_graphs: bool,
    base_symbol: &str,
    theme: &HashMap<String, String>,
    lowcolor: bool,
    meter_bg: &str,
    reset: &str,
) -> String {
    if use_graphs {
        let grad = gradient(grad_name, theme, lowcolor);
        Graph::new(
            GraphOpts {
                width: mem_meter.max(0) as usize,
                height: graph_height.max(0) as usize,
                gradient: grad,
                symbol: base_symbol.to_string(),
                invert: false,
                no_zero: false,
                max_value: 0,
                offset: 0,
            },
            reset,
            data,
        )
        .render()
        .to_string()
    } else {
        let grad = gradient(grad_name, theme, lowcolor);
        meter(
            mem_meter.max(0) as usize,
            &grad,
            meter_bg,
            reset,
            data.last().copied().unwrap_or(0),
            false,
        )
    }
}

/// Mem + swap section (:1346-1392): Total line, per-name items, swap block,
/// trailing divider.
#[allow(clippy::too_many_arguments)]
fn render_mem_swap(
    input: &MemDrawInput,
    geom: &MemGeom,
    pal: &Palette,
    theme: &HashMap<String, String>,
    base_symbol: &str,
) -> String {
    let f = &input.flags;
    let (x, y, height) = (geom.base.x, geom.base.y, geom.base.height);
    let (mem_width, mem_meter, graph_height, mem_size) = (
        geom.mem_width,
        geom.mem_meter,
        geom.graph_height,
        geom.mem_size,
    );
    let mut out = String::new();
    let cx = 1i64;
    let mut cy = 1i64;
    // In-box divider + cursor-up (:1347-1349). Empty when graph_height == 0
    // (mem_graphs=false); `divider.empty()` then drives the :1381 branch.
    let mem_div = if graph_height > 0 {
        format!(
            "{}{}{}{}{}{}{}{}{}",
            mv_l(2),
            pal.box_color,
            crate::symbols::box_chars::DIV_LEFT,
            pal.div_line,
            &crate::symbols::box_chars::H_LINE.repeat((mem_width - 1).max(0) as usize),
            if f.show_disks {
                ""
            } else {
                pal.box_color.as_str()
            },
            crate::symbols::box_chars::DIV_RIGHT,
            mv_l(mem_width - 1),
            pal.main_fg,
        )
    } else {
        String::new()
    };
    // Cursor-up after a tall graph (:1349: `Mv::l(mem_width-2) +
    // Mv::u(graph_height-1)`).
    let up = if graph_height >= 2 {
        format!("{}{}", mv_l(mem_width - 2), mv_u(graph_height - 1))
    } else {
        String::new()
    };
    let big_mem = mem_width > 21; // :1350

    // Total line (:1352).
    out += &mv_to(y + 1, x + 2);
    out += &pal.title;
    out += FX_B;
    out += "Total:";
    out += &rjust_b(
        &human_bytes(input.total_mem, false, f.base_10),
        (mem_width - 9).max(0) as usize,
    );
    out += FX_UB;
    out += &pal.main_fg;

    // Combined mem + swap names (:1353-1354).
    let mut comb_names = vec!["used", "available", "cached", "free"];
    if f.show_swap && input.has_swap && !f.swap_disk {
        comb_names.extend(["swap_used", "swap_free"]);
    }
    let empty_vec: Vec<i64> = Vec::new();
    for name in comb_names {
        if cy > height - 4 {
            break; // :1356
        }
        let mut title = String::new();
        if name == "swap_used" {
            if cy > height - 5 {
                break; // :1359
            }
            if height - cy > 6 {
                // :1360-1363
                if graph_height > 0 {
                    out += &mv_to(y + 1 + cy, x + 1 + cx);
                    out += &mem_div;
                }
                cy += 1;
            }
            // :1364-1366
            out += &mv_to(y + 1 + cy, x + 1 + cx);
            out += &pal.title;
            out += FX_B;
            out += "Swap:";
            out += &rjust_b(
                &human_bytes(
                    input.stats.get("swap_total").copied().unwrap_or(0),
                    false,
                    f.base_10,
                ),
                (mem_width - 8).max(0) as usize,
            );
            out += &pal.main_fg;
            out += FX_UB;
            cy += 1;
            title = "Used".to_string();
        } else if name == "swap_free" {
            title = "Free".to_string(); // :1369-1370
        }
        if title.is_empty() {
            title = capitalize(name); // :1372
        }
        let humanized = human_bytes(
            input.stats.get(name).copied().unwrap_or(0),
            false,
            f.base_10,
        ); // :1373
        let offset = if mem_div.is_empty() {
            0.max(9 - humanized.len() as i64)
        } else {
            0
        }; // :1374 (byte size, as C++ string::size)
        let data = input
            .percent
            .get(name)
            .map(|v| v.as_slice())
            .unwrap_or(&empty_vec);
        // Swap widgets use the stripped gradient (`name.substr(5)`).
        let grad_name = if let Some(stripped) = name.strip_prefix("swap_") {
            stripped
        } else {
            name
        };
        let graphics = widget_for(
            grad_name,
            data,
            mem_meter,
            graph_height,
            f.use_graphs,
            base_symbol,
            theme,
            f.common.lowcolor,
            &pal.meter_bg,
            &pal.reset,
        );
        if mem_size > 2 {
            // :1379-1383 (two-line items).
            out += &mv_to(y + 1 + cy, x + 1 + cx);
            out += &mem_div;
            out += &title
                .chars()
                .take(if big_mem { 10 } else { 5 })
                .collect::<String>();
            out += ":";
            out += &mv_to(y + 1 + cy, x + cx + mem_width - 2 - humanized.len() as i64);
            if mem_div.is_empty() {
                out += &mv_l(offset);
                out += &" ".repeat(offset as usize);
                out += &humanized;
            } else {
                out += &trans(&humanized);
            }
            out += &mv_to(y + 2 + cy, x + cx + if graph_height >= 2 { 0 } else { 1 });
            out += &graphics;
            out += &up;
            out += &rjust_b(&format!("{}%", data.last().copied().unwrap_or(0)), 4);
            cy += if graph_height == 0 {
                2
            } else {
                graph_height + 1
            };
        } else {
            // :1385-1389 (single-line items).
            out += &mv_to(y + 1 + cy, x + 1 + cx);
            out += &ljust_b(&title, if mem_size > 1 { 5 } else { 1 });
            if graph_height < 2 {
                out += " ";
            }
            out += &graphics;
            out += &pal.title;
            out += &rjust_b(&humanized, if mem_size > 1 { 9 } else { 7 });
            cy += if graph_height == 0 { 1 } else { graph_height };
        }
    }
    // Trailing divider (:1391-1392).
    if graph_height > 0 && cy < height - 2 {
        out += &mv_to(y + 1 + cy, x + 1 + cx);
        out += &mem_div;
    }
    out
}

/// `io_graph_speeds` parsing (:1289-1299): whitespace entries
/// `mount:speed`, kept when the mount is a known disk and the speed
/// `isint`s; `stoi` overflow (`out_of_range`) skips the entry.
fn custom_speeds(speeds: &str, disks: &HashMap<String, DiskDraw>) -> HashMap<String, i64> {
    let mut out = HashMap::new();
    if speeds.is_empty() {
        return out;
    }
    for entry in ssplit(speeds, ' ') {
        let vals: Vec<&str> = entry.split(':').collect();
        if vals.len() == 2
            && disks.contains_key(vals[0])
            && !vals[1].is_empty()
            && vals[1].bytes().all(|b| b.is_ascii_digit())
        {
            if let Ok(v) = vals[1].parse::<i64>() {
                out.insert(vals[0].to_string(), v);
            }
        }
    }
    out
}

/// Io-mode disks section (:1401-1439). Only `io_mode=true` reaches here
/// (harness false — covered by the io-mode smoke test, not fixtures).
#[allow(clippy::too_many_arguments)]
fn render_disks_io(
    input: &MemDrawInput,
    geom: &MemGeom,
    pal: &Palette,
    theme: &HashMap<String, String>,
    base_symbol: &str,
    graph_bg: &str,
    disk_div: &str,
    hu_div: &str,
    big_disk: bool,
) -> (String, i64) {
    let f = &input.flags;
    let (x, y, height) = (geom.base.x, geom.base.y, geom.base.height);
    let (disks_width, cx) = (geom.disks_width, geom.mem_width);
    let mut out = String::new();
    let mut cy = 0i64;
    // :1285-1287 (integer floor/ceil as in C++).
    let disks_io_h = ((((height - 2 - input.disk_ios * 2) as f64) / (input.disk_ios.max(1) as f64))
        .floor() as i64)
        .max(if f.io_graph_combined { 1 } else { 2 });
    let half_height = (disks_io_h as f64 / 2.0).ceil() as i64;
    let speeds = custom_speeds(input.io_graph_speeds, input.disks);
    for mount in input.disks_order.iter() {
        let Some(disk) = input.disks.get(mount) else {
            continue; // :1403
        };
        if cy > height - 3 {
            break; // :1404
        }
        if disk.io_read.is_empty() {
            continue; // :1406
        }
        let total = human_bytes(disk.total, !big_disk, f.base_10); // :1407
        out += &mv_to(y + 1 + cy, x + 1 + cx);
        out += disk_div;
        out += &pal.title;
        out += FX_B;
        out += &uresize(&disk.name, (disks_width - 8).max(0) as usize, false);
        out += &mv_to(y + 1 + cy, x + cx + disks_width - total.len() as i64);
        out += &trans(&total);
        out += FX_UB;
        if big_disk {
            // :1410-1413
            let used_percent = disk.used_percent.to_string();
            out += &mv_to(
                y + 1 + cy,
                x + 1 + cx + (disks_width as f64 / 2.0).round() as i64
                    - (used_percent.len() as f64 / 2.0).round() as i64
                    - 1,
            );
            out += hu_div;
            out += &used_percent;
            out += "%";
            out += hu_div;
        }
        // Activity row (:1414-1417).
        out += &mv_to(y + 2 + cy, x + 1 + cx);
        cy += 1;
        if big_disk {
            out += " IO% ";
        } else {
            out += " IO   ";
            out += &mv_l(2);
        }
        out += &pal.inactive;
        out += &graph_bg.repeat((disks_width - 6).max(0) as usize);
        out += &mv_l(disks_width - 6);
        out += Graph::new(
            GraphOpts {
                width: (disks_width - 6).max(0) as usize,
                height: 1,
                gradient: gradient("available", theme, f.common.lowcolor),
                symbol: base_symbol.to_string(),
                invert: false,
                no_zero: false,
                max_value: 0,
                offset: 0,
            },
            &pal.reset,
            &disk.io_activity,
        )
        .render();
        out += &pal.main_fg;
        if cy > height - 3 {
            break; // :1418
        }
        if f.io_graph_combined {
            // :1419-1428
            let speed = speeds.get(mount).copied().unwrap_or(100) << 20;
            let combined: Vec<i64> = disk
                .io_read
                .iter()
                .zip(disk.io_write.iter())
                .map(|(r, w)| r + w)
                .collect();
            let comb_val = disk.io_read.last().copied().unwrap_or(0)
                + disk.io_write.last().copied().unwrap_or(0);
            let humanized = format!(
                "{}{}{}",
                if disk.io_write.last().copied().unwrap_or(0) > 0 {
                    "▼"
                } else {
                    ""
                },
                if disk.io_read.last().copied().unwrap_or(0) > 0 {
                    "▲"
                } else {
                    ""
                },
                if comb_val > 0 {
                    format!(
                        "{}{}",
                        mv_r(1),
                        human_bytes(comb_val as u64, true, f.base_10)
                    )
                } else {
                    "RW".to_string()
                },
            );
            if disks_io_h == 1 {
                out += &mv_to(y + 1 + cy, x + 1 + cx);
                out += "     ";
            }
            out += &mv_to(y + 1 + cy, x + 1 + cx);
            out += Graph::new(
                GraphOpts {
                    width: disks_width.max(0) as usize,
                    height: disks_io_h.max(0) as usize,
                    gradient: gradient("available", theme, f.common.lowcolor),
                    symbol: base_symbol.to_string(),
                    invert: false,
                    no_zero: true,
                    max_value: speed,
                    offset: 0,
                },
                &pal.reset,
                &combined,
            )
            .render();
            out += &mv_to(y + 1 + cy, x + 1 + cx);
            out += &pal.main_fg;
            out += &humanized;
            cy += disks_io_h;
        } else {
            // :1429-1437 (split read/write).
            let speed = speeds.get(mount).copied().unwrap_or(100) << 20;
            let human_read = if disk.io_read.last().copied().unwrap_or(0) > 0 {
                format!(
                    "▲{}",
                    human_bytes(
                        disk.io_read.last().copied().unwrap_or(0) as u64,
                        true,
                        f.base_10
                    )
                )
            } else {
                "R".to_string()
            };
            let human_write = if disk.io_write.last().copied().unwrap_or(0) > 0 {
                format!(
                    "▼{}",
                    human_bytes(
                        disk.io_write.last().copied().unwrap_or(0) as u64,
                        true,
                        f.base_10
                    )
                )
            } else {
                "W".to_string()
            };
            if disks_io_h <= 3 {
                out += &mv_to(y + 1 + cy, x + 1 + cx);
                out += "     ";
                out += &mv_to(y + cy + disks_io_h, x + 1 + cx);
                out += "     ";
            }
            out += &mv_to(y + 1 + cy, x + 1 + cx);
            out += Graph::new(
                GraphOpts {
                    width: disks_width.max(0) as usize,
                    height: half_height.max(0) as usize,
                    gradient: gradient("free", theme, f.common.lowcolor),
                    symbol: base_symbol.to_string(),
                    invert: false,
                    no_zero: true,
                    max_value: speed,
                    offset: 0,
                },
                &pal.reset,
                &disk.io_read,
            )
            .render();
            out += &mv_l(disks_width);
            out += &mv_d(1);
            out += Graph::new(
                GraphOpts {
                    width: disks_width.max(0) as usize,
                    height: (disks_io_h - half_height).max(0) as usize,
                    gradient: gradient("used", theme, f.common.lowcolor),
                    symbol: base_symbol.to_string(),
                    invert: true,
                    no_zero: true,
                    max_value: speed,
                    offset: 0,
                },
                &pal.reset,
                &disk.io_write,
            )
            .render();
            out += &mv_to(y + 1 + cy, x + 1 + cx);
            out += &human_read;
            out += &mv_to(y + cy + disks_io_h, x + 1 + cx);
            out += &human_write;
            cy += disks_io_h;
        }
    }
    (out, cy)
}

/// Normal disks section (:1442-1477): totals, io-activity row, Used/Free
/// meter rows.
#[allow(clippy::too_many_arguments)]
fn render_disks_normal(
    input: &MemDrawInput,
    geom: &MemGeom,
    pal: &Palette,
    theme: &HashMap<String, String>,
    base_symbol: &str,
    graph_bg: &str,
    disk_div: &str,
    hu_div: &str,
    big_disk: bool,
) -> (String, i64) {
    let f = &input.flags;
    let (x, y, height) = (geom.base.x, geom.base.y, geom.base.height);
    let (disks_width, disk_meter, cx) = (geom.disks_width, geom.disk_meter, geom.mem_width);
    let mut out = String::new();
    let mut cy = 0i64;
    let n_disks = input.disks.len() as i64;
    // Free meters exist iff `disks.size()*3 <= height-1` (:1334-1335);
    // the used-meter loop's `i` never increments (:1331), so every known
    // disk has a used meter — transcribed as "present means metered".
    let free_metered = n_disks * 3 < height;
    for mount in input.disks_order.iter() {
        let Some(disk) = input.disks.get(mount) else {
            continue; // :1443
        };
        if cy > height - 3 {
            break; // :1444
        }
        if disk.name.is_empty() {
            continue; // :1446 (used meter always present, see above)
        }
        let comb_val = if disk.io_read.is_empty() {
            0i64
        } else {
            disk.io_read.last().copied().unwrap_or(0) + disk.io_write.last().copied().unwrap_or(0)
        }; // :1447
        let human_io = if comb_val > 0 {
            format!(
                "{}{}{}",
                if disk.io_write.last().copied().unwrap_or(0) > 0 && big_disk {
                    "▼"
                } else {
                    ""
                },
                if disk.io_read.last().copied().unwrap_or(0) > 0 && big_disk {
                    "▲"
                } else {
                    ""
                },
                human_bytes(comb_val as u64, true, f.base_10),
            )
        } else {
            String::new()
        }; // :1448-1449
        let human_total = human_bytes(disk.total, !big_disk, f.base_10); // :1450
        let human_used = human_bytes(disk.used, !big_disk, f.base_10); // :1451
        let human_free = human_bytes(disk.free, !big_disk, f.base_10); // :1452

        // Title row (:1454-1455).
        out += &mv_to(y + 1 + cy, x + 1 + cx);
        out += disk_div;
        out += &pal.title;
        out += FX_B;
        out += &uresize(&disk.name, (disks_width - 8).max(0) as usize, false);
        out += &mv_to(y + 1 + cy, x + cx + disks_width - human_total.len() as i64);
        out += &trans(&human_total);
        out += FX_UB;
        out += &pal.main_fg;
        if big_disk && !human_io.is_empty() {
            // :1456-1457
            out += &mv_to(
                y + 1 + cy,
                x + 1 + cx + (disks_width as f64 / 2.0).round() as i64
                    - (human_io.len() as f64 / 2.0).round() as i64
                    - 1,
            );
            out += hu_div;
            out += &human_io;
            out += hu_div;
        }
        cy += 1;
        if cy > height - 3 {
            break; // :1458
        }
        // Activity row (:1459-1464).
        if f.show_io_stat && !disk.io_read.is_empty() {
            out += &mv_to(y + 1 + cy, x + 1 + cx);
            if big_disk {
                out += " IO% ";
            } else {
                out += " IO   ";
                out += &mv_l(2);
            }
            out += &pal.inactive;
            out += &graph_bg.repeat((disks_width - 6).max(0) as usize);
            out += &gradient("available", theme, f.common.lowcolor)
                [(disk.io_activity.last().copied().unwrap_or(0).clamp(50, 100)) as usize];
            out += &mv_l(disks_width - 6);
            out += Graph::new(
                GraphOpts {
                    width: (disks_width - 6).max(0) as usize,
                    height: 1,
                    gradient: gradient("available", theme, f.common.lowcolor),
                    symbol: base_symbol.to_string(),
                    invert: false,
                    no_zero: false,
                    max_value: 0,
                    offset: 0,
                },
                &pal.reset,
                &disk.io_activity,
            )
            .render();
            out += &pal.main_fg;
            if !big_disk {
                // :1462
                out += &mv_to(y + 1 + cy, x + cx + 1);
                out += &pal.main_fg;
                out += &human_io;
            }
            cy += 1;
            if cy > height - 3 {
                break; // :1463
            }
        }

        // Used row (:1466-1467).
        out += &mv_to(y + 1 + cy, x + 1 + cx);
        if big_disk {
            out += " Used:";
            out += &rjust_b(&format!("{}%", disk.used_percent), 4);
        } else {
            out += "U";
        }
        out += " ";
        out += &meter(
            disk_meter.max(0) as usize,
            &gradient("used", theme, f.common.lowcolor),
            &pal.meter_bg,
            &pal.reset,
            disk.used_percent,
            false,
        );
        out += &rjust_b(&human_used, if big_disk { 9 } else { 5 });
        cy += 1;
        if cy > height - 3 {
            break; // :1468
        }

        // Free row (:1470-1475).
        if free_metered
            && n_disks * 3 + if f.show_io_stat { input.disk_ios } else { 0 } <= height - 1
        {
            out += &mv_to(y + 1 + cy, x + 1 + cx);
            if big_disk {
                out += " Free:";
                out += &rjust_b(&format!("{}%", disk.free_percent), 4);
            } else {
                out += "F";
            }
            out += " ";
            out += &meter(
                disk_meter.max(0) as usize,
                &gradient("free", theme, f.common.lowcolor),
                &pal.meter_bg,
                &pal.reset,
                disk.free_percent,
                false,
            );
            out += &rjust_b(&human_free, if big_disk { 9 } else { 5 });
            cy += 1;
            if n_disks * 4 + if f.show_io_stat { input.disk_ios } else { 0 } <= height - 1 {
                cy += 1; // :1474 blank gap row
            }
        }
    }
    (out, cy)
}

/// Draw the mem box. `geom` is the mem part of `Layout` (calcSizes);
/// `theme` is the Default-keyed map (`default_theme()`).
pub fn draw_mem(input: &MemDrawInput, geom: &MemGeom, theme: &HashMap<String, String>) -> String {
    // data_same → cached out (Graph::operator(data_same=true) returns `out`).
    if input.data_same {
        return input.prev.unwrap_or("").to_string();
    }
    let f = &input.flags;
    let lowcolor = f.common.lowcolor;
    let tbg = f.common.theme_background;

    let pal = Palette::new("mem_box", theme, lowcolor, tbg);

    // Symbol resolution (:1248 + Graph ctor :498-501).
    let base_symbol: &str = if f.common.tty_mode || input.graph_symbol_mem_cfg == "tty" {
        "tty"
    } else if input.graph_symbol_mem_cfg != "default" {
        input.graph_symbol_mem_cfg
    } else {
        input.graph_symbol_cfg
    };
    let table_key = format!("{base_symbol}_up");
    let graph_bg = graph_table(&table_key).map(|t| t[6]).unwrap_or(" ");

    let mut out = String::new();
    out += &render_frame(input, geom, &pal);
    out += &render_mem_swap(input, geom, &pal, theme, base_symbol);

    // Disks (:1394-1480).
    if f.show_disks {
        let big_disk = geom.disks_width >= 25; // :1398
        let disk_div = format!(
            "{}{}{}{}{}{}{}{}",
            mv_l(1),
            pal.div_line,
            crate::symbols::box_chars::DIV_LEFT,
            &crate::symbols::box_chars::H_LINE.repeat(geom.disks_width.max(0) as usize),
            pal.box_color,
            FX_UB,
            crate::symbols::box_chars::DIV_RIGHT,
            mv_l(geom.disks_width),
        ); // :1399
        let hu_div = format!(
            "{}{}{}",
            pal.div_line,
            crate::symbols::box_chars::H_LINE,
            pal.main_fg,
        ); // :1400
        let cy = if f.io_mode {
            let (text, cy) = render_disks_io(
                input,
                geom,
                &pal,
                theme,
                base_symbol,
                graph_bg,
                &disk_div,
                &hu_div,
                big_disk,
            );
            out += &text;
            cy
        } else {
            let (text, cy) = render_disks_normal(
                input,
                geom,
                &pal,
                theme,
                base_symbol,
                graph_bg,
                &disk_div,
                &hu_div,
                big_disk,
            );
            out += &text;
            cy
        };
        // Trailing divider (:1479).
        if cy < geom.base.height - 2 {
            out += &mv_to(geom.base.y + 1 + cy, geom.base.x + 1 + geom.mem_width);
            out += &disk_div;
        }
    }

    out += &pal.reset; // :1483 (+Fx::reset)
    out
}
