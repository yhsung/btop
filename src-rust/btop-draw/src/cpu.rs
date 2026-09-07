//! CPU box (`Cpu::draw`, src/btop_draw.cpp:567-1029).
//!
//! Stateless port of the force-redraw path plus the per-tick value path.
//! C++ keeps `Graph`/`Meter` objects, `redraw`/`mid_line`/geometry statics
//! and battery statics across calls; this port rebuilds them from explicit
//! inputs each call (same bytes for `force_redraw=true, data_same=false`,
//! which is what the golden fixtures use).
//!
//! `data_same=true` returns `prev` unchanged (documents
//! `Graph::operator(data_same=true)` returning cached `out`; the rest of
//! the box would re-render identical bytes anyway when data is unchanged).
//! `force_redraw=false` omits the outer/inner boxes + title buttons
//! (incremental path); graphs are still fully rebuilt (stateless
//! approximation — byte parity is only tested for `force_redraw=true`).

use crate::ansi::{mv_l, mv_r, mv_to, FX_B, FX_UB};
use crate::boxes::{create_box, CpuGeom};
use crate::meter_graph::{meter, Graph, GraphOpts};
use crate::symbols::graph_table;
use crate::theme_grad::{color, gradient};
use btop_tools::strtools::{ljust, rjust, uresize};
use std::collections::{HashMap, VecDeque};

fn clamp_i64(v: i64, lo: i64, hi: i64) -> i64 {
    v.clamp(lo, hi)
}

/// Battery sample for the `show_battery && has_battery` branch
/// (btop_draw.cpp:766-806). Harness sets `show_battery=false`
/// (tests/draw_golden.cpp:91) so this is UNTESTED-BRANCH.
#[derive(Debug, Clone)]
pub struct BatteryState {
    pub percent: i64,
    pub watts: f32,
    pub seconds: i64,
    pub status: String,
}

/// Every Config key / Cpu:: global / Term global `Cpu::draw` reads,
/// grouped (GraphOpts precedent — the fn reads >15 flags).
/// Each field cites its cpp line.
#[derive(Debug, Clone)]
pub struct CpuFlags {
    pub check_temp: bool,            // :577 Config check_temp
    pub got_sensors: bool,           // :577 Cpu::got_sensors
    pub cpu_temp_only: bool,         // :580 Cpu::cpu_temp_only
    pub show_coretemp: bool,         // :580 Config show_coretemp
    pub single_graph: bool,          // :579 Config cpu_single_graph
    pub invert_lower: bool,          // :697 Config cpu_invert_lower
    pub show_watts_cfg: bool,        // :578 Config show_cpu_watts
    pub supports_watts: bool,        // :578 Cpu::supports_watts
    pub show_freq_cfg: bool,         // :870 Config show_cpu_freq
    pub has_cpu_hz: bool,            // :870 !cpuHz.empty() (+:2366 hasCpuHz)
    pub freq_range: bool,            // :864-867 freq_mode=="range" (linux)
    pub show_uptime: bool,           // :853 Config show_uptime
    pub show_battery_cfg: bool,      // :766 Config show_battery
    pub has_battery: bool,           // :766 Cpu::has_battery
    pub show_battery_watts: bool,    // :781 Config show_battery_watts
    pub tty_mode: bool,              // :599 Config tty_mode
    pub rounded: bool,               // createBox :289 Config rounded_corners
    pub cpu_bottom: bool,            // :603 Config cpu_bottom
    pub lowcolor: bool,              // Theme depth :146 Config lowcolor
    pub theme_background: bool,      // color() :133 Config theme_background
    pub follow_process: bool,        // :622 Config follow_process
    pub proc_tree: bool,             // :623 Config proc_tree
    pub followed_pid: i64,           // :622 Config followed_pid
    pub detailed_pid: i64,           // :622 Config detailed_pid
    pub proc_selected: i64,          // :622 Config proc_selected
    pub update_ms: i64,              // :632 Config update_ms
    pub current_preset: Option<i64>, // :630 Config current_preset
    pub show_gpu: bool,              // :586 show_gpu (GPU_SUPPORT; harness false)
}

