//! GPU box (`Gpu::draw`, src/btop_draw.cpp:1050-1221).
//!
//! Stateless port of the force-redraw path plus the per-tick value path.
//! C++ keeps `Graph`/`Meter` objects, `redraw`/geometry statics and the
//! `box` strings across calls; this port rebuilds them from explicit inputs
//! each call (same bytes for `force_redraw=true, data_same=false`, which
//! is what the golden fixtures use).
//!
//! `data_same=true` returns `prev` unchanged (documents
//! `Graph::operator(data_same=true)` returning cached `out`; the rest of
//! the box would re-render identical bytes anyway when data is unchanged).
//! `force_redraw=false` omits the outer/inner boxes (incremental path);
//! graphs are still fully rebuilt (stateless approximation — byte parity is
//! only tested for `force_redraw=true`).
//!
//! Stateless-Graph contract: callers must pass full histories, never deltas
//! (every `Graph::new` below receives the complete `Vec` contents;
//! `Graph::push` incrementalism is not used).
//!
//! Reuse decisions:
//! - Shared `Palette`/`rjust_b`/`human_bytes`/`celsius_to` from boxes.rs
//!   (Step 0 dedupe); the outer frame keeps `Theme::c("cpu_box")` verbatim
//!   (:2413 carries a `TODO gpu_box` — there is no `gpu_box` key in draw).
//! - `floating_humanizer` with `HumanOpts` (Task 2) for vram/pcie; the
//!   pcie call passes `start=1` (btop_draw.cpp:1206-1207).
//! - `base_10_sizes` (plus the `base_10_bitrate` Auto rule, which only
//!   matters for bit+per_second — gpu never makes such a call) is resolved
//!   by the caller into `GpuFlags::base_10`.

use crate::ansi::{mv_d, mv_l, mv_r, mv_to, mv_u, FX_B, FX_UB};
use crate::boxes::{celsius_to, create_box, human_bytes, rjust_b, CommonFlags, GpuGeom, Palette};
use crate::meter_graph::{meter, Graph, GraphOpts};
use crate::symbols::box_chars::{
    DIV_DOWN, DIV_LEFT, DIV_RIGHT, DIV_UP, H_LINE, ROUND_RIGHT_DOWN, TITLE_LEFT, TITLE_LEFT_DOWN,
    TITLE_RIGHT, TITLE_RIGHT_DOWN, V_LINE,
};
use crate::symbols::graph_table;
use crate::theme_grad::gradient;
use btop_tools::strtools::{floating_humanizer, uresize, HumanOpts};
use std::collections::HashMap;

fn clamp_i64(v: i64, lo: i64, hi: i64) -> i64 {
    v.clamp(lo, hi)
}

/// Per-section capability gates, mirroring `gpu_info_supported`
/// (src/btop_shared.hpp:136-149, all default `true`). The harness fixture
/// (`tests/draw_golden.cpp:fixed_gpu`) leaves them defaulted, so every
/// section renders in `gpu_S0/S1.ans`; the smoke tests flip each one off.
#[derive(Debug, Clone)]
pub struct GpuSupported {
    pub gpu_utilization: bool,   // :1085/:1114 util graphs + GPU meter
    pub mem_utilization: bool,   // :1102/:1177 vram utilization graph
    pub gpu_clock: bool,         // :1133 gpu clock title
    pub mem_clock: bool,         // :1183/:1194 vram clock title
    pub pwr_usage: bool,         // :1100/:1140 PWR meter
    pub pwr_state: bool,         // :1101/:1144 P-state suffix
    pub temp_info: bool,         // :1068/:1098 temp graph + readout
    pub mem_total: bool,         // :1104/:1159 vram section
    pub mem_used: bool,          // :1104/:1159 vram section
    pub pcie_txrx: bool,         // :1205 TX/RX footer
    pub encoder_utilization: bool, // :1106/:1150 ENC meter
    pub decoder_utilization: bool, // :1150 DEC meter
}

