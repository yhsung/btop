//! Locale hunt + UTF-8 detection (P4 T3).
//!
//! Transcription of `src/btop.cpp:936-1002` ("Try to find and set a UTF-8
//! locale"), branch by branch. C++ line numbers are cited per branch below.
//!
//! Design notes:
//! - [`hunt_locale`] is the injectable core: env access (`get_env` /
//!   `set_env`) and the process-global `setlocale(3)` boundary
//!   ([`LocaleOps`]) are parameters, so the 6+ scenarios run hermetically
//!   in `tests/locale_unit.rs` without mutating process env or the process
//!   locale (parallel-safe; no serial guard needed).
//! - [`init_locale`] is the thin production wrapper T7 (`fn main`) calls
//!   before init: real `std::env` + [`RealLocaleOps`], log lines echoed to
//!   stderr (the C++ side uses `Logger::debug/warning`; the Rust logger
//!   port is not wired at boot yet — T7 owns verbosity).
//! - Tests never touch the real `setlocale`: only mocks implement
//!   [`LocaleOps`] in tests. The one place production *probes* the locale
//!   and restores it is [`RealLocaleOps::std_locale_name`], which saves the
//!   current `LC_ALL` via `setlocale(LC_ALL, NULL)` first and restores it
//!   afterwards.
//! - Hard failure (`Err`) carries the exact C++ `Global::exit_error_msg`
//!   text (cpp:995); the caller decides (sets `exit_error_msg` +
//!   `clean_quit(1)`). We never touch `World::signal_flags` here (T4 owns
//!   the atomics).
//! - `is_macos` is a runtime flag (production passes
//!   `cfg!(target_os = "macos")`) so both platform branches are testable
//!   on any host. The `CFLocale*` `extern "C"` block itself is still
//!   `#[cfg(target_os = "macos")]`-gated.
//! - Signature deviation from the brief sketch (`init_locale(&mut World)`):
//!   the hunt needs `force_utf` (C++ `cli.force_utf`, cpp:992) and the
//!   `World` carries nothing the hunt writes yet, so the wrapper is
//!   `init_locale(&mut World, force_utf: bool)`.

use btop_runner::sink::World;

/// Process-global locale boundary (wraps `setlocale(3)`).
///
/// `set_all` returning `None` means `setlocale` returned null (failure);
/// `Some` carries the resulting locale string (cpp:937 calls `setlocale`
/// twice — null-check then value; we call once and bind).
pub trait LocaleOps {
    /// `setlocale(LC_ALL, locale)`.
    fn set_all(&self, locale: &str) -> Option<String>;
    /// C++ `std::locale("").name()` (cpp:956): `None`/empty/`"*"` when
    /// unavailable (the C++ `catch (...)` clears `found` — same effect).
    fn std_locale_name(&self) -> Option<String>;
    /// macOS `CFLocaleCopyCurrent` identifier (cpp:973-977). `None` =
    /// lookup unavailable (non-mac build, or null CF objects) → plan
    /// fallback tries `en_US.UTF-8` directly; `Some("")` = CF returned an
    /// empty id → warn only (cpp:978-980).
    fn macos_locale_id(&self) -> Option<String>;
}

/// Captured `Logger::debug` / `Logger::warning` lines (C++ uses globals;
/// here the caller owns the sink so tests can assert on it).
#[derive(Debug, Default)]
pub struct HuntLog {
    pub debug: Vec<String>,
    pub warning: Vec<String>,
}

/// C++ `str_to_upper(s_replace(s, "-", "")).ends_with("UTF8")` (cpp:938,
/// :945, :958). Used for the current-locale, env-var, and split-piece
/// checks alike.
pub fn is_utf8_locale_name(s: &str) -> bool {
    let upper: String = s
        .chars()
        .filter(|c| *c != '-')
        .flat_map(|c| c.to_uppercase())
        .collect();
    upper.ends_with("UTF8")
}

