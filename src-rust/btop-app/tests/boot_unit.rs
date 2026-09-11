//! Boot init-chain headless unit tests (P4 T5).
//!
//! Every scenario drives [`btop_app::boot`] with a [`MockFs`] / [`MockProbe`];
//! no test touches the real filesystem, env, or syscalls, so the suite is
//! hermetic and parallel-safe.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use btop_app::boot::{
    apply_preset_default, init_config_dirs, shared_init, trim_name, FsOps, SysProbe,
};
use btop_config::cli::Cli;
use btop_runner::sink::World;

// ── MockFs ──────────────────────────────────────────────────────────────────

/// Scripted filesystem boundary. `config_base`/`exe_dir` pin the env-derived
/// roots; `readable` pins `is_readable_dir`; `mkdir_fail` pins
/// `create_dir_all` failures. Every creation attempt is recorded.
struct MockFs {
    config_base: Option<PathBuf>,
    exe_dir: Option<PathBuf>,
    readable: HashSet<PathBuf>,
    mkdir_fail: HashSet<PathBuf>,
    created: RefCell<Vec<PathBuf>>,
}

impl MockFs {
    fn new() -> Self {
        Self {
            config_base: None,
            exe_dir: None,
            readable: HashSet::new(),
            mkdir_fail: HashSet::new(),
            created: RefCell::new(Vec::new()),
        }
    }

    fn with_base(mut self, p: &str) -> Self {
        self.config_base = Some(PathBuf::from(p));
        self
    }

    fn with_exe(mut self, p: &str) -> Self {
        self.exe_dir = Some(PathBuf::from(p));
        self
    }

    fn with_readable(mut self, p: &str) -> Self {
        self.readable.insert(PathBuf::from(p));
        self
    }

    fn with_mkdir_fail(mut self, p: &str) -> Self {
        self.mkdir_fail.insert(PathBuf::from(p));
        self
    }
}

impl FsOps for MockFs {
    fn config_base(&self) -> Option<PathBuf> {
        self.config_base.clone()
    }

    fn create_dir_all(&self, path: &Path) -> Result<(), String> {
        self.created.borrow_mut().push(path.to_path_buf());
        if self.mkdir_fail.contains(path) {
            return Err(format!("cannot create {}", path.display()));
        }
        Ok(())
    }

    fn is_readable_dir(&self, path: &Path) -> bool {
        self.readable.contains(path)
    }

    fn exe_dir(&self) -> Option<PathBuf> {
        self.exe_dir.clone()
    }
}

// ── MockProbe ───────────────────────────────────────────────────────────────

/// Scripted [`SysProbe`]: raw values flow through `shared_init`'s C++
/// fallbacks (cores<1 → 1, page<=0 → 4096, mach<=0 → 100, clk<=0 → 100).
struct MockProbe {
    cores: i64,
    page: i64,
    mach: f64,
    clk: i64,
    mem: u64,
    cpu: String,
}

impl MockProbe {
    fn new() -> Self {
        Self {
            cores: 8,
            page: 16384,
            mach: 1.0,
            clk: 100,
            mem: 17_179_869_184,
            cpu: "Apple M1 Pro".to_string(),
        }
    }
}

impl SysProbe for MockProbe {
    fn core_count(&self) -> i64 {
        self.cores
    }
    fn page_size(&self) -> i64 {
        self.page
    }
    fn mach_tick(&self) -> f64 {
        self.mach
    }
    fn clk_tick(&self) -> i64 {
        self.clk
    }
    fn total_mem(&self) -> u64 {
        self.mem
    }
    fn cpu_name_raw(&self) -> String {
        self.cpu.clone()
    }
}

// ── helpers ─────────────────────────────────────────────────────────────────

fn world_with_preset_keys() -> World {
    let mut w = World::default();
    // `World::default` seeds the compiled-in bool keys it knows; the preset
    // position flags are only inserted on demand in sink tests, so mirror
    // that here (C++ always has them via compiled defaults).
    w.config.bools.insert("mem_below_net".into(), false);
    w.config.bools.insert("proc_left".into(), false);
    w
}

// ── init_config_dirs ────────────────────────────────────────────────────────

#[test]
fn config_file_override_skips_dir_block() {
    let mut w = world_with_preset_keys();
    let cli = Cli {
        config_file: Some(PathBuf::from("/tmp/x.conf")),
        ..Cli::default()
    };
    let fs = MockFs::new().with_base("/cfg");
    let warnings = init_config_dirs(&mut w, &cli, &fs).unwrap();
    assert_eq!(w.conf_file, PathBuf::from("/tmp/x.conf"));
    assert!(w.conf_dir.as_os_str().is_empty());
    assert!(w.user_theme_dir.as_os_str().is_empty());
    assert!(fs.created.borrow().is_empty());
    let _ = warnings;
}

