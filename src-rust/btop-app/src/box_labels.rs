//! Box labels, outline pre-print, and the terminal-too-small loop (P4 T6).
//!
//! C++ truth:
//! - outlines: `Draw::calcSizes` renders each box frame with
//!   `Draw::createBox` (src/btop_draw.cpp:2363 cpu, :2487 mem, :2524 net,
//!   :2548 proc, :2413 gpu) and caches the strings in `Cpu::box` etc.
//!   (`extern string box`, src/btop_shared.hpp:209/266/313/365); the boot
//!   tail prints them verbatim (src/btop.cpp:1093-1097).
//! - min-size loop: `term_resize` tail (src/btop.cpp:154-209) — the
//!   "Terminal size too small" screen (:163-180), `Input::poll(10)` between
//!   renders (:187), `q` → `clean_quit(0)` (:190), digit → `toggle_box`
//!   (:191-199), recompute min size (:200-202).
//! - `Config::toggle_box` (src/btop_config.cpp:739-761) lives on
//!   [`btop_config::Config`] (transcribed there, not duplicated here).
//!
//! NOTE on label strings: the brief cites `Cpu::box = "CPU: "` at
//! "cpp:88-91", but no such assignment exists in the C++ tree (verified by
//! grepping `src/` + `osx/` for `::box =` and `"CPU` — the only hits are
//! the lowercase outline titles in `btop_draw.cpp` and `"CPU:"` menu labels
//! in `src/btop_menu.cpp:430,453`). The outline titles this module emits
//! are therefore the lowercase draw titles (`"cpu"`, `"mem"`, `"net"`,
//! `"proc"`, `"gpuN"`), each cited at its `createBox` call site below.

use std::sync::Mutex;
use std::time::Duration;

use btop_draw::ansi::mv_to;
use btop_draw::boxes::{create_box, Palette};
use btop_runner::sink::World;

use crate::term::TermWrapper;

// ── Labels ──────────────────────────────────────────────────────────────────

/// Outline title of the CPU box (`"cpu"`, src/btop_draw.cpp:2363).
pub const CPU_BOX_LABEL: &str = "cpu";
/// Outline title of the memory box (`"mem"`, src/btop_draw.cpp:2487).
pub const MEM_BOX_LABEL: &str = "mem";
/// Outline title of the network box (`"net"`, src/btop_draw.cpp:2524).
pub const NET_BOX_LABEL: &str = "net";
/// Outline title of the process box (`"proc"`, src/btop_draw.cpp:2548).
pub const PROC_BOX_LABEL: &str = "proc";

/// `(box key, outline title)` pairs, in C++ print order
/// (src/btop.cpp:1097 `Cpu::box << Mem::box << Net::box << Proc::box`).
pub const BOX_LABELS: &[(&str, &str)] = &[
    ("cpu", CPU_BOX_LABEL),
    ("mem", MEM_BOX_LABEL),
    ("net", NET_BOX_LABEL),
    ("proc", PROC_BOX_LABEL),
];

/// Dynamic GPU panel title (`"gpu" + N`, src/btop_draw.cpp:2413).
/// GPU labels are count-based, so they are generated at print time from
/// `World::gpu_count` instead of being cached constants.
pub fn gpu_box_label(panel: u32) -> String {
    format!("gpu{panel}")
}

/// Outline numbering per box (`createBox` `num` arg): cpu 1 (:2363), mem 2
/// (:2487), net 3 (:2524), proc 4 (:2548), gpuN `(N+5)%10` (:2413).
/// Unknown names suppress numbering (`num == 0`, `create_box` contract).
pub fn box_number(name: &str) -> i64 {
    match name {
        "cpu" => 1,
        "mem" => 2,
        "net" => 3,
        "proc" => 4,
        _ if name.starts_with("gpu") => name[3..].parse::<i64>().map(|n| (n + 5) % 10).unwrap_or(0),
        _ => 0,
    }
}

// ── print_box_outlines ──────────────────────────────────────────────────────

/// Synchronized-output escapes bracketing the outline print
/// (`Term::sync_start`/`sync_end`, src/btop_tools.cpp:768-769).
pub const SYNC_START: &str = "\x1b[?2026h";
/// See [`SYNC_START`].
pub const SYNC_END: &str = "\x1b[?2026l";

