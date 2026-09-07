//! Box outlines, banner, clock render, and box layout (`calcSizes`).
//!
//! Ports `Draw::createBox` (src/btop_draw.cpp:279-331), `Draw::banner_gen`
//! (:138-177), the render tail of `Draw::update_clock` (:382-392), and
//! `Draw::calcSizes` (:2248-2551, GPU_SUPPORT build). `TextEdit` (:179-277)
//! is skipped (M4).

use crate::ansi::{mv_d, mv_l, mv_r, mv_to, FX_B, FX_I, FX_UB};
use crate::symbols::box_chars::{
    H_LINE, LEFT_DOWN, LEFT_UP, RIGHT_DOWN, RIGHT_UP, ROUND_LEFT_DOWN, ROUND_LEFT_UP,
    ROUND_RIGHT_DOWN, ROUND_RIGHT_UP, TITLE_LEFT, TITLE_LEFT_DOWN, TITLE_RIGHT, TITLE_RIGHT_DOWN,
    V_LINE,
};
use crate::symbols::SUPERSCRIPT;
use crate::theme_grad::color;
use btop_config::theme::{dec_to_color, hex_to_color};
use btop_tools::strtools::{floating_humanizer, ljust, rjust, uresize, HumanOpts};
use std::collections::HashMap;

// ── CommonFlags ─────────────────────────────────────────────────────────────

/// Flags shared by every box draw. Task 5+ (mem/net/proc/gpu) embed this
/// same struct instead of re-declaring the fields per box — that is the 4×
/// duplication this stops.
///
/// Lives in boxes.rs (not lib.rs) because `create_box` / `banner_gen` /
/// `render_clock` already thread exactly these theme/mode knobs as params;
/// this struct is their future grouped form. `temp_scale` is
/// box-independent but shared by every temp-rendering box, so it rides
/// along rather than staying a per-box string.
#[derive(Debug, Clone)]
pub struct CommonFlags {
    pub tty_mode: bool,         // Config tty_mode (:599, createBox :289)
    pub rounded: bool,          // Config rounded_corners (createBox :289)
    pub lowcolor: bool,         // Config lowcolor (Theme depth :146)
    pub theme_background: bool, // Config theme_background (color() :133)
    pub temp_scale: String,     // Config temp_scale (:602)
}

impl CommonFlags {
    /// Harness defaults (tests/draw_golden.cpp setup() + btop_config.cpp
    /// compiled-in defaults): tty off, rounded on, full color, themed
    /// background, celsius.
    pub fn harness_defaults() -> Self {
        Self {
            tty_mode: false,
            rounded: true,
            lowcolor: false,
            theme_background: true,
            temp_scale: "celsius".to_string(),
        }
    }
}

// ── Shared draw helpers (Step 0: deduped from cpu/mem/net) ────────────────

/// `rjust`/`ljust` with C++ defaults (`utf=false, wide=false,
/// limit=true`, src/btop_tools.cpp:346-376): byte sizes, truncate overlong.
/// Every just input in mem/net/gpu draw is humanizer/number/title ASCII,
/// where byte length == scalar count, so the shared scalar helpers agree.
///
/// NOTE: cpu.rs intentionally does NOT use these — its core/temp columns
/// pass `limit=false` (never truncate). Unifying would change bytes.
pub fn rjust_b(s: &str, x: usize) -> String {
    rjust(s, x, true)
}

pub fn ljust_b(s: &str, x: usize) -> String {
    ljust(s, x, true)
}

/// Thin wrapper over [`floating_humanizer`] for plain byte counts
/// (`bit=false, per_second=false`; mem.rs `human()` shape, also the net
/// max-label shape). Callers needing bit/per_second make explicit
/// `floating_humanizer` calls.
pub fn human_bytes(value: u64, shorten: bool, base_10: bool) -> String {
    floating_humanizer(
        value,
        0,
        HumanOpts {
            shorten,
            bit: false,
            per_second: false,
            base_10,
        },
    )
}

/// Celsius readout conversion, mirroring the `celsius_to` helper used by
/// cpu/gpu draw (btop_draw.cpp). Returns `(value, unit)`.
pub fn celsius_to(celsius: i64, scale: &str) -> (i64, &'static str) {
    match scale {
        "celsius" => (celsius, "°C"),
        "fahrenheit" => ((celsius as f64 * 1.8 + 32.0).round() as i64, "°F"),
        "kelvin" => ((celsius as f64 + 273.15).round() as i64, "K "),
        "rankine" => ((celsius as f64 * 1.8 + 491.67).round() as i64, "°R"),
        _ => (0, ""),
    }
}

/// Resolved palette for one draw call (`Theme::c` under the caller flags).
/// Union of the cpu/mem/net per-box palettes: `box_color` is the box's own
/// key (`cpu_box`/`mem_box`/`net_box`/`gpu_box`); every other field is
/// shared. Boxes that never read a field (e.g. net never reads
/// `meter_bg`) simply leave it unread — one constructor, zero drift.
#[derive(Debug, Clone)]
pub struct Palette {
    pub box_color: String,
    pub div_line: String,
    pub main_fg: String,
    pub title: String,
    pub hi_fg: String,
    pub inactive: String,
    pub meter_bg: String,
    pub graph_text: String,
    pub reset: String,
}

impl Palette {
    /// Resolve every color for `box_key` (`cpu_box`, `mem_box`, `net_box`,
    /// `gpu_box`) plus the shared keys. `reset` is `Fx::reset`
    /// (`reset_base + main_fg + main_bg`, btop_tools.cpp:749 /
    /// btop_theme.cpp:468).
    pub fn new(
        box_key: &str,
        theme: &HashMap<String, String>,
        lowcolor: bool,
        theme_background: bool,
    ) -> Self {
        Self {
            box_color: color(box_key, theme, lowcolor, theme_background),
            div_line: color("div_line", theme, lowcolor, theme_background),
            main_fg: color("main_fg", theme, lowcolor, theme_background),
            title: color("title", theme, lowcolor, theme_background),
            hi_fg: color("hi_fg", theme, lowcolor, theme_background),
            inactive: color("inactive_fg", theme, lowcolor, theme_background),
            meter_bg: color("meter_bg", theme, lowcolor, theme_background),
            graph_text: color("graph_text", theme, lowcolor, theme_background),
            reset: format!(
                "{}{}{}",
                "\x1b[0m",
                color("main_fg", theme, lowcolor, theme_background),
                color("main_bg", theme, lowcolor, theme_background),
            ),
        }
    }
}

