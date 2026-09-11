//! Terminal setup/teardown + size queries.
//!
//! Rust transcription of C++ `Term` (`src/btop_tools.cpp:55-183`,
//! escape constants at `:757-765`). Single-threaded by contract:
//! callers use it from the main thread only (T7 owns it).

use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::Mutex;

use nix::sys::termios::{self, SetArg, Termios};
use nix::unistd;

/// Escape-code constants (`src/btop_tools.cpp:757-765`,
/// `Fx::e == "\x1b["`, `src/btop_tools.cpp:734`).
pub const HIDE_CURSOR: &str = "\x1b[?25l";
pub const SHOW_CURSOR: &str = "\x1b[?25h";
pub const ALT_SCREEN: &str = "\x1b[?1049h";
pub const NORMAL_SCREEN: &str = "\x1b[?1049l";
pub const CLEAR: &str = "\x1b[2J\x1b[0;0f";
pub const MOUSE_ON: &str = "\x1b[?1002h\x1b[?1015h\x1b[?1006h";
pub const MOUSE_OFF: &str = "\x1b[?1002l\x1b[?1015l\x1b[?1006l";

/// Writer for the mouse-enable sequence (uses [`crate::ESC`] prefix).
pub fn mouse_on() -> String {
    format!("{esc}?1002h{esc}?1015h{esc}?1006h", esc = crate::ESC)
}

/// Writer for the mouse-disable sequence (uses [`crate::ESC`] prefix).
pub fn mouse_off() -> String {
    format!("{esc}?1002l{esc}?1015l{esc}?1006l", esc = crate::ESC)
}

/// Owned terminal handle (`src/btop_tools.cpp:57-63` globals folded in).
pub struct Term {
    width: AtomicU16,
    height: AtomicU16,
    initialized: AtomicBool,
    original_termios: Mutex<Option<Termios>>,
    current_tty: Mutex<Option<String>>,
}

impl Term {
    /// Headless-safe defaults: 80x24, uninitialized. Touches no fds.
    pub fn new() -> Self {
        Self {
            width: AtomicU16::new(80),
            height: AtomicU16::new(24),
            initialized: AtomicBool::new(false),
            original_termios: Mutex::new(None),
            current_tty: Mutex::new(None),
        }
    }

    pub fn width(&self) -> u16 {
        self.width.load(Ordering::Relaxed)
    }

