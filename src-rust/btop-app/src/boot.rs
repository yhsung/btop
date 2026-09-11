//! Boot init chain: config dirs → `Config::load` → `Shared::init` → preset.
//!
//! C++ truth:
//! - config/log/theme dirs: src/btop.cpp:862-925 (`conf_dir` block `:862-885`,
//!   relative theme path `:888-906`, absolute fallbacks `:908-913`, custom
//!   dir `:915-918`);
//! - `init_config`: src/btop.cpp:325-353 (`Config::load` + `lowcolor` +
//!   `log_level` + `proc_filter`);
//! - `Shared::init`: src/osx/btop_collect.cpp `Shared::init` (`coreCount`,
//!   `pageSize`, `machTck`/`clkTck`, `totalMem`, `Cpu::cpuName`, sensor list);
//! - presets: src/btop.cpp:1093 (`presetsValid`) + `:1094-1097` (`cli.preset`
//!   → `current_preset` → `apply_preset`), body src/btop_config.cpp:515-550
//!   (canonical Rust port: `btop_runner::sink::apply_preset`, called here —
//!   not duplicated).
//!
//! `calc_sizes` outlines + `update_ms`/`future_time` (btop.cpp:1099-1106) are
//! T6/T7 territory (geometry + main-loop clock); this module stops at the
//! preset. `Logger::init` (btop.cpp:871-874) has no Rust logger port yet, so
//! load/setup warnings are returned to the caller (T7 routes them).

use std::path::{Path, PathBuf};

use btop_config::cli::Cli;
use btop_runner::sink::{self, World};

/// Default preset list head (`Config::preset_list`, src/btop_config.cpp:473).
const DEFAULT_PRESET: &str = "cpu:0:default,mem:0:default,net:0:default,proc:0:default";

// ── FsOps ───────────────────────────────────────────────────────────────────

/// Filesystem/env boundary for [`init_config_dirs`]. Mocked in tests so no
/// test touches the real home directory or theme installs.
pub trait FsOps {
    /// Candidate config dir (`XDG_CONFIG_HOME/btop` or `~/.config/btop`,
    /// src/btop_config.cpp:403-462 minus the writability triage, which boot
    /// folds into `create_dir_all`). `None` = no base (boot skips the block,
    /// mirroring `get_config_dir` returning `{}`).
    fn config_base(&self) -> Option<PathBuf>;
    /// `create_directories` (src/btop_config.cpp:448, src/btop.cpp:880).
    fn create_dir_all(&self, path: &Path) -> Result<(), String>;
    /// `is_directory` + `R_OK` readable (src/btop.cpp:897, :909).
    fn is_readable_dir(&self, path: &Path) -> bool;
    /// Binary directory (`Global::self_path`, src/btop.cpp:888-906).
    fn exe_dir(&self) -> Option<PathBuf>;
}

/// Live [`FsOps`] (`std::fs` + `std::env`).
pub struct RealFs;

impl FsOps for RealFs {
    fn config_base(&self) -> Option<PathBuf> {
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
            let xdg = PathBuf::from(xdg);
            if xdg.exists() {
                return Some(xdg.join("btop"));
            }
        } else if let Some(home) = std::env::var_os("HOME") {
            let home = PathBuf::from(home);
            if home.exists() {
                return Some(home.join(".config").join("btop"));
            }
        }
        None
    }

    fn create_dir_all(&self, path: &Path) -> Result<(), String> {
        std::fs::create_dir_all(path).map_err(|e| format!("{}: {e}", path.display()))
    }

    fn is_readable_dir(&self, path: &Path) -> bool {
        // No `access(2)` in std; a successful `read_dir` proves the directory
        // is present and readable (DEVIATION from `access(R_OK)`, noted).
        path.is_dir() && std::fs::read_dir(path).is_ok()
    }

    fn exe_dir(&self) -> Option<PathBuf> {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
    }
}

// ── SysProbe ────────────────────────────────────────────────────────────────

/// OS probe boundary for [`shared_init`]. Raw values flow through the C++
/// fallbacks in `shared_init`; a probe reports failure with the out-of-range
/// value (`<= 0` / empty), never with an error.
pub trait SysProbe {
    /// `sysconf(_SC_NPROCESSORS_ONLN)` (osx `Shared::init`).
    fn core_count(&self) -> i64;
    /// `sysconf(_SC_PAGE_SIZE)`.
    fn page_size(&self) -> i64;
    /// `mach_timebase_info` numer/denom ratio (`machTck`).
    fn mach_tick(&self) -> f64;
    /// `sysconf(_SC_CLK_TCK)` (`clkTck`).
    fn clk_tick(&self) -> i64;
    /// `sysctlbyname("hw.memsize")` (`totalMem`); 0 = unknown.
    fn total_mem(&self) -> u64;
    /// `sysctlbyname("machdep.cpu.brand_string")` (raw, untrimmed).
    fn cpu_name_raw(&self) -> String;
}