// ── createBox ─────────────────────────────────────────────────────────────

/// Outline box, porting `Draw::createBox` (src/btop_draw.cpp:279-331).
///
/// `line_color` empty selects `div_line`; `num == 0` suppresses numbering
/// (tty: decimal, else superscript, clamped to 0-9); corners are rounded
/// unless `tty_mode` or `!rounded`. Colors and reset are caller-provided
/// (`Theme::c("div_line"/"hi_fg"/"title")`, `Fx::reset`); the two `bool`s
/// are Config `tty_mode` / `rounded_corners`.
///
/// Layout always passes positive geometry; degenerate `width < 1` or
/// `height < 1` defensively returns an empty string (C++ would underflow
/// `width - 1`/`width - 2` into huge repeats).
#[allow(clippy::too_many_arguments)]
pub fn create_box(
    x: i64,
    y: i64,
    width: i64,
    height: i64,
    line_color: &str,
    fill: bool,
    title: &str,
    title2: &str,
    num: i64,
    div_line: &str,
    hi_fg: &str,
    title_color: &str,
    reset: &str,
    tty_mode: bool,
    rounded: bool,
) -> String {
    if width < 1 || height < 1 {
        return String::new();
    }
    debug_assert!(width >= 1 && height >= 1, "box geometry must be positive");
    let lc = if line_color.is_empty() {
        div_line
    } else {
        line_color
    };
    let numbering = if num == 0 {
        String::new()
    } else if tty_mode {
        format!("{hi_fg}{num}")
    } else {
        format!("{hi_fg}{}", SUPERSCRIPT[num.clamp(0, 9) as usize])
    };
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

    let mut out = String::from(reset) + lc;
    // Horizontal lines (top + bottom rows).
    for hpos in [y, y + height - 1] {
        out += &mv_to(hpos, x);
        out += &H_LINE.repeat((width - 1) as usize);
    }
    // Vertical lines with optional fill.
    for hpos in y + 1..y + height - 1 {
        out += &mv_to(hpos, x);
        out += V_LINE;
        out += &if fill {
            " ".repeat((width - 2) as usize)
        } else {
            mv_r(width - 2)
        };
        out += V_LINE;
    }
    // Corners.
    out += &mv_to(y, x);
    out += left_up;
    out += &mv_to(y, x + width - 1);
    out += right_up;
    out += &mv_to(y + height - 1, x);
    out += left_down;
    out += &mv_to(y + height - 1, x + width - 1);
    out += right_down;
    // Titles.
    if !title.is_empty() {
        out += &mv_to(y, x + 2);
        out += TITLE_LEFT;
        out += FX_B;
        out += &numbering;
        out += title_color;
        out += title;
        out += FX_UB;
        out += lc;
        out += TITLE_RIGHT;
    }
    if !title2.is_empty() {
        out += &mv_to(y + height - 1, x + 2);
        out += TITLE_LEFT_DOWN;
        out += FX_B;
        out += &numbering;
        out += title_color;
        out += title2;
        out += FX_UB;
        out += lc;
        out += TITLE_RIGHT_DOWN;
    }
    out += reset;
    out += &mv_to(y + 1, x + 1);
    out
}

// ── banner ────────────────────────────────────────────────────────────────

/// Banner source lines `(hex, art)`, transcribed from `Global::Banner_src`
/// (src/btop.cpp:89-96).
///
/// WARNING: hand copy — bump with btop.cpp `Banner_src` (:89-96).
pub const BANNER_SRC: &[(&str, &str)] = &[
    ("#E62525", "██████╗ ████████╗ ██████╗ ██████╗"),
    ("#CD2121", "██╔══██╗╚══██╔══╝██╔═══██╗██╔══██╗   ██╗    ██╗"),
    (
        "#B31D1D",
        "██████╔╝   ██║   ██║   ██║██████╔╝ ██████╗██████╗",
    ),
    (
        "#9A1919",
        "██╔══██╗   ██║   ██║   ██║██╔═══╝  ╚═██╔═╝╚═██╔═╝",
    ),
    ("#801414", "██████╔╝   ██║   ╚██████╔╝██║        ╚═╝    ╚═╝"),
    ("#000000", "╚═════╝    ╚═╝    ╚═════╝ ╚═╝"),
];

/// btop version, transcribed from `Global::Version` (src/btop.cpp:97).
///
/// WARNING: hand copy — bump with btop.cpp `Version` (:97).
pub const BTOP_VERSION: &str = "1.4.7";

/// Unicode scalar count = C++ `ulen(s)` (btop_tools.hpp:177-179 counts
/// non-continuation bytes). Banner art is single-width so `wide` is moot.
fn ulen(s: &str) -> usize {
    s.chars().count()
}