    pub fn height(&self) -> u16 {
        self.height.load(Ordering::Relaxed)
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Relaxed)
    }

    pub fn current_tty(&self) -> Option<String> {
        self.current_tty.lock().unwrap().clone()
    }

    /// Toggle terminal input echo (`src/btop_tools.cpp:66-72`).
    fn set_echo(&self, on: bool) -> bool {
        // SAFETY: fd 0 is stdin for the process lifetime; the borrow
        // confers no ownership and never outlives this call.
        let fd = unsafe { std::os::fd::BorrowedFd::borrow_raw(0) };
        let mut settings = match termios::tcgetattr(fd) {
            Ok(s) => s,
            Err(_) => return false,
        };
        if on {
            settings.local_flags |= termios::LocalFlags::ECHO;
        } else {
            settings.local_flags &= !termios::LocalFlags::ECHO;
        }
        // SAFETY: tcsetattr is a plain fd syscall wrapper; `settings`
        // is a valid termios value read from the same fd.
        termios::tcsetattr(fd, SetArg::TCSANOW, &settings).is_ok()
    }

    /// Toggle canonical (line-buffered) input mode
    /// (`src/btop_tools.cpp:75-88`).
    fn set_linebuffered(&self, on: bool) -> bool {
        // SAFETY: same short-lived stdin borrow as set_echo.
        let fd = unsafe { std::os::fd::BorrowedFd::borrow_raw(0) };
        let mut settings = match termios::tcgetattr(fd) {
            Ok(s) => s,
            Err(_) => return false,
        };
        if on {
            settings.local_flags |= termios::LocalFlags::ICANON;
        } else {
            settings.local_flags &= !termios::LocalFlags::ICANON;
            settings.control_chars[nix::libc::VMIN] = 0;
            settings.control_chars[nix::libc::VTIME] = 0;
        }
        // SAFETY: same fd-safe tcsetattr wrapper as above.
        termios::tcsetattr(fd, SetArg::TCSANOW, &settings).is_ok()
    }

    /// Query terminal size via `TIOCGWINSZ` on stdout, falling back to
    /// `/dev/tty` (`src/btop_tools.cpp:91-117`).
    ///
    /// When `only_check` is set the stored size is left untouched and
    /// only the change flag is returned. Returns `false` when the size
    /// could not be determined at all.
    pub fn refresh(&self, only_check: bool) -> bool {
        // `src/btop_tools.cpp:94`: sticky once `/dev/tty` fallback is used.
        static USES_DEV_TTY: AtomicBool = AtomicBool::new(false);

        // SAFETY: raw ioctl on fds 1 / /dev/tty with a stack `winsize`
        // out-param; no aliasing beyond the call.
        fn winsize_of(fd: i32) -> Option<nix::libc::winsize> {
            let mut ws: nix::libc::winsize = unsafe { std::mem::zeroed() };
            let rc = unsafe { nix::libc::ioctl(fd, nix::libc::TIOCGWINSZ, &mut ws) };
            if rc < 0 {
                None
            } else {
                Some(ws)
            }
        }

        let mut ws = if USES_DEV_TTY.load(Ordering::Relaxed) {
            None
        } else {
            winsize_of(1)
        };
        if ws.map(|w| (w.ws_col, w.ws_row)) == Some((0, 0)) {
            ws = None;
        }
        if ws.is_none() {
            // SAFETY: `c"/dev/tty"` is a valid NUL-terminated path;
            // returns an owned fd or -1.
            let fd = unsafe {
                nix::libc::open(
                    c"/dev/tty".as_ptr(),
                    nix::libc::O_RDONLY | nix::libc::O_CLOEXEC,
                )
            };
            // SAFETY: `fd` is either -1 or an owned fd from open; ioctl
            // takes no ownership, close releases it exactly once.
            if fd != -1 {
                ws = winsize_of(fd);
                unsafe { nix::libc::close(fd) };
            } else {
                return false;
            }
            USES_DEV_TTY.store(true, Ordering::Relaxed);
        }
        let ws = match ws {
            Some(w) => w,
            None => return false,
        };
        let (cols, rows) = (ws.ws_col, ws.ws_row);
        if self.width.load(Ordering::Relaxed) != cols || self.height.load(Ordering::Relaxed) != rows
        {
            if !only_check {
                self.width.store(cols, Ordering::Relaxed);
                self.height.store(rows, Ordering::Relaxed);
            }
            return true;
        }
        false
    }

    /// Minimum size for the current box layout. The real computation
    /// needs box min-width constants (TBD); until then the default
    /// `(100, 24)` stands in (`src/btop_tools.cpp:119-148` stubbed).
    pub fn get_min_size(&self) -> (u16, u16) {
        (100, 24)
    }

    /// Check for a valid tty, save terminal options and install raw-ish
    /// mode + alt screen (`src/btop_tools.cpp:150-174`).
    pub fn init_with_mouse(&self, mouse_enabled: bool) -> bool {
        if !self.is_initialized() {
            // SAFETY: isatty performs no mutation; fd 0 is stdin.
            let tty = unsafe { nix::libc::isatty(0) } == 1;
            self.initialized.store(tty, Ordering::Relaxed);
            if tty {
                // SAFETY: short-lived stdin borrow, as above.
                let fd = unsafe { std::os::fd::BorrowedFd::borrow_raw(0) };
                if let Ok(saved) = termios::tcgetattr(fd) {
                    *self.original_termios.lock().unwrap() = Some(saved);
                }
                let name = unistd::ttyname(fd)
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| "unknown".to_owned());
                *self.current_tty.lock().unwrap() = Some(name);

                self.set_echo(false);
                self.set_linebuffered(false);
                self.refresh(false);

                use std::io::Write as _;
                let stdout = std::io::stdout();
                let mut out = stdout.lock();
                let _ = out.write_all(ALT_SCREEN.as_bytes());
                let _ = out.write_all(HIDE_CURSOR.as_bytes());
                let _ = out.write_all(if mouse_enabled {
                    MOUSE_ON.as_bytes()
                } else {
                    MOUSE_OFF.as_bytes()
                });
                let _ = out.flush();
            }
        }
        self.is_initialized()
    }

    /// Default init with mouse reporting on
    /// (`src/btop_tools.cpp:168-169`).
    pub fn init(&self) -> bool {
        self.init_with_mouse(true)
    }

    /// Restore saved terminal options and leave the alt screen
    /// (`src/btop_tools.cpp:176-182`).
    pub fn restore(&self) {
        if self.is_initialized() {
            // SAFETY: short-lived stdin borrow, as above.
            let fd = unsafe { std::os::fd::BorrowedFd::borrow_raw(0) };
            if let Some(saved) = self.original_termios.lock().unwrap().as_ref() {
                // SAFETY: restores the termios snapshot taken in init on
                // the same fd.
                let _ = termios::tcsetattr(fd, SetArg::TCSANOW, saved);
            }
            use std::io::Write as _;
            let stdout = std::io::stdout();
            let mut out = stdout.lock();
            let _ = out.write_all(MOUSE_OFF.as_bytes());
            let _ = out.write_all(CLEAR.as_bytes());
            let _ = out.write_all(b"\x1b[0m");
            let _ = out.write_all(NORMAL_SCREEN.as_bytes());
            let _ = out.write_all(SHOW_CURSOR.as_bytes());
            let _ = out.flush();
            self.initialized.store(false, Ordering::Relaxed);
        }
    }
}

impl Default for Term {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_headless_safe() {
        let t = Term::new();
        assert_eq!((t.width(), t.height()), (80, 24));
        assert!(!t.is_initialized());
        assert_eq!(t.get_min_size(), (100, 24));
    }

    #[test]
    fn escape_constants_match_cpp() {
        // src/btop_tools.cpp:757-765 with Fx::e == "\x1b["
        assert_eq!(HIDE_CURSOR, "\x1b[?25l");
        assert_eq!(SHOW_CURSOR, "\x1b[?25h");
        assert_eq!(ALT_SCREEN, "\x1b[?1049h");
        assert_eq!(NORMAL_SCREEN, "\x1b[?1049l");
        assert_eq!(CLEAR, "\x1b[2J\x1b[0;0f");
        assert_eq!(MOUSE_ON, "\x1b[?1002h\x1b[?1015h\x1b[?1006h");
        assert_eq!(MOUSE_OFF, "\x1b[?1002l\x1b[?1015l\x1b[?1006l");
        assert_eq!(mouse_on(), MOUSE_ON);
        assert_eq!(mouse_off(), MOUSE_OFF);
    }

    #[test]
    fn restore_without_init_is_noop() {
        // Must not touch any fd when never initialized.
        Term::new().restore();
    }
}
