//! Single-threaded tick (was main loop + secondary `_runner` thread).
//!
//! C++ truth: `src/btop.cpp:1105-1180` (main loop) + `:468-727` (`_runner`
//! body). The two are consolidated here so P4 owns only the init/signal/term
//! scaffolding. The shape: schedule → per-box (collect → assemble → draw) →
//! merge (cpu+gpu+mem+net+proc [+clock if !paused]) → overlay dim wrap →
//! empty-bg hint → terminal_sync wrap.
//!
//! Per-box backend call list (transcribed from cpp:533-644, granular
//! methods on `MacOsBackend`):
//! - cpu:   cpu_ticks, load_avg, package_temp, core_temps
//! - gpu:   gpu_residency, gpu_energy, hid_temps (always called when a
//!   `gpu*` box is in `shown_boxes`; per-panel indexing is the caller's
//!   concern via `state.gpu_panel`)
//! - mem:   vm_raw, swap_raw, disk_raw(mount) for every persisted mount
//! - net:   if_counters
//! - proc:  proc_list
//!
//! `assemble_*` lives in `super::wiring`; the tick drives the per-box
//! sequence and hands the resulting inputs to `btop_draw::*::draw_*`.
//!
//! Object-safety: `MacOsBackend` is object-safe (all methods `&mut self`,
//! no generics, no `Self` returns) — the tick takes `&mut dyn MacOsBackend`
//! so P4 can swap `RealBackend` at runtime. Verified against the trait
//! declaration (`src-rust/btop-collect/src/backend.rs`).

use btop_collect::backend::apply_disks_filter;
use btop_collect::backend::MacOsBackend;
use btop_config::theme::default_theme;
use btop_draw::ansi::FX_UB;
use btop_draw::boxes::{calc_sizes, LayoutInput};
use btop_draw::cpu::draw_cpu;
use btop_draw::mem::draw_mem;
use btop_draw::net::draw_net;
use btop_draw::proc_::draw_proc;

use super::sink::{Sys, World};
use super::wiring::{assemble_cpu, assemble_mem, assemble_net, assemble_proc, DiskSample};

/// Per-tick inputs. `backend` drives all collect calls; `now_ms` is the
/// P4-supplied monotonic clock; `force_redraw_in` triggers an out-of-band
/// pass (resize/menu/ApplyTheme paths); `overlay` is the menu/msgbox
/// string to dim-merge on top (empty = no overlay).
pub struct TickInput<'a> {
    pub backend: &'a mut dyn MacOsBackend,
    pub now_ms: u64,
    pub pending_resize: bool,
    pub should_quit: bool,
    pub force_redraw_in: bool,
    pub overlay: String,
}

/// Per-tick outputs. `out` is the bytes that P4 writes to stdout (already
/// terminal-sync wrapped if `World::terminal_sync` is set); `ran_boxes` is
/// the list of boxes that actually drew this tick (for P4 bookkeeping);
/// `next_in_ms` is the deadline for the next scheduled tick (relative to
/// `now_ms`-style time, not wall-clock); `pause_output` mirrors the C++
/// `Runner::pause_output` (set when overlay is non-empty AND
/// `background_update` is off).
pub struct TickOutput {
    pub out: String,
    pub ran_boxes: Vec<String>,
    pub next_in_ms: u64,
    pub pause_output: bool,
}

