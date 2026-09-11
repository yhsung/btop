/// Escape sequence start (`Fx::e == "\x1b["`, src/btop_tools.cpp:734).
/// Single owner for the ESC prefix; btop-draw re-uses this const.
pub const ESC: &str = "\x1b[";

pub mod mouse;
pub mod strtools;
pub mod term;
pub mod wcwidth;
