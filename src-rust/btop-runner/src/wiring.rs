//! Per-box wiring: central tick state plus the collect→draw assembly fns.
//!
//! C++ truth: per-box collect→draw data flow (osx `collect` + `btop_draw`
//! inputs). This crate owns the caller-side lifecycle the M-ports left
//! explicit: history push/trim, net offset persistence, proc cpu deltas.
//!
//! [`AppState`] holds every draw input's borrowed data plus the UI/selection
//! state Tasks 3–4 mutate. Each `assemble_*` runs the collect-side math,
//! persists tick state, and returns the matching `btop_draw` input struct
//! borrowing `AppState` (no per-tick clones on the hot path).

use btop_collect::backend::ProcRaw;
use btop_collect::cpu::{push_trimmed, sensor_index, update_core, update_total};
use btop_collect::gpu::{gpu_clock, gpu_util, power_mw, sensor_avg, EnergyUnit};
use btop_collect::mem::{disk_usage, io_activity, io_delta, mem_percent, vm_stats};
use btop_collect::net::{track_top, update_counter};
use btop_collect::proc_::proc_cpu_percent;
use btop_config::config::Config;
use btop_draw::boxes::{BoxGeom, Clock, ProcGeom};
use btop_draw::cpu::{BatteryState, CpuDrawInput, CpuFlags};
use btop_draw::gpu::{GpuDrawInput, GpuFlags, GpuSupported};
use btop_draw::mem::{DiskDraw, MemDrawInput, MemFlags};
use btop_draw::net::{NetDrawInput, NetFlags, NetStat};
use btop_draw::proc_::{matches_filter, ProcDetail, ProcDrawInput, ProcFlags, ProcInfo};
use btop_tools::mouse::MouseMap;
use std::cmp::Ordering;
use std::cmp::Reverse;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::Arc;

// ── SignalFlags ───────────────────────────────────────────────────────────
// C++ atomics at btop.cpp:118-124 (`resized`, `quitting`, `should_quit`,
// `should_sleep`, `_runner_started`, `init_conf`, `reload_conf`),
// btop.cpp:133 (`resizing`), and btop.cpp:356-359 (`stopping`, `waiting`,
// `redraw`, `coreNum_reset`). The handler-written subset lands here as
// `Arc<AtomicBool>` flags the signal handlers flip and the main loop
// polls (see `btop_app::signals` for the signal→flag table).
//
// OWNERSHIP (binding): every flag is an `Arc<AtomicBool>`, so
// `SignalFlags::clone()` shares state — a handler installed with a clone
// observes the same flags as the main loop's copy. `Clone` is therefore
// REQUIRED (not impossible-by-design): boot clones the flags once for
// the installer and keeps the original in `World.signal_flags` (see
// `sink::World`), and both sides observe identical state. `World` keeps
// holding `SignalFlags` BY VALUE (field name/type path stable); sharing
// happens one level down, inside each flag.
#[derive(Debug, Default, Clone)]
pub struct SignalFlags {
    /// SIGWINCH: terminal resized (btop.cpp:118, handler :316-318).
    pub resized: Arc<AtomicBool>,
    /// SIGINT: quit requested (btop.cpp:120, handler :292-296).
    pub should_quit: Arc<AtomicBool>,
    /// SIGTSTP: sleep requested (btop.cpp:121, handler :297-306).
    pub should_sleep: Arc<AtomicBool>,
    /// SIGCONT: resume requested. DEVIATION: C++ calls `_resume()`
    /// (Term::init + term_resize, :271-274) inline in the handler; that
    /// is not async-signal-safe, so the Rust port records a flag and the
    /// main loop performs the re-init.
    pub do_continue: Arc<AtomicBool>,
    /// Poll wake. DEVIATION: C++ calls `Input::interrupt()` at :293,
    /// :301, :317, :324 (and documents SIGUSR1 as the "Input::poll
    /// interrupt", :319-321); the single-threaded port has no input thread
    /// to kick, so every interrupt site records this flag instead. The loop
    /// never reads it — wakeup is the EINTR/timeout break, not the flag —
    /// so it is handler-observability only; tests assert it as proof the
    /// handler ran.
    pub interrupt_input: Arc<AtomicBool>,
    /// SIGUSR2: reload config (btop.cpp:124, handler :322-325).
    pub reload_conf: Arc<AtomicBool>,
    /// SIGINT/SIGTSTP while running. DEVIATION: C++ sets
    /// `Runner::stopping` only `if (Runner::active)` (:294, :300); the
    /// single-threaded port has no runner thread, so the flag is set
    /// unconditionally — the loop is always "active" once booted.
    pub stopping: Arc<AtomicBool>,
    /// Exit in progress (`Global::quitting`, btop.cpp:119, set by
    /// `clean_quit` at :211-213). Written by the atexit hook
    /// (`btop_app::signals::AtExitHook::run`); read by T7 `clean_quit`
    /// as the re-entrancy guard.
    pub quitting: Arc<AtomicBool>,
}

impl SignalFlags {
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear the per-tick signal flags (main-loop drain + test helper).
    /// `quitting` is deliberately EXCLUDED: it is the T7 `clean_quit`
    /// re-entrancy guard ("exit already in progress"), and draining it
    /// would let a second exit path re-enter cleanup. All stores use
    /// `Ordering::SeqCst`: the port is single-threaded, but SeqCst keeps
    /// handler/loop/test ordering total without measurable cost here.
    pub fn clear_all(&self) {
        for flag in [
            &self.resized,
            &self.should_quit,
            &self.should_sleep,
            &self.do_continue,
            &self.interrupt_input,
            &self.reload_conf,
            &self.stopping,
        ] {
            flag.store(false, AtomicOrdering::SeqCst);
        }
    }
}

// ── HistoryStore ──────────────────────────────────────────────────────────
// DEVIATION from the plan sketch (`cpu_total/cpu_cores/mem_used/net_down/
// net_up/gpu`): the draw input structs borrow richer shapes, so the store
// mirrors exactly what they borrow — otherwise `assemble_*` could not return
// borrowing inputs:
// - `cpu_total` → `cpu_percent` map (`CpuDrawInput::percent` is
//   `&HashMap<String, VecDeque<i64>>`; at least `"total"` is present).
// - `mem_used` → `mem` map (`MemDrawInput::percent` needs
//   used/available/cached/free/swap_* histories).
// - `net_down/net_up` → `net` map (`NetDrawInput::bandwidth` borrows
//   `&HashMap<String, Vec<i64>>` keyed `"download"`/`"upload"`).
// - `gpu` → `gpu_percent` map + `gpu_temp`/`gpu_mem_util` vecs
//   (`GpuDrawInput` borrows the percent map plus temp/mem-util slices).
// - `cpu_temp` added (`CpuDrawInput::temp` borrows `&[VecDeque<i64>]`,
//   `[0]` = package).
#[derive(Debug, Default)]
pub struct HistoryStore {
    pub cpu_percent: HashMap<String, VecDeque<i64>>,
    pub cpu_cores: Vec<VecDeque<i64>>,
    pub cpu_temp: Vec<VecDeque<i64>>,
    pub mem: HashMap<String, Vec<i64>>,
    pub net: HashMap<String, Vec<i64>>,
    pub gpu_percent: HashMap<String, Vec<i64>>,
    pub gpu_temp: Vec<i64>,
    pub gpu_mem_util: Vec<i64>,
}