/// Build the banner body over `src`, returning `(body, width)`. Ports the
/// `banner.empty()` fill (:142-174): tty takes fixed red/grey pairs,
/// otherwise `hex_to_color(line hex)` + a `(120 - z*12)` grey ramp;
/// spaces in the art become cursor-right skips; rows join with
/// `Mv::l(ulen) + Mv::d(1)`; tail is the version stamp.
pub fn build_banner(
    src: &[(&str, &str)],
    tty_mode: bool,
    lowcolor: bool,
    main_fg: &str,
    reset: &str,
    version: &str,
) -> (String, usize) {
    let mut banner = String::new();
    let mut width = 0usize;
    let mut oc = String::new();
    for (z, (hex, art)) in src.iter().enumerate() {
        let w = ulen(art);
        if w > width {
            width = w;
        }
        let (fg, bg) = if tty_mode {
            (
                if z > 2 { "\x1b[31m" } else { "\x1b[91m" }.to_string(),
                if z > 2 { "\x1b[90m" } else { "\x1b[37m" }.to_string(),
            )
        } else {
            let fg = hex_to_color(hex, lowcolor, "fg");
            let g = 120 - z as i32 * 12;
            // dec_to_color clamps in C++; 120-5*12=60 stays in range.
            let bg = dec_to_color(g as u8, g as u8, g as u8, lowcolor, "fg");
            (fg, bg)
        };
        // Step through the art 3 bytes at a time (block glyphs are
        // 3-byte UTF-8); a 1-byte space becomes Mv::r(1) (`i -= 2`
        // cancels two of the three stepped bytes).
        let bytes = art.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            let letter = if bytes[i] == b' ' {
                i += 1;
                mv_r(1)
            } else {
                let s = art.get(i..i + 3).unwrap_or("").to_string();
                i += 3;
                s
            };
            let b_color = if letter == "█" { &fg } else { &bg };
            if *b_color != oc {
                banner += b_color;
            }
            banner += &letter;
            oc = b_color.clone();
        }
        if z + 1 < src.len() {
            banner += &mv_l(w as i64);
            banner += &mv_d(1);
        }
    }
    banner += &mv_r(18 - version.len() as i64);
    banner += main_fg;
    banner += FX_B;
    banner += FX_I;
    banner += "v";
    banner += version;
    banner += reset;
    (banner, width)
}

/// btop++ banner, porting `Draw::banner_gen` (src/btop_draw.cpp:138-177)
/// minus the `static` cache (callers re-invoke on redraw): position with
/// `Mv::to`, centered on `term_width` when asked. `lowcolor` is Config
/// `lowcolor`, `main_fg`/`reset` are `Theme::c("main_fg")` / `Fx::reset`.
// Param count mirrors the C++ signature's globals (Config x2 + Term + Theme).
#[allow(clippy::too_many_arguments)]
pub fn banner_gen(
    y: i64,
    x: i64,
    centered: bool,
    term_width: i64,
    tty_mode: bool,
    lowcolor: bool,
    main_fg: &str,
    reset: &str,
) -> String {
    let (banner, width) =
        build_banner(BANNER_SRC, tty_mode, lowcolor, main_fg, reset, BTOP_VERSION);
    let pos = if centered {
        mv_to(y, term_width / 2 - width as i64 / 2)
    } else {
        mv_to(y, x)
    };
    pos + &banner
}

// ── clock ─────────────────────────────────────────────────────────────────

/// Caller-built wall time. Time acquisition (`time()` + `strf_time` +
/// `/uptime` expansion, :333-381) is M4; this struct carries the finished
/// `clock_str` split in two for future format flexibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clock {
    pub time: String,
    pub date: String,
}

impl Clock {
    /// Display string. M4-owned choice: date first (`%F %T` style),
    /// whichever part is present when only one is.
    pub fn display(&self) -> String {
        match (self.date.is_empty(), self.time.is_empty()) {
            (true, _) => self.time.clone(),
            (_, true) => self.date.clone(),
            (false, false) => format!("{} {}", self.date, self.time),
        }
    }
}

/// Clock-box content, porting the render tail of `Draw::update_clock`
/// (src/btop_draw.cpp:382-392).
///
/// `prev_len` is the previous clock byte length (`clock_len` static);
/// returns `(out, new_len)`. `battery_cols` folds
/// `Term::width >= 100 && show_battery && has_battery ? 22 : 0`;
/// `cpu_bottom` selects the title glyphs; `resized` suppresses the
/// old-clock erase. Lengths are BYTES (`string::size`), as in C++.
// Param count mirrors the C++ statics/Config reads made explicit.
#[allow(clippy::too_many_arguments)]
pub fn render_clock(
    clock: &Clock,
    prev_len: usize,
    x: i64,
    y: i64,
    width: i64,
    cpu_bottom: bool,
    resized: bool,
    battery_cols: i64,
    cpu_box_color: &str,
    title_color: &str,
) -> (String, usize) {
    let s = uresize(
        &clock.display(),
        (10.max(width - 66 - battery_cols)) as usize,
        false,
    );
    let mut out = String::new();
    if s.len() != prev_len && !resized && prev_len > 0 {
        out += &mv_to(y, x + width / 2 - prev_len as i64 / 2);
        out += FX_UB;
        out += cpu_box_color;
        out += &H_LINE.repeat(prev_len);
    }
    let len = s.len();
    let (title_left, title_right) = if cpu_bottom {
        (TITLE_LEFT_DOWN, TITLE_RIGHT_DOWN)
    } else {
        (TITLE_LEFT, TITLE_RIGHT)
    };
    out += &mv_to(y, x + width / 2 - len as i64 / 2);
    out += FX_UB;
    out += cpu_box_color;
    out += title_left;
    out += title_color;
    out += FX_B;
    out += &s;
    out += cpu_box_color;
    out += FX_UB;
    out += title_right;
    (out, len)
}

// ── calcSizes ─────────────────────────────────────────────────────────────

/// Geometry of one box: position + size (`x`, `y`, `width`, `height`
/// statics per box namespace) and whether it is shown. Hidden boxes keep
/// the C++ reset state (`x = y = 1`, zero size).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxGeom {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
    pub shown: bool,
}

/// One visible GPU panel input: the `shown_panels` index plus its
/// `gpu_b_height_offsets` entry. `panel` shapes only the title string
/// (caller concern); it is kept so the input mirrors `shown_panels` 1:1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuPanel {
    pub panel: i64,
    pub b_offset: i64,
}