/// Print one outline frame per visible box into `out`, mirroring
/// src/btop.cpp:1093-1097 (`Cpu::box << Mem::box << Net::box << Proc::box`,
/// wrapped in the sync pair when `terminal_sync`).
///
/// Each frame is re-assembled with [`create_box`] from the cached geometry
/// in `World::layout` (T7's `RecalcLayout` fills it; the C++ side likewise
/// re-renders the `box` caches in `calcSizes` just above the print,
/// src/btop.cpp:1092) using the same per-box arguments as the draw sites:
/// `<box>_box` line color, `fill=true`, outer titles (`cpu_bottom` swaps the
/// cpu title to the bottom edge, :2363), and the numbering above. Colors
/// resolve through [`Palette`] so missing theme keys degrade to `""`
/// (`color()` returns empty for unknown keys) instead of panicking.
///
/// Boxes whose geometry is hidden or degenerate contribute nothing
/// (`create_box` returns `""` for `width < 1 || height < 1`).
/// GPU panels are intentionally NOT printed: the C++ print line covers only
/// the four main boxes (GPU outlines ride inside the per-tick draw).
pub fn print_box_outlines(world: &World, out: &mut String) {
    if world.terminal_sync {
        out.push_str(SYNC_START);
    }
    let tty = world.config.get_b("tty_mode").unwrap_or(false);
    let rounded = world.config.get_b("rounded_corners").unwrap_or(true);
    let lowcolor = world.config.get_b("lowcolor").unwrap_or(false);
    let theme_bg = world.config.get_b("theme_background").unwrap_or(true);
    let cpu_bottom = world.config.get_b("cpu_bottom").unwrap_or(false);

    for (key, title) in BOX_LABELS {
        let (geom, color_key, title_top, title_bottom) = match *key {
            "cpu" => (
                &world.layout.cpu.base,
                "cpu_box",
                if cpu_bottom { "" } else { *title },
                if cpu_bottom { *title } else { "" },
            ),
            "mem" => (&world.layout.mem.base, "mem_box", *title, ""),
            "net" => (&world.layout.net.base, "net_box", *title, ""),
            _ => (&world.layout.proc.base, "proc_box", *title, ""),
        };
        if !geom.shown {
            continue;
        }
        let pal = Palette::new(color_key, &world.theme, lowcolor, theme_bg);
        out.push_str(&create_box(
            geom.x,
            geom.y,
            geom.width,
            geom.height,
            &pal.box_color,
            true,
            title_top,
            title_bottom,
            box_number(key),
            &pal.div_line,
            &pal.hi_fg,
            &pal.title,
            &pal.reset,
            tty,
            rounded,
        ));
    }
    if world.terminal_sync {
        out.push_str(SYNC_END);
    }
}

// ── min_size_loop ───────────────────────────────────────────────────────────

/// Outcome of [`min_size_loop`]. `q` does NOT call `clean_quit` here —
/// that sequence is T7 territory (see [`crate::clean_quit`]); the caller
/// (T7 `main` / `main_loop`) maps [`MinSizeOutcome::QuitRequested`] to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MinSizeOutcome {
    /// Terminal fits (or a re-check showed it fits): proceed to the loop.
    Ready,
    /// User pressed `q`: T7 should `clean_quit(0)`.
    QuitRequested,
}

/// Mockable input-poll seam over `Input::poll`/`Input::get`
/// (src/btop_input.cpp:95-121). Production boot passes [`RealInputPoll`];
/// tests pass a scripted double — the loop must never block on real stdin
/// in tests.
pub trait InputPoll: Send + Sync {
    /// `true` when a key is available (C++ `poll` also buffers the bytes;
    /// impls buffer so [`InputPoll::get`] can serve them).
    fn poll(&self, timeout_ms: u64) -> bool;
    /// Decoded key (C++ `Input::get()`). Only single-char compares (`q`,
    /// digits) matter here; multi-byte escapes never match those arms, so
    /// impls may return the raw bytes for anything else.
    fn get(&self) -> String;
}

/// Production [`InputPoll`]: `poll(2)` on stdin (src/btop_input.cpp:95-120)
/// then a single non-blocking-shape `read` of the waiting bytes.
///
/// DEVIATIONS: C++ waits with `pselect` under the SIGUSR1 mask and drains
/// stdin in a loop (its fd is non-blocking after `Input::init`); here
/// plain `nix::poll::poll` runs with no masked `pselect` (an external
/// SIGUSR1 pends — nothing in-port sends it; internal wakes ride
/// `interrupt_input` / EINTR / the poll timeout) and a single `read` serves (one keypress per small-screen
/// iteration is enough — the loop re-polls). Key decoding (`Input::get`'s
/// escape tables) is skipped: raw bytes compare equal for the `q`/digit
/// arms this loop inspects.
pub struct RealInputPoll {
    pending: Mutex<String>,
}