impl GpuSupported {
    /// C++ default: every capability on (`gpu_info_supported` in-class
    /// initializers, btop_shared.hpp:136-149).
    pub fn all() -> Self {
        Self {
            gpu_utilization: true,
            mem_utilization: true,
            gpu_clock: true,
            mem_clock: true,
            pwr_usage: true,
            pwr_state: true,
            temp_info: true,
            mem_total: true,
            mem_used: true,
            pcie_txrx: true,
            encoder_utilization: true,
            decoder_utilization: true,
        }
    }
}

/// Every Config key `Gpu::draw` reads, grouped (GraphOpts precedent).
/// Each field cites its cpp line.
#[derive(Debug, Clone)]
pub struct GpuFlags {
    pub check_temp: bool,   // :1068 Config check_temp (with temp_info)
    pub mirror_graph: bool, // :1073 Config gpu_mirror_graph (negated: single_graph)
    pub invert_lower: bool, // :1093 Config cpu_invert_lower (lower graph)
    /// Resolved `base_10_sizes` (harness false; the `base_10_bitrate`
    /// Auto/True/False override only affects bit+per_second calls, which
    /// gpu never makes).
    pub base_10: bool,
    /// Shared box-independent flags (see [`CommonFlags`]).
    pub common: CommonFlags,
}

impl GpuFlags {
    /// Harness defaults (tests/draw_golden.cpp setup() + btop_config.cpp
    /// compiled-in defaults): check_temp=true, gpu_mirror_graph=true,
    /// cpu_invert_lower=true, base_10=false.
    pub fn harness_defaults() -> Self {
        Self {
            check_temp: true,
            mirror_graph: true,
            invert_lower: true,
            base_10: false,
            common: CommonFlags::harness_defaults(),
        }
    }
}

/// All inputs `Gpu::draw` reads, as explicit params.
/// Mirrors btop_draw.cpp:1050-1221 (plus the frame built in
/// `calcSizes`, :2413-2424).
pub struct GpuDrawInput<'a> {
    /// `gpu.gpu_percent` (:1086/:1105/:1115 — "gpu-totals",
    /// "gpu-vram-totals", "gpu-pwr-totals" full histories; `safeVal`
    /// falls back to empty when a key is missing).
    pub percent: &'a HashMap<String, Vec<i64>>,
    pub gpu_clock_speed: i64, // :1134 MHz
    pub pwr_usage: i64,       // :1141/:1143 mW
    /// `gpu.pwr_state` (:1144; member has no default init in C++ — the
    /// harness pins 8, tests/draw_golden.cpp:296).
    pub pwr_state: i64,
    /// `gpu.temp` full history (:1099/:1124-1126).
    pub temp: &'a [i64],
    pub temp_max: i64, // :1099/:1125 (harness 95; used raw, no :604-style guard)
    pub mem_total: u64, // :1173/:1191 bytes
    pub mem_used: u64,  // :1162/:1192 bytes
    /// `gpu.mem_utilization_percent` full history (:1103/:1180).
    pub mem_utilization: &'a [i64],
    pub mem_clock_speed: i64, // :1184/:1195 MHz
    /// PCIe throughput in KB/s (:1206-1207). Negative RX/TX means manually
    /// disabled, hiding the footer even when `pcie_txrx` is supported.
    pub pcie_tx: i64,
    pub pcie_rx: i64,
    pub encoder_utilization: i64, // :1152 %
    pub decoder_utilization: i64, // :1154 %
    pub supported: GpuSupported,
    /// Resolved panel name: `custom_gpu_nameN` or `gpu_names[N]`
    /// (:2421-2424; harness "Golden GPU").
    pub gpu_name: &'a str,
    /// `shown_panels[i]` (:2413: outer title `"gpu{panel}"`, number
    /// `(panel+5)%10`).
    pub panel: i64,
    pub graph_symbol_cfg: &'a str,     // :1071 Config graph_symbol
    pub graph_symbol_gpu_cfg: &'a str, // :1071 Config graph_symbol_gpu
    pub flags: GpuFlags,
    pub force_redraw: bool,    // :1067 force_redraw → redraw
    pub data_same: bool,       // :1115+ Graph data_same
    pub prev: Option<&'a str>, // cached out for data_same
}