// ── AppState ──────────────────────────────────────────────────────────────
#[derive(Debug)]
pub struct AppState {
    // cpu collect state
    pub cpu_old_totals: Vec<i64>,
    pub cpu_old_idles: Vec<i64>,
    pub load_avg: [f64; 3],
    pub temp_max: i64,
    pub usage_watts: f32,
    pub active_cpus: Option<Vec<i32>>,
    pub core_count: u64,
    // cpu draw strings / flags (Config-owned in C++; mirrored here so the
    // returned input can borrow them — `apply_config` refreshes from Config)
    pub cpu_name: String,
    pub custom_cpu_name: String,
    pub cpu_hz: String,
    pub cpu_graph_upper: String,
    pub cpu_graph_lower: String,
    pub available_fields: Vec<String>,
    pub graph_symbol: String,
    pub graph_symbol_cpu: String,
    pub graph_symbol_mem: String,
    pub graph_symbol_net: String,
    pub graph_symbol_gpu: String,
    pub graph_symbol_proc: String,
    pub battery: Option<BatteryState>,
    pub uptime_secs: u64,
    pub term_width: i64,
    pub cpu_flags: CpuFlags,
    // mem state (stats map mirrors `Mem::stats`; histories live in `hist`)
    pub mem_stats: HashMap<String, u64>,
    pub mem_disks: HashMap<String, DiskDraw>,
    pub mem_disks_order: Vec<String>,
    pub total_mem: u64,
    pub has_swap: bool,
    pub disk_ios: i64,
    pub io_graph_speeds: String,
    pub disk_last_io: HashMap<String, (u64, u64)>,
    pub mem_flags: MemFlags,
    // net state: per-iface (down, up) tuples — the caller-owned lifecycle
    // (`update_counter` returns `new_offset`; wiring persists it here).
    // `net_total` is re-derived each tick but persisted for Task 4 reads.
    pub net_last: HashMap<String, (u64, u64)>,
    pub net_rollover: HashMap<String, (u64, u64)>,
    pub net_offset: HashMap<String, (u64, u64)>,
    pub net_top: HashMap<String, (u64, u64)>,
    pub net_total: HashMap<String, (u64, u64)>,
    pub net_stat: HashMap<String, NetStat>,
    pub net_graph_max: HashMap<String, u64>,
    pub selected_iface: String,
    pub net_interfaces: Vec<String>, // Task 3 CycleIface cycles this list
    pub ipv4: String,
    pub ipv6: String,
    pub old_ip: String,
    pub net_connected: bool,
    pub net_download_cfg: i64,
    pub net_upload_cfg: i64,
    pub net_flags: NetFlags,
    // gpu state
    pub gpu_clock_speed: i64,
    pub pwr_usage: i64,
    pub pwr_state: i64,
    pub gpu_temp_max: i64,
    pub gpu_mem_total: u64,
    pub gpu_mem_used: u64,
    pub gpu_mem_clock: i64,
    pub pcie_tx: i64,
    pub pcie_rx: i64,
    pub enc_util: i64,
    pub dec_util: i64,
    pub gpu_supported: GpuSupported,
    pub gpu_name: String,
    pub gpu_panel: i64,
    pub gpu_flags: GpuFlags,
    // proc state
    pub procs: Vec<ProcRaw>,
    pub proc_view: Vec<ProcInfo>,
    pub proc_last_ticks: HashMap<u64, u64>,
    pub last_cputimes: u64,
    /// machTck/clkTck factor (cpp:679-691). P4/live calibration out of
    /// scope — tests inject a fixed factor; tick passes the backend value.
    pub tick_factor: f64,
    /// System page size (`Shared::pageSize`, osx/btop_collect.cpp
    /// `Shared::init`). Probed at boot with the 4096 fallback; the mem
    /// assembler takes its page stride per call, this pins the boot value.
    pub page_size: i64,
    pub proc_sorting: String,
    pub proc_reversed: bool,
    pub proc_tree: bool,
    pub proc_filter: String,
    pub proc_selected: i64,
    pub proc_start: i64,
    /// Reserved for Task 3 scroll math (`Proc::selection` port). Unused here.
    pub proc_scroll_pos: i64,
    pub proc_followed: i64,
    pub followed_pid: i64,
    pub detailed_pid: u64,
    pub restore_pid: i64,
    pub update_following: bool,
    pub should_return: bool,
    pub last_selected: i64,
    pub was_last: bool,
    pub prev_banner: bool,
    pub detail_cpu: VecDeque<i64>,
    pub detail_mem: VecDeque<i64>,
    pub detail: Option<ProcDetail>,
    pub per_core: bool,
    pub proc_flags: ProcFlags,
    // shared
    pub hist: HistoryStore,
    pub mouse_maps: Vec<MouseMap>,
    /// `btop_draw::boxes::Clock` (verified export path — no local duplicate).
    pub clock: Clock,
    pub overlay: String,
    pub force_redraw: bool,
    /// Wiring-side canonical geometry post-calcSizes. Split from the
    /// INPUT-side geometry P1 `process` needs in its ViewState: ViewState
    /// carries the geometry the input layer acts on; this field is what
    /// Task 4 writes after `calc_sizes` and what proc-mouse handling reads.
    pub proc_geom: ProcGeom,
    /// `show_detailed` adjust for `select_max` (draw :1540-1550): whether
    /// the detail pane is open, shrinking the list window. Task 4 maintains.
    pub show_detailed_adj: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            cpu_old_totals: Vec::new(),
            cpu_old_idles: Vec::new(),
            load_avg: [0.0, 0.0, 0.0],
            temp_max: 95, // harness value (draw CpuDrawInput fixture)
            usage_watts: 0.0,
            active_cpus: None,
            core_count: 8, // LayoutInput::defaults core_count
            cpu_name: String::new(),
            custom_cpu_name: String::new(),
            cpu_hz: String::new(),
            cpu_graph_upper: "Auto".to_string(), // btop_config.cpp:285
            cpu_graph_lower: "Auto".to_string(), // btop_config.cpp:286
            available_fields: vec!["total".to_string()],
            graph_symbol: "braille".to_string(),
            graph_symbol_cpu: "default".to_string(),
            graph_symbol_mem: "default".to_string(),
            graph_symbol_net: "default".to_string(),
            graph_symbol_gpu: "default".to_string(),
            graph_symbol_proc: "default".to_string(),
            battery: None,
            uptime_secs: 0,
            term_width: 100,
            cpu_flags: CpuFlags::harness_defaults(),
            mem_stats: HashMap::new(),
            mem_disks: HashMap::new(),
            mem_disks_order: Vec::new(),
            total_mem: 0,
            has_swap: false,
            disk_ios: 0,
            io_graph_speeds: String::new(), // btop_config.cpp:297
            disk_last_io: HashMap::new(),
            mem_flags: MemFlags::harness_defaults(),
            net_last: HashMap::new(),
            net_rollover: HashMap::new(),
            net_offset: HashMap::new(),
            net_top: HashMap::new(),
            net_total: HashMap::new(),
            net_stat: HashMap::new(),
            net_graph_max: HashMap::new(),
            selected_iface: String::new(),
            net_interfaces: Vec::new(),
            ipv4: String::new(),
            ipv6: String::new(),
            old_ip: String::new(),
            net_connected: true,
            net_download_cfg: 100, // btop_config.cpp:386
            net_upload_cfg: 100,   // btop_config.cpp:387
            net_flags: NetFlags::harness_defaults(),
            gpu_clock_speed: 0,
            pwr_usage: 0,
            pwr_state: 8, // draw harness pins 8 (gpu.rs:127-129)
            gpu_temp_max: 95,
            gpu_mem_total: 0,
            gpu_mem_used: 0,
            gpu_mem_clock: 0,
            // 0 shows a `TX:0B/s` footer; negative hides it
            // (manually-disabled semantics, gpu.rs:138-139).
            pcie_tx: 0,
            pcie_rx: 0,
            enc_util: 0,
            dec_util: 0,
            gpu_supported: GpuSupported::all(),
            gpu_name: "GPU".to_string(),
            gpu_panel: 0,
            gpu_flags: GpuFlags::harness_defaults(),
            procs: Vec::new(),
            proc_view: Vec::new(),
            proc_last_ticks: HashMap::new(),
            last_cputimes: 0,
            tick_factor: 1.0,
            page_size: 4096,
            proc_sorting: "cpu lazy".to_string(), // btop_config.cpp:284
            proc_reversed: false,
            proc_tree: false,
            proc_filter: String::new(),
            proc_selected: 0,
            proc_start: 0,
            proc_scroll_pos: 0,
            proc_followed: 0,
            followed_pid: 0,
            detailed_pid: 0,
            restore_pid: 0,
            update_following: false,
            should_return: false,
            last_selected: 0,
            was_last: false,
            prev_banner: false,
            detail_cpu: VecDeque::new(),
            detail_mem: VecDeque::new(),
            detail: None,
            per_core: false,
            proc_flags: ProcFlags::harness_defaults(),
            hist: HistoryStore::default(),
            mouse_maps: Vec::new(),
            clock: Clock {
                time: String::new(),
                date: String::new(),
            },
            overlay: String::new(),
            force_redraw: false,
            proc_geom: ProcGeom {
                base: BoxGeom {
                    x: 1,
                    y: 1,
                    width: 0,
                    height: 0,
                    shown: false,
                },
                select_max: 0,
            },
            show_detailed_adj: false,
        }
    }
}

