//! Term singleton with mockable operations (P4 T2).
//!
//! `TermWrapper` mirrors the C++ `Term` interface
//! (`src/btop_tools.cpp:55-183`) through the object-safe [`TermOps`]
//! trait so headless tests can substitute [`MockOps`] without touching
//! real termios fds. Production code uses [`RealOps`], a thin delegate
//! over [`btop_tools::term::Term`]. T7's `clean_quit`/main loop consume
//! this wrapper.

use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, Ordering};
use std::sync::Arc;

/// Width/height/initialized snapshot, mirroring the C++ `Term` globals
/// (`src/btop_tools.cpp:57-60`).
pub struct TermState {
    pub width: AtomicU16,
    pub height: AtomicU16,
    pub initialized: AtomicBool,
}

impl TermState {
    pub fn new() -> Self {
        Self {
            width: AtomicU16::new(80),
            height: AtomicU16::new(24),
            initialized: AtomicBool::new(false),
        }
    }
}

impl Default for TermState {
    fn default() -> Self {
        Self::new()
    }
}

/// Object-safe terminal operations. `&self` receivers so callers can
/// share an `Arc<dyn TermOps>` between the wrapper and tests.
pub trait TermOps: Send + Sync {
    fn init(&self) -> bool;
    fn refresh(&self, only_check: bool) -> bool;
    fn restore(&self);
    fn get_min_size(&self) -> (u16, u16);
    fn width(&self) -> u16;
    fn height(&self) -> u16;
    fn is_initialized(&self) -> bool;
}

/// Production ops: delegate every call to [`btop_tools::term::Term`].
pub struct RealOps {
    inner: btop_tools::term::Term,
}

impl RealOps {
    pub fn new() -> Self {
        Self {
            inner: btop_tools::term::Term::new(),
        }
    }
}

impl Default for RealOps {
    fn default() -> Self {
        Self::new()
    }
}

impl TermOps for RealOps {
    fn init(&self) -> bool {
        self.inner.init()
    }

    fn refresh(&self, only_check: bool) -> bool {
        self.inner.refresh(only_check)
    }

    fn restore(&self) {
        self.inner.restore();
    }

    fn get_min_size(&self) -> (u16, u16) {
        self.inner.get_min_size()
    }

    fn width(&self) -> u16 {
        self.inner.width()
    }

    fn height(&self) -> u16 {
        self.inner.height()
    }

    fn is_initialized(&self) -> bool {
        self.inner.is_initialized()
    }
}

/// Recording test double. Counters use atomics so `&self` trait
/// methods can record while tests hold an `Arc<MockOps>` alongside
/// the wrapper (and the type stays `Send + Sync`).
pub struct MockOps {
    pub init_calls: AtomicU32,
    pub refresh_calls: AtomicU32,
    pub restore_calls: AtomicU32,
    pub init_ret: AtomicBool,
    pub refresh_ret: AtomicBool,
    pub width: AtomicU16,
    pub height: AtomicU16,
    pub min_width: AtomicU16,
    pub min_height: AtomicU16,
    pub initialized: AtomicBool,
}

impl MockOps {
    pub fn new() -> Self {
        Self {
            init_calls: AtomicU32::new(0),
            refresh_calls: AtomicU32::new(0),
            restore_calls: AtomicU32::new(0),
            init_ret: AtomicBool::new(true),
            refresh_ret: AtomicBool::new(false),
            width: AtomicU16::new(80),
            height: AtomicU16::new(24),
            min_width: AtomicU16::new(100),
            min_height: AtomicU16::new(24),
            initialized: AtomicBool::new(false),
        }
    }

    /// After a successful `init`, report this size from `width/height`.
    pub fn with_size(self, width: u16, height: u16) -> Self {
        self.width.store(width, Ordering::Relaxed);
        self.height.store(height, Ordering::Relaxed);
        self
    }
}

impl Default for MockOps {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for MockOps {
    fn clone(&self) -> Self {
        Self {
            init_calls: AtomicU32::new(self.init_calls.load(Ordering::Relaxed)),
            refresh_calls: AtomicU32::new(self.refresh_calls.load(Ordering::Relaxed)),
            restore_calls: AtomicU32::new(self.restore_calls.load(Ordering::Relaxed)),
            init_ret: AtomicBool::new(self.init_ret.load(Ordering::Relaxed)),
            refresh_ret: AtomicBool::new(self.refresh_ret.load(Ordering::Relaxed)),
            width: AtomicU16::new(self.width.load(Ordering::Relaxed)),
            height: AtomicU16::new(self.height.load(Ordering::Relaxed)),
            min_width: AtomicU16::new(self.min_width.load(Ordering::Relaxed)),
            min_height: AtomicU16::new(self.min_height.load(Ordering::Relaxed)),
            initialized: AtomicBool::new(self.initialized.load(Ordering::Relaxed)),
        }
    }
}

impl TermOps for MockOps {
    fn init(&self) -> bool {
        self.init_calls.fetch_add(1, Ordering::Relaxed);
        let ok = self.init_ret.load(Ordering::Relaxed);
        self.initialized.store(ok, Ordering::Relaxed);
        ok
    }

    fn refresh(&self, _only_check: bool) -> bool {
        self.refresh_calls.fetch_add(1, Ordering::Relaxed);
        self.refresh_ret.load(Ordering::Relaxed)
    }

    fn restore(&self) {
        self.restore_calls.fetch_add(1, Ordering::Relaxed);
        self.initialized.store(false, Ordering::Relaxed);
    }

    fn get_min_size(&self) -> (u16, u16) {
        (
            self.min_width.load(Ordering::Relaxed),
            self.min_height.load(Ordering::Relaxed),
        )
    }

    fn width(&self) -> u16 {
        self.width.load(Ordering::Relaxed)
    }

    fn height(&self) -> u16 {
        self.height.load(Ordering::Relaxed)
    }

    fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Relaxed)
    }
}

/// Singleton-facing wrapper: caches ops results in [`TermState`] so
/// readers never touch fds.
pub struct TermWrapper {
    state: TermState,
    ops: Arc<dyn TermOps>,
}

impl TermWrapper {
    pub fn new(ops: Arc<dyn TermOps>) -> Self {
        Self {
            state: TermState::new(),
            ops,
        }
    }

    /// Construct with production [`RealOps`].
    pub fn with_real_ops() -> Self {
        Self::new(Arc::new(RealOps::new()))
    }

    fn sync_dims(&self) {
        self.state.width.store(self.ops.width(), Ordering::Relaxed);
        self.state
            .height
            .store(self.ops.height(), Ordering::Relaxed);
    }

    pub fn init(&self) -> bool {
        let ok = self.ops.init();
        self.state.initialized.store(ok, Ordering::Relaxed);
        if ok {
            self.sync_dims();
        }
        ok
    }

    pub fn refresh(&self, only_check: bool) -> bool {
        let changed = self.ops.refresh(only_check);
        if changed && !only_check {
            self.sync_dims();
        }
        changed
    }

    pub fn restore(&self) {
        self.ops.restore();
        self.state.initialized.store(false, Ordering::Relaxed);
    }

    pub fn get_min_size(&self) -> (u16, u16) {
        self.ops.get_min_size()
    }

    pub fn width(&self) -> u16 {
        self.state.width.load(Ordering::Relaxed)
    }

    pub fn height(&self) -> u16 {
        self.state.height.load(Ordering::Relaxed)
    }

    pub fn is_initialized(&self) -> bool {
        self.state.initialized.load(Ordering::Relaxed)
    }
}