/// Graphs built from the current histories (`:1082-1107` redraw block).
/// Each is `Some` only when its section is supported — unsupported
/// sections skip both construction and render, exactly like C++.
/// `meter_width` is the :1096 GPU meter width (kept beside the graphs it
/// sizes so `render_util` needs no extra width plumbing).
struct BuiltGraphs {
    upper: Option<Graph>,
    lower: Option<Graph>,
    temp: Option<Graph>,
    mem_used: Option<Graph>,
    mem_util: Option<Graph>,
    meter_width: i64,
}

/// Outer + inner boxes (:2413-2424). Returns empty when `!force_redraw`
/// (incremental path). The outer frame keeps `Theme::c("cpu_box")`
/// verbatim (:2413 `TODO gpu_box`); the inner title is the resolved gpu
/// name cut to `b_width - 5`.
fn render_frame(input: &GpuDrawInput, geom: &GpuGeom, pal: &Palette) -> String {
    if !input.force_redraw {
        return String::new();
    }
    let f = &input.flags;
    let mut out = String::new();
    out += &create_box(
        geom.x,
        geom.y,
        geom.width,
        geom.height,
        &pal.box_color,
        true,
        &format!("gpu{}", input.panel),
        "",
        (input.panel + 5) % 10,
        &pal.div_line,
        &pal.hi_fg,
        &pal.title,
        &pal.reset,
        f.common.tty_mode,
        f.common.rounded,
    );
    let name = uresize(input.gpu_name, (geom.b_width - 5).max(0) as usize, false);
    out += &create_box(
        geom.b_x,
        geom.b_y,
        geom.b_width,
        geom.b_inner_h,
        "",
        false,
        &name,
        "",
        0,
        &pal.div_line,
        &pal.hi_fg,
        &pal.title,
        &pal.reset,
        f.common.tty_mode,
        f.common.rounded,
    );
    out
}

