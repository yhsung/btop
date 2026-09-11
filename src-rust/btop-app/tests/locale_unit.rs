//! Locale hunt headless unit tests (P4 T3).
//!
//! Every scenario drives [`btop_app::locale::hunt_locale`] with a fake env
//! map and a [`MockOps`]; no test touches the real process locale or
//! process env, so the suite is hermetic and parallel-safe.

use std::cell::RefCell;
use std::collections::HashMap;

use btop_app::locale::{hunt_locale, is_utf8_locale_name, HuntLog, LocaleOps};

/// Scripted `setlocale(3)` boundary. `set_results` maps the requested
/// locale argument to its return (`None` = null = failure); anything not
/// listed echoes the argument back (success). All calls are recorded.
struct MockOps {
    set_results: HashMap<String, Option<String>>,
    std_name: Option<String>,
    mac_id: Option<String>,
    calls: RefCell<Vec<String>>,
}

impl MockOps {
    fn new() -> Self {
        Self {
            set_results: HashMap::new(),
            std_name: None,
            mac_id: None,
            calls: RefCell::new(Vec::new()),
        }
    }

    fn with_set(mut self, arg: &str, ret: Option<&str>) -> Self {
        self.set_results
            .insert(arg.to_string(), ret.map(str::to_string));
        self
    }

    fn with_std_name(mut self, name: &str) -> Self {
        self.std_name = Some(name.to_string());
        self
    }

    fn with_mac_id(mut self, id: &str) -> Self {
        self.mac_id = Some(id.to_string());
        self
    }

    fn set_calls(&self) -> Vec<String> {
        self.calls
            .borrow()
            .iter()
            .filter(|c| c.starts_with("set:"))
            .cloned()
            .collect()
    }
}

impl LocaleOps for MockOps {
    fn set_all(&self, locale: &str) -> Option<String> {
        self.calls.borrow_mut().push(format!("set:{locale}"));
        match self.set_results.get(locale) {
            Some(ret) => ret.clone(),
            None => Some(locale.to_string()),
        }
    }

    fn std_locale_name(&self) -> Option<String> {
        self.calls.borrow_mut().push("std_name".to_string());
        self.std_name.clone()
    }

    fn macos_locale_id(&self) -> Option<String> {
        self.calls.borrow_mut().push("mac_id".to_string());
        self.mac_id.clone()
    }
}

/// Fake process env: reads served from the map, writes recorded.
struct FakeEnv {
    vars: HashMap<String, String>,
    writes: RefCell<Vec<(String, String)>>,
    set_ok: bool,
}

impl FakeEnv {
    fn new(pairs: &[(&str, &str)]) -> Self {
        Self {
            vars: pairs
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            writes: RefCell::new(Vec::new()),
            set_ok: true,
        }
    }

    fn get(&self, key: &str) -> Option<String> {
        self.vars.get(key).cloned()
    }

    fn set(&self, key: &str, val: &str) -> bool {
        self.writes
            .borrow_mut()
            .push((key.to_string(), val.to_string()));
        self.set_ok
    }
}

fn hunt(
    env: &FakeEnv,
    ops: &MockOps,
    force_utf: bool,
    is_macos: bool,
) -> (Result<String, String>, HuntLog) {
    let mut log = HuntLog::default();
    let get = |k: &str| env.get(k);
    let mut set = |k: &str, v: &str| env.set(k, v);
    let res = hunt_locale(&get, &mut set, ops, force_utf, is_macos, &mut log);
    (res, log)
}

#[test]
fn utf8_predicate_strips_dashes_and_ignores_case() {
    assert!(is_utf8_locale_name("en_US.UTF-8"));
    assert!(is_utf8_locale_name("en_US.utf8"));
    assert!(is_utf8_locale_name("C.UTF8"));
    assert!(!is_utf8_locale_name("C"));
    assert!(!is_utf8_locale_name("POSIX"));
    assert!(!is_utf8_locale_name("en_US.ISO-8859-1"));
}

#[test]
fn current_locale_already_utf8_short_circuits() {
    // cpp:937-940 — setlocale(LC_ALL,"") already UTF-8, no ';': done.
    let env = FakeEnv::new(&[("LANG", "C")]);
    let ops = MockOps::new().with_set("", Some("en_US.UTF-8"));
    let (res, log) = hunt(&env, &ops, false, false);
    assert!(res.is_ok());
    assert_eq!(ops.set_calls(), vec!["set:".to_string()]);
    assert!(log.debug.iter().any(|l| l.contains("Using locale")));
    assert!(log.warning.is_empty());
}

#[test]
fn env_lang_utf8_is_adopted() {
    // cpp:944-952 — LANG carries UTF-8: adopt it.
    let env = FakeEnv::new(&[("LANG", "en_US.UTF-8")]);
    let ops = MockOps::new().with_set("", Some("C"));
    let (res, log) = hunt(&env, &ops, false, false);
    assert_eq!(res.unwrap(), "en_US.UTF-8");
    assert!(ops.set_calls().contains(&"set:en_US.UTF-8".to_string()));
    assert!(log
        .debug
        .iter()
        .any(|l| l.contains("Setting LC_ALL=en_US.UTF-8")));
}

