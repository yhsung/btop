//! Terminal cursor-movement and text-style escape codes.
//!
//! Ports `Mv` (src/btop_tools.hpp:105-126) and the `Fx` members used by
//! src/btop_draw.cpp (`b` x73, `ub` x74, `reset` x14, `i`, `ul`/`uul`).
//! Every escape VERIFIED, never guessed:
//! - `Fx::e == "\x1b["` (src/btop_tools.cpp:734) — owned as
//!   [`btop_tools::ESC`], re-exported here so callers have one owner.
//! - `Mv::to(l,c) = e + l + ';' + c + 'f'`, `r/l/u/d` append `C/D/A/B`
//!   (src/btop_tools.hpp:107-119); `save/restore = e + "s"/"u"`
//! - `Fx::b = e + "1m"`, `ub = e + "22m"`, `i = e + "3m"`,
//!   `ul = e + "4m"`, `uul = e + "24m"` (src/btop_tools.cpp:735-747)
//! - `Fx::reset` is runtime state: `reset_base (e + "0m") + Term fg/bg`
//!   (src/btop_theme.cpp:40,470). Callers pass it in; the `"cpu"`-box
//!   fixtures imply a concrete value resolved by the theme (Task 3).

/// Escape sequence start (`Fx::e`); single owner is [`btop_tools::ESC`].
pub use btop_tools::ESC;
use btop_tools::ESC as ESC_FMT;

/// Move cursor to line, column (`Mv::to`).
pub fn mv_to(line: i64, col: i64) -> String {
    format!("{ESC_FMT}{line};{col}f")
}

/// Move cursor right `x` columns (`Mv::r`).
pub fn mv_r(x: i64) -> String {
    format!("{ESC_FMT}{x}C")
}

/// Move cursor left `x` columns (`Mv::l`).
pub fn mv_l(x: i64) -> String {
    format!("{ESC_FMT}{x}D")
}

/// Move cursor up `x` lines (`Mv::u`).
pub fn mv_u(x: i64) -> String {
    format!("{ESC_FMT}{x}A")
}

/// Move cursor down `x` lines (`Mv::d`).
pub fn mv_d(x: i64) -> String {
    format!("{ESC_FMT}{x}B")
}

/// Save cursor position (`Mv::save`).
pub const MV_SAVE: &str = "\x1b[s";
/// Restore saved cursor position (`Mv::restore`).
pub const MV_RESTORE: &str = "\x1b[u";

/// Bold on (`Fx::b`).
pub const FX_B: &str = "\x1b[1m";
/// Bold off (`Fx::ub`).
pub const FX_UB: &str = "\x1b[22m";
/// Italic on (`Fx::i`).
pub const FX_I: &str = "\x1b[3m";
/// Underline on (`Fx::ul`).
pub const FX_UL: &str = "\x1b[4m";
/// Underline off (`Fx::uul`).
pub const FX_UUL: &str = "\x1b[24m";
/// Blink on (`Fx::bl`).
pub const FX_BL: &str = "\x1b[5m";
/// Blink off (`Fx::ubl`).
pub const FX_UBL: &str = "\x1b[25m";
/// Reset base (`Fx::reset_base`); runtime `Fx::reset` appends Term fg/bg.
pub const FX_RESET_BASE: &str = "\x1b[0m";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_match_cpp() {
        assert_eq!(mv_to(3, 5), "\x1b[3;5f");
        assert_eq!(mv_r(2), "\x1b[2C");
        assert_eq!(mv_l(4), "\x1b[4D");
        assert_eq!(mv_u(1), "\x1b[1A");
        assert_eq!(mv_d(7), "\x1b[7B");
        assert_eq!(MV_SAVE, "\x1b[s");
        assert_eq!(MV_RESTORE, "\x1b[u");
        assert_eq!(FX_B, "\x1b[1m");
        assert_eq!(FX_UB, "\x1b[22m");
        assert_eq!(FX_I, "\x1b[3m");
        assert_eq!(FX_UL, "\x1b[4m");
        assert_eq!(FX_UUL, "\x1b[24m");
        assert_eq!(FX_RESET_BASE, "\x1b[0m");
    }
}