/// Live [`SysProbe`] (`nix` sysconf/libc only — zero new deps).
pub struct RealProbe;

impl SysProbe for RealProbe {
    fn core_count(&self) -> i64 {
        sysconf_long(nix::libc::_SC_NPROCESSORS_ONLN)
    }

    fn page_size(&self) -> i64 {
        sysconf_long(nix::libc::_SC_PAGE_SIZE)
    }

    fn mach_tick(&self) -> f64 {
        mach_tick_factor()
    }

    fn clk_tick(&self) -> i64 {
        sysconf_long(nix::libc::_SC_CLK_TCK)
    }

    fn total_mem(&self) -> u64 {
        #[cfg(target_os = "macos")]
        {
            sysctl_u64("hw.memsize").unwrap_or(0)
        }
        #[cfg(not(target_os = "macos"))]
        {
            0
        }
    }

    fn cpu_name_raw(&self) -> String {
        #[cfg(target_os = "macos")]
        {
            sysctl_string("machdep.cpu.brand_string").unwrap_or_default()
        }
        #[cfg(not(target_os = "macos"))]
        {
            String::new()
        }
    }
}

/// Raw `libc::sysconf` (`-1` = failure). Used instead of
/// `nix::unistd::sysconf` because nix 0.29 gates `_NPROCESSORS_ONLN` behind
/// `linux_android`, while `_SC_*` constants are available on macOS through
/// `nix::libc` — same call, no new dependency.
fn sysconf_long(name: i32) -> i64 {
    unsafe { nix::libc::sysconf(name) as i64 }
}

/// `mach_timebase_info` numer/denom (`machTck`, osx `Shared::init`).
/// `<= 0.0` = failure (caller falls back to 100.0).
#[cfg(target_os = "macos")]
fn mach_tick_factor() -> f64 {
    // Declared directly against libSystem: `nix::libc` still re-exports the
    // symbol but marks it deprecated (points at the `mach2` crate), and the
    // task budget is zero new dependencies. Layout matches
    // `mach_timebase_info_data_t { uint32_t numer, denom; }`, returns
    // `KERN_SUCCESS (0)` on success.
    #[repr(C)]
    struct TimebaseInfo {
        numer: u32,
        denom: u32,
    }
    extern "C" {
        fn mach_timebase_info(info: *mut TimebaseInfo) -> i32;
    }
    let mut info = TimebaseInfo { numer: 0, denom: 0 };
    let r = unsafe { mach_timebase_info(&mut info) };
    if r == 0 && info.denom != 0 {
        f64::from(info.numer) / f64::from(info.denom)
    } else {
        -1.0
    }
}

#[cfg(not(target_os = "macos"))]
fn mach_tick_factor() -> f64 {
    -1.0
}

/// `sysctlbyname(3)` returning the first 8 bytes as `u64` (little-endian).
#[cfg(target_os = "macos")]
fn sysctl_u64(name: &str) -> Option<u64> {
    let bytes = sysctl_bytes(name)?;
    if bytes.len() < 8 {
        return None;
    }
    let mut arr = [0u8; 8];
    arr.copy_from_slice(&bytes[..8]);
    Some(u64::from_le_bytes(arr))
}

/// `sysctlbyname(3)` as a NUL-trimmed string.
#[cfg(target_os = "macos")]
fn sysctl_string(name: &str) -> Option<String> {
    let bytes = sysctl_bytes(name)?;
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8(bytes[..end].to_vec()).ok()
}