/// Every global `calcSizes` reads, made explicit (grouped per the GraphOpts
/// precedent — the raw function reads 20+ globals):
/// - `term_w`/`term_h`: `Term::width`/`height`.
/// - `shown_boxes`: Config `shown_boxes` (raw string; `contains` checks
///   run on it exactly like C++).
/// - `cpu_bottom`, `mem_below_net`, `proc_left`: Config layout flags.
/// - `core_count`: `Shared::coreCount`.
/// - `show_temp`: `check_temp && got_sensors`.
/// - `cpu_width_p`/`cpu_height_p` (100/32, btop_draw.cpp:546),
///   `mem_width_p` (45, :1223), `net_height_p` (28, :1489),
///   `gpu_height_p` (32, :1031), `gpu_min_height`/`gpu_min_width`
///   (8/41, :1032): per-namespace percent/min constants.
/// - `show_disks`, `swap_disk`, `mem_graphs`, `has_swap`
///   (`Mem::has_swap`): mem section Config/state.
/// - `swap_upload_download`: net inner-box title order.
/// - `gpus_extra_height`: `show_gpu_info` × `Gpu::count`/`shown`
///   (`"On"` → count, `"Auto"` → count − shown, else 0).
/// - `gpu_total_height`: pre-pass `Gpu::total_height` (0 with no panels).
/// - `gpu_panels`: parsed `gpuN` words (`shown_panels` + offsets).
///
/// Title-only inputs (`custom_cpu_name`, `cpuName`, `cpuHz`, gpu names)
/// are excluded: they shape title strings, not geometry.
#[derive(Debug, Clone)]
pub struct LayoutInput {
    pub term_w: i64,
    pub term_h: i64,
    pub shown_boxes: String,
    pub cpu_bottom: bool,
    pub mem_below_net: bool,
    pub proc_left: bool,
    pub core_count: i64,
    pub show_temp: bool,
    pub cpu_width_p: i64,
    pub cpu_height_p: i64,
    pub mem_width_p: i64,
    pub net_height_p: i64,
    pub show_disks: bool,
    pub swap_disk: bool,
    pub mem_graphs: bool,
    pub has_swap: bool,
    pub swap_upload_download: bool,
    pub gpus_extra_height: i64,
    pub gpu_total_height: i64,
    pub gpu_panels: Vec<GpuPanel>,
    pub gpu_height_p: i64,
    pub gpu_min_height: i64,
    pub gpu_min_width: i64,
}

impl LayoutInput {
    /// Standard harness defaults for `term_w` × `term_h`: the percent/min
    /// constants (`cpu_width_p` 100 / `cpu_height_p` 32, btop_draw.cpp:546;
    /// `mem_width_p` 45, :1223; `net_height_p` 28, :1489; `gpu_height_p`
    /// 32, `gpu_min_height` 8 / `gpu_min_width` 41, :1031-1032) plus the
    /// S0 harness flags (all boxes shown, `core_count` 8, `show_temp`
    /// true, `show_disks`/`mem_graphs`/`has_swap` true, the rest false /
    /// zero / empty). Callers override the fields that differ.
    pub fn defaults(term_w: usize, term_h: usize) -> Self {
        Self {
            term_w: term_w as i64,
            term_h: term_h as i64,
            shown_boxes: "cpu mem net proc".to_string(),
            cpu_bottom: false,
            mem_below_net: false,
            proc_left: false,
            core_count: 8,
            show_temp: true,
            cpu_width_p: 100,
            cpu_height_p: 32,
            mem_width_p: 45,
            net_height_p: 28,
            show_disks: true,
            swap_disk: false,
            mem_graphs: true,
            has_swap: true,
            swap_upload_download: false,
            gpus_extra_height: 0,
            gpu_total_height: 0,
            gpu_panels: vec![],
            gpu_height_p: 32,
            gpu_min_height: 8,
            gpu_min_width: 41,
        }
    }
}

/// CPU box geometry + inner stats box (`b_*`, :2332-2361).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuGeom {
    pub base: BoxGeom,
    pub b_columns: i64,
    pub b_column_size: i64,
    pub b_x: i64,
    pub b_y: i64,
    pub b_width: i64,
    pub b_height: i64,
}

/// Mem box geometry + meter/graph splits (:2455-2485).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemGeom {
    pub base: BoxGeom,
    pub mem_width: i64,
    pub disks_width: i64,
    pub divider: i64,
    pub item_height: i64,
    pub mem_size: i64,
    pub mem_meter: i64,
    pub graph_height: i64,
    pub disk_meter: i64,
}

/// Net box geometry + inner stats box + up/down graph heights (:2517-2522).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetGeom {
    pub base: BoxGeom,
    pub b_x: i64,
    pub b_y: i64,
    pub b_width: i64,
    pub b_height: i64,
    pub d_graph_height: i64,
    pub u_graph_height: i64,
}

/// Proc box geometry + visible-row bound (:2547).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcGeom {
    pub base: BoxGeom,
    pub select_max: i64,
}

/// One GPU panel's outer + inner stats boxes (:2395-2426).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuGeom {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
    pub b_x: i64,
    pub b_y: i64,
    pub b_width: i64,
    /// Inner stats box height (`b_offset + 2`).
    pub b_inner_h: i64,
    /// Full inner height reused by `Gpu::draw` (`height - 2`).
    pub b_full_h: i64,
}

/// Full layout. Mirrors the GPU_SUPPORT build (this repo builds with it:
/// CMakeLists.txt:165,206); the non-GPU `#else` height branches are not
/// ported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub cpu: CpuGeom,
    pub mem: MemGeom,
    pub net: NetGeom,
    pub proc: ProcGeom,
    pub gpu_panels: Vec<GpuGeom>,
    pub gpu_total_height: i64,
}

fn hidden() -> BoxGeom {
    BoxGeom {
        x: 1,
        y: 1,
        width: 0,
        height: 0,
        shown: false,
    }
}

// Float rounding for the layout helpers below: `round`/`ceil`/`floor`
// mirror C++ half-away-from-zero semantics (Rust `f64::round` matches);
// the mem height's inner term is integer math.

