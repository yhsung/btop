//! Signal handlers with mockable install (P4 T4).
//!
//! C++ truth: install block at `src/btop.cpp:1047-1065`, `_signal_handler`
//! at `:290-322`, `_crash_handler`, `_exit_handler` at `:276-278`
//! (`clean_quit(-1)`), `_sleep` at `:264-269`, `_resume` at `:271-274`.
//!
//! Signal→flag table (see [`SignalFlags`] fields for per-flag notes):
//!
//! | Signal   | C++ (`_signal_handler`)              | Rust flag(s) set                    |
//! |----------|--------------------------------------|-------------------------------------|
//! | SIGINT   | `should_quit=true`, `stopping=true` (if active), `Input::interrupt()` (:292-296) | `should_quit`, `stopping`, `interrupt_input` |
//! | SIGTSTP  | `should_sleep=true`, `stopping=true`, `Input::interrupt()` when active, else `_sleep()` (:297-306) | `should_sleep`, `stopping`, `interrupt_input` (never sleeps inline) |
//! | SIGCONT  | `_resume()` = `Term::init()` + `term_resize(true)` (:307-309) | `do_continue`, `interrupt_input` (loop re-inits) |
//! | SIGWINCH | `resized=true`, `Input::interrupt()` (:316-318) | `resized`, `interrupt_input` |
//! | SIGUSR1  | no-op ("Input::poll interrupt", :319-321) | `interrupt_input` (poll wake) |
//! | SIGUSR2  | `reload_conf=true`, `Input::interrupt()` (:322-325) | `reload_conf`, `interrupt_input` |
//!
//! Crash signals (install block :1059-1063): SIGSEGV, SIGABRT, SIGTRAP,
//! SIGBUS, SIGILL → [`on_crash`] (restore term if initialized + re-raise,
//! mirroring `_crash_handler`).
//!
//! # Handler safety
//!
//! The `on_*` fns only perform atomic stores (`Ordering::SeqCst`) on the
//! passed [`SignalFlags`] — no allocation, no locking, no I/O — so they
//! are async-signal-safe and may run in real signal context via the
//! `extern "C"` trampolines installed by [`RealInstaller`]. The
//! trampolines additionally read two lock-free globals ([`HANDLER_FLAGS`]
//! via [`handler_flags`], [`CRASH_TERM_PTR`]); both are plain
//! atomic-pointer loads, likewise signal-safe. Everything else (term
//! re-init, config reload, sleep/resume, cleanup) is deferred to the
//! main loop, which polls the flags.
//!
//! # Testability
//!
//! Tests never deliver real signals: they call the `on_*` fns directly,
//! drive install through [`MockInstaller`], the crash path through
//! [`MockCrashEnv`], and atexit through [`MockAtExit`] / [`exit_hook_fn`].

use std::os::raw::c_int;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use nix::sys::signal::{sigaction, SaFlags, SigAction, SigHandler, SigSet, SigmaskHow, Signal};

pub use btop_runner::wiring::SignalFlags;

use crate::term::TermWrapper;

// ── Handler fns (directly testable; also called from trampolines) ─────────

/// SIGINT (btop.cpp:292-296).
pub fn on_sigint(flags: &SignalFlags) {
    flags.should_quit.store(true, Ordering::SeqCst);
    flags.stopping.store(true, Ordering::SeqCst);
    flags.interrupt_input.store(true, Ordering::SeqCst);
}

/// SIGTSTP (btop.cpp:297-306): record only — the main loop performs the
/// stop/restore sequence (`_sleep`, :264-269) outside signal context.
pub fn on_sigtstp(flags: &SignalFlags) {
    flags.should_sleep.store(true, Ordering::SeqCst);
    flags.stopping.store(true, Ordering::SeqCst);
    flags.interrupt_input.store(true, Ordering::SeqCst);
}

/// SIGCONT (btop.cpp:307-309): record only — the main loop performs
/// `_resume` (`Term::init` + forced resize, :271-274).
pub fn on_sigcont(flags: &SignalFlags) {
    flags.do_continue.store(true, Ordering::SeqCst);
    flags.interrupt_input.store(true, Ordering::SeqCst);
}

/// SIGWINCH (btop.cpp:316-318).
pub fn on_sigwinch(flags: &SignalFlags) {
    flags.resized.store(true, Ordering::SeqCst);
    flags.interrupt_input.store(true, Ordering::SeqCst);
}

/// SIGUSR1 (btop.cpp:319-321): C++ no-op poll interrupt → wake flag.
pub fn on_sigusr1(flags: &SignalFlags) {
    flags.interrupt_input.store(true, Ordering::SeqCst);
}