/// Util graphs + GPU meter + temp readout + clock title (:1114-1137).
/// `rows_used` enters at 1 and leaves at 2 when the section runs.
#[allow(clippy::too_many_arguments)]
fn render_util(
    input: &GpuDrawInput,
    geom: &GpuGeom,
    pal: &Palette,
    graphs: &BuiltGraphs,
    cpu_grad: &[String],
    temp_grad: &[String],
    graph_bg: &str,
    show_temps: bool,
    single_graph: bool,
    graph_up_height: i64,
    safe_max: i64,
    rows_used: i64,
) -> (String, i64) {
    let mut out = String::new();
    let rows_used = rows_used + 1; // :1130 (section always costs one row)
    let empty: &[i64] = &[];
    let totals = input
        .percent
        .get("gpu-totals")
        .map(|v| v.as_slice())
        .unwrap_or(empty);
    let (x, y) = (geom.x, geom.y);
    let (b_x, b_y) = (geom.b_x, geom.b_y);
    out += FX_UB;
    out += &mv_to(y + rows_used - 1, x + 1); // :1115 (rows_used pre-increment value)
    if let Some(upper) = &graphs.upper {
        out += upper.render();
    }
    // Lower graph (:1117; no Fx::ub prefix, verbatim).
    if !single_graph {
        if let Some(lower) = &graphs.lower {
            out += &mv_to(y + rows_used - 1 + graph_up_height, x + 1);
            out += lower.render();
        }
    }
    // GPU meter line (:1119-1120; value field is 5 wide here, not 4).
    let back = totals.last().copied().unwrap_or(0);
    out += &mv_to(b_y + rows_used - 1, b_x + 1);
    out += &pal.main_fg;
    out += FX_B;
    out += "GPU ";
    out += &meter(
        graphs.meter_width.max(0) as usize,
        cpu_grad,
        &pal.meter_bg,
        &pal.reset,
        back,
        false,
    );
    out += &cpu_grad[clamp_i64(back, 0, 100) as usize];
    out += &rjust_b(&back.to_string(), 5);
    out += &pal.main_fg;
    out += "%";
    // Temperature graph + readout (:1123-1128; 6-wide bg, not cpu's 5).
    if show_temps {
        let tback = input.temp.last().copied().unwrap_or(0);
        let (temp_v, unit) = celsius_to(tback, &input.flags.common.temp_scale); // :1124
        out += " ";
        out += &pal.inactive;
        out += &graph_bg.repeat(6);
        out += &mv_l(6);
        out += &temp_grad[clamp_i64(tback * 100 / safe_max, 0, 100) as usize]; // :1125
        if let Some(temp) = &graphs.temp {
            out += temp.render();
        }
        out += &rjust_b(&temp_v.to_string(), 4); // :1127
        out += &pal.main_fg;
        out += unit;
    }
    out += &pal.div_line;
    out += V_LINE; // :1129
    // Clock title (:1133-1137; bare glyphs, no box-color prefix).
    if input.supported.gpu_clock {
        let clock = input.gpu_clock_speed.to_string();
        out += &mv_to(b_y, b_x + geom.b_width - 12);
        out += &pal.div_line;
        out += &H_LINE.repeat((5 - clock.len() as i64).max(0) as usize);
        out += TITLE_LEFT;
        out += FX_B;
        out += &pal.title;
        out += &clock;
        out += " MHz";
        out += FX_UB;
        out += &pal.div_line;
        out += TITLE_RIGHT;
    }
    (out, rows_used)
}

/// PWR meter + P-state (:1140-1147).
fn render_pwr(
    input: &GpuDrawInput,
    geom: &GpuGeom,
    pal: &Palette,
    cached_grad: &[String],
    rows_used: i64,
) -> (String, i64) {
    let mut out = String::new();
    if !input.supported.pwr_usage {
        return (out, rows_used);
    }
    let empty: &[i64] = &[];
    let back = input
        .percent
        .get("gpu-pwr-totals")
        .map(|v| v.as_slice())
        .unwrap_or(empty)
        .last()
        .copied()
        .unwrap_or(0);
    out += &mv_to(geom.b_y + rows_used, geom.b_x + 1);
    out += &pal.main_fg;
    out += FX_B;
    out += "PWR ";
    out += &meter(
        (geom.b_width
            - if input.supported.pwr_state && input.pwr_state != 32 {
                25
            } else {
                12
            })
        .max(0) as usize, // :1101
        cached_grad,
        &pal.meter_bg,
        &pal.reset,
        back,
        false,
    );
    out += &cached_grad[clamp_i64(back, 0, 100) as usize];
    // Milliwatts → W with magnitude precision (:1143).
    let prec = if input.pwr_usage < 10_000 {
        2
    } else if input.pwr_usage < 100_000 {
        1
    } else {
        0
    };
    out += &format!(
        "{:>5.prec$}",
        input.pwr_usage as f64 / 1000.0,
        prec = prec
    );
    out += &pal.main_fg;
    out += "W";
    // P-state (:1144-1145; 32 is NVML_PSTATE_UNKNOWN — hidden).
    if input.supported.pwr_state && input.pwr_state != 32 {
        out += " P-state: ";
        out += if input.pwr_state > 9 { "" } else { " " };
        out += "P";
        out += &cached_grad[clamp_i64(input.pwr_state, 0, 100) as usize];
        out += &input.pwr_state.to_string();
    }
    (out, rows_used + 1) // :1146
}

