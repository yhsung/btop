//! Signal handler headless unit tests (P4 T4).
//!
//! All handler behavior runs through direct `on_*` invocation and the
//! `MockInstaller` / `MockCrashEnv` / `MockAtExit` doubles; no test
//! delivers a real signal, installs a real sigaction, or calls `raise`.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::Arc;

use btop_app::signals::{
    exit_hook_fn, install_signals, on_sigcont, on_sigint, on_sigtstp, on_sigusr1, on_sigusr2,
    on_sigwinch, sigusr1_block_mask, AtExitHook, CrashEnv, MockAtExit, MockCrashEnv, MockInstaller,
    RealInstaller, SignalInstaller, CRASH_SIGNALS,
};
use btop_app::term::{MockOps, TermWrapper};
use btop_runner::wiring::SignalFlags;
use nix::sys::signal::Signal;

fn flags() -> Arc<SignalFlags> {
    Arc::new(SignalFlags::new())
}

fn load(f: &AtomicBool) -> bool {
    f.load(Ordering::SeqCst)
}

fn all_clear(f: &SignalFlags) -> bool {
    !(load(&f.resized)
        || load(&f.should_quit)
        || load(&f.should_sleep)
        || load(&f.do_continue)
        || load(&f.interrupt_input)
        || load(&f.reload_conf)
        || load(&f.stopping)
        || load(&f.quitting))
}

#[test]
fn default_flags_all_clear() {
    assert!(all_clear(&SignalFlags::default()));
}

#[test]
fn clone_shares_state_with_handlers() {
    // Ownership ruling: `SignalFlags::clone()` shares every flag — a
    // handler installed with a clone observes the same state as the
    // main loop's copy.
    let a = flags();
    let b = a.clone();
    on_sigwinch(&b);
    assert!(load(&a.resized));
    assert!(load(&a.interrupt_input));
    on_sigint(&a);
    assert!(load(&b.should_quit));
}

#[test]
fn sigint_sets_should_quit_stopping_interrupt() {
    let f = flags();
    on_sigint(&f);
    assert!(load(&f.should_quit));
    assert!(load(&f.stopping));
    assert!(load(&f.interrupt_input));
    assert!(!load(&f.should_sleep));
    assert!(!load(&f.resized));
    assert!(!load(&f.reload_conf));
}

#[test]
fn sigtstp_sets_should_sleep_stopping_interrupt() {
    let f = flags();
    on_sigtstp(&f);
    assert!(load(&f.should_sleep));
    assert!(load(&f.stopping));
    assert!(load(&f.interrupt_input));
    assert!(!load(&f.should_quit));
}

#[test]
fn sigcont_sets_do_continue_and_wakes_poll() {
    let f = flags();
    on_sigcont(&f);
    assert!(load(&f.do_continue));
    assert!(load(&f.interrupt_input));
    assert!(!load(&f.should_quit));
    assert!(!load(&f.resized));
}

#[test]
fn sigwinch_sets_resized_and_wakes_poll() {
    let f = flags();
    on_sigwinch(&f);
    assert!(load(&f.resized));
    assert!(load(&f.interrupt_input));
    assert!(!load(&f.should_quit));
}

#[test]
fn sigusr1_only_wakes_poll() {
    let f = flags();
    on_sigusr1(&f);
    assert!(load(&f.interrupt_input));
    assert!(!load(&f.should_quit));
    assert!(!load(&f.resized));
    assert!(!load(&f.reload_conf));
    assert!(!load(&f.do_continue));
}

#[test]
fn sigusr2_sets_reload_conf_and_wakes_poll() {
    let f = flags();
    on_sigusr2(&f);
    assert!(load(&f.reload_conf));
    assert!(load(&f.interrupt_input));
    assert!(!load(&f.should_quit));
}

#[test]
fn clear_all_resets_every_flag() {
    let f = flags();
    on_sigint(&f);
    on_sigwinch(&f);
    on_sigusr2(&f);
    f.quitting.store(true, Ordering::SeqCst);
    assert!(!all_clear(&f));
    f.clear_all();
    assert!(all_clear(&f));
}

