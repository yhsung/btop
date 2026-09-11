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

/// Init chain: config dirs → Config::load → Shared::init → apply_preset
/// (T5). C++ truth: src/btop.cpp:862-925 (dirs), :325-353 (init_config),
/// osx/btop_collect.cpp Shared::init, btop.cpp:1093 + btop_config.cpp:515-550
/// (presetsValid + apply_preset).
pub mod boot;

/// Cached box-label strings (T5). Placeholder.
pub mod box_labels {}

/// clean_quit(sig) sequence matching cpp:211-261 (T6). Placeholder.
pub mod clean_quit {}

/// Main loop: pselect + tick + Input::process + Menu::process (T7).
/// Placeholder.
pub mod main_loop {}