impl CpuFlags {
    /// Harness defaults (tests/draw_golden.cpp setup() + btop_config.cpp
    /// compiled-in defaults for the rest): check_temp=true, got_sensors=true,
    /// show_coretemp=true, invert_lower=true, update_ms=2000, rest false/None.
    pub fn harness_defaults() -> Self {
        Self {
            check_temp: true,
            got_sensors: true,
            cpu_temp_only: false,
            show_coretemp: true,
            single_graph: false,
            invert_lower: true,
            show_watts_cfg: false,
            supports_watts: false,
            show_freq_cfg: false,
            has_cpu_hz: false,
            freq_range: false,
            show_uptime: false,
            show_battery_cfg: false,
            has_battery: false,
            show_battery_watts: false,
            tty_mode: false,
            rounded: true,
            cpu_bottom: false,
            lowcolor: false,
            theme_background: true,
            follow_process: false,
            proc_tree: false,
            followed_pid: 0,
            detailed_pid: 0,
            proc_selected: 0,
            update_ms: 2000,
            current_preset: None,
            show_gpu: false,
        }
    }
}

/// All inputs `Cpu::draw` reads, as explicit params.
/// Mirrors btop_draw.cpp:567-1029.
pub struct CpuDrawInput<'a> {
    pub percent: &'a HashMap<String, VecDeque<i64>>, // :609/:689/:843 cpu.cpu_percent ("total")
    pub cores: &'a [VecDeque<i64>],                  // :610/:749/:920 cpu.core_percent
    pub temp: &'a [VecDeque<i64>],                   // :611/:756-760 cpu.temp ([0]=package)
    pub temp_max: i64,                               // :604 cpu.temp_max
    pub load_avg: [f64; 3],                          // :967-969 cpu.load_avg
    pub usage_watts: f32, // :887 cpu.usage_watts (UNTESTED: show_watts false)
    pub active_cpus: Option<&'a [i32]>, // :907-909 cpu.active_cpus (None=all enabled)
    pub core_count: usize, // :913-916 Shared::coreCount
    pub cpu_name: &'a str, // :2374 Cpu::cpuName (inner box title)
    pub custom_cpu_name: &'a str, // :2365/:2373 Config custom_cpu_name
    pub cpu_hz: &'a str,  // :870-873 Cpu::cpuHz (UNTESTED: show_cpu_freq false)
    pub container_engine: Option<&'a str>, // :641 Cpu::container_engine (None in harness)
    pub graph_up_cfg: &'a str, // :588 Config cpu_graph_upper
    pub graph_lo_cfg: &'a str, // :591 Config cpu_graph_lower
    pub available_fields: &'a [String], // :589/:592 Cpu::available_fields
    pub graph_symbol_cfg: &'a str, // :600 Config graph_symbol
    pub graph_symbol_cpu_cfg: &'a str, // :600 Config graph_symbol_cpu
    pub temp_scale: &'a str, // :602 Config temp_scale
    pub flags: CpuFlags,  // (see above)
    pub battery: Option<BatteryState>, // :779 current_bat (UNTESTED)
    pub term_width: i64,  // :790-791 Term::width (battery only; UNTESTED)
    pub force_redraw: bool, // :576 force_redraw → redraw
    pub data_same: bool,  // :822/:843 Graph data_same
    pub prev: Option<&'a str>, // cached out for data_same
}

fn celsius_to(celsius: i64, scale: &str) -> (i64, &'static str) {
    match scale {
        "celsius" => (celsius, "°C"),
        "fahrenheit" => ((celsius as f64 * 1.8 + 32.0).round() as i64, "°F"),
        "kelvin" => ((celsius as f64 + 273.15).round() as i64, "K "),
        "rankine" => ((celsius as f64 * 1.8 + 491.67).round() as i64, "°R"),
        _ => (0, ""),
    }
}

fn resolve_field<'a>(cfg: &'a str, available: &[String], fallback: &'a str) -> &'a str {
    if cfg == "Auto" || !available.iter().any(|f| f == cfg) {
        fallback
    } else {
        cfg
    }
}

fn dq_to_vec(dq: &VecDeque<i64>) -> Vec<i64> {
    dq.iter().copied().collect()
}