// ── Config → flags ────────────────────────────────────────────────────────
// `btop_config::Config` has no compiled-in defaults map (`Config::new()` is
// empty); each field falls back to the current value (which `Default`
// seeds from the harness/C++ defaults cited there). Only keys present in
// the maps override — missing keys keep current values.

fn get_s<'a>(cfg: &'a Config, cur: &'a str, key: &str) -> &'a str {
    cfg.get_s(key).unwrap_or(cur)
}

fn get_b(cfg: &Config, cur: bool, key: &str) -> bool {
    cfg.get_b(key).unwrap_or(cur)
}

impl AppState {
    /// Refresh mirrored strings/flags from Config (Task 4 calls per tick or
    /// on ReloadConfig; tests inject keys via direct map insert since
    /// `set_*` rejects unknown keys).
    pub fn apply_config(&mut self, cfg: &Config) {
        let c = &self.cpu_flags.common;
        let common = btop_draw::boxes::CommonFlags {
            tty_mode: get_b(cfg, c.tty_mode, "tty_mode"),
            rounded: get_b(cfg, c.rounded, "rounded_corners"),
            lowcolor: get_b(cfg, c.lowcolor, "lowcolor"),
            theme_background: get_b(cfg, c.theme_background, "theme_background"),
            temp_scale: get_s(cfg, &c.temp_scale, "temp_scale").to_string(),
        };
        let base_10 = get_b(cfg, false, "base_10_sizes");
        // NOTE: `cpu_name` is runtime (`Cpu::cpuName` from the OS), not
        // Config — only `custom_cpu_name` is mirrored here; `cpu_name` is
        // never touched so per-tick `apply_config` calls cannot erase it.
        self.custom_cpu_name = get_s(cfg, &self.custom_cpu_name, "custom_cpu_name").to_string();
        self.cpu_graph_upper = get_s(cfg, &self.cpu_graph_upper, "cpu_graph_upper").to_string();
        self.cpu_graph_lower = get_s(cfg, &self.cpu_graph_lower, "cpu_graph_lower").to_string();
        self.graph_symbol = get_s(cfg, &self.graph_symbol, "graph_symbol").to_string();
        for (dst, key) in [
            (&mut self.graph_symbol_cpu, "graph_symbol_cpu"),
            (&mut self.graph_symbol_mem, "graph_symbol_mem"),
            (&mut self.graph_symbol_net, "graph_symbol_net"),
            (&mut self.graph_symbol_gpu, "graph_symbol_gpu"),
            (&mut self.graph_symbol_proc, "graph_symbol_proc"),
        ] {
            *dst = get_s(cfg, dst, key).to_string();
        }
        let f = &self.cpu_flags;
        self.cpu_flags = CpuFlags {
            check_temp: get_b(cfg, f.check_temp, "check_temp"),
            got_sensors: f.got_sensors, // runtime, not Config
            cpu_temp_only: get_b(cfg, f.cpu_temp_only, "cpu_temp_only"),
            show_coretemp: get_b(cfg, f.show_coretemp, "show_coretemp"),
            single_graph: get_b(cfg, f.single_graph, "cpu_single_graph"),
            invert_lower: get_b(cfg, f.invert_lower, "cpu_invert_lower"),
            show_watts_cfg: get_b(cfg, f.show_watts_cfg, "show_cpu_watts"),
            supports_watts: f.supports_watts, // runtime
            show_freq_cfg: get_b(cfg, f.show_freq_cfg, "show_cpu_freq"),
            has_cpu_hz: f.has_cpu_hz, // runtime
            freq_range: f.freq_range, // runtime
            show_uptime: get_b(cfg, f.show_uptime, "show_uptime"),
            show_battery_cfg: get_b(cfg, f.show_battery_cfg, "show_battery"),
            has_battery: f.has_battery, // runtime
            show_battery_watts: get_b(cfg, f.show_battery_watts, "show_battery_watts"),
            common: common.clone(),
            cpu_bottom: get_b(cfg, f.cpu_bottom, "cpu_bottom"),
            follow_process: get_b(cfg, f.follow_process, "follow_process"),
            proc_tree: get_b(cfg, f.proc_tree, "proc_tree"),
            followed_pid: f.followed_pid,     // runtime
            detailed_pid: f.detailed_pid,     // runtime
            proc_selected: f.proc_selected,   // runtime
            update_ms: f.update_ms,           // tick-owned; not refreshed here
            current_preset: f.current_preset, // runtime
            show_gpu: f.show_gpu,             // runtime
        };
        let f = &self.mem_flags;
        self.mem_flags = MemFlags {
            show_swap: get_b(cfg, f.show_swap, "show_swap"),
            swap_disk: get_b(cfg, f.swap_disk, "swap_disk"),
            show_disks: get_b(cfg, f.show_disks, "show_disks"),
            show_io_stat: get_b(cfg, f.show_io_stat, "show_io_stat"),
            io_mode: get_b(cfg, f.io_mode, "io_mode"),
            io_graph_combined: get_b(cfg, f.io_graph_combined, "io_graph_combined"),
            use_graphs: get_b(cfg, f.use_graphs, "mem_graphs"),
            base_10,
            common: common.clone(),
        };
        self.io_graph_speeds = get_s(cfg, &self.io_graph_speeds, "io_graph_speeds").to_string();
        let f = &self.net_flags;
        self.net_flags = NetFlags {
            net_sync: get_b(cfg, f.net_sync, "net_sync"),
            net_auto: get_b(cfg, f.net_auto, "net_auto"),
            swap_upload_download: get_b(cfg, f.swap_upload_download, "swap_upload_download"),
            base_10,
            common: common.clone(),
        };
        if let Some(v) = cfg.get_i("net_download") {
            self.net_download_cfg = v;
        }
        if let Some(v) = cfg.get_i("net_upload") {
            self.net_upload_cfg = v;
        }
        let f = &self.gpu_flags;
        self.gpu_flags = GpuFlags {
            check_temp: get_b(cfg, f.check_temp, "check_temp"),
            mirror_graph: get_b(cfg, f.mirror_graph, "gpu_mirror_graph"),
            invert_lower: get_b(cfg, f.invert_lower, "cpu_invert_lower"),
            base_10,
            common: common.clone(),
        };
        let f = &self.proc_flags;
        self.proc_flags = ProcFlags {
            proc_tree: get_b(cfg, f.proc_tree, "proc_tree"),
            proc_colors: get_b(cfg, f.proc_colors, "proc_colors"),
            proc_gradient: get_b(cfg, f.proc_gradient, "proc_gradient"),
            mem_bytes: get_b(cfg, f.mem_bytes, "proc_mem_bytes"),
            vim_keys: get_b(cfg, f.vim_keys, "vim_keys"),
            show_graphs: get_b(cfg, f.show_graphs, "proc_cpu_graphs"),
            pause_proc_list: get_b(cfg, f.pause_proc_list, "pause_proc_list"),
            follow_process: get_b(cfg, f.follow_process, "follow_process"),
            per_core: get_b(cfg, f.per_core, "proc_per_core"),
            reversed: get_b(cfg, f.reversed, "proc_reversed"),
            filtering: get_b(cfg, f.filtering, "proc_filtering"),
            base_10,
            common,
        };
        self.proc_sorting = get_s(cfg, &self.proc_sorting, "proc_sorting").to_string();
        self.proc_tree = self.proc_flags.proc_tree;
        self.proc_reversed = self.proc_flags.reversed;
        self.per_core = self.proc_flags.per_core;
    }
}