/// CPU box geometry + inner stats box (`b_*`, :2332-2361).
///
/// Reads `term_w`, `term_h`, `shown_boxes`, `cpu_bottom`, `core_count`,
/// `show_temp`, `cpu_width_p`, `cpu_height_p`, `gpus_extra_height`,
/// `gpu_total_height` (pre-pass minimum) and `gpu_panels` (length only).
pub fn cpu_geom(input: &LayoutInput) -> CpuGeom {
    let boxes = &input.shown_boxes;
    let cpu_shown = boxes.contains("cpu");
    let mem_shown = boxes.contains("mem");
    let net_shown = boxes.contains("net");
    let proc_shown = boxes.contains("proc");
    let gpu_shown = input.gpu_panels.len() as i64;
    if !cpu_shown {
        return CpuGeom {
            base: hidden(),
            b_columns: 0,
            b_column_size: 0,
            b_x: 0,
            b_y: 0,
            b_width: 0,
            b_height: 0,
        };
    }
    let width = ((input.term_w as f64) * (input.cpu_width_p as f64) / 100.0).round() as i64;
    // btop_draw.cpp:2319-2320 special: GPU shown while mem/net/proc are
    // hidden fills the remaining rows instead of the percent formula.
    let mut height = if gpu_shown != 0 && !(mem_shown || net_shown || proc_shown) {
        input.term_h - input.gpu_total_height - input.gpus_extra_height
    } else {
        let pct = if boxes.trim() == "cpu" {
            100
        } else {
            // Integer division: height_p / (shown + 1).
            input.cpu_height_p / (gpu_shown + 1) + if gpu_shown != 0 { 5 } else { 0 }
        };
        (((input.term_h as f64) * (pct as f64) / 100.0).ceil() as i64).max(8)
    };
    if height <= input.term_h - input.gpus_extra_height {
        height += input.gpus_extra_height;
    }
    let x = 1;
    let y = if input.cpu_bottom {
        input.term_h - height + 1
    } else {
        1
    };

    let b_columns = 2.max(
        (((input.core_count + 1) as f64) / ((height - input.gpus_extra_height - 5) as f64)).ceil()
            as i64,
    );
    let show_temp = input.show_temp;
    // b_column_size 2/1/0 cascade (:2336-2352); the trailing
    // `b_column_size == 0` fill covers both the third branch and the
    // recompute-`b_columns` else branch.
    let t = show_temp as i64;
    let limit = width - width / 3;
    let (b_columns, b_column_size, b_width) = if b_columns * (21 + 12 * t) < limit {
        (
            b_columns,
            2,
            29.max((21 + 12 * t) * b_columns - (b_columns - 1)),
        )
    } else if b_columns * (15 + 6 * t) < limit {
        (b_columns, 1, (15 + 6 * t) * b_columns - (b_columns - 1))
    } else if b_columns * (8 + 6 * t) < limit {
        (b_columns, 0, (8 + 6 * t) * b_columns + 1)
    } else {
        let bc = limit / (8 + 6 * t);
        (bc, 0, (8 + 6 * t) * bc + 1)
    };
    let b_height = (height - 2).min(
        ((input.core_count as f64) / (b_columns as f64)).ceil() as i64
            + 4
            + input.gpus_extra_height,
    );
    let b_x = x + width - b_width - 1;
    let b_y =
        y + ((height - 2) as f64 / 2.0).ceil() as i64 - (b_height as f64 / 2.0).ceil() as i64 + 1;
    CpuGeom {
        base: BoxGeom {
            x,
            y,
            width,
            height,
            shown: true,
        },
        b_columns,
        b_column_size,
        b_x,
        b_y,
        b_width,
        b_height,
    }
}

/// GPU panels' outer + inner stats boxes (:2381-2428). Returns the panels
/// plus the summed total height (the `Gpu::total_height` overwrite).
///
/// Reads `term_w`, `term_h`, `shown_boxes`, `cpu_bottom`, `gpu_panels`,
/// `gpu_height_p`, `gpu_min_height`, `gpu_min_width`. `cpu_h` is the
/// computed CPU box height (0 when hidden).
pub fn gpu_geoms(input: &LayoutInput, cpu_h: i64) -> (Vec<GpuGeom>, i64) {
    let boxes = &input.shown_boxes;
    let cpu_shown = boxes.contains("cpu");
    let mem_shown = boxes.contains("mem");
    let net_shown = boxes.contains("net");
    let proc_shown = boxes.contains("proc");
    let gpu_shown = input.gpu_panels.len() as i64;
    let mut gpu_panels = Vec::new();
    let mut gpu_total = 0i64;
    if gpu_shown != 0 {
        for (i, panel) in input.gpu_panels.iter().enumerate() {
            let mut height: i64;
            let width = input.term_w;
            if cpu_shown {
                if !(mem_shown || net_shown || proc_shown) {
                    height = input.gpu_min_height;
                } else {
                    height = cpu_h;
                }
            } else if !(mem_shown || net_shown || proc_shown) {
                height = (input.term_h - gpu_total) / (gpu_shown - i as i64)
                    + if i == 0 {
                        (input.term_h - gpu_total) % (gpu_shown - i as i64)
                    } else {
                        0
                    };
            } else {
                height = (((input.term_h as f64) * (input.gpu_height_p as f64)
                    / (gpu_shown as f64)
                    / 100.0)
                    .ceil() as i64)
                    .max(input.gpu_min_height);
            }
            let b_inner_h = panel.b_offset + 2;
            if height + cpu_h == input.term_h - 1 {
                height += 1;
            }
            height = height.max(b_inner_h + 2);
            let x = 1;
            let y = 1 + gpu_total + (!input.cpu_bottom) as i64 * (cpu_shown as i64) * cpu_h;
            let b_width = (width / 2).clamp(input.gpu_min_width, 65);
            gpu_total += height;
            let b_x = x + width - b_width - 1;
            let b_y = y + (((height - 2 - b_inner_h) as f64) / 2.0).ceil() as i64 + 1;
            gpu_panels.push(GpuGeom {
                x,
                y,
                width,
                height,
                b_x,
                b_y,
                b_width,
                b_inner_h,
                b_full_h: height - 2,
            });
        }
    }
    (gpu_panels, gpu_total)
}