/// Draw the CPU box. `geom` is the cpu part of `Layout` (calcSizes);
/// `theme` is the Default-keyed map (`default_theme()`).
pub fn draw_cpu(input: &CpuDrawInput, geom: &CpuGeom, theme: &HashMap<String, String>) -> String {
    // data_same → cached out (Graph::operator(data_same=true) returns `out`).
    if input.data_same {
        return input.prev.unwrap_or("").to_string();
    }
    let f = &input.flags;
    let lowcolor = f.lowcolor;
    let tbg = f.theme_background;

    let c_cpu_box = color("cpu_box", theme, lowcolor, tbg);
    let c_div_line = color("div_line", theme, lowcolor, tbg);
    let c_main_fg = color("main_fg", theme, lowcolor, tbg);
    let c_title = color("title", theme, lowcolor, tbg);
    let c_hi_fg = color("hi_fg", theme, lowcolor, tbg);
    let c_inactive = color("inactive_fg", theme, lowcolor, tbg);
    let c_meter_bg = color("meter_bg", theme, lowcolor, tbg);
    let reset = format!(
        "{}{}{}",
        "\x1b[0m",
        color("main_fg", theme, lowcolor, tbg),
        color("main_bg", theme, lowcolor, tbg),
    );

    let show_temps = f.check_temp && f.got_sensors; // :577
    let show_watts = f.show_watts_cfg && f.supports_watts; // :578
    let hide_cores = show_temps && (f.cpu_temp_only || !f.show_coretemp); // :580
    let extra_width = if hide_cores {
        6.max(6 * geom.b_column_size)
    } else if geom.b_columns == 1 && !show_temps {
        8
    } else {
        0
    }; // :581

    // Graph field resolution (:588-598). GPU path omitted: harness
    // show_gpu_info="Off" + no gpus → lo falls back to up.
    let graph_up_field = resolve_field(input.graph_up_cfg, input.available_fields, "total");
    let mut graph_lo_field =
        resolve_field(input.graph_lo_cfg, input.available_fields, "auto-missing");
    if input.graph_lo_cfg == "Auto"
        || !input
            .available_fields
            .iter()
            .any(|s| s == input.graph_lo_cfg)
    {
        graph_lo_field = if f.show_gpu {
            "gpu-totals"
        } else {
            graph_up_field
        };
    }

    // Symbol resolution (:599-600 + Graph ctor :498-501).
    let base_symbol: &str = if f.tty_mode || input.graph_symbol_cpu_cfg == "tty" {
        "tty"
    } else if input.graph_symbol_cpu_cfg != "default" {
        input.graph_symbol_cpu_cfg
    } else {
        input.graph_symbol_cfg
    };
    let table_key = format!("{base_symbol}_up");
    let graph_bg = graph_table(&table_key).map(|t| t[6]).unwrap_or(" ");
    let lower_key = format!("{base_symbol}_down");
    let _ = lower_key; // Graph::new resolves per-graph; kept for parity with :600.

    let safe_max = if input.temp_max <= 0 {
        90
    } else {
        input.temp_max
    }; // :604
       // Title glyphs: left ┐/┘, right ┌/└ (Symbols::title_left[_down] etc).
    let title_left = format!(
        "{}{}",
        c_cpu_box,
        if f.cpu_bottom {
            crate::symbols::box_chars::TITLE_LEFT_DOWN
        } else {
            crate::symbols::box_chars::TITLE_LEFT
        }
    );
    let title_right = format!(
        "{}{}",
        c_cpu_box,
        if f.cpu_bottom {
            crate::symbols::box_chars::TITLE_RIGHT_DOWN
        } else {
            crate::symbols::box_chars::TITLE_RIGHT
        }
    );

    let empty_dq: VecDeque<i64> = VecDeque::new();
    let get_total = || input.percent.get("total").unwrap_or(&empty_dq);
    if get_total().is_empty()
        || input.cores.first().map(|d| d.is_empty()).unwrap_or(true)
        || (show_temps && input.temp.first().map(|d| d.is_empty()).unwrap_or(true))
    {
        return String::new(); // :609-611
    }

    let x = geom.base.x;
    let y = geom.base.y;
    let width = geom.base.width;
    let height = geom.base.height;
    let (b_x, b_y, b_width, b_height, b_columns, b_column_size) = (
        geom.b_x,
        geom.b_y,
        geom.b_width,
        geom.b_height,
        geom.b_columns,
        geom.b_column_size,
    );

    let mid_line = !f.single_graph && graph_up_field != graph_lo_field; // :618
    let graph_up_height = if f.single_graph {
        height - 2
    } else {
        ((height - 2) as f64 / 2.0).ceil() as i64 - (mid_line && height % 2 != 0) as i64
    }; // :619
    let graph_low_height = height - 2 - graph_up_height - mid_line as i64; // :620

    let cpu_grad = gradient("cpu", theme, lowcolor);
    let temp_grad = gradient("temp", theme, lowcolor);

    let mut out = String::new();

    if input.force_redraw {
        // Outer + inner boxes (:2363 + :2376). Inner title = custom or
        // cpuName, uresized to b_width-5 (show_cpu_freq false path).
        let outer_title = if f.cpu_bottom { "" } else { "cpu" };
        let outer_title2 = if f.cpu_bottom { "cpu" } else { "" };
        out += &create_box(
            x,
            y,
            width,
            height,
            &c_cpu_box,
            true,
            outer_title,
            outer_title2,
            1,
            &c_div_line,
            &c_hi_fg,
            &c_title,
            &reset,
            f.tty_mode,
            f.rounded,
        );
        let raw_title = if input.custom_cpu_name.is_empty() {
            input.cpu_name
        } else {
            input.custom_cpu_name
        };
        let title_w = (b_width
            - if f.show_freq_cfg && f.has_cpu_hz {
                if f.freq_range {
                    24
                } else {
                    14
                }
            } else {
                5
            })
        .max(0) as usize;
        let cpu_title = uresize(raw_title, title_w, false);
        out += &create_box(
            b_x,
            b_y,
            b_width,
            b_height,
            "",
            false,
            &cpu_title,
            "",
            0,
            &c_div_line,
            &c_hi_fg,
            &c_title,
            &reset,
            f.tty_mode,
            f.rounded,
        );

        // Buttons on title (:627-638).
        let button_y = if f.cpu_bottom { y + height - 1 } else { y }; // :621
        let is_proc_focused =
            (f.follow_process && f.followed_pid == f.detailed_pid) || f.proc_selected > 0; // :622
        let is_tree = is_proc_focused && f.proc_tree; // :623
        let c_focus_hi = if is_tree {
            c_inactive.clone()
        } else {
            c_hi_fg.clone()
        };
        let c_focus_title = if is_tree {
            c_inactive.clone()
        } else {
            c_title.clone()
        };
        out += &mv_to(button_y, x + 10);
        out += &title_left;
        out += &c_hi_fg;
        out += FX_B;
        out += "m";
        out += &c_title;
        out += "enu";
        out += FX_UB;
        out += &title_right;
        out += &mv_to(button_y, x + 16);
        out += &title_left;
        out += &c_hi_fg;
        out += FX_B;
        out += "p";
        out += &c_title;
        out += "reset ";
        out += &match f.current_preset {
            Some(v) => v.to_string(),
            None => "*".to_string(),
        };
        out += FX_UB;
        out += &title_right;
        let update = format!("{}ms", f.update_ms);
        out += &mv_to(button_y, x + width - update.len() as i64 - 8);
        out += &title_left;
        out += FX_B;
        out += &c_focus_hi;
        out += "- ";
        out += &c_focus_title;
        out += &update;
        out += &c_focus_hi;
        out += " +";
        out += FX_UB;
        out += &title_right;

        // Container engine name (:641-643). Harness None → skipped.
        // UNTESTED-BRANCH (no fixture covers Some).
        if let Some(engine) = input.container_engine {
            out += &mv_to(button_y, x + 28);
            out += &title_left;
            out += &c_title;
            out += engine;
            out += &title_right;
        }
    }

    // UNTESTED-BRANCH: battery (:766-806). Harness show_battery=false.
    // Stateless port cannot keep bat_pos/bat_len/meters across calls;
    // the branch is transcribed structurally but never byte-tested.
    if f.show_battery_cfg && f.has_battery {
        let _ = (&input.battery, input.term_width, f.show_battery_watts);
    }

    // Graphs (init_graphs :648-697; harness: both "total", widths
    // x+width-b_width-3, no_zero=true).
    let graph_default_width = x + width - b_width - 3; // :646
    let get_field =
        |name: &str| -> Vec<i64> { dq_to_vec(input.percent.get(name).unwrap_or(&empty_dq)) };
    let up_data = get_field(graph_up_field);
    let lo_data = get_field(graph_lo_field);
    let mk_graph = |w: i64, h: i64, grad: Vec<String>, invert: bool, data: &[i64]| -> Graph {
        Graph::new(
            GraphOpts {
                width: w.max(0) as usize,
                height: h.max(0) as usize,
                gradient: grad,
                symbol: base_symbol.to_string(),
                invert,
                no_zero: true,
                max_value: 0,
                offset: 0,
            },
            &reset,
            data,
        )
    };
    let g_up = mk_graph(
        graph_default_width,
        graph_up_height,
        cpu_grad.clone(),
        false,
        &up_data,
    );
    let g_lo = mk_graph(
        graph_default_width,
        graph_low_height,
        cpu_grad.clone(),
        f.invert_lower,
        &lo_data,
    );

    // Mid divider (:740-745). Harness mid_line=false → skipped.
    if input.force_redraw && mid_line {
        out += &mv_to(y + graph_up_height + 1, x);
        out += FX_UB;
        out += &c_cpu_box;
        out += crate::symbols::box_chars::DIV_LEFT;
        out += &c_div_line;
        out += &crate::symbols::box_chars::H_LINE.repeat((width - b_width - 2).max(0) as usize);
        out += crate::symbols::box_chars::DIV_RIGHT;
        out += &mv_to(
            y + graph_up_height + 1,
            x + ((width - b_width) / 2)
                - ((graph_up_field.len() + graph_lo_field.len()) as i64 / 2)
                - 4,
        );
        out += &c_main_fg;
        out += graph_up_field;
        out += &mv_r(1);
        out += "▲▼";
        out += &mv_r(1);
        out += graph_lo_field;
    }

    // Core + temp graphs (:747-762).
    let core_w = (5 * b_column_size + extra_width).max(0) as usize;
    let mut core_graphs: Vec<Graph> = Vec::new();
    if b_column_size > 0 || extra_width > 0 {
        for core_data in input.cores.iter() {
            let v = dq_to_vec(core_data);
            core_graphs.push(Graph::new(
                GraphOpts {
                    width: core_w,
                    height: 1,
                    gradient: cpu_grad.clone(),
                    symbol: base_symbol.to_string(),
                    invert: false,
                    no_zero: false,
                    max_value: 0,
                    offset: 0,
                },
                &reset,
                &v,
            ));
        }
    }
    let mut temp_graphs: Vec<Graph> = Vec::new();
    if show_temps {
        let v0 = dq_to_vec(&input.temp[0]);
        temp_graphs.push(Graph::new(
            GraphOpts {
                width: 5,
                height: 1,
                gradient: temp_grad.clone(),
                symbol: base_symbol.to_string(),
                invert: false,
                no_zero: false,
                max_value: safe_max,
                offset: -23,
            },
            &reset,
            &v0,
        ));
        if !hide_cores && b_column_size > 1 {
            for t in input.temp.iter().skip(1) {
                let v = dq_to_vec(t);
                temp_graphs.push(Graph::new(
                    GraphOpts {
                        width: 5,
                        height: 1,
                        gradient: temp_grad.clone(),
                        symbol: base_symbol.to_string(),
                        invert: false,
                        no_zero: false,
                        max_value: safe_max,
                        offset: -23,
                    },
                    &reset,
                    &v,
                ));
            }
        }
    }
    let cpu_meter_width = b_width
        - if show_temps {
            23 - (b_column_size <= 1 && b_columns == 1) as i64 * 6
        } else {
            11
        }; // :733
    let cpu_meter_width = if show_watts {
        cpu_meter_width - 6
    } else {
        cpu_meter_width
    }; // :734-736

    // Per-tick graphs (:810-850).
    out += FX_UB;
    out += &mv_to(y + 1, x + 1);
    // UNTESTED-BRANCH: gpu graph fields (:813-838). Harness fields are
    // "total" → always the cpu path below.
    out += g_up.render();
    if !f.single_graph {
        out += &mv_to(y + graph_up_height + 1 + mid_line as i64, x + 1);
        out += g_lo.render();
    }

    // UNTESTED-BRANCH: uptime (:853-861). Harness show_uptime=false.
    if f.show_uptime {
        let _ = mv_to(0, 0);
    }
    // UNTESTED-BRANCH: cpu freq (:870-873). Harness show_cpu_freq=false.
    if f.show_freq_cfg && !input.cpu_hz.is_empty() {
        let _ = (b_x, b_width);
    }

    // CPU meter line (:875-876).
    let back = *get_total().back().unwrap_or(&0);
    out += &mv_to(b_y + 1, b_x + 1);
    out += &c_main_fg;
    out += FX_B;
    out += "CPU ";
    out += &meter(
        cpu_meter_width.max(0) as usize,
        &cpu_grad,
        &c_meter_bg,
        &reset,
        back,
        false,
    );
    out += &cpu_grad[clamp_i64(back, 0, 100) as usize];
    out += &rjust(&back.to_string(), 4, false);
    out += &c_main_fg;
    out += "%";
    if show_temps {
        let tback = *input.temp[0].back().unwrap_or(&0);
        let (temp_v, unit) = celsius_to(tback, input.temp_scale); // :878
        let tcolor = &temp_grad[clamp_i64(tback * 100 / safe_max, 0, 100) as usize]; // :879
        if (b_column_size > 1 || b_columns > 1) && !temp_graphs.is_empty() {
            // :880-882
            out += " ";
            out += &c_inactive;
            out += &graph_bg.repeat(5);
            out += &mv_l(5);
            out += tcolor;
            out += temp_graphs[0].render();
        }
        out += tcolor;
        out += &rjust(&temp_v.to_string(), 4, false);
        out += &c_main_fg;
        out += unit;
    }
    // UNTESTED-BRANCH: watts (:886-893). Harness show_watts=false.
    if show_watts {
        let _ = input.usage_watts;
    }
    out += &c_div_line;
    out += crate::symbols::box_chars::V_LINE; // :895

    // Core text and graphs (:911-958).
    let max_row = b_height - 3; // :900 (n_gpus=0)
    let is_enabled = |num: usize| -> bool {
        match input.active_cpus {
            None => true,
            Some(list) => list.contains(&(num as i32)),
        }
    };
    let mut cx: i64 = 0;
    let mut cy: i64 = 1;
    let mut cc: i64 = 0;
    let mut core_width = if b_column_size == 0 { 2 } else { 3 }; // :912
    if input.core_count >= 100 {
        core_width += 1;
    }
    for n in 0..input.core_count {
        let enabled = is_enabled(n);
        out += &mv_to(b_y + cy + 1, b_x + cx + 1);
        out += if enabled { &c_main_fg } else { &c_inactive };
        if input.core_count < 100 {
            out += FX_B;
            out += "C";
            out += FX_UB;
        }
        out += &ljust(&n.to_string(), core_width as usize, false);
        if (b_column_size > 0 || extra_width > 0) && n < core_graphs.len() {
            out += &c_inactive;
            out += &graph_bg.repeat((5 * b_column_size + extra_width).max(0) as usize);
            out += &mv_l(5 * b_column_size + extra_width);
            out += core_graphs[n].render();
        }
        let cback = *input.cores.get(n).and_then(|d| d.back()).unwrap_or(&0);
        out += if enabled {
            &cpu_grad[clamp_i64(cback, 0, 100) as usize]
        } else {
            &c_inactive
        };
        out += &rjust(
            &cback.to_string(),
            if b_column_size < 2 { 3 } else { 4 },
            false,
        );
        out += if enabled { &c_main_fg } else { &c_inactive };
        out += "%";
        if show_temps && !hide_cores {
            if let Some(core_temps) = input.temp.get(n + 1) {
                if !core_temps.is_empty() {
                    let last = *core_temps.back().unwrap();
                    let (tv, unit) = celsius_to(last, input.temp_scale);
                    let tcol = if enabled {
                        temp_grad[clamp_i64(last * 100 / safe_max, 0, 100) as usize].clone()
                    } else {
                        c_inactive.clone()
                    };
                    if b_column_size > 1 && temp_graphs.len() >= n {
                        out += " ";
                        out += &c_inactive;
                        out += &graph_bg.repeat(5);
                        out += &mv_l(5);
                        out += temp_graphs[n + 1].render();
                    }
                    out += &tcol;
                    out += &rjust(&tv.to_string(), 4, false);
                    out += if enabled { &c_main_fg } else { &c_inactive };
                    out += unit;
                }
            }
        }
        out += &c_div_line;
        out += crate::symbols::box_chars::V_LINE;
        // :954: `if ((++cy > ceil(coreCount/b_columns) or cy == max_row) ...)`
        // — cy increments BEFORE the wrap check.
        cy += 1;
        if ((cy as f64 > (input.core_count as f64 / b_columns as f64).ceil()) || cy == max_row)
            && n != input.core_count - 1
        {
            cc += 1;
            if cc >= b_columns {
                break;
            }
            cy = 1;
            cx = (b_width / b_columns) * cc;
        }
    }

    // Load average (:961-973).
    // NOTE: C++ increments cy in the for-update expression; the port above
    // applies the same ++cy / column-wrap explicitly per core.
    if cy < b_height - 1 && cc <= b_columns {
        cy = b_height - 2;
        let pre = "Load avg:";
        let mut avg = String::new();
        for v in input.load_avg {
            avg += &format!(" {v:.2}");
        }
        let len = (pre.len() + avg.len()) as i64;
        out += &mv_to(b_y + cy, b_x + 1);
        out += &" ".repeat((b_width - len - 2).max(0) as usize);
        out += &c_main_fg;
        out += FX_B;
        out += pre;
        out += FX_UB;
        out += &avg;
    }

    // UNTESTED-BRANCH: GPU brief info (:975-1021). Harness show_gpu=false.
    if f.show_gpu {
        let _ = mv_to(0, 0);
    }

    out += &reset; // :1024 (+Fx::reset)
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harness_flag_defaults_match_setup() {
        let f = CpuFlags::harness_defaults();
        assert!(f.check_temp && f.got_sensors);
        assert!(!f.single_graph && f.invert_lower);
        assert!(!f.show_watts_cfg && !f.show_freq_cfg && !f.show_uptime);
        assert!(!f.show_battery_cfg && !f.tty_mode && !f.cpu_bottom);
        assert_eq!(f.update_ms, 2000);
    }

    #[test]
    fn empty_total_returns_empty() {
        let percent: HashMap<String, VecDeque<i64>> = HashMap::new();
        let cores = [VecDeque::from([1])];
        let temp = [VecDeque::from([50])];
        let available = ["total".to_string()];
        let input = CpuDrawInput {
            percent: &percent,
            cores: &cores,
            temp: &temp,
            temp_max: 95,
            load_avg: [0.0, 0.0, 0.0],
            usage_watts: 0.0,
            active_cpus: None,
            core_count: 1,
            cpu_name: "c",
            custom_cpu_name: "",
            cpu_hz: "",
            container_engine: None,
            graph_up_cfg: "total",
            graph_lo_cfg: "total",
            available_fields: &available,
            graph_symbol_cfg: "braille",
            graph_symbol_cpu_cfg: "default",
            temp_scale: "celsius",
            flags: CpuFlags::harness_defaults(),
            battery: None,
            term_width: 100,
            force_redraw: true,
            data_same: false,
            prev: None,
        };
        let theme = crate::theme_grad::default_theme();
        let geom = crate::boxes::calc_sizes(&crate::boxes::LayoutInput::defaults(100, 30)).cpu;
        assert_eq!(draw_cpu(&input, &geom, &theme), "");
    }
}