// ── assemble fns ──────────────────────────────────────────────────────────

/// One disk's raw sample (backend-shaped; `mount` keys `disk_raw`).
pub struct DiskSample {
    pub mount: String,
    pub name: String,
    pub blocks: u64,
    pub bfree: u64,
    pub frsize: u64,
    pub read_bytes: u64,
    pub write_bytes: u64,
}

pub fn assemble_cpu<'a>(
    state: &'a mut AppState,
    ticks: &[[u64; 4]],
    load: [f64; 3],
    width: usize,
    temp_package: Option<i64>,
    temp_cores: &[i64],
) -> CpuDrawInput<'a> {
    // DEVIATION from the sketch (`ticks, load, width` only): draw borrows
    // temp histories and the backend owns the sensors, so the package/core
    // temps ride along as extra params (None/empty = sensor gate closed,
    // histories keep their old values).
    let n = ticks.len();
    // Core count follows the tick length (C++ `Shared::coreCount` is fixed
    // at init and ticks always match it); this keeps `cmult` in
    // `assemble_proc` consistent without a second caller-owned update.
    state.core_count = n as u64;
    if state.cpu_old_totals.len() != n {
        state.cpu_old_totals.resize(n, 0);
        state.cpu_old_idles.resize(n, 0);
    }
    if state.hist.cpu_cores.len() != n {
        state.hist.cpu_cores.resize_with(n, VecDeque::new);
    }
    if state.hist.cpu_temp.len() != n + 1 {
        state.hist.cpu_temp.resize_with(n + 1, VecDeque::new);
    }
    let old_tot: i64 = state.cpu_old_totals.iter().sum();
    let old_idle: i64 = state.cpu_old_idles.iter().sum();
    let mut new_tot = 0i64;
    let mut new_idle = 0i64;
    for (i, t) in ticks.iter().enumerate() {
        let (pct, tot, idle) = update_core(state.cpu_old_totals[i], state.cpu_old_idles[i], *t);
        state.cpu_old_totals[i] = tot;
        state.cpu_old_idles[i] = idle;
        new_tot += tot;
        new_idle += idle;
        push_trimmed(&mut state.hist.cpu_cores[i], pct, 40); // cpp:1071
    }
    // Latest global delta, recorded for Task 4 (tick derives the proc
    // `delta_total` from the same tick sums; P4/live calibration of the
    // absolute `cputimes` base is out of scope).
    state.last_cputimes = (new_tot - old_tot).max(0) as u64;
    let total_pct = update_total(old_tot, old_idle, new_tot, new_idle);
    let cap = (width * 2).max(1); // cpp:1088,1100 cpu fields cap width*2
    push_trimmed(
        state
            .hist
            .cpu_percent
            .entry("total".to_string())
            .or_default(),
        total_pct,
        cap,
    );
    state.load_avg = load;
    if let Some(pkg) = temp_package {
        push_trimmed(&mut state.hist.cpu_temp[0], pkg, 20); // cpp:870
    }
    if !temp_cores.is_empty() {
        let ns = temp_cores.len();
        for i in 0..n {
            // Collect-side interleave (cpp:880).
            let t = temp_cores[sensor_index(i, n, ns).min(ns - 1)];
            push_trimmed(&mut state.hist.cpu_temp[i + 1], t, 20);
        }
    }
    CpuDrawInput {
        percent: &state.hist.cpu_percent,
        cores: &state.hist.cpu_cores,
        temp: &state.hist.cpu_temp,
        temp_max: state.temp_max,
        load_avg: state.load_avg,
        usage_watts: state.usage_watts,
        active_cpus: state.active_cpus.as_deref(),
        core_count: n,
        cpu_name: &state.cpu_name,
        custom_cpu_name: &state.custom_cpu_name,
        cpu_hz: &state.cpu_hz,
        container_engine: None, // P4/live fills from Cpu::container_engine
        graph_up_cfg: &state.cpu_graph_upper,
        graph_lo_cfg: &state.cpu_graph_lower,
        available_fields: &state.available_fields,
        graph_symbol_cfg: &state.graph_symbol,
        graph_symbol_cpu_cfg: &state.graph_symbol_cpu,
        flags: state.cpu_flags.clone(),
        battery: state.battery.clone(),
        uptime_secs: state.uptime_secs,
        term_width: state.term_width,
        force_redraw: state.force_redraw,
        // Stateless wiring always re-renders; change-detection is P4 work.
        data_same: false,
        prev: None,
    }
}