#[test]
fn creates_user_theme_dir_and_sets_paths() {
    let mut w = world_with_preset_keys();
    let cli = Cli::default();
    let fs = MockFs::new()
        .with_base("/cfg")
        .with_exe("/opt/btop")
        .with_readable("/opt/share/btop/themes");
    let _ = init_config_dirs(&mut w, &cli, &fs).unwrap();
    assert_eq!(w.conf_dir, PathBuf::from("/cfg"));
    assert_eq!(w.conf_file, PathBuf::from("/cfg/btop.conf"));
    assert_eq!(w.user_theme_dir, PathBuf::from("/cfg/themes"));
    assert_eq!(w.theme_dir, PathBuf::from("/opt/share/btop/themes"));
    let created = fs.created.borrow();
    assert!(created.contains(&PathBuf::from("/cfg")));
    assert!(created.contains(&PathBuf::from("/cfg/themes")));
}

#[test]
fn mkdir_failure_returns_err() {
    let mut w = world_with_preset_keys();
    let cli = Cli::default();
    let fs = MockFs::new().with_base("/cfg").with_mkdir_fail("/cfg");
    let err = init_config_dirs(&mut w, &cli, &fs).unwrap_err();
    assert!(err.contains("/cfg"), "unexpected error: {err}");
}

#[test]
fn theme_dir_falls_back_to_absolute_paths() {
    let mut w = world_with_preset_keys();
    let cli = Cli::default();
    let fs = MockFs::new()
        .with_base("/cfg")
        .with_exe("/opt/btop")
        .with_readable("/usr/local/share/btop/themes");
    let _ = init_config_dirs(&mut w, &cli, &fs).unwrap();
    assert_eq!(w.theme_dir, PathBuf::from("/usr/local/share/btop/themes"));
}

#[test]
fn user_theme_mkdir_failure_clears_dir_but_ok() {
    // btop.cpp:880-884 — clear + warn, keep going.
    let mut w = world_with_preset_keys();
    let fs = MockFs::new()
        .with_base("/cfg")
        .with_mkdir_fail("/cfg/themes");
    let warnings = init_config_dirs(&mut w, &Cli::default(), &fs).unwrap();
    assert!(w.user_theme_dir.as_os_str().is_empty());
    assert!(warnings.iter().any(|m| m.contains("user theme")));
    assert_eq!(w.conf_file, PathBuf::from("/cfg/btop.conf"));
}

#[test]
fn missing_exe_dir_skips_relative_scan() {
    let mut w = world_with_preset_keys();
    let fs = MockFs::new()
        .with_base("/cfg")
        .with_readable("/usr/share/btop/themes");
    let _ = init_config_dirs(&mut w, &Cli::default(), &fs).unwrap();
    assert_eq!(w.theme_dir, PathBuf::from("/usr/share/btop/themes"));
}

#[test]
fn no_theme_dir_leaves_empty() {
    let mut w = world_with_preset_keys();
    let cli = Cli::default();
    let fs = MockFs::new().with_base("/cfg").with_exe("/opt/btop");
    let _ = init_config_dirs(&mut w, &cli, &fs).unwrap();
    assert!(w.theme_dir.as_os_str().is_empty());
}

#[test]
fn custom_themes_dir_from_cli() {
    let mut w = world_with_preset_keys();
    let cli = Cli {
        themes_dir: Some(PathBuf::from("/mine/themes")),
        ..Cli::default()
    };
    let fs = MockFs::new().with_base("/cfg");
    let _ = init_config_dirs(&mut w, &cli, &fs).unwrap();
    assert_eq!(w.custom_theme_dir, PathBuf::from("/mine/themes"));
}

#[test]
fn lowcolor_filter_and_debug_applied() {
    let mut w = world_with_preset_keys();
    let cli = Cli {
        low_color: true,
        filter: Some("ssh".to_string()),
        debug: true,
        ..Cli::default()
    };
    let fs = MockFs::new().with_base("/cfg");
    let _ = init_config_dirs(&mut w, &cli, &fs).unwrap();
    assert_eq!(w.config.get_b("lowcolor"), Some(true));
    assert_eq!(w.config.get_s("proc_filter"), Some("ssh"));
    assert_eq!(w.log_level, "DEBUG");
}

#[test]
fn lowcolor_follows_truecolor_when_flag_absent() {
    let mut w = world_with_preset_keys();
    w.config.bools.insert("truecolor".into(), true);
    let fs = MockFs::new().with_base("/cfg");
    let _ = init_config_dirs(&mut w, &Cli::default(), &fs).unwrap();
    assert_eq!(w.config.get_b("lowcolor"), Some(false));

    let mut w = world_with_preset_keys();
    w.config.bools.insert("truecolor".into(), false);
    let _ = init_config_dirs(&mut w, &Cli::default(), &fs).unwrap();
    assert_eq!(w.config.get_b("lowcolor"), Some(true));
}

