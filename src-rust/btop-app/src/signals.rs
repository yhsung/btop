//! Signal handlers with mockable install (P4 T4).
//!
//! C++ truth: install block at `src/btop.cpp:1048-1065`, `_signal_handler`
//! at `:290-322`, `_crash_handler` at `:280-288`
//! (`signal(sig, SIG_DFL)` + `raise(sig)` at `:286-287`), `_exit_handler`
//! at `:276-278` (`clean_quit(-1)`), `_sleep` at `:264-269`,
//! `_resume` at `:271-274`.
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
//! Crash signals (install block :1056-1060): SIGSEGV, SIGABRT, SIGTRAP,
//! SIGBUS, SIGILL → [`on_crash`] (restore term if initialized, reset the
//! handler to `SIG_DFL`, re-raise — mirroring `_crash_handler`).
//! The restore path is best-effort (not strictly async-signal-safe),
//! same as upstream — see [`on_crash`].
//!
//! # Handler safety
//!
//! The `on_sig*` fns only perform atomic stores (`Ordering::SeqCst`) on the
//! passed [`SignalFlags`] — no allocation, no locking, no I/O — so they
//! are async-signal-safe and may run in real signal context via the
//! `extern "C"` trampolines installed by [`RealInstaller`]. [`on_crash`]
//! is deliberately excluded from that claim: like the C++ `_crash_handler`
//! it restores the terminal (ioctl/write), which is best-effort rather
//! than strictly async-signal-safe — same as upstream. The
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
/// is sound while the owning [`OWNING_FLAGS`] entry is alive.
static HANDLER_FLAGS: AtomicPtr<SignalFlags> = AtomicPtr::new(std::ptr::null_mut());

/// Term published at install for the crash trampoline (null = no term;
/// restore is skipped but the reset + re-raise still run).
static CRASH_TERM_PTR: AtomicPtr<TermWrapper> = AtomicPtr::new(std::ptr::null_mut());

/// Owning refs that keep the trampoline pointees alive for process
/// lifetime. The type system enforces this: [`publish_flags`] takes
/// `Arc`s (not borrows), clones them here, and never removes them, so a
/// short-lived caller `Arc` cannot leave a dangling trampoline pointer.
/// These are only locked at install time (never in signal context).
static OWNING_FLAGS: Mutex<Option<Arc<SignalFlags>>> = Mutex::new(None);
static OWNING_TERM: Mutex<Option<Arc<TermWrapper>>> = Mutex::new(None);

/// Publish handler state for the trampolines.
///
/// Takes shared ownership (`Arc`) and stashes a clone in the owning
/// statics, so the raw pointers in [`HANDLER_FLAGS`]/[`CRASH_TERM_PTR`]
/// stay valid as long as any signal can be delivered. Call once at boot
/// (via [`RealInstaller::install`]) with the process-lifetime flags/term.
fn publish_flags(flags: Arc<SignalFlags>, term: Option<Arc<TermWrapper>>) {
    let flags_ptr = Arc::as_ptr(&flags) as *mut SignalFlags;
    let term_ptr = term
        .as_ref()
        .map_or_else(std::ptr::null_mut, |t| Arc::as_ptr(t) as *mut TermWrapper);
    // Store owners first so the pointers are valid before they become
    // visible to handlers. `expect` is fine: install never runs in
    // signal context, and a poisoned boot mutex is unrecoverable.
    *OWNING_FLAGS
        .lock()
        .expect("signal flags owner slot poisoned") = Some(flags);
    *OWNING_TERM.lock().expect("crash term owner slot poisoned") = term;
    HANDLER_FLAGS.store(flags_ptr, Ordering::SeqCst);
    CRASH_TERM_PTR.store(term_ptr, Ordering::SeqCst);
}

/// Flags for the current trampoline, if published.
fn handler_flags() -> Option<&'static SignalFlags> {
    // SAFETY: null-checked; non-null implies a live allocation owned by
    // `OWNING_FLAGS`, which `publish_flags` never removes.
    unsafe { HANDLER_FLAGS.load(Ordering::SeqCst).as_ref() }
}