pub fn assemble_net<'a>(
    state: &'a mut AppState,
    counters: &[(String, u64, u64)],
    dt_ms: u64,
    width: usize,
) -> NetDrawInput<'a> {
    // Full interface list every tick — Task 3 CycleIface cycles it.
    state.net_interfaces = counters.iter().map(|(n, _, _)| n.clone()).collect();
    let pick = counters
        .iter()
        .position(|(n, _, _)| *n == state.selected_iface)
        .or_else(|| counters.first().map(|_| 0));
    if let Some(i) = pick {
        let (name, down, up) = (counters[i].0.clone(), counters[i].1, counters[i].2);
        let (ld, lu) = state.net_last.get(&name).copied().unwrap_or((0, 0));
        let (rd, ru) = state.net_rollover.get(&name).copied().unwrap_or((0, 0));
        let (od, ou) = state.net_offset.get(&name).copied().unwrap_or((0, 0));
        let (td, tu) = state.net_top.get(&name).copied().unwrap_or((0, 0));
        // `update_counter` 4-tuple wired with the caller-owned lifecycle:
        // every returned rollover/offset is persisted back below.
        // Cold start (no entry): last = 0, so the first speed is the
        // delta-from-zero — same as the C++ zero-initialised stat; P4 may
        // add a priming tick.
        let (speed_d, total_d, roll_d, off_d) = update_counter(down, ld, rd, od, dt_ms);
        let (speed_u, total_u, roll_u, off_u) = update_counter(up, lu, ru, ou, dt_ms);
        let top_d = track_top(speed_d, td);
        let top_u = track_top(speed_u, tu);
        state.net_last.insert(name.clone(), (down, up));
        state.net_rollover.insert(name.clone(), (roll_d, roll_u));
        state.net_offset.insert(name.clone(), (off_d, off_u));
        state.net_top.insert(name.clone(), (top_d, top_u));
        state.net_total.insert(name.clone(), (total_d, total_u));
        let cap = (width * 2).max(1);
        push_trim_vec(
            state.hist.net.entry("download".to_string()).or_default(),
            speed_d as i64,
            cap,
        );
        push_trim_vec(
            state.hist.net.entry("upload".to_string()).or_default(),
            speed_u as i64,
            cap,
        );
        state.net_stat.insert(
            "download".to_string(),
            NetStat {
                speed: speed_d,
                top: top_d,
                total: total_d,
                offset: off_d,
            },
        );
        state.net_stat.insert(
            "upload".to_string(),
            NetStat {
                speed: speed_u,
                top: top_u,
                total: total_u,
                offset: off_u,
            },
        );
        // Auto-scale ceilings: max-observed (monotonic) approximation —
        // C++ decays/rounds the ceiling, which needs cross-tick latent
        // state Task 4 can refine.
        for (dir, speed) in [("download", speed_d), ("upload", speed_u)] {
            let e = state.net_graph_max.entry(dir.to_string()).or_insert(0);
            *e = (*e).max(speed);
        }
    }
    NetDrawInput {
        bandwidth: &state.hist.net,
        stat: &state.net_stat,
        ipv4: &state.ipv4,
        ipv6: &state.ipv6,
        connected: state.net_connected,
        selected_iface: &state.selected_iface,
        graph_max: &state.net_graph_max,
        net_download_cfg: state.net_download_cfg,
        net_upload_cfg: state.net_upload_cfg,
        graph_symbol_cfg: &state.graph_symbol,
        graph_symbol_net_cfg: &state.graph_symbol_net,
        old_ip: &state.old_ip,
        flags: state.net_flags.clone(),
        force_redraw: state.force_redraw,
        data_same: false,
        prev: None,
    }
}

/// Push + trim for the `Vec<i64>` histories (`VecDeque` has
/// [`push_trimmed`](btop_collect::cpu::push_trimmed); plain vecs need the
/// same cap rule for the mem/net/gpu/detail maps).
fn push_trim_vec(h: &mut Vec<i64>, v: i64, cap: usize) {
    h.push(v);
    if h.len() > cap {
        h.drain(..h.len() - cap);
    }
}

pub fn assemble_mem<'a>(
    state: &'a mut AppState,
    vm: (u64, u64, u64, u64, u64),
    swap: (u64, u64, u64),
    total_mem: u64,
    disks: &[DiskSample],
    width: usize,
) -> MemDrawInput<'a> {
    let (active, wired, free_pages, external, page) = vm;
    let derived = vm_stats(active, wired, free_pages, external, page, total_mem);
    let (swap_total, _swap_avail, swap_used) = swap;
    let swap_free = swap_total.saturating_sub(swap_used);
    state.total_mem = total_mem;
    state.has_swap = swap_total > 0;
    for (k, v) in [
        ("used", derived.used),
        ("available", derived.avail),
        ("cached", derived.cached),
        ("free", derived.free),
        ("swap_total", swap_total),
        ("swap_used", swap_used),
        ("swap_free", swap_free),
    ] {
        state.mem_stats.insert(k.to_string(), v);
    }
    let cap = (width * 2).max(1);
    for (k, v, t) in [
        ("used", derived.used, total_mem),
        ("available", derived.avail, total_mem),
        ("cached", derived.cached, total_mem),
        ("free", derived.free, total_mem),
        ("swap_used", swap_used, swap_total),
        ("swap_free", swap_free, swap_total),
    ] {
        push_trim_vec(
            state.hist.mem.entry(k.to_string()).or_default(),
            mem_percent(v, t),
            cap,
        );
    }
    state.mem_disks_order = disks.iter().map(|d| d.mount.clone()).collect();
    let mut ios = 0i64;
    for d in disks {
        let usage = disk_usage(d.blocks, d.bfree, d.frsize);
        let (last_r, last_w) = state.disk_last_io.get(&d.mount).copied().unwrap_or((0, 0));
        let rd = io_delta(d.read_bytes, last_r);
        let wd = io_delta(d.write_bytes, last_w);
        state
            .disk_last_io
            .insert(d.mount.clone(), (d.read_bytes, d.write_bytes));
        let entry = state
            .mem_disks
            .entry(d.mount.clone())
            .or_insert_with(|| DiskDraw {
                name: d.name.clone(),
                total: 0,
                used: 0,
                free: 0,
                used_percent: 0,
                free_percent: 0,
                io_read: Vec::new(),
                io_write: Vec::new(),
                io_activity: Vec::new(),
            });
        entry.name = d.name.clone();
        entry.total = usage.total;
        entry.used = usage.used;
        entry.free = usage.free;
        entry.used_percent = usage.used_percent;
        entry.free_percent = mem_percent(usage.free, usage.total);
        push_trim_vec(&mut entry.io_read, rd as i64, cap);
        push_trim_vec(&mut entry.io_write, wd as i64, cap);
        push_trim_vec(&mut entry.io_activity, io_activity(rd, wd), cap);
        if !entry.io_read.is_empty() {
            ios += 1;
        }
    }
    // `Mem::disk_ios`: disks carrying io series (drives the io-mode graph
    // height, mem.rs:1285-1287).
    state.disk_ios = ios;
    MemDrawInput {
        stats: &state.mem_stats,
        percent: &state.hist.mem,
        disks: &state.mem_disks,
        disks_order: &state.mem_disks_order,
        total_mem: state.total_mem,
        has_swap: state.has_swap,
        disk_ios: state.disk_ios,
        io_graph_speeds: &state.io_graph_speeds,
        graph_symbol_cfg: &state.graph_symbol,
        graph_symbol_mem_cfg: &state.graph_symbol_mem,
        flags: state.mem_flags.clone(),
        force_redraw: state.force_redraw,
        data_same: false,
        prev: None,
    }
}