/// Mem box geometry + meter/graph splits (:2455-2485).
///
/// Reads `term_w`, `term_h`, `shown_boxes`, `cpu_bottom`, `mem_below_net`,
/// `proc_left`, `mem_width_p`, `net_height_p`, `show_disks`, `swap_disk`,
/// `mem_graphs`, `has_swap`. `cpu_h`/`gpu_total_height` are the computed
/// CPU height and GPU total height.
pub fn mem_geom(input: &LayoutInput, cpu_h: i64, gpu_total_height: i64) -> MemGeom {
    let boxes = &input.shown_boxes;
    let cpu_shown = boxes.contains("cpu");
    let mem_shown = boxes.contains("mem");
    let net_shown = boxes.contains("net");
    let proc_shown = boxes.contains("proc");
    let gpu_shown = input.gpu_panels.len() as i64;
    if !mem_shown {
        return MemGeom {
            base: hidden(),
            mem_width: 0,
            disks_width: 0,
            divider: 0,
            item_height: 0,
            mem_size: 0,
            mem_meter: 0,
            graph_height: 0,
            disk_meter: 0,
        };
    }
    let width = ((input.term_w as f64)
        * ((if proc_shown { input.mem_width_p } else { 100 }) as f64)
        / 100.0)
        .round() as i64;
    // Inner term is integer math (:2439).
    let net_term =
        input.net_height_p * (net_shown as i64) * 4 / ((gpu_shown != 0 && cpu_shown) as i64 + 4);
    let height = ((input.term_h as f64) * ((100 - net_term) as f64) / 100.0).floor() as i64
        - cpu_h
        - gpu_total_height;
    let x = if input.proc_left && proc_shown {
        input.term_w - width + 1
    } else {
        1
    };
    let y = if input.mem_below_net && net_shown {
        input.term_h - height + 1 - if input.cpu_bottom { cpu_h } else { 0 }
    } else {
        (if input.cpu_bottom { 1 } else { cpu_h + 1 }) + gpu_total_height
    };
    let (mem_width, disks_width, divider) = if input.show_disks {
        let mut mw = (((width - 3) as f64) / 2.0).ceil() as i64;
        mw += mw % 2;
        (mw, width - mw - 2, x + mw)
    } else {
        // C++ leaves `divider` stale here; the pure version reports `x`.
        (width - 1, 0, x)
    };
    let swap_block = input.has_swap && !input.swap_disk;
    let item_height = if swap_block { 6 } else { 4 };
    let mem_size = if height - (if swap_block { 3 } else { 2 }) > 2 * item_height {
        3
    } else if mem_width > 25 {
        2
    } else {
        1
    };
    let mut mem_meter = 0.max(mem_width - if mem_size > 2 { 7 } else { 17 });
    if mem_size == 1 {
        mem_meter += 6;
    }
    let mut graph_height = 0;
    if input.mem_graphs {
        graph_height = 1.max(
            ((((height - (if swap_block { 2 } else { 1 }))
                - (if mem_size == 3 { 2 } else { 1 }) * item_height) as f64)
                / (item_height as f64))
                .round() as i64,
        );
        if graph_height > 1 {
            mem_meter += 6;
        }
    }
    let mut disk_meter = 0;
    if input.show_disks {
        disk_meter = (-14).max(width - mem_width - 23);
        if disks_width < 25 {
            disk_meter += 14;
        }
    }
    MemGeom {
        base: BoxGeom {
            x,
            y,
            width,
            height,
            shown: true,
        },
        mem_width,
        disks_width,
        divider,
        item_height,
        mem_size,
        mem_meter,
        graph_height,
        disk_meter,
    }
}

/// Net box geometry + inner stats box + up/down graph heights (:2517-2522).
///
/// Reads `term_w`, `term_h`, `shown_boxes`, `cpu_bottom`, `mem_below_net`,
/// `proc_left`, `mem_width_p`, `swap_upload_download` (title order only,
/// no geometry). `cpu_h`/`gpu_total_height`/`mem_h` are computed heights.
pub fn net_geom(input: &LayoutInput, cpu_h: i64, gpu_total_height: i64, mem_h: i64) -> NetGeom {
    let boxes = &input.shown_boxes;
    let net_shown = boxes.contains("net");
    let mem_shown = boxes.contains("mem");
    let proc_shown = boxes.contains("proc");
    if !net_shown {
        return NetGeom {
            base: hidden(),
            b_x: 0,
            b_y: 0,
            b_width: 0,
            b_height: 0,
            d_graph_height: 0,
            u_graph_height: 0,
        };
    }
    let width = ((input.term_w as f64)
        * ((if proc_shown { input.mem_width_p } else { 100 }) as f64)
        / 100.0)
        .round() as i64;
    let height = input.term_h - cpu_h - gpu_total_height - mem_h;
    let x = if input.proc_left && proc_shown {
        input.term_w - width + 1
    } else {
        1
    };
    let y = if input.mem_below_net && mem_shown {
        (if input.cpu_bottom { 1 } else { cpu_h + 1 }) + gpu_total_height
    } else {
        input.term_h - height + 1 - if input.cpu_bottom { cpu_h } else { 0 }
    };
    let b_width = if width > 45 { 27 } else { 19 };
    let b_height = if height > 10 { 9 } else { height - 2 };
    let b_x = x + width - b_width - 1;
    // Integer divisions, as in C++.
    let b_y = y + (height - 2) / 2 - b_height / 2 + 1;
    let d_graph_height = (((height - 2) as f64) / 2.0).round() as i64;
    let u_graph_height = height - 2 - d_graph_height;
    let _ = input.swap_upload_download; // title order only, no geometry
    NetGeom {
        base: BoxGeom {
            x,
            y,
            width,
            height,
            shown: true,
        },
        b_x,
        b_y,
        b_width,
        b_height,
        d_graph_height,
        u_graph_height,
    }
}

/// Proc box geometry + visible-row bound (:2547).
///
/// Reads `term_w`, `term_h`, `shown_boxes`, `cpu_bottom`, `proc_left`.
/// `cpu_h`/`gpu_total_height` are computed heights; `mem_w`/`net_w` are
/// the computed mem/net widths (used when those boxes are shown).
pub fn proc_geom(
    input: &LayoutInput,
    cpu_h: i64,
    gpu_total_height: i64,
    mem_w: i64,
    net_w: i64,
) -> ProcGeom {
    let boxes = &input.shown_boxes;
    let cpu_shown = boxes.contains("cpu");
    let mem_shown = boxes.contains("mem");
    let net_shown = boxes.contains("net");
    let proc_shown = boxes.contains("proc");
    if !proc_shown {
        return ProcGeom {
            base: hidden(),
            select_max: 0,
        };
    }
    let width = input.term_w
        - if mem_shown {
            mem_w
        } else if net_shown {
            net_w
        } else {
            0
        };
    let height = input.term_h - cpu_h - gpu_total_height;
    let x = if input.proc_left {
        1
    } else {
        input.term_w - width + 1
    };
    let y = if input.cpu_bottom && cpu_shown {
        1
    } else {
        cpu_h + 1
    };
    ProcGeom {
        base: BoxGeom {
            x,
            y,
            width,
            height,
            shown: true,
        },
        select_max: height - 3,
    }
}