/// SIGUSR2 (btop.cpp:322-325).
pub fn on_sigusr2(flags: &SignalFlags) {
    flags.reload_conf.store(true, Ordering::SeqCst);
    flags.interrupt_input.store(true, Ordering::SeqCst);
}

// ── Trampoline globals ────────────────────────────────────────────────────

/// Flags published at install for the `extern "C"` trampolines.
/// `AtomicPtr` load/store is async-signal-safe; the pointed-to
/// `SignalFlags` only performs atomic ops, so dereferencing in a handler
/// is sound while the boot scope that owns the `Arc<SignalFlags>` is
/// alive (which is until process exit — see SAFETY on [`publish_flags`]).
static HANDLER_FLAGS: AtomicPtr<SignalFlags> = AtomicPtr::new(std::ptr::null_mut());

/// Term published at install for the crash trampoline (null = no term;
/// restore is skipped but the signal is still re-raised).
static CRASH_TERM_PTR: AtomicPtr<TermWrapper> = AtomicPtr::new(std::ptr::null_mut());

/// Publish handler state for the trampolines.
///
/// # Safety
/// The caller must guarantee `flags` (and `term`, if given) outlive any
/// delivered signal. In practice [`RealInstaller::install`] is called
/// once at boot with the process-lifetime `Arc<SignalFlags>` /
/// `Arc<TermWrapper>`, which satisfies this trivially.
fn publish_flags(flags: &SignalFlags, term: Option<&Arc<TermWrapper>>) {
    HANDLER_FLAGS.store(
        flags as *const SignalFlags as *mut SignalFlags,
        Ordering::SeqCst,
    );
    let ptr = term.map_or_else(std::ptr::null_mut, |t| Arc::as_ptr(t) as *mut TermWrapper);
    CRASH_TERM_PTR.store(ptr, Ordering::SeqCst);
}

/// Flags for the current trampoline, if published.
fn handler_flags() -> Option<&'static SignalFlags> {
    // SAFETY: null-checked; non-null implies a live boot-owned allocation
    // per the `publish_flags` contract.
    unsafe { HANDLER_FLAGS.load(Ordering::SeqCst).as_ref() }
}

/// Term for the crash trampoline, if published and alive (same contract).
fn crash_term() -> Option<&'static TermWrapper> {
    unsafe { CRASH_TERM_PTR.load(Ordering::SeqCst).as_ref() }
}

extern "C" fn sigint_trampoline(_sig: c_int) {
    if let Some(flags) = handler_flags() {
        on_sigint(flags);
    }
}

extern "C" fn sigtstp_trampoline(_sig: c_int) {
    if let Some(flags) = handler_flags() {
        on_sigtstp(flags);
    }
}

extern "C" fn sigcont_trampoline(_sig: c_int) {
    if let Some(flags) = handler_flags() {
        on_sigcont(flags);
    }
}

extern "C" fn sigwinch_trampoline(_sig: c_int) {
    if let Some(flags) = handler_flags() {
        on_sigwinch(flags);
    }
}

extern "C" fn sigusr1_trampoline(_sig: c_int) {
    if let Some(flags) = handler_flags() {
        on_sigusr1(flags);
    }
}

extern "C" fn sigusr2_trampoline(_sig: c_int) {
    if let Some(flags) = handler_flags() {
        on_sigusr2(flags);
    }
}

extern "C" fn crash_trampoline(sig: c_int) {
    let env = RealCrashEnv { term: crash_term() };
    on_crash(&env, sig);
}

// ── Installer ─────────────────────────────────────────────────────────────

/// Mockable signal-install seam. Production boot passes
/// [`RealInstaller`]; tests pass [`MockInstaller`].
pub trait SignalInstaller: Send + Sync {
    fn install(&self, flags: &SignalFlags) -> Result<(), String>;
}

/// Production installer: `sigaction` per the C++ install block
/// (btop.cpp:1047-1065) + SIGUSR1 process-mask block (:1060-1063).
pub struct RealInstaller {
    term: Option<Arc<TermWrapper>>,
}

impl RealInstaller {
    /// Install without a crash-restore term (crash path still re-raises).
    pub fn new() -> Self {
        Self { term: None }
    }

    /// Install with `term` published for the crash trampoline's restore.
    pub fn with_term(term: Arc<TermWrapper>) -> Self {
        Self { term: Some(term) }
    }
}

impl Default for RealInstaller {
    fn default() -> Self {
        Self::new()
    }
}