/// GPU assembly from backend-shaped samples (`residency`/`clocks` rows,
/// an `energy` reading, vram counters, one temp). `width` caps histories
/// (see [`assemble_cpu`]).
#[allow(clippy::too_many_arguments)]
pub fn assemble_gpu<'a>(
    state: &'a mut AppState,
    residency: &[(String, u64)],
    clocks: &[(String, u64, u64)],
    energy: (u64, EnergyUnit),
    dt_ms: u64,
    vram_used: u64,
    vram_total: u64,
    temp_c: f64,
    width: usize,
) -> GpuDrawInput<'a> {
    let util = gpu_util(residency);
    let clock = gpu_clock(clocks);
    let power = power_mw(energy.0, energy.1, dt_ms);
    let power_i = power.min(i64::MAX as u64) as i64;
    let temp = sensor_avg(&[temp_c]);
    let vram_pct = mem_percent(vram_used, vram_total);
    let cap = (width * 2).max(1);
    push_trim_vec(
        state
            .hist
            .gpu_percent
            .entry("gpu-totals".to_string())
            .or_default(),
        util,
        cap,
    );
    push_trim_vec(
        state
            .hist
            .gpu_percent
            .entry("gpu-vram-totals".to_string())
            .or_default(),
        vram_pct,
        cap,
    );
    // No TDP basis is ported, so the pwr "percent" is the clamped raw mW —
    // keeps the meter in range; P4/live can rescale once a max is known.
    push_trim_vec(
        state
            .hist
            .gpu_percent
            .entry("gpu-pwr-totals".to_string())
            .or_default(),
        power_i.clamp(0, 100),
        cap,
    );
    push_trim_vec(&mut state.hist.gpu_temp, temp, cap);
    push_trim_vec(&mut state.hist.gpu_mem_util, vram_pct, cap);
    state.gpu_clock_speed = clock as i64;
    state.pwr_usage = power_i;
    state.gpu_mem_total = vram_total;
    state.gpu_mem_used = vram_used;
    GpuDrawInput {
        percent: &state.hist.gpu_percent,
        gpu_clock_speed: state.gpu_clock_speed,
        pwr_usage: state.pwr_usage,
        pwr_state: state.pwr_state,
        temp: &state.hist.gpu_temp,
        temp_max: state.gpu_temp_max,
        mem_total: state.gpu_mem_total,
        mem_used: state.gpu_mem_used,
        mem_utilization: &state.hist.gpu_mem_util,
        mem_clock_speed: state.gpu_mem_clock,
        pcie_tx: state.pcie_tx,
        pcie_rx: state.pcie_rx,
        encoder_utilization: state.enc_util,
        decoder_utilization: state.dec_util,
        supported: state.gpu_supported.clone(),
        gpu_name: &state.gpu_name,
        panel: state.gpu_panel,
        graph_symbol_cfg: &state.graph_symbol,
        graph_symbol_gpu_cfg: &state.graph_symbol_gpu,
        flags: state.gpu_flags.clone(),
        force_redraw: state.force_redraw,
        data_same: false,
        prev: None,
    }
}

pub fn assemble_proc(
    state: &mut AppState,
    raws: Vec<ProcRaw>,
    delta_total: u64,
    width: usize,
) -> ProcDrawInput<'_> {
    // `cmult` feeds per-PROC cpu% (cpp:1758 `per_core ? coreCount : 1`) —
    // deliberately computed HERE, not in assemble_cpu.
    let cmult = if state.per_core {
        (state.core_count as i64).max(1)
    } else {
        1
    };
    let ncore = state.core_count;
    let factor = state.tick_factor;
    let mut ordered: Vec<ProcInfo> = Vec::with_capacity(raws.len());
    for raw in &raws {
        // First sighting seeds the baseline (delta 0 → cpu 0); the delta
        // forms on the next tick, same as the C++ no-history start.
        let last = state
            .proc_last_ticks
            .get(&raw.pid)
            .copied()
            .unwrap_or(raw.cpu_ticks);
        let delta = raw.cpu_ticks.saturating_sub(last);
        let cpu_p = proc_cpu_percent(delta, delta_total, factor, cmult, ncore);
        state.proc_last_ticks.insert(raw.pid, raw.cpu_ticks);
        ordered.push(ProcInfo {
            pid: raw.pid,
            name: raw.name.clone(),
            // P4/live fills cmd/user/nice/prefix from the OS; headless
            // tests only carry name/mem/threads.
            cmd: raw.name.clone(),
            short_cmd: raw.name.clone(),
            threads: raw.threads,
            user: String::new(),
            mem: raw.mem_bytes,
            cpu_p,
            p_nice: 0,
            prefix: String::new(),
            tree_index: 0,
        });
    }
    state.procs = raws;
    // Minimal headless sort-key set ("cpu lazy" default contains "cpu").
    // P4/live may pre-sort richer C++ keys; the match arms stay prefix-free
    // on purpose (unknown keys fall back to pid, never panic).
    match state.proc_sorting.as_str() {
        s if s.contains("cpu") => {
            ordered.sort_by(|a, b| b.cpu_p.partial_cmp(&a.cpu_p).unwrap_or(Ordering::Equal))
        }
        s if s.contains("mem") => ordered.sort_by_key(|a| Reverse(a.mem)),
        s if s.contains("program") || s.contains("name") => {
            ordered.sort_by(|a, b| a.name.cmp(&b.name))
        }
        _ => ordered.sort_by_key(|a| a.pid),
    }
    if state.proc_reversed {
        ordered.reverse();
    }
    let numpids = ordered.len() as i64;
    for (i, p) in ordered.iter_mut().enumerate() {
        // All visible headlessly; P4/live assigns the `== len` hidden
        // sentinel from its ppid walk in tree mode.
        p.tree_index = i;
    }
    // Detail deques track `detailed_pid` across ticks.
    let cap = (width * 2).max(1);
    if state.detailed_pid != 0 {
        if let Some(entry) = ordered.iter().find(|p| p.pid == state.detailed_pid) {
            push_trimmed(&mut state.detail_cpu, entry.cpu_p.round() as i64, cap);
            push_trimmed(
                &mut state.detail_mem,
                entry.mem.min(i64::MAX as u64) as i64,
                cap,
            );
            let detail = state.detail.get_or_insert_with(|| ProcDetail {
                entry: entry.clone(),
                status: "Running".to_string(),
                elapsed: String::new(),
                parent: String::new(),
                io_read: String::new(),
                io_write: String::new(),
                memory: String::new(),
                first_mem: -1,
                cpu_history: Vec::new(),
                mem_history: Vec::new(),
            });
            detail.entry = entry.clone();
            detail.cpu_history = state.detail_cpu.iter().copied().collect();
            detail.mem_history = state.detail_mem.iter().copied().collect();
        }
    } else {
        state.detail = None;
    }
    // List-mode filtering reuses the draw-side matcher; tree-mode
    // filtering is collect-side (`_tree_gen`), so tree mode skips it here.
    let view: Vec<ProcInfo> = if state.proc_tree || state.proc_filter.is_empty() {
        ordered
    } else {
        ordered
            .into_iter()
            .filter(|p| matches_filter(p, state.proc_filter.as_str()))
            .collect()
    };
    state.proc_view = view;
    ProcDrawInput {
        procs: &state.proc_view,
        numpids,
        total_mem: state.total_mem,
        sorting: &state.proc_sorting,
        start: state.proc_start,
        selected: state.proc_selected,
        followed: state.proc_followed,
        followed_pid: state.followed_pid,
        detailed_pid: state.detailed_pid as i64,
        restore_pid: state.restore_pid,
        update_following: state.update_following,
        should_return: state.should_return,
        last_selected: state.last_selected,
        was_last: state.was_last,
        prev_banner: state.prev_banner,
        filter: if state.proc_filter.is_empty() {
            None
        } else {
            Some(state.proc_filter.as_str())
        },
        detailed: state.detail.as_ref(),
        graph_symbol_cfg: &state.graph_symbol,
        graph_symbol_proc_cfg: &state.graph_symbol_proc,
        flags: state.proc_flags.clone(),
        force_redraw: state.force_redraw,
        data_same: false,
        prev: None,
    }
}