/// ENC + DEC meters (:1150-1157). Both share one `enc_meter` width.
fn render_enc_dec(
    input: &GpuDrawInput,
    geom: &GpuGeom,
    pal: &Palette,
    cpu_grad: &[String],
    rows_used: i64,
) -> (String, i64) {
    let mut out = String::new();
    if !(input.supported.encoder_utilization && input.supported.decoder_utilization) {
        return (out, rows_used);
    }
    let enc_w = (geom.b_width / 2 - 10).max(0) as usize; // :1107
    out += &mv_to(geom.b_y + rows_used, geom.b_x + 1);
    out += &pal.main_fg;
    out += FX_B;
    out += "ENC ";
    out += &meter(
        enc_w,
        cpu_grad,
        &pal.meter_bg,
        &pal.reset,
        input.encoder_utilization,
        false,
    );
    out += &cpu_grad[clamp_i64(input.encoder_utilization, 0, 100) as usize];
    out += &rjust_b(&input.encoder_utilization.to_string(), 4);
    out += &pal.main_fg;
    out += "%";
    out += &pal.div_line;
    out += V_LINE;
    out += &pal.main_fg;
    out += FX_B;
    out += "DEC ";
    out += &meter(
        enc_w,
        cpu_grad,
        &pal.meter_bg,
        &pal.reset,
        input.decoder_utilization,
        false,
    );
    out += &cpu_grad[clamp_i64(input.decoder_utilization, 0, 100) as usize];
    out += &rjust_b(&input.decoder_utilization.to_string(), 4);
    out += &pal.main_fg;
    out += "%";
    (out, rows_used + 1) // :1156
}