/// Term for the crash trampoline, if published and alive (same contract
/// via `OWNING_TERM`).
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
/// [`RealInstaller`]; tests pass [`MockInstaller`]. `install` takes an
/// `Arc` (not a borrow) so the production impl can transfer shared
/// ownership into the trampoline globals — see [`publish_flags`].
pub trait SignalInstaller: Send + Sync {
    fn install(&self, flags: Arc<SignalFlags>) -> Result<(), String>;
}

/// Production installer: `sigaction` per the C++ install block
/// (btop.cpp:1048-1065: regular :1049-1054) + crash handlers (:1056-1060)
/// + SIGUSR1 process-mask block (:1062-1065).
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
    fn install(&self, flags: Arc<SignalFlags>) -> Result<(), String> {
        publish_flags(flags, self.term.clone());
        // btop.cpp:1049-1054: regular handlers.
        install_sigaction(Signal::SIGINT, sigint_trampoline)?;
        install_sigaction(Signal::SIGTSTP, sigtstp_trampoline)?;
        install_sigaction(Signal::SIGCONT, sigcont_trampoline)?;
        install_sigaction(Signal::SIGWINCH, sigwinch_trampoline)?;
        install_sigaction(Signal::SIGUSR1, sigusr1_trampoline)?;
        install_sigaction(Signal::SIGUSR2, sigusr2_trampoline)?;
        // btop.cpp:1056-1060: crash handlers.
        for sig in CRASH_SIGNALS {
            install_sigaction(sig, crash_trampoline)?;
        }
        // btop.cpp:1062-1065: block SIGUSR1 so it only wakes `pselect`
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
    fn install(&self, _flags: Arc<SignalFlags>) -> Result<(), String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) {
            Err("mock install failure".to_string())
        } else {
            Ok(())
        }
    }
}

/// Boot seam: install via the given installer (real or mock).
pub fn install_signals(
    installer: &dyn SignalInstaller,
    flags: Arc<SignalFlags>,
) -> Result<(), String> {
    installer.install(flags)
}

/// The SIGUSR1 mask from the C++ install block (btop.cpp:1062-1065).
/// [`RealInstaller`] applies it process-wide; T7's `pselect` uses the
/// same set as its wait mask.
pub fn sigusr1_block_mask() -> SigSet {
    let mut mask = SigSet::empty();
    mask.add(Signal::SIGUSR1);
    mask
}

// ── Crash path ────────────────────────────────────────────────────────────

/// Crash signals installed alongside the regular handlers
/// (btop.cpp:1056-1060, verbatim order).
pub const CRASH_SIGNALS: [Signal; 5] = [
    Signal::SIGSEGV,
    Signal::SIGABRT,
    Signal::SIGTRAP,
    Signal::SIGBUS,
    Signal::SIGILL,
];

/// Testable crash environment: term restore + reset-to-default + re-raise,
/// mirroring `_crash_handler` ("restore terminal before crashing … re-raise
/// the signal to get default behavior", btop.cpp:280-288).
///
/// The restore step is best-effort (not strictly async-signal-safe — it
/// performs terminal ioctl/write), same as upstream; the reset + raise
/// steps are signal-safe.
pub trait CrashEnv: Send + Sync {
    fn term_initialized(&self) -> bool;
    fn restore_term(&self);
    /// Reset the crashing signal's disposition to `SIG_DFL` (btop.cpp:286).
    /// Must run before [`CrashEnv::reraise`]: without it the re-raised
    /// signal would re-enter the crash trampoline (infinite recursion).
    fn reset_default(&self, sig: c_int);
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