#[test]
fn mock_install_records_call_and_succeeds() {
    let mock = MockInstaller::new();
    let f = flags();
    assert!(install_signals(&mock, &f).is_ok());
    assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
    assert!(install_signals(&mock, &f).is_ok());
    assert_eq!(mock.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn mock_install_failure_path_propagates() {
    let mock = MockInstaller::new();
    mock.set_fail(true);
    let f = flags();
    let err = install_signals(&mock, &f).unwrap_err();
    assert!(!err.is_empty());
    assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn real_installer_constructs_without_term() {
    // Constructor only — never installs (no real sigaction in tests).
    let _ = RealInstaller::new();
}

#[test]
fn crash_restores_term_when_initialized() {
    let env = MockCrashEnv::new(true);
    btop_app::signals::on_crash(&env, Signal::SIGSEGV as i32);
    assert_eq!(env.restore_calls.load(Ordering::SeqCst), 1);
    assert_eq!(env.raised.load(Ordering::SeqCst), Signal::SIGSEGV as i32);
}

#[test]
fn crash_skips_restore_when_uninitialized() {
    let env = MockCrashEnv::new(false);
    btop_app::signals::on_crash(&env, Signal::SIGABRT as i32);
    assert_eq!(env.restore_calls.load(Ordering::SeqCst), 0);
    assert_eq!(env.raised.load(Ordering::SeqCst), Signal::SIGABRT as i32);
}

#[test]
fn crash_signal_list_matches_cpp_install_block() {
    // src/btop.cpp:1059-1063 install block: SIGSEGV, SIGABRT, SIGTRAP,
    // SIGBUS, SIGILL.
    assert_eq!(
        CRASH_SIGNALS,
        [
            Signal::SIGSEGV,
            Signal::SIGABRT,
            Signal::SIGTRAP,
            Signal::SIGBUS,
            Signal::SIGILL,
        ]
    );
}

#[test]
fn sigusr1_block_mask_contains_sigusr1() {
    let mask = sigusr1_block_mask();
    assert!(mask.contains(Signal::SIGUSR1));
    assert!(!mask.contains(Signal::SIGINT));
}

fn term(initialized: bool) -> (Arc<MockOps>, Arc<TermWrapper>) {
    let ops = Arc::new(MockOps::new());
    let term = Arc::new(TermWrapper::new(ops.clone()));
    if initialized {
        assert!(term.init());
    }
    (ops, term)
}

#[test]
fn exit_hook_restores_term_and_sets_quitting() {
    let (ops, term) = term(true);
    let f = flags();
    let hook = exit_hook_fn(f.clone(), term);
    hook();
    assert_eq!(ops.restore_calls.load(Ordering::SeqCst), 1);
    assert!(load(&f.quitting));
}

#[test]
fn exit_hook_without_init_only_sets_quitting() {
    let (ops, term) = term(false);
    let f = flags();
    AtExitHook::new(f.clone(), term).run();
    assert_eq!(ops.restore_calls.load(Ordering::SeqCst), 0);
    assert!(load(&f.quitting));
}

#[test]
fn atexit_registration_recorded_by_mock() {
    let mock = MockAtExit::new();
    let (_, term) = term(false);
    let f = flags();
    assert!(btop_app::signals::register_atexit(&mock, f, term).is_ok());
    assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn atexit_failure_path_propagates() {
    let mock = MockAtExit::new();
    mock.set_fail(true);
    let (_, term) = term(false);
    let f = flags();
    let err = btop_app::signals::register_atexit(&mock, f, term).unwrap_err();
    assert!(!err.is_empty());
}

// Compile-time pins: the doubles stay object-safe / shareable the way
// boot code uses them (`&dyn SignalInstaller`, `&dyn CrashEnv`).
#[test]
fn doubles_are_object_safe() {
    fn assert_installer<T: SignalInstaller + Send + Sync>() {}
    fn assert_crash<T: CrashEnv + Send + Sync>() {}
    assert_installer::<MockInstaller>();
    assert_installer::<RealInstaller>();
    assert_crash::<MockCrashEnv>();
    let _ = AtomicU32::new(0);
    let _ = AtomicI32::new(0);
}