/// VRAM section (:1159-1197): used/total header + graphs when both counts
/// exist, else a single total/usage line. Costs no row (`rows_used`
/// unchanged — the mem-clock title reuses the incoming row).
fn render_vram(
    input: &GpuDrawInput,
    geom: &GpuGeom,
    pal: &Palette,
    graphs: &BuiltGraphs,
    rows_used: i64,
) -> String {
    let mut out = String::new();
    let s = &input.supported;
    if !(s.mem_total || s.mem_used) {
        return out;
    }
    let f = &input.flags;
    let b_width = geom.b_width;
    out += &mv_to(geom.b_y + rows_used, geom.b_x);
    if s.mem_total && s.mem_used {
        let used_str = human_bytes(input.mem_used, false, f.base_10); // :1162
        // :1164 — the leading `(mem_total or mem_used)` factor is 1 on
        // this arm (both true); kept as a bool-to-int product, verbatim.
        let offset = (s.mem_total || s.mem_used) as i64
            * (1 + 2 * (s.mem_total && s.mem_used) as i64 + 2 * s.mem_utilization as i64);
        out += &pal.div_line;
        out += DIV_LEFT;
        out += H_LINE;
        out += TITLE_LEFT;
        out += FX_B;
        out += &pal.title;
        out += "vram";
        out += &pal.div_line;
        out += FX_UB;
        out += TITLE_RIGHT;
        out += &H_LINE.repeat((b_width / 2 - 8).max(0) as usize);
        out += DIV_UP;
        out += &mv_d(offset);
        out += &mv_l(1);
        out += DIV_DOWN;
        out += &mv_l(1);
        out += &mv_u(1);
        out += &(format!("{V_LINE}{}{}", mv_l(1), mv_u(1))).repeat((offset - 1).max(0) as usize);
        out += DIV_UP;
        out += H_LINE;
        out += &pal.title;
        out += "Used:";
        out += &pal.div_line;
        out += &H_LINE
            .repeat((b_width / 2 + b_width % 2 - 9 - used_str.len() as i64).max(0) as usize);
        out += &pal.title;
        out += &used_str;
        out += &pal.div_line;
        out += H_LINE;
        out += DIV_RIGHT;
        out += &mv_d(1);
        out += &mv_l(b_width / 2 - 1);
        if let Some(g) = &graphs.mem_used {
            out += g.render(); // :1172
        }
        let empty: &[i64] = &[];
        let vram_back = input
            .percent
            .get("gpu-vram-totals")
            .map(|v| v.as_slice())
            .unwrap_or(empty)
            .last()
            .copied()
            .unwrap_or(0);
        out += &mv_l(b_width - 3);
        out += &mv_u(1 + 2 * s.mem_utilization as i64);
        out += &pal.main_fg;
        out += FX_B;
        out += "Total:";
        out += &rjust_b(
            &human_bytes(input.mem_total, false, f.base_10),
            (b_width / 2 - 9).max(0) as usize,
        ); // :1173
        out += FX_UB;
        out += &mv_r(3);
        out += &rjust_b(&vram_back.to_string(), 3);
        out += "%";
        // Memory utilization (:1177-1180).
        if s.mem_utilization {
            out += &mv_l(b_width / 2 + 6);
            out += &mv_d(1);
            out += &pal.div_line;
            out += DIV_LEFT;
            out += H_LINE;
            out += &pal.title;
            out += "Utilization:";
            out += &pal.div_line;
            out += &H_LINE.repeat((b_width / 2 - 14).max(0) as usize);
            out += DIV_RIGHT;
            out += &mv_l(b_width / 2);
            out += &mv_d(1);
            if let Some(g) = &graphs.mem_util {
                out += g.render();
            }
            out += &mv_l(b_width / 2 - 1);
            out += &mv_u(1);
            out += &rjust_b(
                &input.mem_utilization.last().copied().unwrap_or(0).to_string(),
                3,
            );
            out += "%";
        }
        // Memory clock title (:1183-1187; bare glyphs).
        if s.mem_clock {
            let clock = input.mem_clock_speed.to_string();
            out += &mv_to(geom.b_y + rows_used, geom.b_x + b_width / 2 - 11);
            out += &pal.div_line;
            out += &H_LINE.repeat((5 - clock.len() as i64).max(0) as usize);
            out += TITLE_LEFT;
            out += FX_B;
            out += &pal.title;
            out += &clock;
            out += " MHz";
            out += FX_UB;
            out += &pal.div_line;
            out += TITLE_RIGHT;
        }
    } else {
        // Single-count line (:1188-1196).
        out += &pal.main_fg;
        out += &mv_r(1);
        if s.mem_total {
            out += "VRAM total:";
            out += &rjust_b(
                &human_bytes(input.mem_total, false, f.base_10),
                (b_width / (1 + s.mem_clock as i64) - 14).max(0) as usize,
            ); // :1191
        } else {
            out += "VRAM usage:";
            out += &rjust_b(
                &human_bytes(input.mem_used, false, f.base_10),
                (b_width / (1 + s.mem_clock as i64) - 14).max(0) as usize,
            ); // :1192
        }
        if s.mem_clock {
            out += "   VRAM clock:";
            out += &rjust_b(
                &format!("{} MHz", input.mem_clock_speed),
                (b_width / 2 - 13).max(0) as usize,
            ); // :1195
        }
    }
    out
}