#[test]
fn log_level_from_config_when_not_debug() {
    let mut w = world_with_preset_keys();
    w.config
        .strings
        .insert("log_level".into(), "WARNING".into());
    let fs = MockFs::new().with_base("/cfg");
    let _ = init_config_dirs(&mut w, &Cli::default(), &fs).unwrap();
    assert_eq!(w.log_level, "WARNING");
}

#[test]
fn missing_conf_file_returns_warnings_but_ok() {
    // Rust `Config::load` warns on unreadable files (documented deviation
    // from C++, which silently marks `write_new`); boot surfaces them.
    let mut w = world_with_preset_keys();
    let fs = MockFs::new().with_base("/cfg");
    let warnings = init_config_dirs(&mut w, &Cli::default(), &fs).unwrap();
    assert!(!warnings.is_empty());
}

// ── shared_init ─────────────────────────────────────────────────────────────

#[test]
fn probe_values_written_to_state() {
    let mut w = world_with_preset_keys();
    shared_init(&mut w, &MockProbe::new()).unwrap();
    assert_eq!(w.state.core_count, 8);
    assert_eq!(w.state.page_size, 16384);
    assert!((w.state.tick_factor - 0.01).abs() < 1e-12);
    assert_eq!(w.state.total_mem, 17_179_869_184);
    // `trim_name` strips the vendor ("Apple M1 Pro" → "M1 Pro").
    assert_eq!(w.state.cpu_name, "M1 Pro");
}

#[test]
fn probe_fallbacks_match_cpp_defaults() {
    let mut w = world_with_preset_keys();
    let probe = MockProbe {
        cores: 0,
        page: -5,
        mach: 0.0,
        clk: 0,
        mem: 0,
        cpu: String::new(),
    };
    shared_init(&mut w, &probe).unwrap();
    assert_eq!(w.state.core_count, 1);
    assert_eq!(w.state.page_size, 4096);
    assert!((w.state.tick_factor - 1.0).abs() < 1e-12);
    assert_eq!(w.state.cpu_name, "");
}

#[test]
fn trim_name_vectors() {
    // Xeon arm: token after `CPU` (btop_shared.cpp `trim_name`).
    assert_eq!(
        trim_name("Intel(R) Xeon(R) CPU E5-2670 v3 @ 2.30GHz".to_string()),
        "E5-2670"
    );
    // Ryzen arm: `Ryzen` + next two tokens.
    assert_eq!(
        trim_name("AMD Ryzen 7 5800X 8-Core Processor".to_string()),
        "Ryzen 7 5800X"
    );
    // Intel arm without a trailing `@` clock part.
    assert_eq!(
        trim_name("Intel(R) Core(TM) CPU E5-2670".to_string()),
        "E5-2670"
    );
    // Generic fallback strips vendor words.
    assert_eq!(trim_name("Apple M1 Pro".to_string()), "M1 Pro");
    assert_eq!(trim_name(String::new()), "");
}

// ── apply_preset_default ────────────────────────────────────────────────────

#[test]
fn none_preset_returns_false_and_keeps_current() {
    let mut w = world_with_preset_keys();
    w.current_preset = Some(2);
    assert!(!apply_preset_default(&mut w, None));
    assert_eq!(w.current_preset, Some(2));
}

#[test]
fn default_preset_index_zero_applies() {
    let mut w = world_with_preset_keys();
    assert!(apply_preset_default(&mut w, Some(0)));
    assert_eq!(w.current_preset, Some(0));
    assert_eq!(w.config.get_s("shown_boxes"), Some("cpu mem net proc"));
}

#[test]
fn preset_index_clamps_to_last() {
    let mut w = world_with_preset_keys();
    w.config
        .strings
        .insert("presets".into(), "cpu:0:braille".to_string());
    assert!(apply_preset_default(&mut w, Some(9)));
    assert_eq!(w.current_preset, Some(1));
    assert_eq!(w.config.get_s("shown_boxes"), Some("cpu"));
}

#[test]
fn invalid_presets_string_falls_back_to_default() {
    let mut w = world_with_preset_keys();
    w.config
        .strings
        .insert("presets".into(), "bogus!!!".to_string());
    assert!(apply_preset_default(&mut w, Some(1)));
    assert_eq!(w.current_preset, Some(0));
    assert_eq!(w.config.get_s("shown_boxes"), Some("cpu mem net proc"));
}

#[test]
fn mem_preset_writes_position_and_graph_flags() {
    let mut w = world_with_preset_keys();
    w.config
        .strings
        .insert("presets".into(), "mem:1:braille,proc:0:block".to_string());
    assert!(apply_preset_default(&mut w, Some(1)));
    assert_eq!(w.config.get_b("mem_below_net"), Some(true));
    assert_eq!(w.config.get_b("proc_left"), Some(false));
    assert_eq!(w.config.get_s("graph_symbol_mem"), Some("braille"));
}