fn install_sigaction(sig: Signal, handler: extern "C" fn(c_int)) -> Result<(), String> {
    let action = SigAction::new(
        SigHandler::Handler(handler),
        SaFlags::SA_RESTART,
        SigSet::empty(),
    );
    // SAFETY: the handler only performs atomic stores/loads on published
    // globals (see module docs); installing it is the whole point.
    unsafe { sigaction(sig, &action) }
        .map(|_| ())
        .map_err(|e| format!("sigaction({sig:?}) failed: {e}"))
}

impl SignalInstaller for RealInstaller {
    fn install(&self, flags: &SignalFlags) -> Result<(), String> {
        publish_flags(flags, self.term.as_ref());
        // btop.cpp:1048-1053: regular handlers.
        install_sigaction(Signal::SIGINT, sigint_trampoline)?;
        install_sigaction(Signal::SIGTSTP, sigtstp_trampoline)?;
        install_sigaction(Signal::SIGCONT, sigcont_trampoline)?;
        install_sigaction(Signal::SIGWINCH, sigwinch_trampoline)?;
        install_sigaction(Signal::SIGUSR1, sigusr1_trampoline)?;
        install_sigaction(Signal::SIGUSR2, sigusr2_trampoline)?;
        // btop.cpp:1059-1063: crash handlers.
        for sig in CRASH_SIGNALS {
            install_sigaction(sig, crash_trampoline)?;
        }
        // btop.cpp:1060-1063: block SIGUSR1 so it only wakes `pselect`
        // (T7 applies the same mask at the poll site).
        let mask = sigusr1_block_mask();
        nix::sys::signal::sigprocmask(SigmaskHow::SIG_BLOCK, Some(&mask), None)
            .map_err(|e| format!("sigprocmask(SIGUSR1 block) failed: {e}"))?;
        Ok(())
    }
}

/// Recording test double: counts installs, fails on demand.
pub struct MockInstaller {
    pub calls: AtomicU32,
    fail: AtomicBool,
}

impl MockInstaller {
    pub fn new() -> Self {
        Self {
            calls: AtomicU32::new(0),
            fail: AtomicBool::new(false),
        }
    }

    pub fn set_fail(&self, fail: bool) {
        self.fail.store(fail, Ordering::SeqCst);
    }
}

impl Default for MockInstaller {
    fn default() -> Self {
        Self::new()
    }
}

impl SignalInstaller for MockInstaller {
    fn install(&self, _flags: &SignalFlags) -> Result<(), String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) {
            Err("mock install failure".to_string())
        } else {
            Ok(())
        }
    }
}

/// Boot seam: install via the given installer (real or mock).
pub fn install_signals(installer: &dyn SignalInstaller, flags: &SignalFlags) -> Result<(), String> {
    installer.install(flags)
}

/// The SIGUSR1 mask from the C++ install block (btop.cpp:1060-1063).
/// [`RealInstaller`] applies it process-wide; T7's `pselect` uses the
/// same set as its wait mask.
pub fn sigusr1_block_mask() -> SigSet {
    let mut mask = SigSet::empty();
    mask.add(Signal::SIGUSR1);
    mask
}

// ── Crash path ────────────────────────────────────────────────────────────

/// Crash signals installed alongside the regular handlers
/// (btop.cpp:1059-1063, verbatim order).
pub const CRASH_SIGNALS: [Signal; 5] = [
    Signal::SIGSEGV,
    Signal::SIGABRT,
    Signal::SIGTRAP,
    Signal::SIGBUS,
    Signal::SIGILL,
];

/// Testable crash environment: term restore + re-raise, mirroring
/// `_crash_handler` ("restore terminal before crashing … re-raise the
/// signal to get default behavior").
pub trait CrashEnv: Send + Sync {
    fn term_initialized(&self) -> bool;
    fn restore_term(&self);
    fn reraise(&self, sig: c_int);
}

/// Production env over the install-published term (may be absent).
pub struct RealCrashEnv {
    term: Option<&'static TermWrapper>,
}

impl CrashEnv for RealCrashEnv {
    fn term_initialized(&self) -> bool {
        self.term.is_some_and(TermWrapper::is_initialized)
    }

    fn restore_term(&self) {
        if let Some(term) = self.term {
            term.restore();
        }
    }

    fn reraise(&self, sig: c_int) {
        // `libc::raise` is async-signal-safe; nix only wraps it.
        // The `Signal` round-trip documents intent; fall back to the raw
        // number for trap-like values the enum may not cover.
        if let Ok(sig) = Signal::try_from(sig) {
            let _ = nix::sys::signal::raise(sig);
        } else {
            unsafe {
                nix::libc::raise(sig);
            }
        }
    }
}

/// Recording test double: no real restore (beyond the flag) and no real
/// raise — the signal number is recorded instead.
pub struct MockCrashEnv {
    initialized: AtomicBool,
    pub restore_calls: AtomicU32,
    pub raised: AtomicI32,
}