#[test]
fn env_last_utf8_hit_wins() {
    // cpp:944-952 — every UTF-8 value is tried; the last one wins `found`.
    let env = FakeEnv::new(&[
        ("LANG", "en_US.UTF-8"),
        ("LC_ALL", "sv_SE.UTF-8"),
        ("LC_CTYPE", "C"),
    ]);
    let ops = MockOps::new().with_set("", Some("C"));
    let (res, _) = hunt(&env, &ops, false, false);
    assert_eq!(res.unwrap(), "sv_SE.UTF-8");
}

#[test]
fn std_locale_split_piece_is_adopted() {
    // cpp:953-969 — no UTF-8 in env; std::locale("").name() composite
    // split on ';', first UTF-8 piece adopted (name after '=').
    let env = FakeEnv::new(&[("LANG", "C"), ("LC_ALL", "C"), ("LC_CTYPE", "C")]);
    let ops = MockOps::new()
        .with_set("", Some("C"))
        .with_std_name("LC_CTYPE=en_US.UTF-8;LC_NUMERIC=C;LC_TIME=C");
    let (res, log) = hunt(&env, &ops, false, false);
    assert_eq!(res.unwrap(), "en_US.UTF-8");
    assert!(ops.set_calls().contains(&"set:en_US.UTF-8".to_string()));
    // The hunt clears LC_ALL/LANG before consulting std::locale (cpp:954).
    let writes = env.writes.borrow();
    assert!(writes.contains(&("LC_ALL".to_string(), String::new())));
    assert!(writes.contains(&("LANG".to_string(), String::new())));
    assert!(log
        .debug
        .iter()
        .any(|l| l.contains("Setting LC_ALL=en_US.UTF-8")));
}

#[test]
fn macos_fallback_uses_cf_locale_id() {
    // cpp:972-990 — found still empty on macOS: "<CF id>.UTF-8" tried.
    let env = FakeEnv::new(&[("LANG", "C")]);
    let ops = MockOps::new().with_set("", Some("C")).with_mac_id("fr_FR");
    let (res, log) = hunt(&env, &ops, false, true);
    assert_eq!(res.unwrap(), "fr_FR.UTF-8");
    assert!(ops.set_calls().contains(&"set:fr_FR.UTF-8".to_string()));
    assert!(log.debug.iter().any(|l| l.contains("fr_FR.UTF-8")));
}

#[test]
fn macos_unavailable_cf_lookup_falls_back_to_en_us() {
    // Plan fallback: CF lookup itself unavailable (None) — try en_US.UTF-8
    // directly (mirrors the cpp:984 fallback arm without needing the id).
    let env = FakeEnv::new(&[("LANG", "C")]);
    let ops = MockOps::new().with_set("", Some("C"));
    let (res, _) = hunt(&env, &ops, false, true);
    assert_eq!(res.unwrap(), "en_US.UTF-8");
    assert!(ops.set_calls().contains(&"set:en_US.UTF-8".to_string()));
}

#[test]
fn macos_empty_cf_id_warns_and_continues() {
    // cpp:978-980 — CF id present but empty: warn, continue anyway (macOS
    // never hard-fails the hunt; there is no clean_quit on this path).
    let env = FakeEnv::new(&[("LANG", "C")]);
    let ops = MockOps::new().with_set("", Some("C")).with_mac_id("");
    let (res, log) = hunt(&env, &ops, false, true);
    assert!(res.is_ok());
    assert!(log
        .warning
        .iter()
        .any(|l| l.contains("Some symbols might not display correctly")));
}

#[test]
fn set_failure_warns_but_proceeds() {
    // cpp:947-950 — env locale UTF-8 but setlocale rejects it: warn,
    // continue; final "Setting LC_ALL=" debug suppressed (cpp:999).
    let env = FakeEnv::new(&[("LANG", "en_US.UTF-8")]);
    let ops = MockOps::new()
        .with_set("", Some("C"))
        .with_set("en_US.UTF-8", None);
    let (res, log) = hunt(&env, &ops, false, false);
    assert_eq!(res.unwrap(), "en_US.UTF-8");
    assert!(log
        .warning
        .iter()
        .any(|l| l.contains("Failed to set locale")));
    assert!(!log.debug.iter().any(|l| l.contains("Setting LC_ALL=")));
}

#[test]
fn no_utf8_with_force_utf_warns_and_proceeds() {
    // cpp:992-993 — nothing found, --force-utf: warn, start anyway.
    let env = FakeEnv::new(&[("LANG", "C")]);
    let ops = MockOps::new().with_set("", Some("C"));
    let (res, log) = hunt(&env, &ops, true, false);
    assert!(res.is_ok());
    assert!(log.warning.iter().any(|l| l.contains("Forcing")));
}

#[test]
fn no_utf8_without_force_utf_is_hard_error() {
    // cpp:994-997 — nothing found, no --force-utf: exit_error_msg +
    // clean_quit(1); here surfaced as Err for the caller (T7) to decide.
    let env = FakeEnv::new(&[("LANG", "C")]);
    let ops = MockOps::new().with_set("", Some("C"));
    let (res, _) = hunt(&env, &ops, false, false);
    assert_eq!(
        res.unwrap_err(),
        "No UTF-8 locale detected!\nUse --force-utf argument to force start if you're sure your terminal can handle it."
    );
}