/// Injectable locale hunt. Returns `Ok(found)` where `found` is the adopted
/// locale value, or the already-active one from branch 1, or `""` when
/// proceeding without one (force-utf / macOS-warn paths). `Err(message)` is
/// the hard-fail text for the caller to surface (cpp:995-996).
#[allow(clippy::too_many_arguments)]
pub fn hunt_locale(
    get_env: &dyn Fn(&str) -> Option<String>,
    set_env: &mut dyn FnMut(&str, &str) -> bool,
    ops: &dyn LocaleOps,
    force_utf: bool,
    is_macos: bool,
    log: &mut HuntLog,
) -> Result<String, String> {
    // Branch 1 — cpp:937-940: setlocale(LC_ALL,"") already UTF-8 and not a
    // ';'-joined per-category list → done, no env hunt.
    if let Some(current) = ops.set_all("") {
        if !current.contains(';') && is_utf8_locale_name(&current) {
            log.debug.push(format!("Using locale {current}"));
            return Ok(current);
        }
    }

    // Branch 2 — cpp:944-952: LANG/LC_ALL/LC_CTYPE loop (in this order).
    // Every UTF-8 value is tried; the last one wins `found` (C++ keeps
    // looping after a hit). A rejected value warns and sets `set_failure`
    // (which suppresses the final "Setting LC_ALL=" debug, cpp:999).
    let mut found = String::new();
    let mut set_failure = false;
    for loc_env in ["LANG", "LC_ALL", "LC_CTYPE"] {
        if let Some(val) = get_env(loc_env) {
            if is_utf8_locale_name(&val) {
                found = val;
                if ops.set_all(&found).is_none() {
                    set_failure = true;
                    log.warning
                        .push(format!("Failed to set locale {found} continuing anyway."));
                }
            }
        }
    }

    // Branch 3 — cpp:953-969: nothing in env; clear LC_ALL/LANG and probe
    // std::locale("").name(), split ';', adopt the first piece whose value
    // is UTF-8 (name after '=', or the whole piece when there is no '='
    // — C++ `substr(find('=') + 1)` with npos wraps to 0). A piece whose
    // setlocale fails keeps `found` but does not stop the scan.
    if found.is_empty() && set_env("LC_ALL", "") && set_env("LANG", "") {
        if let Some(loc) = ops.std_locale_name() {
            if !loc.is_empty() && loc != "*" {
                for piece in loc.split(';') {
                    if is_utf8_locale_name(piece) {
                        let name = match piece.find('=') {
                            Some(i) => &piece[i + 1..],
                            None => piece,
                        };
                        found = name.to_string();
                        if ops.set_all(&found).is_some() {
                            break;
                        }
                    }
                }
            }
        }
    }

    if is_macos {
        // Branch 4a — cpp:972-990 (__APPLE__): CFLocaleCopyCurrent id.
        if found.is_empty() {
            match ops.macos_locale_id() {
                Some(id) if !id.is_empty() => {
                    let candidate = format!("{id}.UTF-8");
                    if ops.set_all(&candidate).is_some() {
                        log.debug.push(format!("Setting LC_ALL={candidate}"));
                        found = candidate;
                    } else if ops.set_all("en_US.UTF-8").is_some() {
                        // cpp:984 fallback.
                        log.debug.push("Setting LC_ALL=en_US.UTF-8".to_string());
                        found = "en_US.UTF-8".to_string();
                    } else {
                        log.warning
                            .push("Failed to set macos locale, continuing anyway.".to_string());
                    }
                }
                None => {
                    // Plan fallback: CF lookup unavailable — try en_US.UTF-8
                    // directly (same arm as cpp:984, without needing the id).
                    if ops.set_all("en_US.UTF-8").is_some() {
                        log.debug.push("Setting LC_ALL=en_US.UTF-8".to_string());
                        found = "en_US.UTF-8".to_string();
                    } else {
                        log.warning
                            .push("Failed to set macos locale, continuing anyway.".to_string());
                    }
                }
                Some(_) => {
                    // cpp:978-980: empty CF id — warn, continue anyway.
                    log.warning.push(
                        "No UTF-8 locale detected! Some symbols might not display correctly."
                            .to_string(),
                    );
                }
            }
        } else if !set_failure {
            // cpp:999-1001 tail (macOS side of the #ifdef).
            log.debug.push(format!("Setting LC_ALL={found}"));
        }
    } else {
        // Branch 4b — cpp:992-997 (non-Apple): force-utf warn vs hard fail.
        if found.is_empty() && force_utf {
            log.warning.push(
                "No UTF-8 locale detected! Forcing start with --force-utf argument.".to_string(),
            );
        } else if found.is_empty() {
            return Err("No UTF-8 locale detected!\nUse --force-utf argument to force start if you're sure your terminal can handle it.".to_string());
        } else if !set_failure {
            // cpp:999-1001 tail.
            log.debug.push(format!("Setting LC_ALL={found}"));
        }
    }

    Ok(found)
}

/// Production `setlocale(3)` boundary. Side effects are the point here
/// (the hunt *sets* the process locale); tests use mocks instead and never
/// construct this.
pub struct RealLocaleOps;