#[cfg(target_os = "macos")]
fn sysctl_bytes(name: &str) -> Option<Vec<u8>> {
    use nix::libc::{c_char, c_void, size_t, sysctlbyname};
    use std::ffi::CString;
    let cname = CString::new(name).ok()?;
    let mut size: size_t = 0;
    // Size probe (`oldp == NULL`).
    let r = unsafe {
        sysctlbyname(
            cname.as_ptr() as *const c_char,
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if r != 0 || size == 0 || size > (1 << 20) {
        return None;
    }
    let mut buf = vec![0u8; size];
    let r = unsafe {
        sysctlbyname(
            cname.as_ptr() as *const c_char,
            buf.as_mut_ptr() as *mut c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if r != 0 {
        return None;
    }
    buf.truncate(size);
    Some(buf)
}

// ── trim_name ───────────────────────────────────────────────────────────────

/// Shorten a raw CPU brand string. Port of `Cpu::trim_name`
/// (src/btop_shared.cpp): Xeon/Intel + `CPU` → the token after `CPU`;
/// Ryzen → `Ryzen` + next two tokens; otherwise strip vendor words
/// (`Processor`, `CPU`, `(R)`, `(TM)`, `Intel`, `AMD`, `Apple`, `Core`).
pub fn trim_name(name: String) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    let has = |t: &str| words.iter().any(|w| *w == t);
    let token_after_cpu = || {
        let pos = words.iter().position(|w| *w == "CPU")?;
        let next = words.get(pos + 1)?;
        if next.ends_with(')') {
            None
        } else {
            Some((*next).to_string())
        }
    };

    let mut out: String;
    if (name.contains("Xeon") || has("Duo")) && has("CPU") {
        out = token_after_cpu().unwrap_or_default();
    } else if has("Ryzen") {
        let ryz = words.iter().position(|w| *w == "Ryzen").unwrap_or(0);
        out = "Ryzen".to_string();
        let mut tokens = 0;
        let mut i = ryz + 1;
        while i < words.len() && tokens < 2 {
            let p = words[i];
            if !matches!(p, "AI" | "PRO" | "H" | "HX") {
                tokens += 1;
            }
            out.push(' ');
            out.push_str(p);
            i += 1;
        }
    } else if name.contains("Intel") && has("CPU") {
        out = token_after_cpu().unwrap_or_default();
    } else {
        out = String::new();
    }

    if out.is_empty() && !words.is_empty() {
        let mut s = String::new();
        for n in &words {
            if *n == "@" {
                break;
            }
            s.push_str(n);
            s.push(' ');
        }
        s.pop();
        for rep in [
            "Processor",
            "CPU",
            "(R)",
            "(TM)",
            "Intel",
            "AMD",
            "Apple",
            "Core",
        ] {
            s = s.replace(rep, "");
        }
        while s.contains("  ") {
            s = s.replace("  ", " ");
        }
        out = s.trim().to_string();
    }
    out
}

// ── init_config_dirs ────────────────────────────────────────────────────────

/// Config/theme directory setup + `Config::load` + cli overrides.
///
/// Mirrors src/btop.cpp:862-925 (dirs) and `init_config` (:325-353).
/// `Ok(warnings)` carries `Config::load` warnings plus any non-fatal setup
/// warning (user-theme `mkdir` failure clears `user_theme_dir`, btop.cpp:880);
/// `Err` is the fatal config-base `mkdir` failure. `Config::load` is skipped
/// when `conf_file` is empty (C++ early return, src/btop_config.cpp:766-767).
///
/// DEVIATIONS: returns the warnings (C++ logs them via `Logger`, which has
/// no Rust port — T7 routes them); readability is `is_dir + read_dir ok`
/// (no `access(R_OK)` in std); `Logger::init` for the log file is out of
/// scope (no logger port); `Global::debug` is not stored (the `log_level`
/// write below carries it).
pub fn init_config_dirs(w: &mut World, cli: &Cli, fs: &dyn FsOps) -> Result<Vec<String>, String> {
    let mut warnings = Vec::new();

    // btop.cpp:866-884 — explicit `--config` skips the whole conf-dir block.
    if let Some(path) = &cli.config_file {
        w.conf_file = path.clone();
    } else if let Some(dir) = fs.config_base() {
        fs.create_dir_all(&dir)
            .map_err(|e| format!("could not create config dir {}: {e}", dir.display()))?;
        w.conf_dir = dir.clone();
        w.conf_file = dir.join("btop.conf");
        w.user_theme_dir = dir.join("themes");
        if let Err(e) = fs.create_dir_all(&w.user_theme_dir) {
            warnings.push(format!(
                "Failed to create user theme directory {}: {e}",
                w.user_theme_dir.display()
            ));
            w.user_theme_dir = PathBuf::new();
        }
    }

    // btop.cpp:893-913 — relative theme path, then absolute fallbacks.
    if let Some(exe) = fs.exe_dir() {
        // C++ `fs::canonical`s the join; do it lexically so the check stays
        // inside `FsOps` (mockable, no fs touch here).
        let rel = normalize(&exe.join("../share/btop/themes"));
        if fs.is_readable_dir(&rel) {
            w.theme_dir = rel;
        }
    }
    if w.theme_dir.as_os_str().is_empty() {
        for path in ["/usr/local/share/btop/themes", "/usr/share/btop/themes"] {
            let p = PathBuf::from(path);
            if fs.is_readable_dir(&p) {
                w.theme_dir = p;
                break;
            }
        }
    }

    // btop.cpp:915-918.
    if let Some(path) = &cli.themes_dir {
        w.custom_theme_dir = path.clone();
    }

    // `init_config` (btop.cpp:325-353).
    if !w.conf_file.as_os_str().is_empty() {
        warnings.extend(w.config.load(&w.conf_file));
    }
    let lowcolor = cli.low_color || !w.config.get_b("truecolor").unwrap_or(false);
    let _ = w.config.set_b("lowcolor", lowcolor);
    if cli.debug {
        w.log_level = "DEBUG".to_string();
    } else if let Some(level) = w.config.get_s("log_level").map(str::to_string) {
        w.log_level = level;
    }
    if let Some(filter) = &cli.filter {
        let _ = w.config.set_s("proc_filter", filter.clone());
    }

    Ok(warnings)
}

/// Lexical `..`/`.` resolution (`fs::canonical` without touching the fs,
/// so the theme scan stays inside the mockable `FsOps` boundary).
fn normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            _ => out.push(c.as_os_str()),
        }
    }
    out
}