/// Strip ANSI color/style escapes (`Fx::uncolor`, `src/btop_tools.cpp:755`
/// `color_regex = "\033\[\d+;?\d?;?\d*;?\d*;?\d*(m){1}"` — same shape as
/// the more permissive `\x1b\[` followed by `;`-separated digits and `m`).
/// Cursor moves (`f`/`s`/`u`/`C`/`D`/`A`/`B`) are NOT stripped — `uncolor`
/// only removes `m`-terminated sequences.
fn uncolor(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        // ESC '[' (2 bytes) opens an SGR sequence.
        if i + 1 < bytes.len() && bytes[i] == 0x1b && bytes[i + 1] == b'[' {
            // Consume until 'm' (the SGR terminator). C++ regex is
            // permissive about the digit set; we accept digits + ';'
            // (matches the live fixtures verbatim).
            let mut j = i + 2;
            let mut ok = false;
            while j < bytes.len() {
                let c = bytes[j];
                if c == b'm' {
                    ok = true;
                    j += 1;
                    break;
                }
                if !c.is_ascii_digit() && c != b';' {
                    break;
                }
                j += 1;
            }
            if ok {
                i = j;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_default()
}

/// Cached "No boxes shown!" hint. Mirrors cpp:666-690: produced once per
/// runner session (the `if (empty_bg.empty())` gate keeps it pinned) and
/// re-emitted on every empty + !paused tick.
fn empty_bg(theme: &std::collections::HashMap<String, String>, term_w: i64, term_h: i64) -> String {
    use btop_draw::ansi::{mv_to, FX_B};
    let x = term_w / 2 - 10;
    let y = term_h / 2 - 10;
    let main_fg = theme.get("main_fg").cloned().unwrap_or_default();
    let title_fg = theme.get("title").cloned().unwrap_or_default();
    let hi_fg = theme.get("hi_fg").cloned().unwrap_or_default();
    // P3 keeps the hint textual (no real banner gen — P4 owns Draw::banner_gen
    // integration). A minimal "No boxes shown!" + key list, copied verbatim
    // from cpp:670-688 with the banner_gen argument dropped.
    let mut s = String::new();
    s += &mv_to(y + 6, x);
    s += &title_fg;
    s += FX_B;
    s += "No boxes shown!";
    s += FX_UB;
    s += "\n";
    for (dy, k, label) in [
        (8i64, "1", "Show CPU box"),
        (9, "2", "Show MEM box"),
        (10, "3", "Show NET box"),
        (11, "4", "Show PROC box"),
        (12, "5-0", "Show GPU boxes"),
        (13, "esc", "Show menu"),
        (14, "q", "Quit"),
    ] {
        s += &mv_to(y + dy, if dy == 12 || dy == 13 { x - 2 } else { x });
        s += &hi_fg;
        s += k;
        s += " ";
        s += &main_fg;
        s += "| ";
        s += label;
        s += "\n";
    }
    let _ = ();
    s
}

/// Drive one tick of the runner. Returns the bytes P4 will write to
/// stdout plus the scheduling hint for the next call.
pub fn tick(w: &mut World, sys: &mut dyn Sys, t: TickInput<'_>) -> TickOutput {
    // ── Schedule gate ──────────────────────────────────────────────────────
    // cpp:1152: `if (time_ms() >= future_time and not Global::resized)`.
    // The runner owns `next_tick_ms`; we keep it on `World::next_tick_ms`
    // (initialized on first tick from now_ms + update_ms). Early ticks
    // return an empty payload and the remaining time.
    if !t.force_redraw_in && t.now_ms < w.next_tick_ms && !t.pending_resize {
        return TickOutput {
            out: String::new(),
            ran_boxes: Vec::new(),
            next_in_ms: w.next_tick_ms.saturating_sub(t.now_ms),
            pause_output: w.paused,
        };
    }

    // cpp:1096,1153: term_sync + Runner::run flags. force_redraw_in
    // mirrors the resize path (no_update=true, force=true). Per-box draw
    // inputs each take `force_redraw` explicitly (proc is the only one
    // currently threaded here; the rest use Theme state — see
    // `assemble_cpu/mem/net` for the no_update/force_redraw plumbing).
    // Fan the in-flag out to the shared AppState slot the assemblers read:
    // assignment (not OR) consumes it, so a forced tick redraws boxes once
    // and the following scheduled ticks go back to content-only. Placed
    // after the early-return gate so a skipped tick never eats a pending
    // redraw request.
    w.state.force_redraw = t.force_redraw_in;
    let force_redraw = t.force_redraw_in;
    let update_ms = w.config.get_i("update_ms").unwrap_or(2000).max(100) as u64;

    // ── Layout (calcSizes) ─────────────────────────────────────────────────
    // cpp:1093/1138: Draw::calcSizes re-runs on every resize/ApplyTheme
    // trigger. `w.recalc_layout` is the sink-driven request flag.
    let term_w = w.state.term_width.max(20) as usize;
    let term_h = w.state.term_height.max(10);
    if w.recalc_layout {
        let shown_boxes = w
            .config
            .get_s("shown_boxes")
            .unwrap_or("cpu mem net proc")
            .to_string();
        let cpu_bottom = w.config.get_b("cpu_bottom").unwrap_or(false);
        let mem_below_net = w.config.get_b("mem_below_net").unwrap_or(false);
        let proc_left = w.config.get_b("proc_left").unwrap_or(false);
        // Layout-flag fallbacks mirror `btop --default-config` (verified):
        // show_disks/mem_graphs/swap_disk default true.
        let show_disks = w.config.get_b("show_disks").unwrap_or(true);
        let mem_graphs = w.config.get_b("mem_graphs").unwrap_or(true);
        let swap_disk = w.config.get_b("swap_disk").unwrap_or(true);
        let swap_upload_download = w.config.get_b("swap_upload_download").unwrap_or(false);
        let has_swap = w.state.has_swap;
        let li = LayoutInput {
            term_w: term_w as i64,
            term_h,
            shown_boxes,
            cpu_bottom,
            mem_below_net,
            proc_left,
            core_count: w.state.core_count as i64,
            show_temp: true,
            cpu_width_p: 100,
            cpu_height_p: 32,
            mem_width_p: 45,
            net_height_p: 28,
            show_disks,
            swap_disk,
            mem_graphs,
            has_swap,
            swap_upload_download,
            gpus_extra_height: 0,
            gpu_total_height: 0,
            gpu_panels: vec![],
            gpu_height_p: 32,
            gpu_min_height: 8,
            gpu_min_width: 41,
        };
        let layout = calc_sizes(&li);
        // cpp:1093 + cpp:1632: Proc::select_max is set by calcSizes; mirror
        // it into both World::layout (draw-side) and state.proc_geom
        // (input-side used by sink selection math).
        w.layout = layout.clone();
        w.state.proc_geom.select_max = layout.proc.select_max;
        w.recalc_layout = false;
        // cpp:1129: Draw::banner_gen(0,0,false,true) (headless no-op here;
        // menu system owns the overlay string on the menu path).
        let _ = sys;
    }

    // ── Resolve shown boxes ────────────────────────────────────────────────
    let shown_boxes = w
        .config
        .get_s("shown_boxes")
        .unwrap_or("cpu mem net proc")
        .to_string();
    let boxes: Vec<&str> = shown_boxes.split_whitespace().collect();
    let has_cpu = boxes.contains(&"cpu");
    let has_mem = boxes.contains(&"mem");
    let has_net = boxes.contains(&"net");
    let has_proc = boxes.contains(&"proc");
    let gpu_panels: Vec<&str> = boxes
        .iter()
        .copied()
        .filter(|b| b.starts_with("gpu"))
        .collect();

    // ── Per-box collect → assemble → draw ──────────────────────────────────
    //
    // Pause gate (cpp:551/575/597/617/637): each `Cpu/Gpu/Mem/Net/Proc::draw`
    // call sits behind `if (not pause_output)`. The C++ updates
    // `pause_output` AFTER box draws (cpp:664), so the gate reads the
    // *prior-tick* value: the first tick with overlay still emits boxes
    // (pause_output was false); subsequent ticks suppress them (pause_output
    // is true from the previous tick). We snapshot `w.paused` here and use
    // it as the gate.
    let paused = w.paused;
    let theme = default_theme();
    let mut output = String::new();
    let mut ran_boxes: Vec<String> = Vec::new();

    // CPU
    if has_cpu && !paused {
        let ticks = t.backend.cpu_ticks().unwrap_or_default();
        let load = t.backend.load_avg().unwrap_or([0.0, 0.0, 0.0]);
        let pkg = t.backend.package_temp().unwrap_or(None);
        let core_t = t.backend.core_temps().unwrap_or_default();
        let cpu_input = assemble_cpu(&mut w.state, &ticks, load, term_w, pkg, &core_t);
        let s = draw_cpu(&cpu_input, &w.layout.cpu, &theme);
        output.push_str(&s);
        ran_boxes.push("cpu".to_string());
    }

    // GPU (only when at least one panel is shown; always try collect even
    // when no panel is in shown_boxes — mirrors cpp:511-528 gate).
    //
    // DEVIATION from cpp:524-528: a GPU panel needs a `GpuGeom` from
    // calc_sizes, which requires `gpu_panels` to be non-empty when
    // building the `LayoutInput`. P3 does not pre-build GPU geometries
    // (the proc-detail-golden path has no GPU); the full GPU tick pass is
    // P4 work. We still drain the backend calls (so consumers see them)
    // but emit no box bytes — equivalent to cpp:585 `if (... not
    // gpu_panels.empty() and not gpus_ref.empty()) { draw }`.
    if !gpu_panels.is_empty() && !paused {
        let _residency = t.backend.gpu_residency().unwrap_or_default();
        let _energy = t
            .backend
            .gpu_energy()
            .unwrap_or((0u64, btop_collect::gpu::EnergyUnit::Nano));
        let _hid = t.backend.hid_temps().unwrap_or_default();
        // Drop GPU geometry / assembly for now — see comment above.
        if let Some(_geom) = w.layout.gpu_panels.first() {
            // P4 will wire this; intentionally a no-op in P3.
            let _ = _geom;
        }
        ran_boxes.push("gpu".to_string());
    }

    // MEM
    if has_mem && !paused {
        let vm = t.backend.vm_raw().unwrap_or((0, 0, 0, 0, 4096));
        let swap = t.backend.swap_raw().unwrap_or((0, 0, 0));
        // Mem total comes from a separate OS probe; the headless harness
        // doesn't expose it on the backend, so we fall back to the
        // assembled state value (0 means "unknown").
        let total_mem = w.state.total_mem;
        // Disk discovery (C++ osx/btop_collect.cpp:1284-1345): enumerate
        // mounts every tick, apply disks_filter, prune unmounted state.
        // The order refresh is skipped when discovery is empty (headless
        // ReplayBackend default) so unit goldens seeding mem_disks_order
        // directly keep working; live getmntinfo never returns empty.
        let filter = w.config.get_s("disks_filter").unwrap_or("").to_string();
        let discovered = apply_disks_filter(&t.backend.disk_mounts().unwrap_or_default(), &filter);
        if !discovered.is_empty() {
            w.state.mem_disks_order = discovered.iter().map(|(m, _)| m.clone()).collect();
            w.state
                .mem_disks
                .retain(|m, _| discovered.iter().any(|(dm, _)| dm == m));
            w.state
                .disk_last_io
                .retain(|m, _| discovered.iter().any(|(dm, _)| dm == m));
        }
        let names: std::collections::HashMap<&str, &str> = discovered
            .iter()
            .map(|(m, n)| (m.as_str(), n.as_str()))
            .collect();
        let disks: Vec<DiskSample> = w
            .state
            .mem_disks_order
            .iter()
            .map(|mount| {
                let (blocks, bfree, frsize) = t.backend.disk_raw(mount).unwrap_or((0, 0, 0));
                let (read_bytes, write_bytes) = w
                    .state
                    .mem_disks
                    .get(mount)
                    .map(|d| {
                        (
                            d.io_read.last().copied().unwrap_or(0) as u64,
                            d.io_write.last().copied().unwrap_or(0) as u64,
                        )
                    })
                    .unwrap_or((0, 0));
                DiskSample {
                    mount: mount.clone(),
                    name: names
                        .get(mount.as_str())
                        .map(|s| s.to_string())
                        .or_else(|| w.state.mem_disks.get(mount).map(|d| d.name.clone()))
                        .unwrap_or_else(|| mount.clone()),
                    blocks,
                    bfree,
                    frsize,
                    read_bytes,
                    write_bytes,
                }
            })
            .collect();
        let mem_input = assemble_mem(&mut w.state, vm, swap, total_mem, &disks, term_w);
        let s = draw_mem(&mem_input, &w.layout.mem, &theme);
        output.push_str(&s);
        ran_boxes.push("mem".to_string());
    }

    // NET
    if has_net && !paused {
        let counters = t.backend.if_counters().unwrap_or_default();
        let net_input = assemble_net(&mut w.state, &counters, update_ms, term_w);
        let s = draw_net(&net_input, &w.layout.net, &theme);
        output.push_str(&s);
        ran_boxes.push("net".to_string());
    }

    // PROC
    if has_proc && !paused {
        let raws = t.backend.proc_list().unwrap_or_default();
        // delta_total is the cpu-side delta (cpp:633: `proc` is collected
        // independently of `cpu`, but per-cpu deltas feed the per-process
        // `cpu_p` math; we re-use the assembled state.last_cputimes for
        // parity with the existing wiring tests).
        let delta_total = w.state.last_cputimes;
        let proc_input = assemble_proc(&mut w.state, raws, delta_total, term_w);
        // cpp:637: pass force_redraw / no_update to Proc::draw.
        let mut proc_input = proc_input;
        proc_input.force_redraw = force_redraw;
        let s = draw_proc(&proc_input, &w.layout.proc, &theme, &mut Vec::new());
        output.push_str(&s);
        ran_boxes.push("proc".to_string());
    }

    // ── Pause gate (overlay dim) ───────────────────────────────────────────
    // cpp:664: `if (not conf.overlay.empty() and not conf.background_update)
    // pause_output = true;` — applied AFTER box draws. The new value
    // gates the NEXT tick's box draws (via the `paused` snapshot above).
    let pause_output = !t.overlay.is_empty() && !w.background_update;
    w.paused = pause_output;

    // cpp:663: clock appended only when not paused (the clock IS the only
    // thing that still renders under pause_output, per the existing C++:
    // actually re-reading cpp:663 shows the inverse: `if (not pause_output)
    // output += conf.clock;` — clock is suppressed under pause. We mirror.)
    if !pause_output && (!w.state.clock.time.is_empty() || !w.state.clock.date.is_empty()) {
        use btop_draw::boxes::render_clock;
        let (clk, _len) = render_clock(
            &w.state.clock,
            0, // prev_len = 0 (no erase on first draw)
            1,
            1,
            term_w as i64,
            false,
            false,
            0,
            "",
            "",
        );
        output.push_str(&clk);
    }

    // cpp:665-691: empty + !paused → empty_bg hint cached on first miss.
    if output.is_empty() && !pause_output {
        if w.empty_bg.is_empty() {
            w.empty_bg = empty_bg(&theme, term_w as i64, term_h);
        }
        output.push_str(&w.empty_bg);
    }

    // ── Final merge ────────────────────────────────────────────────────────
    // cpp:720-724: `cout << (term_sync ? Term::sync_start : "") << (conf.
    // overlay.empty() ? output : (output.empty() ? "" : Fx::ub +
    // Theme::c("inactive_fg") + Fx::uncolor(output)) + conf.overlay) <<
    // (term_sync ? Term::sync_end : "") << flush;`
    let merged = if !t.overlay.is_empty() {
        if output.is_empty() {
            t.overlay.clone()
        } else {
            let inactive_fg = theme.get("inactive_fg").cloned().unwrap_or_default();
            format!("{}{}{}{}", FX_UB, inactive_fg, uncolor(&output), t.overlay,)
        }
    } else {
        output.clone()
    };
    let final_out = if w.terminal_sync {
        format!("{}{}\x1b[?2026l", "\x1b[?2026h", merged)
    } else {
        merged
    };

    // cpp:1155: `future_time = time_ms() + update_ms;`
    w.next_tick_ms = t.now_ms + update_ms;

    TickOutput {
        out: final_out,
        ran_boxes,
        next_in_ms: update_ms,
        pause_output,
    }
}
