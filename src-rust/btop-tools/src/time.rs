//! Wall-clock formatting for the header clock (`Draw::update_clock`,
//! src/btop_draw.cpp:333-392; `Tools::strf_time`, src/btop_tools.cpp:536).
//!
//! `strftime_local` is a thin wrapper over libc `localtime_r` + `strftime`
//! (full format fidelity, no chrono dependency). `clock_string` adds the
//! btop `/user` `/host` `/uptime` substitutions (btop_draw.cpp:368-380).
//! All identity/time acquisition stays with the caller — this module is
//! pure over injected values (headless-testable).

use crate::strtools::sec_to_dhms;

extern "C" {
    // time.h: `struct tm *localtime_r(const time_t *, struct tm *);`
    // `size_t strftime(char *, size_t, const char *, const struct tm *);`
    // The tm buffer is opaque bytes: sizeof(struct tm) == 56 on macOS
    // (C-measured via clang probe); strftime reads it, Rust never does.
    fn localtime_r(t: *const i64, tm: *mut u8) -> *mut u8;
    fn strftime(dst: *mut u8, max: usize, fmt: *const u8, tm: *const u8) -> usize;
    // unistd.h: `uid_t getuid(void);` `int gethostname(char *, size_t);`
    // pwd.h:121 `struct passwd *getpwuid(uid_t);` (pw_name = char *@0).
    fn getuid() -> u32;
    fn getpwuid(uid: u32) -> *const u8;
    fn gethostname(name: *mut u8, len: usize) -> i32;
}

/// Login name for `/user` (getuid → getpwuid); "" when unresolvable.
pub fn local_user() -> String {
    #[cfg(target_os = "macos")]
    {
        use std::ffi::CStr;
        use std::os::raw::c_char;
        // SAFETY: getuid always succeeds; pw_name@0 null-checked.
        let pwd = unsafe { getpwuid(getuid()) };
        if pwd.is_null() {
            return String::new();
        }
        let name = unsafe { *(pwd as *const *const c_char) };
        if name.is_null() {
            return String::new();
        }
        unsafe { CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned()
    }
    #[cfg(not(target_os = "macos"))]
    {
        String::new()
    }
}

/// Hostname for `/host`; "" when unresolvable.
pub fn local_host() -> String {
    #[cfg(target_os = "macos")]
    {
        let mut buf = vec![0u8; 256];
        // SAFETY: 256B out-buffer, length passed.
        let rc = unsafe { gethostname(buf.as_mut_ptr(), buf.len()) };
        if rc != 0 {
            return String::new();
        }
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..end]).into_owned()
    }
    #[cfg(not(target_os = "macos"))]
    {
        String::new()
    }
}

/// Format `epoch_secs` with an strftime format in LOCAL time.
/// Returns "" when the format is empty or libc reports truncation.
pub fn strftime_local(fmt: &str, epoch_secs: i64) -> String {
    if fmt.is_empty() {
        return String::new();
    }
    let mut tm = vec![0u8; 56];
    // SAFETY: tm is 56 zeroed bytes (C-measured sizeof); localtime_r fills
    // it on success. fmt is NUL-terminated via +1 byte.
    let ok = unsafe { localtime_r(&epoch_secs, tm.as_mut_ptr()) };
    if ok.is_null() {
        return String::new();
    }
    let mut f = fmt.as_bytes().to_vec();
    f.push(0);
    let mut out = vec![0u8; 256];
    // SAFETY: dst fits 256B; fmt NUL-terminated; tm from localtime_r above.
    let n = unsafe { strftime(out.as_mut_ptr(), out.len(), f.as_ptr(), tm.as_ptr()) };
    if n == 0 {
        return String::new();
    }
    String::from_utf8_lossy(&out[..n]).into_owned()
}

/// Full header-clock string: substitutions first, then strftime
/// (btop_draw.cpp:353-381 order: strf first, then /user/host/uptime replace
/// — equivalent here since substitution tokens contain no `%`).
/// `uptime_secs` renders like the cpu `up` row (`sec_to_dhms`, truncated
/// past 8 chars, btop_draw.cpp:371-373).
pub fn clock_string(
    fmt: &str,
    epoch_secs: i64,
    user: &str,
    host: &str,
    uptime_secs: u64,
) -> String {
    let mut up = sec_to_dhms(uptime_secs, false, false);
    if up.len() > 8 {
        up.truncate(up.len() - 3);
    }
    let rendered = strftime_local(fmt, epoch_secs);
    rendered
        .replace("/user", user)
        .replace("/host", host)
        .replace("/uptime", &up)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_format_yields_empty() {
        assert_eq!(strftime_local("", 1_700_000_000), "");
    }

    #[test]
    fn clock_shape_hh_mm_ss() {
        // Local-tz dependent values: assert shape only (8 chars, colons).
        let s = strftime_local("%H:%M:%S", 1_700_000_000);
        assert_eq!(s.len(), 8, "got {s:?}");
        assert_eq!(&s[2..3], ":");
        assert_eq!(&s[5..6], ":");
    }

    #[test]
    fn substitutions_replace_before_strftime() {
        let s = clock_string("/user@/host up /uptime %H", 1_700_000_000, "u", "h", 90_061);
        assert!(s.starts_with("u@h up "), "got {s:?}");
        assert!(s.contains("1d"), "uptime dhms in {s:?}");
        assert!(!s.contains("/user"), "tokens consumed: {s:?}");
    }

    #[test]
    fn unknown_tokens_pass_through() {
        let s = clock_string("plain", 1_700_000_000, "u", "h", 0);
        assert_eq!(s, "plain");
    }
}