/// PCIe TX/RX footer (:1205-1213). `height` is the LOCAL draw height
/// (`b_offset + 4`, :1075) — not the box height. Negative RX/TX hides the
/// footer (manually disabled, not unsupported).
fn render_pcie(input: &GpuDrawInput, geom: &GpuGeom, pal: &Palette, height: i64) -> String {
    let mut out = String::new();
    let s = &input.supported;
    if !(s.pcie_txrx && !(input.pcie_rx < 0 || input.pcie_tx < 0)) {
        return out;
    }
    let f = &input.flags;
    // (value, shorten=0, start=1, bit=0, per_second=1) — start=1 skips the
    // base unit, so KB/s values render from KiB up.
    let bit_opts = HumanOpts {
        shorten: false,
        bit: false,
        per_second: true,
        base_10: f.base_10,
    };
    let tx_str = floating_humanizer(input.pcie_tx as u64, 1, bit_opts);
    let rx_str = floating_humanizer(input.pcie_rx as u64, 1, bit_opts);
    let b_width = geom.b_width;
    out += &mv_to(geom.b_y + height - 3, geom.b_x + 2);
    out += &pal.div_line;
    out += TITLE_LEFT_DOWN;
    out += &pal.title;
    out += FX_B;
    out += "TX:";
    out += FX_UB;
    out += &pal.div_line;
    out += TITLE_RIGHT_DOWN;
    out += &H_LINE.repeat((b_width / 2 - 9 - tx_str.len() as i64).max(0) as usize);
    out += TITLE_LEFT_DOWN;
    out += &pal.title;
    out += FX_B;
    out += &tx_str;
    out += FX_UB;
    out += &pal.div_line;
    out += TITLE_RIGHT_DOWN;
    // Divider continues the vram header only when both counts exist.
    out += if s.mem_total && s.mem_used {
        DIV_DOWN
    } else {
        H_LINE
    };
    out += TITLE_LEFT_DOWN;
    out += &pal.title;
    out += FX_B;
    out += "RX:";
    out += FX_UB;
    out += &pal.div_line;
    out += TITLE_RIGHT_DOWN;
    out += &H_LINE.repeat((b_width / 2 + b_width % 2 - 9 - rx_str.len() as i64).max(0) as usize);
    out += TITLE_LEFT_DOWN;
    out += &pal.title;
    out += FX_B;
    out += &rx_str;
    out += FX_UB;
    out += &pal.div_line;
    out += TITLE_RIGHT_DOWN;
    out += ROUND_RIGHT_DOWN;
    out
}