impl RealInputPoll {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(String::new()),
        }
    }
}

impl Default for RealInputPoll {
    fn default() -> Self {
        Self::new()
    }
}

/// Clamp a poll quantum to the `nix::poll` timeout width. All live call
/// sites pass ≤1000ms, so this only hardens against future callers: a
/// bare `as u16` would wrap (e.g. 100_000 → 34464), saturation caps at
/// `u16::MAX` instead.
fn saturate_poll_quantum(timeout_ms: u64) -> u16 {
    u16::try_from(timeout_ms).unwrap_or(u16::MAX)
}

impl InputPoll for RealInputPoll {
    fn poll(&self, timeout_ms: u64) -> bool {
        use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
        use std::os::fd::BorrowedFd;
        // SAFETY: fd 0 is stdin for the process lifetime; `poll` takes no
        // ownership and performs no mutation.
        let fd = unsafe { BorrowedFd::borrow_raw(0) };
        let mut fds = [PollFd::new(fd, PollFlags::POLLIN)];
        let ready = poll(
            &mut fds,
            PollTimeout::from(saturate_poll_quantum(timeout_ms)),
        )
        .unwrap_or(0);
        if ready <= 0 {
            return false;
        }
        let mut buf = [0u8; 1024];
        // SAFETY: same short-lived stdin borrow as above.
        let n = unsafe { nix::libc::read(0, buf.as_mut_ptr().cast(), buf.len()) };
        if n <= 0 {
            return false;
        }
        let chunk = String::from_utf8_lossy(&buf[..n as usize]).into_owned();
        *self.pending.lock().unwrap() = chunk;
        true
    }

    fn get(&self) -> String {
        std::mem::take(&mut *self.pending.lock().unwrap())
    }
}

/// Digit→box table (GPU_SUPPORT build, src/btop.cpp:146).
const ALL_BOXES: &[&str] = &[
    "gpu5", "cpu", "mem", "net", "proc", "gpu0", "gpu1", "gpu2", "gpu3", "gpu4",
];

/// Digit legality gate (src/btop_input.cpp:243): index 0 needs 5+ GPUs,
/// index ≥ 5 needs `intKey - 4 <= Gpu::count`. Mirrors
/// `sink::toggle_box_legal` (same transcription; that helper is private to
/// the sink, so the rule is repeated here rather than reached through —
/// T7 unification note: either make the sink helper `pub(crate)` visible to
/// `btop-app` or move the table+gate into `btop-config` next to
/// `toggle_box`; until then the two copies are kept in sync by inspection).
///
/// `Input::interrupt` ownership (cpp:207 loop-tail note): C++ wakes the
/// blocked `Input::poll` so the toggle path re-renders promptly. The port
/// needs no equivalent — our `InputPoll::poll(10)` times out on its own
/// and the loop re-checks, so there is no sleeper to interrupt.
fn box_toggle_legal(index: usize, gpu_count: u32) -> bool {
    let n = index as i64;
    if index >= ALL_BOXES.len() {
        return false;
    }
    if (n == 0 && (gpu_count as i64) < 5) || (n >= 5 && n - 4 > gpu_count as i64) {
        return false;
    }
    !ALL_BOXES[index].is_empty()
}