// ── shared_init ─────────────────────────────────────────────────────────────

/// Platform probe init. Mirrors osx `Shared::init`: `coreCount` (<1 → 1),
/// `pageSize` (<=0 → 4096), `machTck` (failure → 100.0), `clkTck` (<=0 →
/// 100), `totalMem` (`hw.memsize`, 0 stays 0 with no fallback in C++),
/// `Cpu::cpuName` via `trim_name` (empty stays empty, C++ logs and returns
/// `""`). Writes go to `w.state` (`AppState` already owns `core_count`,
/// `tick_factor = machTck/clkTck`, `total_mem`, `cpu_name`; `page_size` was
/// added minimally for the probe).
///
/// Full sensor-list seeding (`Cpu::got_sensors`, `core_mapping`, IOHID/SMC
/// probes) is an AppleSilicon-only STUB: out of scope per the M2 plan, to be
/// wired when the sensor backend lands. Always returns `Ok` — every probe
/// has its C++ fallback, so there is no fatal path; the `Result` keeps the
/// T7 call-chain shape (`Shared::init` is inside try/catch, btop.cpp:1029).
pub fn shared_init(w: &mut World, probe: &dyn SysProbe) -> Result<(), String> {
    let cores = probe.core_count();
    w.state.core_count = if cores < 1 { 1 } else { cores as u64 };

    let page = probe.page_size();
    w.state.page_size = if page <= 0 { 4096 } else { page };

    let mach = probe.mach_tick();
    let mach = if mach <= 0.0 { 100.0 } else { mach };
    let clk = probe.clk_tick();
    let clk = if clk <= 0 { 100 } else { clk };
    w.state.tick_factor = mach / clk as f64;

    w.state.total_mem = probe.total_mem();
    w.state.cpu_name = trim_name(probe.cpu_name_raw());
    Ok(())
}

// ── apply_preset_default ────────────────────────────────────────────────────

/// Apply the `--preset` default. Mirrors src/btop.cpp:1093-1097:
/// `presetsValid(getS("presets"))` rebuilds the list, the requested index is
/// clamped to the last entry, and `sink::apply_preset` (the canonical
/// src/btop_config.cpp:515-550 port — called, not duplicated) applies it
/// with the terminal-size gate pinned open (`term_ok=true`; the min-size
/// loop is T6). On success sets `w.current_preset` to the clamped index; on
/// `None` (no request) returns `false` with `current_preset` untouched —
/// the caller decides keep-vs-reset.
///
/// The preset index rides as a parameter because `Cli` is not stored in
/// `World` (DEVIATION from the brief's `(&mut World) -> bool` shape, which
/// would need a Cli field on `World`).
pub fn apply_preset_default(w: &mut World, preset: Option<u32>) -> bool {
    let idx = match preset {
        Some(i) => i as usize,
        None => return false,
    };
    // btop.cpp:1093 — validate; on failure C++ keeps the old list, i.e. the
    // default head only (src/btop_config.cpp:476-477, :510).
    let presets = w.config.get_s("presets").unwrap_or("").to_string();
    let mut list = vec![DEFAULT_PRESET.to_string()];
    if w.config.string_valid("presets", &presets) {
        list.extend(
            presets
                .split(' ')
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        );
    }
    let idx = idx.min(list.len().saturating_sub(1));
    let Some(choice) = list.get(idx).cloned() else {
        return false;
    };
    if choice.is_empty() {
        return false;
    }
    let outcome = sink::apply_preset(w, &choice, true);
    if outcome.ok {
        w.current_preset = Some(idx);
    }
    outcome.ok
}