/// Pure transcription of `Draw::calcSizes` geometry (src/btop_draw.cpp:
/// 2248-2551). Side effects (clearing `box` strings, `Global::clock`,
/// mouse mappings, `redraw` flags, `Proc::p_graphs` resets) are caller
/// concerns; this returns geometry only. Orchestration calling
/// [`cpu_geom`], [`gpu_geoms`], [`mem_geom`], [`net_geom`], [`proc_geom`]
/// in C++ order (cpu → gpu → mem → net → proc); later boxes reuse the
/// computed heights/widths exactly as the C++ statics do.
pub fn calc_sizes(input: &LayoutInput) -> Layout {
    let cpu = cpu_geom(input);
    let cpu_h = cpu.base.height;

    let (gpu_panels, gpu_computed_total) = gpu_geoms(input, cpu_h);
    let gpu_total_height = if input.gpu_panels.is_empty() {
        input.gpu_total_height
    } else {
        gpu_computed_total
    };

    let mem = mem_geom(input, cpu_h, gpu_total_height);
    let (mem_w, mem_h) = (mem.base.width, mem.base.height);

    let net = net_geom(input, cpu_h, gpu_total_height, mem_h);
    let (net_w, _net_h) = (net.base.width, net.base.height);

    let proc = proc_geom(input, cpu_h, gpu_total_height, mem_w, net_w);

    Layout {
        cpu,
        mem,
        net,
        proc,
        gpu_panels,
        gpu_total_height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESET: &str = "\x1b[0m";
    const LC: &str = "L";
    const HI: &str = "H";
    const TI: &str = "T";

    #[test]
    fn small_box_exact_string() {
        // Hand-derived from the formula: horizontals at y=1,3 (width-1=4
        // dashes), one filled row, round corners (rounded=true).
        assert_eq!(
            create_box(1, 1, 5, 3, LC, false, "", "", 0, "D", HI, TI, RESET, false, true),
            format!(
                "{RESET}{LC}\x1b[1;1f────\x1b[3;1f────\
                 \x1b[2;1f│\x1b[3C│\
                 \x1b[1;1f╭\x1b[1;5f╮\x1b[3;1f╰\x1b[3;5f╯\
                 {RESET}\x1b[2;2f"
            )
        );
    }

    #[test]
    fn box_fill_titles_numbering_tty() {
        // tty_mode: square corners, decimal numbering, fill spaces.
        let out = create_box(
            2, 4, 6, 4, "", true, "ab", "cd", 7, "D", HI, TI, RESET, true, true,
        );
        assert!(out.starts_with(&format!("{RESET}D")));
        assert!(out.contains(&format!("\x1b[4;2f{HLINE}", HLINE = H_LINE.repeat(5))));
        assert!(out.contains(&format!("│{}│", " ".repeat(4))));
        assert!(out.contains("┌")); // square, not round
        assert!(out.contains(&format!("{HI}7{TI}ab")));
        assert!(out.contains(&format!("{HI}7{TI}cd")));
        assert!(out.ends_with("\x1b[5;3f"));
    }

    #[test]
    fn box_superscript_numbering() {
        let out = create_box(
            1, 1, 6, 3, LC, false, "t", "", 3, "D", HI, TI, RESET, false, false,
        );
        assert!(out.contains(&format!("{HI}³{TI}t")));
    }

    #[test]
    fn banner_single_line_exact_string() {
        // One-line src: "█" takes fg, the space becomes Mv::r(1) in the
        // grey-ramp bg (dec_to_color default depth is "fg":
        // btop_theme.hpp). Colors re-emit on change, like C++.
        let (body, width) = build_banner(&[("#ff0000", "█ █")], false, false, "FG", RESET, "1.4.7");
        assert_eq!(width, 3);
        assert_eq!(
            body,
            format!(
                "\x1b[38;2;255;0;0m█\x1b[38;2;120;120;120m\x1b[1C\
                 \x1b[38;2;255;0;0m█\x1b[13CFG\x1b[1m\x1b[3mv1.4.7{RESET}"
            )
        );
    }

    #[test]
    fn banner_tty_colors_and_centering() {
        let (body, _) = build_banner(&[("#ff0000", "█░")], true, false, "FG", RESET, "1.4.7");
        assert!(body.starts_with("\x1b[91m█\x1b[37m░"));
        let out = banner_gen(2, 3, true, 100, false, false, "FG", RESET);
        // Widest real banner line is 49 cells (rows 3-4) → x = 50 - 49/2 = 26.
        assert!(out.starts_with("\x1b[2;26f"));
        assert!(out.contains("v1.4.7"));
        let out2 = banner_gen(2, 3, false, 100, false, false, "FG", RESET);
        assert!(out2.starts_with("\x1b[2;3f"));
    }

    #[test]
    fn clock_renders_title_box_content() {
        // First draw (prev 0): no erase, single positioned title run.
        let c = Clock {
            time: "12:00".to_string(),
            date: String::new(),
        };
        let (out, len) = render_clock(&c, 0, 1, 1, 100, false, false, 0, "CB", "TI");
        assert_eq!(len, 5);
        assert_eq!(
            out,
            format!("\x1b[1;49f\x1b[22mCB┐TI\x1b[1m12:00CB\x1b[22m┌")
        );
        // Changed length without resize: erase old run first.
        let (out2, len2) = render_clock(&c, 8, 1, 1, 100, false, false, 0, "CB", "TI");
        assert_eq!(len2, 5);
        assert!(out2.starts_with("\x1b[1;47f\x1b[22mCB──────"));
        // cpu_bottom flips the title glyphs.
        let (out3, _) = render_clock(&c, 5, 1, 1, 100, true, false, 0, "CB", "TI");
        assert!(out3.contains("┘") && out3.contains("└"));
    }

    /// Harness S0 setup: 100x30, "cpu mem net proc", cpu_bottom=false,
    /// mem_below_net=false, proc_left=false, coreCount=8,
    /// show_temp (check_temp && got_sensors)=true, show_disks=true,
    /// swap_disk=false, mem_graphs=true, has_swap=true, GPU build with no
    /// panels. All values below hand-derived from the C++ math.
    fn s0_input() -> LayoutInput {
        LayoutInput::defaults(100, 30)
    }

    #[test]
    fn create_box_degenerate_returns_empty() {
        // Defensive guard: layout guarantees positivity; non-positive
        // geometry returns empty instead of underflowing repeats.
        for (w, h) in [(0, 5), (5, 0), (0, 0), (-3, 4), (4, -2)] {
            assert_eq!(
                create_box(1, 1, w, h, LC, false, "", "", 0, "D", HI, TI, RESET, false, true),
                String::new(),
                "w={w} h={h}"
            );
        }
    }

    #[test]
    fn cpu_gpu_only_height_fills_remaining_rows() {
        // btop_draw.cpp:2319-2320 special branch: GPU shown while
        // mem/net/proc are hidden → height = term_h - pre-pass
        // gpu_total - gpus_extra (no percent formula, no max(8)).
        // Pre-pass total = 4 + b_offset (7) = 11 per panel.
        let mut input = s0_input();
        input.shown_boxes = "cpu gpu0".to_string();
        input.gpu_panels = vec![GpuPanel {
            panel: 0,
            b_offset: 7,
        }];
        input.gpu_total_height = 11;
        let cpu = cpu_geom(&input);
        // 30 - 11 - 0 = 19 (percent path would give ceil(30*21/100)=7→8).
        assert_eq!(cpu.base.height, 19);
        assert_eq!(cpu.base.width, 100);
    }

    #[test]
    fn s0_all_boxes_geometry() {
        let l = calc_sizes(&s0_input());
        // cpu: w=round(100*100/100)=100, h=max(8,ceil(30*32/100))=10.
        // b_columns=max(2,ceil(9/5))=2; 2*33=66 < 67 → size 2,
        // b_width=max(29,65)=65, b_height=min(8,4+4)=8,
        // b_x=35, b_y=1+4-4+1=2.
        assert_eq!(
            l.cpu,
            CpuGeom {
                base: BoxGeom {
                    x: 1,
                    y: 1,
                    width: 100,
                    height: 10,
                    shown: true
                },
                b_columns: 2,
                b_column_size: 2,
                b_x: 35,
                b_y: 2,
                b_width: 65,
                b_height: 8,
            }
        );
        // mem: w=round(100*45/100)=45,
        // h=floor(30*(100-28*4/4)/100)-10=21-10=11, x=1, y=10+1=11.
        // mem_width=ceil(42/2)=21→22 (even), disks=21, divider=23,
        // item=6, size=1 (8≯12, 22≯25), meter=22-17+6=11,
        // graph=max(1,round(3/6))=1, disk=max(-14,0)+14=14.
        assert_eq!(
            l.mem,
            MemGeom {
                base: BoxGeom {
                    x: 1,
                    y: 11,
                    width: 45,
                    height: 11,
                    shown: true
                },
                mem_width: 22,
                disks_width: 21,
                divider: 23,
                item_height: 6,
                mem_size: 1,
                mem_meter: 11,
                graph_height: 1,
                disk_meter: 14,
            }
        );
        // net: w=45, h=30-10-0-11=9, x=1, y=30-9+1=22.
        // b: 19 wide, 7 high, b_x=26, b_y=22+3-3+1=23,
        // d=round(7/2)=4, u=9-2-4=3.
        assert_eq!(
            l.net,
            NetGeom {
                base: BoxGeom {
                    x: 1,
                    y: 22,
                    width: 45,
                    height: 9,
                    shown: true
                },
                b_x: 26,
                b_y: 23,
                b_width: 19,
                b_height: 7,
                d_graph_height: 4,
                u_graph_height: 3,
            }
        );
        // proc: w=100-45=55, h=30-10=20, x=46, y=11, select=17.
        assert_eq!(
            l.proc,
            ProcGeom {
                base: BoxGeom {
                    x: 46,
                    y: 11,
                    width: 55,
                    height: 20,
                    shown: true
                },
                select_max: 17,
            }
        );
        assert!(l.gpu_panels.is_empty());
    }

    #[test]
    fn hidden_boxes_keep_reset_state() {
        let mut input = s0_input();
        input.shown_boxes = "cpu".to_string();
        let l = calc_sizes(&input);
        assert!(l.cpu.base.shown);
        // trim=="cpu" → full-height cpu box.
        assert_eq!(l.cpu.base.height, 30);
        assert!(!l.mem.base.shown);
        assert_eq!(l.mem.base, hidden());
        // No proc → mem/net span full width (proc uses mem width).
        assert_eq!(l.proc.base, hidden());
    }

    #[test]
    fn gpu_panel_geometry() {
        // Harness gpu pass: 100x30, "gpu0", offset 7, no other boxes.
        let mut input = s0_input();
        input.shown_boxes = "gpu0".to_string();
        input.gpu_panels = vec![GpuPanel {
            panel: 0,
            b_offset: 7,
        }];
        let l = calc_sizes(&input);
        assert!(!l.cpu.base.shown);
        assert_eq!(l.gpu_panels.len(), 1);
        // height=(30-0)/1=30, inner=9, b_width=clamp(50,41,65)=50,
        // b_x=50, b_y=1+ceil(19/2)+1=12, full=28, total=30.
        assert_eq!(
            l.gpu_panels[0],
            GpuGeom {
                x: 1,
                y: 1,
                width: 100,
                height: 30,
                b_x: 50,
                b_y: 12,
                b_width: 50,
                b_inner_h: 9,
                b_full_h: 28,
            }
        );
        assert_eq!(l.gpu_total_height, 30);
    }
}