impl MockCrashEnv {
    pub fn new(initialized: bool) -> Self {
        Self {
            initialized: AtomicBool::new(initialized),
            restore_calls: AtomicU32::new(0),
            raised: AtomicI32::new(-1),
        }
    }
}

impl CrashEnv for MockCrashEnv {
    fn term_initialized(&self) -> bool {
        self.initialized.load(Ordering::SeqCst)
    }

    fn restore_term(&self) {
        self.restore_calls.fetch_add(1, Ordering::SeqCst);
    }

    fn reraise(&self, sig: c_int) {
        self.raised.store(sig, Ordering::SeqCst);
    }
}

/// Crash body shared by the trampoline and tests: restore the term iff
/// initialized, then re-raise unconditionally.
pub fn on_crash(env: &dyn CrashEnv, sig: c_int) {
    if env.term_initialized() {
        env.restore_term();
    }
    env.reraise(sig);
}

// ── atexit ────────────────────────────────────────────────────────────────

/// Exit hook payload: the atexit body (`_exit_handler` → `clean_quit(-1)`
/// at btop.cpp:276-278) reduced to what T4 owns — restore the term and
/// mark `quitting` so a second exit path becomes a no-op. T6's
/// `clean_quit` subsumes this once it lands.
pub struct AtExitHook {
    flags: Arc<SignalFlags>,
    term: Arc<TermWrapper>,
}

impl AtExitHook {
    pub fn new(flags: Arc<SignalFlags>, term: Arc<TermWrapper>) -> Self {
        Self { flags, term }
    }

    pub fn run(self) {
        if self.term.is_initialized() {
            self.term.restore();
        }
        self.flags.quitting.store(true, Ordering::SeqCst);
    }
}

/// The hook body as a callable: tests invoke it directly (no real
/// `atexit`); [`RealAtExit`] registers the same body process-wide.
pub fn exit_hook_fn(flags: Arc<SignalFlags>, term: Arc<TermWrapper>) -> impl FnOnce() {
    let hook = AtExitHook::new(flags, term);
    move || hook.run()
}

/// Mockable atexit-registration seam.
pub trait AtExitRegistrar: Send + Sync {
    fn register(&self, hook: AtExitHook) -> Result<(), String>;
}

/// Pending hook consumed by the `atexit` trampoline. A plain `Mutex` is
/// fine: `atexit` runs at normal exit, never in signal context.
static ATEXIT_HOOK: Mutex<Option<AtExitHook>> = Mutex::new(None);

extern "C" fn atexit_trampoline() {
    let hook = ATEXIT_HOOK.lock().ok().and_then(|mut slot| slot.take());
    if let Some(hook) = hook {
        hook.run();
    }
}

/// Production registrar: stages the hook and registers the trampoline
/// with `atexit` (mirrors `std::atexit(_exit_handler)` at btop.cpp:1047).
pub struct RealAtExit;

impl AtExitRegistrar for RealAtExit {
    fn register(&self, hook: AtExitHook) -> Result<(), String> {
        *ATEXIT_HOOK
            .lock()
            .map_err(|e| format!("atexit slot poisoned: {e}"))? = Some(hook);
        // SAFETY: the trampoline only touches the hook slot; the hook is
        // staged above before registration.
        let ret = unsafe { nix::libc::atexit(atexit_trampoline) };
        if ret != 0 {
            return Err(format!("atexit registration failed: {ret}"));
        }
        Ok(())
    }
}

/// Recording test double: counts registrations, fails on demand. The
/// hook is dropped — tests verify the body via [`exit_hook_fn`].
pub struct MockAtExit {
    pub calls: AtomicU32,
    fail: AtomicBool,
}

impl MockAtExit {
    pub fn new() -> Self {
        Self {
            calls: AtomicU32::new(0),
            fail: AtomicBool::new(false),
        }
    }

    pub fn set_fail(&self, fail: bool) {
        self.fail.store(fail, Ordering::SeqCst);
    }
}

impl Default for MockAtExit {
    fn default() -> Self {
        Self::new()
    }
}

impl AtExitRegistrar for MockAtExit {
    fn register(&self, _hook: AtExitHook) -> Result<(), String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) {
            Err("mock atexit failure".to_string())
        } else {
            Ok(())
        }
    }
}

/// Boot seam: register the exit hook via the given registrar.
pub fn register_atexit<R: AtExitRegistrar>(
    registrar: &R,
    flags: Arc<SignalFlags>,
    term: Arc<TermWrapper>,
) -> Result<(), String> {
    registrar.register(AtExitHook::new(flags, term))
}