/// Block until the terminal fits the current box layout or the user quits.
/// Transcribes the `term_resize` tail (src/btop.cpp:154-209, `force=false`
/// path): `sleep_ms(100)` (:159), the too-small screen (:163-180),
/// `for (; !refresh && !got_key; got_key = poll(10))` (:187),
/// `q` → quit (:190), digit → preset-reset + `toggle_box` (:191-199),
/// recompute min size (:200-202), else-arm refresh break (:208).
///
/// `term.get_min_size()` stands in for `Term::get_min_size(boxes)` — the
/// Rust `Term` still reports the `(100, 24)` stub (see `btop_tools::term`),
/// so per-box minima do not shrink when boxes toggle yet; the loop shape
/// (re-check after every toggle) already matches C++, and the values will
/// follow when the stub is ported. `Config::unlock()` (:152) is a no-op
/// here: boot never locks the Rust `Config`.
pub fn min_size_loop(
    world: &mut World,
    term: &TermWrapper,
    input: &dyn InputPoll,
    out: &mut String,
) -> MinSizeOutcome {
    // NOTE: C++ re-reads `boxes = Config::getS("shown_boxes")` after the
    // toggle (:196) because `Term::get_min_size(boxes)` takes the layout.
    // The Rust `Term` stub takes no boxes argument yet, so there is nothing
    // to refresh here; the re-read (and the per-box minima) arrive with
    // the `get_min_size` port.
    let (mut min_w, mut min_h) = term.get_min_size();
    while term.width() < min_w || term.height() < min_h {
        std::thread::sleep(Duration::from_millis(100));
        if term.width() < min_w || term.height() < min_h {
            render_too_small(term.width(), term.height(), min_w, min_h, out);
            let mut got_key = false;
            while !term.refresh(false) && !got_key {
                got_key = input.poll(10);
            }
            if got_key {
                let key = input.get();
                if key == "q" {
                    return MinSizeOutcome::QuitRequested;
                } else if key.len() == 1 && key.bytes().all(|b| b.is_ascii_digit()) {
                    let int_key: usize = key.parse().unwrap_or(usize::MAX);
                    if box_toggle_legal(int_key, world.gpu_count) {
                        world.current_preset = None;
                        let _ = world.config.toggle_box(ALL_BOXES[int_key]);
                    }
                }
            }
            (min_w, min_h) = term.get_min_size();
        } else if !term.refresh(false) {
            break;
        }
    }
    MinSizeOutcome::Ready
}

/// "Terminal size too small" screen (src/btop.cpp:163-180).
///
/// C++ prints four bare text lines centered on the terminal; this port wraps
/// the same lines in a [`create_box`] frame (brief-mandated reuse) with the
/// same strings, colors (`Global::fg_white/green/red`, src/btop.cpp:100-105:
/// `"\x1b[1;97m"` / `"\x1b[1;92m"` / `"\x1b[1;91m"`), and center-relative
/// positions (`Mv::to(height/2 ± k, width/2 ∓ k)`). Coordinates clamp to ≥ 1
/// so tiny terminals emit valid escapes instead of `0;0f`.
fn render_too_small(width: u16, height: u16, min_w: u16, min_h: u16, out: &mut String) {
    const FG_WHITE: &str = "\x1b[1;97m";
    const FG_GREEN: &str = "\x1b[1;92m";
    const FG_RED: &str = "\x1b[1;91m";
    const CLEAR: &str = "\x1b[2J\x1b[0;0f";
    const RESET: &str = "\x1b[0m";
    let (w, h) = (width as i64, height as i64);
    let cx = |off: i64| (w / 2 + off).max(1);
    let cy = |off: i64| (h / 2 + off).max(1);
    out.push_str(CLEAR);
    out.push_str(FG_WHITE);
    // Frame around the message block (C++ has none — frame is the
    // brief-mandated `create_box` reuse; geometry centers the 46×9 block).
    out.push_str(&create_box(
        cx(-23),
        cy(-5),
        46,
        9,
        "",
        false,
        "terminal too small",
        "",
        0,
        FG_WHITE,
        FG_WHITE,
        FG_WHITE,
        RESET,
        false,
        true,
    ));
    let fg_w = if width < min_w { FG_RED } else { FG_GREEN };
    let fg_h = if height < min_h { FG_RED } else { FG_GREEN };
    out.push_str(&format!(
        "{mv1}{c}Terminal size too small:\
         {mv2} Width = {fw}{w} {c}Height = {fh}{h}\
         {mv3}{c}Needed for current config:\
         {mv4}Width = {min_w} Height = {min_h}{reset}",
        mv1 = mv_to(cy(-2), cx(-11)),
        mv2 = mv_to(cy(-1), cx(-10)),
        mv3 = mv_to(cy(1), cx(-12)),
        mv4 = mv_to(cy(2), cx(-10)),
        c = FG_WHITE,
        fw = fg_w,
        fh = fg_h,
        reset = RESET,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_quantum_saturates_instead_of_wrapping() {
        assert_eq!(saturate_poll_quantum(0), 0);
        assert_eq!(saturate_poll_quantum(1000), 1000);
        assert_eq!(saturate_poll_quantum(u64::from(u16::MAX)), u16::MAX);
        assert_eq!(saturate_poll_quantum(u64::from(u16::MAX) + 1), u16::MAX);
        assert_eq!(saturate_poll_quantum(u64::MAX), u16::MAX);
    }
}