    fn reset_default(&self, sig: c_int) {
        reset_crash_disposition(sig);
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

/// Reset a crashing signal's disposition to `SIG_DFL` (btop.cpp:286),
/// best-effort: errors are ignored because the subsequent re-raise is
/// unconditional. Extracted as a free fn (rather than inline in
/// [`RealCrashEnv::reset_default`]) so its construction is unit-visible;
/// never call it from tests with a live signal — it would disarm the
/// test process's own disposition.
fn reset_crash_disposition(sig: c_int) {
    if let Ok(sig) = Signal::try_from(sig) {
        let action = SigAction::new(SigHandler::SigDfl, SaFlags::empty(), SigSet::empty());
        // SAFETY: installing `SigDfl` performs no user-code execution;
        // the args only describe the default disposition.
        let _ = unsafe { sigaction(sig, &action) };
    } else {
        // Raw number outside nix's `Signal` enum (trap-like values):
        // fall back to `libc::signal`, also async-signal-safe.
        unsafe {
            nix::libc::signal(sig, nix::libc::SIG_DFL);
        }
    }
}

/// Recording test double: no real restore (beyond the flag), no real
/// reset, no real raise — the signal number and the reset→raise order
/// are recorded instead (see [`MockCrashEnv::events`]).
pub struct MockCrashEnv {
    initialized: AtomicBool,
    pub restore_calls: AtomicU32,
    pub raised: AtomicI32,
    pub reset_calls: AtomicU32,
    pub reset_sig: AtomicI32,
    events: Mutex<Vec<&'static str>>,
}

impl MockCrashEnv {
    pub fn new(initialized: bool) -> Self {
        Self {
            initialized: AtomicBool::new(initialized),
            restore_calls: AtomicU32::new(0),
            raised: AtomicI32::new(-1),
            reset_calls: AtomicU32::new(0),
            reset_sig: AtomicI32::new(-1),
            events: Mutex::new(Vec::new()),
        }
    }

    /// Recorded `"reset"`/`"raise"` markers in call order; the crash
    /// contract is `["reset", "raise"]`.
    pub fn events(&self) -> Vec<&'static str> {
        self.events.lock().expect("mock events poisoned").clone()
    }
}

impl CrashEnv for MockCrashEnv {
    fn term_initialized(&self) -> bool {
        self.initialized.load(Ordering::SeqCst)
    }

    fn restore_term(&self) {
        self.restore_calls.fetch_add(1, Ordering::SeqCst);
    }

    fn reset_default(&self, sig: c_int) {
        self.reset_calls.fetch_add(1, Ordering::SeqCst);
        self.reset_sig.store(sig, Ordering::SeqCst);
        self.events
            .lock()
            .expect("mock events poisoned")
            .push("reset");
    }

    fn reraise(&self, sig: c_int) {
        self.raised.store(sig, Ordering::SeqCst);
        self.events
            .lock()
            .expect("mock events poisoned")
            .push("raise");
    }
}

/// Crash body shared by the trampoline and tests: restore the term iff
/// initialized, then reset the disposition to `SIG_DFL` and re-raise
/// unconditionally.
///
/// Mirrors the C++ `_crash_handler` (btop.cpp:280-288); like upstream,
/// the restore step is best-effort (not strictly async-signal-safe),
/// while reset + raise are signal-safe.
pub fn on_crash(env: &dyn CrashEnv, sig: c_int) {
    if env.term_initialized() {
        env.restore_term();
    }
    // Reset-before-raise (btop.cpp:286-287): re-raising with the crash
    // trampoline still installed would re-enter `on_crash` forever.
    env.reset_default(sig);
    env.reraise(sig);
}

// ── atexit ────────────────────────────────────────────────────────────────

/// Exit hook payload: the atexit body (`_exit_handler` → `clean_quit(-1)`
/// at btop.cpp:276-278) reduced to what can run without `World` — restore
/// the term and mark `quitting` so a second exit path becomes a no-op. The
/// T7 `clean_quit` (World-owned: config write, error message, runtime
/// print) runs on the main exit path; this hook only covers abnormal exits
/// (panics) where `World` is already gone.
///
/// Wired to real `atexit` by T7 `fn main` (via [`RealAtExit`]) — the T4
/// "do not wire until clean_quit lands" gate is lifted: every normal exit
/// goes through the real `clean_quit`, and this fallback can only
/// under-clean (never double-clean: `clean_quit`'s `quitting` guard makes
/// the main path idempotent).
pub struct AtExitHook {
    flags: Arc<SignalFlags>,
    term: Arc<TermWrapper>,
}

impl AtExitHook {
    pub fn new(flags: Arc<SignalFlags>, term: Arc<TermWrapper>) -> Self {
        Self { flags, term }
    }

    pub fn run(self) {
        // Atexit fallback (see struct docs): term restore + `quitting`
        // only — the World-owned T7 `clean_quit(-1)` steps run on the main
        // exit path, not here.
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
/// with `atexit` (mirrors `std::atexit(_exit_handler)` at btop.cpp:1048).
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