/// Draw one GPU panel. `geom` is the panel's `GpuGeom` (calcSizes);
/// `theme` is the Default-keyed map (`default_theme()`).
pub fn draw_gpu(input: &GpuDrawInput, geom: &GpuGeom, theme: &HashMap<String, String>) -> String {
    // data_same → cached out (Graph::operator(data_same=true) returns `out`).
    if input.data_same {
        return input.prev.unwrap_or("").to_string();
    }
    let f = &input.flags;
    let lowcolor = f.common.lowcolor;
    let tbg = f.common.theme_background;

    // Outer frame keeps cpu_box verbatim (:2413 `TODO gpu_box`).
    let pal = Palette::new("cpu_box", theme, lowcolor, tbg);

    // Symbol resolution (:1071-1072 + Graph ctor :498-501).
    let base_symbol: &str = if f.common.tty_mode || input.graph_symbol_gpu_cfg == "tty" {
        "tty"
    } else if input.graph_symbol_gpu_cfg != "default" {
        input.graph_symbol_gpu_cfg
    } else {
        input.graph_symbol_cfg
    };
    let table_key = format!("{base_symbol}_up");
    let graph_bg = graph_table(&table_key).map(|t| t[6]).unwrap_or(" ");

    let show_temps = input.supported.temp_info && f.check_temp; // :1068
    let single_graph = !f.mirror_graph; // :1073
    // Local draw height (:1075) — NOT the box height.
    let height = geom.b_inner_h + 2;
    // Graph heights (:1082-1083; b_full_h is b_height_vec post-:2425).
    let graph_up_height = if single_graph {
        geom.b_full_h
    } else {
        (geom.b_full_h + 1) / 2
    };
    let graph_low_height = if single_graph {
        0
    } else {
        geom.b_full_h - graph_up_height
    };

    let cpu_grad = gradient("cpu", theme, lowcolor);
    let temp_grad = gradient("temp", theme, lowcolor);
    let cached_grad = gradient("cached", theme, lowcolor);
    let empty: &[i64] = &[];
    let totals = input
        .percent
        .get("gpu-totals")
        .map(|v| v.as_slice())
        .unwrap_or(empty);
    let vram_totals = input
        .percent
        .get("gpu-vram-totals")
        .map(|v| v.as_slice())
        .unwrap_or(empty);

    // Graph construction (:1085-1107; built when the section is supported —
    // the stateless stand-in for the redraw-gated stateful rebuild).
    let graph_w = (geom.x + geom.width - geom.b_width - 3).max(0) as usize;
    let s = &input.supported;
    // GPU meter width (:1096).
    let meter_width = geom.b_width - if show_temps { 25 } else { 12 };
    let graphs = BuiltGraphs {
        upper: s.gpu_utilization.then(|| {
            Graph::new(
                GraphOpts {
                    width: graph_w,
                    height: graph_up_height.max(0) as usize,
                    gradient: cpu_grad.clone(),
                    symbol: base_symbol.to_string(),
                    invert: false,
                    no_zero: true,
                    max_value: 0,
                    offset: 0,
                },
                &pal.reset,
                totals,
            )
        }),
        lower: (s.gpu_utilization && !single_graph).then(|| {
            Graph::new(
                GraphOpts {
                    width: graph_w,
                    height: graph_low_height.max(0) as usize,
                    gradient: cpu_grad.clone(),
                    symbol: base_symbol.to_string(),
                    invert: f.invert_lower,
                    no_zero: true,
                    max_value: 0,
                    offset: 0,
                },
                &pal.reset,
                totals,
            )
        }),
        temp: (s.gpu_utilization && s.temp_info).then(|| {
            Graph::new(
                GraphOpts {
                    width: 6,
                    height: 1,
                    gradient: temp_grad.clone(),
                    symbol: base_symbol.to_string(),
                    invert: false,
                    no_zero: false,
                    max_value: input.temp_max,
                    offset: -23,
                },
                &pal.reset,
                input.temp,
            )
        }),
        mem_used: (s.mem_used && s.mem_total).then(|| {
            Graph::new(
                GraphOpts {
                    width: (geom.b_width / 2 - 2).max(0) as usize,
                    height: (2 + 2 * s.mem_utilization as i64).max(0) as usize,
                    gradient: gradient("used", theme, lowcolor),
                    symbol: base_symbol.to_string(),
                    invert: false,
                    no_zero: false,
                    max_value: 0,
                    offset: 0,
                },
                &pal.reset,
                vram_totals,
            )
        }),
        mem_util: (s.mem_used && s.mem_total && s.mem_utilization).then(|| {
            Graph::new(
                GraphOpts {
                    width: (geom.b_width / 2 - 1).max(0) as usize,
                    height: 2,
                    gradient: gradient("free", theme, lowcolor),
                    symbol: base_symbol.to_string(),
                    invert: false,
                    no_zero: false,
                    max_value: 100,
                    offset: 4,
                },
                &pal.reset,
                input.mem_utilization,
            )
        }),
        meter_width,
    };

    let mut out = String::new();
    out += &render_frame(input, geom, &pal);

    // General GPU info (:1111+). rows_used starts at 1.
    let mut rows_used = 1i64;
    if s.gpu_utilization {
        let (text, rows) = render_util(
            input,
            geom,
            &pal,
            &graphs,
            &cpu_grad,
            &temp_grad,
            graph_bg,
            show_temps,
            single_graph,
            graph_up_height,
            input.temp_max,
            rows_used,
        );
        out += &text;
        rows_used = rows;
    }
    let (text, rows) = render_pwr(input, geom, &pal, &cached_grad, rows_used);
    out += &text;
    rows_used = rows;
    let (text, rows) = render_enc_dec(input, geom, &pal, &cpu_grad, rows_used);
    out += &text;
    rows_used = rows;
    out += &render_vram(input, geom, &pal, &graphs, rows_used);
    out += &render_pcie(input, geom, &pal, height);

    out += &pal.reset; // :1216 (+Fx::reset)
    out
}