fn read_current_all() -> Option<String> {
    // `setlocale(LC_ALL, NULL)` — pure read, no side effect.
    let ret = unsafe { nix::libc::setlocale(nix::libc::LC_ALL, std::ptr::null()) };
    if ret.is_null() {
        None
    } else {
        Some(
            unsafe { std::ffi::CStr::from_ptr(ret) }
                .to_string_lossy()
                .into_owned(),
        )
    }
}

impl LocaleOps for RealLocaleOps {
    fn set_all(&self, locale: &str) -> Option<String> {
        let c = std::ffi::CString::new(locale).ok()?;
        let ret = unsafe { nix::libc::setlocale(nix::libc::LC_ALL, c.as_ptr()) };
        if ret.is_null() {
            None
        } else {
            Some(
                unsafe { std::ffi::CStr::from_ptr(ret) }
                    .to_string_lossy()
                    .into_owned(),
            )
        }
    }

    fn std_locale_name(&self) -> Option<String> {
        // No Rust equivalent of `std::locale("")`; emulate it: save the
        // current locale, re-derive from env via setlocale(LC_ALL,""),
        // restore the saved value. The save/restore keeps the probe from
        // leaking a changed locale (the caller re-sets it deliberately
        // afterwards when a UTF-8 value is found).
        let saved = read_current_all();
        let probed = self.set_all("");
        if let Some(saved) = saved {
            let _ = self.set_all(&saved);
        }
        probed
    }

    fn macos_locale_id(&self) -> Option<String> {
        macos_locale_id()
    }
}

/// macOS `CFLocaleCopyCurrent` identifier (cpp:973-977), via a tiny
/// `extern "C"` block — no core-foundation crate (zero-deps rule).
/// `None` on any null (id pointer, backing CString ptr). Note
/// `CFStringGetCStringPtr` can return null for non-contiguous strings;
/// that degrades to the `en_US.UTF-8` fallback in [`hunt_locale`].
/// UNVERIFIED on macOS hardware (Linux CI cannot compile this arm) —
/// flagged in the T3 report.
#[cfg(target_os = "macos")]
fn macos_locale_id() -> Option<String> {
    use std::ffi::CStr;
    use std::os::raw::{c_char, c_void};

    type CFTypeRef = *const c_void;
    type CFLocaleRef = *const c_void;
    type CFStringRef = *const c_void;
    const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFLocaleCopyCurrent() -> CFLocaleRef;
        fn CFLocaleGetValue(locale: CFLocaleRef, key: CFTypeRef) -> CFTypeRef;
        fn CFStringGetCStringPtr(s: CFStringRef, encoding: u32) -> *const c_char;
        fn CFRelease(cf: CFTypeRef);
        static kCFLocaleIdentifier: CFTypeRef;
    }

    unsafe {
        let locale = CFLocaleCopyCurrent();
        if locale.is_null() {
            return None;
        }
        let id = CFLocaleGetValue(locale, kCFLocaleIdentifier) as CFStringRef;
        let out = if id.is_null() {
            None
        } else {
            let ptr = CFStringGetCStringPtr(id, K_CF_STRING_ENCODING_UTF8);
            if ptr.is_null() {
                None
            } else {
                Some(CStr::from_ptr(ptr).to_string_lossy().into_owned())
            }
        };
        CFRelease(locale);
        out
    }
}

/// Non-mac build: no `CFLocale*` linkage (the `extern "C"` block above is
/// gated out); the hunt treats this as "lookup unavailable".
#[cfg(not(target_os = "macos"))]
fn macos_locale_id() -> Option<String> {
    None
}

/// Thin production wrapper T7 (`fn main`) calls before init.
///
/// `_world` is reserved for T7 wiring (the hunt returns `Err` and lets the
/// caller set `exit_error_msg` + quit — it never touches `signal_flags`).
/// `force_utf` is C++ `cli.force_utf` (cpp:992).
pub fn init_locale(_world: &mut World, force_utf: bool) -> Result<(), String> {
    let ops = RealLocaleOps;
    let mut log = HuntLog::default();
    let get = |k: &str| std::env::var(k).ok();
    let mut set = |k: &str, v: &str| match (std::ffi::CString::new(k), std::ffi::CString::new(v)) {
        (Ok(kk), Ok(vv)) => unsafe { nix::libc::setenv(kk.as_ptr(), vv.as_ptr(), 1) == 0 },
        _ => false,
    };
    let res = hunt_locale(
        &get,
        &mut set,
        &ops,
        force_utf,
        cfg!(target_os = "macos"),
        &mut log,
    );
    for line in &log.debug {
        eprintln!("{line}");
    }
    for line in &log.warning {
        eprintln!("{line}");
    }
    res.map(|_| ())
}