/// Merge box maps + menu maps in C++ order: boxes draw first, the menu
/// overlay last (`Input::mouse_mappings` is a single keyed map, so overlay
/// entries overwrite box entries on collision). The Vec form keeps both;
/// lookups must use last-wins to match.
pub fn assemble_mouse(box_maps: &[MouseMap], menu_maps: &[MouseMap]) -> Vec<MouseMap> {
    box_maps.iter().chain(menu_maps.iter()).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cpu_state() -> AppState {
        let mut s = AppState::default();
        s.cpu_old_totals = vec![1000];
        s.cpu_old_idles = vec![800];
        s.core_count = 1;
        s
    }

    #[test]
    fn cpu_percent_flows_into_input_and_history() {
        let mut s = cpu_state();
        // prev_tot=1000, prev_idle=800; cur=[150,10,100,840] sums 1100/840.
        let input = assemble_cpu(
            &mut s,
            &[[150, 10, 100, 840]],
            [1.0, 2.0, 3.0],
            50,
            None,
            &[],
        );
        assert_eq!(input.load_avg, [1.0, 2.0, 3.0]);
        let total_back = *input.percent.get("total").unwrap().back().unwrap();
        let core_count = input.core_count;
        drop(input);
        assert_eq!(total_back, 60);
        assert_eq!(core_count, 1);
        assert_eq!(s.cpu_old_totals, vec![1100]);
        assert_eq!(*s.hist.cpu_cores[0].back().unwrap(), 60);
    }

    #[test]
    fn cpu_history_trims_to_width_cap() {
        let mut s = cpu_state();
        s.hist
            .cpu_percent
            .insert("total".to_string(), (0..120).collect());
        let _ = assemble_cpu(
            &mut s,
            &[[150, 10, 100, 840]],
            [0.0, 0.0, 0.0],
            50,
            None,
            &[],
        );
        // cap = width*2 = 100 (cpp:1088,1100).
        assert_eq!(s.hist.cpu_percent.get("total").unwrap().len(), 100);
    }

    #[test]
    fn cpu_temps_push_package_and_interleaved_cores() {
        let mut s = cpu_state();
        let _ = assemble_cpu(
            &mut s,
            &[[150, 10, 100, 840]],
            [0.0, 0.0, 0.0],
            50,
            Some(55),
            &[60],
        );
        assert_eq!(*s.hist.cpu_temp[0].back().unwrap(), 55);
        assert_eq!(*s.hist.cpu_temp[1].back().unwrap(), 60);
    }

    #[test]
    fn net_two_call_speed_from_persisted_last() {
        let mut s = AppState::default();
        s.selected_iface = "eth0".to_string();
        let c1 = vec![("eth0".to_string(), 1_000_000u64, 500_000u64)];
        let _ = assemble_net(&mut s, &c1, 1000, 50);
        let c2 = vec![("eth0".to_string(), 1_002_000u64, 500_500u64)];
        let input = assemble_net(&mut s, &c2, 1000, 50);
        let down_speed = input.stat.get("download").unwrap().speed;
        let up_speed = input.stat.get("upload").unwrap().speed;
        let down_total = input.stat.get("download").unwrap().total;
        drop(input);
        assert_eq!(down_speed, 2000);
        assert_eq!(up_speed, 500);
        assert_eq!(down_total, 1_002_000);
        // history got both speeds appended.
        assert_eq!(s.hist.net.get("download").unwrap().len(), 2);
    }

    #[test]
    fn net_offset_persists_and_resets() {
        let mut s = AppState::default();
        s.selected_iface = "eth0".to_string();
        s.net_offset.insert("eth0".to_string(), (500, 0));
        let c = vec![("eth0".to_string(), 1_002_000u64, 0u64)];
        let input = assemble_net(&mut s, &c, 1000, 50);
        let total = input.stat.get("download").unwrap().total;
        let offset = input.stat.get("download").unwrap().offset;
        drop(input);
        assert_eq!(total, 1_002_000 - 500);
        assert_eq!(offset, 500);
        // offset above val+rollover resets to 0 and persists (cpp:1561).
        s.net_offset.insert("eth0".to_string(), (5_000_000, 0));
        let c2 = vec![("eth0".to_string(), 1_003_000u64, 0u64)];
        let input2 = assemble_net(&mut s, &c2, 1000, 50);
        let offset2 = input2.stat.get("download").unwrap().offset;
        drop(input2);
        assert_eq!(offset2, 0);
        assert_eq!(s.net_offset.get("eth0").unwrap().0, 0);
    }

    #[test]
    fn net_records_interface_list_for_cycle_iface() {
        let mut s = AppState::default();
        let c = vec![
            ("eth0".to_string(), 1u64, 2u64),
            ("wlan0".to_string(), 3u64, 4u64),
        ];
        let _ = assemble_net(&mut s, &c, 1000, 50);
        assert_eq!(
            s.net_interfaces,
            vec!["eth0".to_string(), "wlan0".to_string()]
        );
    }

    fn proc_raw(pid: u64, name: &str, ticks: u64, mem: u64) -> ProcRaw {
        ProcRaw {
            pid,
            name: name.to_string(),
            cpu_ticks: ticks,
            mem_bytes: mem,
            threads: 1,
        }
    }

    #[test]
    fn proc_sorts_filters_and_reverses() {
        let mut s = AppState::default();
        s.tick_factor = 1.0;
        s.core_count = 8;
        s.proc_sorting = "pid".to_string();
        let raws = vec![
            proc_raw(30, "ccc", 1000, 300),
            proc_raw(10, "aaa", 1000, 100),
            proc_raw(20, "bbb", 1000, 200),
        ];
        let input = assemble_proc(&mut s, raws, 8000, 50);
        let pids: Vec<u64> = input.procs.iter().map(|p| p.pid).collect();
        drop(input);
        assert_eq!(pids, vec![10, 20, 30]);

        // reverse flips.
        s.proc_sorting = "pid".to_string();
        s.proc_reversed = true;
        let raws = vec![
            proc_raw(30, "ccc", 1000, 300),
            proc_raw(10, "aaa", 1000, 100),
            proc_raw(20, "bbb", 1000, 200),
        ];
        let input = assemble_proc(&mut s, raws, 8000, 50);
        let pids: Vec<u64> = input.procs.iter().map(|p| p.pid).collect();
        drop(input);
        assert_eq!(pids, vec![30, 20, 10]);

        // filter keeps substring hit (draw-side matcher reused).
        s.proc_reversed = false;
        s.proc_filter = "bb".to_string();
        let raws = vec![
            proc_raw(30, "ccc", 1000, 300),
            proc_raw(10, "aaa", 1000, 100),
            proc_raw(20, "bbb", 1000, 200),
        ];
        let input = assemble_proc(&mut s, raws, 8000, 50);
        let pids: Vec<u64> = input.procs.iter().map(|p| p.pid).collect();
        drop(input);
        assert_eq!(pids, vec![20]);
    }

    #[test]
    fn proc_cpu_p_uses_cmult_and_factor() {
        let mut s = AppState::default();
        s.tick_factor = 1.0;
        s.core_count = 8;
        s.per_core = true;
        s.proc_sorting = "pid".to_string();
        // First tick seeds last map (delta 0 → cpu 0).
        let _ = assemble_proc(&mut s, vec![proc_raw(7, "p", 4000, 100)], 8000, 50);
        // Second tick: delta_proc=4000, delta_total=8000 → A=0.5 → round 1;
        // cmult=8 → 0.008 (matches pure fn).
        let input = assemble_proc(&mut s, vec![proc_raw(7, "p", 8000, 100)], 8000, 50);
        let cpu_p = input.procs[0].cpu_p;
        drop(input);
        let expected = proc_cpu_percent(4000, 8000, 1.0, 8, 8);
        assert_eq!(cpu_p, expected);
        assert_eq!(expected, 0.008);
    }

    #[test]
    fn proc_detail_deques_track_detailed_pid() {
        let mut s = AppState::default();
        s.tick_factor = 1.0;
        s.detailed_pid = 7;
        s.proc_sorting = "pid".to_string();
        let _ = assemble_proc(&mut s, vec![proc_raw(7, "p", 4000, 4096)], 8000, 50);
        let _ = assemble_proc(&mut s, vec![proc_raw(7, "p", 8000, 8192)], 8000, 50);
        assert_eq!(s.detail_cpu.len(), 2);
        assert_eq!(s.detail_mem.len(), 2);
        assert!(s.detail.is_some());
        assert_eq!(s.detail.as_ref().unwrap().entry.pid, 7);
    }

    #[test]
    fn mem_stats_disks_and_history() {
        let mut s = AppState::default();
        // page=4096; active=100, wired=50, free=200, ext=30, total=2_000_000.
        let disks = vec![DiskSample {
            mount: "/".to_string(),
            name: "disk0".to_string(),
            blocks: 1000,
            bfree: 100,
            frsize: 10,
            read_bytes: 1_048_576,
            write_bytes: 0,
        }];
        let input = assemble_mem(
            &mut s,
            (100, 50, 200, 30, 4096),
            (0, 0, 0),
            2_000_000,
            &disks,
            50,
        );
        let used = *input.stats.get("used").unwrap();
        let cached = *input.stats.get("cached").unwrap();
        let d = input.disks.get("/").unwrap().clone();
        drop(input);
        assert_eq!(used, 614_400);
        assert_eq!(cached, 122_880);
        assert_eq!(
            (d.total, d.free, d.used, d.used_percent),
            (10_000, 1_000, 9_000, 90)
        );
        assert_eq!(s.disk_ios, 1);
        assert!(!s.hist.mem.get("used").unwrap().is_empty());
    }

    #[test]
    fn gpu_util_clock_power_flow() {
        let mut s = AppState::default();
        let res = vec![("IDLE".to_string(), 700u64), ("ACTIVE".to_string(), 300u64)];
        let clk = vec![
            ("A".to_string(), 300u64, 600u64),
            ("B".to_string(), 100u64, 1000u64),
        ];
        let input = assemble_gpu(
            &mut s,
            &res,
            &clk,
            (2_000_000, EnergyUnit::Nano),
            1000,
            100,
            1000,
            70.0,
            50,
        );
        assert_eq!(
            *input.percent.get("gpu-totals").unwrap().last().unwrap(),
            30
        );
        assert_eq!(input.gpu_clock_speed, 700);
        assert_eq!(input.pwr_usage, 2);
        assert_eq!(input.mem_total, 1000);
    }

    #[test]
    fn mouse_concat_is_boxes_then_menus() {
        let b = vec![MouseMap {
            x: 1,
            y: 1,
            w: 2,
            h: 1,
            action: "b".to_string(),
        }];
        let m = vec![MouseMap {
            x: 5,
            y: 5,
            w: 2,
            h: 1,
            action: "m".to_string(),
        }];
        let out = assemble_mouse(&b, &m);
        assert_eq!(
            out.iter().map(|e| e.action.as_str()).collect::<Vec<_>>(),
            vec!["b", "m"]
        );
    }

    #[test]
    fn assemble_proc_does_not_touch_scroll_pos() {
        // P3 wiring contract: `assemble_proc` MUST NOT mutate
        // `proc_scroll_pos` — the scroll bar position is owned by the
        // draw layer (computed inside `Proc::draw`, btop_draw.cpp:2192
        // from `proc_start * select_max / (numpids - select_max)`) and
        // Task 4's `apply_scroll` sink helper, NOT by the tick. Pin the
        // invariant here so a future refactor of the tick that lazily
        // "pre-computes" scroll pos cannot silently break the draw
        // side or the sink selection() math (which the trailing-Run
        // suppression in `execute_all` depends on).
        let mut s = AppState::default();
        s.tick_factor = 1.0;
        s.core_count = 8;
        s.proc_sorting = "pid".to_string();
        let raws = vec![
            proc_raw(10, "aaa", 1000, 100),
            proc_raw(20, "bbb", 1000, 200),
        ];
        s.proc_scroll_pos = 7;
        let _ = assemble_proc(&mut s, raws, 8000, 50);
        assert_eq!(
            s.proc_scroll_pos, 7,
            "tick must leave proc_scroll_pos untouched"
        );
    }

    #[test]
    fn apply_config_maps_keys_onto_flags() {
        let mut cfg = Config::new();
        cfg.strings
            .insert("proc_sorting".to_string(), "memory".to_string());
        cfg.strings
            .insert("temp_scale".to_string(), "fahrenheit".to_string());
        cfg.bools.insert("proc_reversed".to_string(), true);
        cfg.bools.insert("proc_per_core".to_string(), true);
        cfg.bools.insert("net_auto".to_string(), false);
        cfg.ints.insert("net_download".to_string(), 50);
        let mut s = AppState::default();
        s.apply_config(&cfg);
        assert_eq!(s.proc_sorting, "memory");
        assert!(s.proc_reversed && s.per_core);
        assert!(!s.net_flags.net_auto);
        assert_eq!(s.net_download_cfg, 50);
        assert_eq!(s.cpu_flags.common.temp_scale, "fahrenheit");
        // missing keys keep current values.
        assert!(s.mem_flags.show_swap);
    }
}
