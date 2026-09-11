//! Term wrapper headless unit tests (P4 T2).
//!
//! All terminal behavior runs through `MockOps`; no test touches real
//! termios fds or installs signal handlers.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use btop_app::term::{MockOps, TermWrapper};

fn harness(mock: MockOps) -> (Arc<MockOps>, TermWrapper) {
    let ops = Arc::new(mock);
    let term = TermWrapper::new(ops.clone());
    (ops, term)
}

fn load(counter: &std::sync::atomic::AtomicU32) -> u32 {
    counter.load(Ordering::Relaxed)
}

#[test]
fn init_success_marks_initialized_and_syncs_size() {
    let (ops, term) = harness(MockOps::new().with_size(120, 40));
    assert!(term.init());
    assert_eq!(load(&ops.init_calls), 1);
    assert!(term.is_initialized());
    assert_eq!((term.width(), term.height()), (120, 40));
}

#[test]
fn init_failure_leaves_uninitialized() {
    let mock = MockOps::new();
    mock.init_ret.store(false, Ordering::Relaxed);
    let (ops, term) = harness(mock);
    assert!(!term.init());
    assert_eq!(load(&ops.init_calls), 1);
    assert!(!term.is_initialized());
}

#[test]
fn refresh_reports_change_and_syncs_unless_only_check() {
    let (ops, term) = harness(MockOps::new().with_size(100, 30));
    ops.refresh_ret.store(true, Ordering::Relaxed);

    assert!(term.refresh(false));
    assert_eq!(load(&ops.refresh_calls), 1);
    assert_eq!((term.width(), term.height()), (100, 30));

    // `only_check` reports the change without syncing stored dims.
    ops.width.store(200, Ordering::Relaxed);
    assert!(term.refresh(true));
    assert_eq!((term.width(), term.height()), (100, 30));
}

#[test]
fn refresh_no_change_keeps_dims() {
    let (_, term) = harness(MockOps::new());
    assert!(!term.refresh(false));
    assert_eq!((term.width(), term.height()), (80, 24));
}

#[test]
fn restore_is_idempotent() {
    let (ops, term) = harness(MockOps::new());
    assert!(term.init());
    term.restore();
    term.restore();
    assert_eq!(load(&ops.restore_calls), 2);
    assert!(!term.is_initialized());
}

#[test]
fn get_min_size_default() {
    let (_, term) = harness(MockOps::new());
    assert_eq!(term.get_min_size(), (100, 24));
}

#[test]
fn real_term_constructor_defaults_are_headless_safe() {
    // Constructor only; never calls init/refresh/restore.
    let t = btop_tools::term::Term::new();
    assert_eq!((t.width(), t.height()), (80, 24));
    assert!(!t.is_initialized());
}
