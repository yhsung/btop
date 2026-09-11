//! `btop-app` — single-process boot, signals, main loop.
//!
//! P4 T1 skeleton. Each module below is a placeholder; the real
//! implementations land across T2–T7 per `docs/superpowers/plans/
//! 2026-09-06-app-p4.md`.

/// Term wrapper (T2).
pub mod term;

/// Locale hunt + UTF-8 detection (T3).
pub mod locale;

/// Signal handlers + atomic flag plumbing (T4).
pub mod signals;

/// Init chain: parse_cli → setuid_drop → init_config_dirs → init_locale
/// → Term::init → configure_tty_mode → Shared::init → set_boxes fallback
/// → Theme → install_signals → presetsValid+apply_preset → min_size_loop
/// → calcSizes → print_box_outlines (T2). Placeholder.
pub mod boot {}

/// Cached box-label strings (T5). Placeholder.
pub mod box_labels {}

/// clean_quit(sig) sequence matching cpp:211-261 (T6). Placeholder.
pub mod clean_quit {}

/// Main loop: pselect + tick + Input::process + Menu::process (T7).
/// Placeholder.
pub mod main_loop {}
