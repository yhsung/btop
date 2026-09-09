//! Options-menu state machine: typing inference, paging, edit/commit and
//! left/right flip/cycle, transcribed from `optionsMenu`
//! (`src/btop_menu.cpp:1298-1742`).
//!
//! SCOPE: pure state + validation. No drawing (Task 6 assembles the overlay
//! from [`OptionRow`]), no key dispatch (Task 5 feeds keys into
//! [`MenuState`]/[`commit_edit`]/[`flip_or_cycle`]).
//!
//! SHOWN_BOXES DECISION (P2 Task 4 Step 0b): C++ validates `shown_boxes`
//! against live terminal/box state (`stringValid`, `:610-619` — needs `Term`
//! + `set_boxes`), so the Rust `Config::string_valid` always passes for it.
//! The commit path below special-cases the key ([`commit_edit`] accepts
//! `shown_boxes` WITHOUT calling `string_valid`): the staged value is
//! validated at layout time in P3 and is never persisted on bare `true`
//! (acceptance stages into [`OptionsStore`] only — no `WriteConfig` is
//! emitted; P3 layout decides what reaches disk).
//!
//! STORE/SINK SPLIT: [`OptionsStore`] holds owned option values, initialized
//! FROM `btop-config` `Config` in P3 (tests construct it literally).
//! [`commit_edit`]/[`flip_or_cycle`] mutate the store in place for the
//! Config write itself (`Config::set`/`flip` in C++) and return ONLY the
//! side-effect [`Action`]s (theme/layout/recollect/...). P3 syncs
//! store → `Config`. Consequences: no generic `SetString` carrier is needed
//! (every string write stages locally); `lowcolor` (truecolor path, `:1503`)
//! and `tty_mode` (force_tty path, `:1511`) flip alongside in the store.
//! `atomic_wait(Runner::active)` (`:1393`, `:1579`) is a sink duty
//! documented on [`Action::ResetPreset`]/[`Action::RefreshCoreMapping`].
//!
//! STATE SPLIT: C++ `theme_refresh`/`screen_redraw`/`recollect` are PER-CALL
//! locals (`:1352-1354`). [`MenuState::theme_refresh`]/
//! [`MenuState::screen_redraw`] mirror them for the overlay layer; the
//! commit/flip fns emit the corresponding `Action`s
//! ([`Action::ApplyTheme`]/[`Action::RecalcLayout`]/[`Action::Run`]) directly,
//! and Task 5 maintains the flags from the returned actions. Flag mapping
//! (per option, with source lines) is pinned by the flip/cycle tests below:
//! - edit commit: `custom_cpu_name`/`custom_gpu_name*` → screen_redraw
//!   (`:1389-1390`); `shown_boxes`/`presets` → screen_redraw + preset reset
//!   (`:1391-1394`); `clock_format` → update_clock + screen_redraw (`:1396-1398`);
//!   `cpu_core_map` → core remap (`:1400-1402`); ints → no flags (`:1405-1406`).
//! - bool flip (`:1496-1529`): screen_redraw ALWAYS (`:1497-1498`); `truecolor` →
//!   theme_refresh + lowcolor (`:1501-1503`); `force_tty` → theme_refresh +
//!   tty_mode (`:1506-1511`); `rounded_corners`/`theme_background` →
//!   theme_refresh (`:1513-1514`); `background_update` → pause_output=false
//!   (`:1515-1516`); `base_10_sizes` → recollect (`:1518-1519`);
//!   `save_config_on_exit` → write() when now false (`:1521-1525`);
//!   `disable_mouse` → Term mouse on/off (`:1527-1529`).
//! - browsable cycle (`:1532-1581`): `color_theme` → theme_refresh (`:1550-1564`);
//!   `log_level` → Logger level (`:1569-1571`); `base_10_bitrate` → recollect
//!   (`:1573-1574`); `proc_sorting`/`cpu_sensor`/`show_gpu_info`/
//!   `graph_symbol*`/`cpu_graph_*` → screen_redraw (`:1576-1577`);
//!   `disable_presets` → preset reset unless new is `"Off"` (`:1578-1580`).
//! - end-of-call sequencing: theme_refresh → setTheme + banner +
//!   screen_redraw (`:1720-1726`); screen_redraw → calcSizes (overlay/clock
//!   preserved, `:1727-1734`); recollect → `Runner::run("all", false, true)`
//!   (`:1735-1738`).
//!
//! DRAW SPLIT: [`option_rows`] returns plain [`OptionRow`] data (name, value,
//! browse index label); Task 6 does all box/banner/mouse-map assembly. The
//! C++ draw block is `:1592-1716`.
//!
//! `update_ms` step is ±100, every other int ±1 (`:1485`).

use std::collections::HashMap;

use btop_config::config::Config;
use btop_input::actions::{Action, RunTarget};
use btop_input::textedit::TextEdit;

/// Options-menu cursor/edit/flag state. Mirrors the C++ per-menu statics
/// (`:1299-1314`: `selected_cat` → [`MenuState::tab`], `selected`, `page`,
/// `editing`, `editor`, `warnings`).
///
/// `Debug`/`Default` are manual: [`TextEdit`] provides neither (and
/// `btop-input/src/textedit.rs` is out of scope for this task), so deriving
/// is impossible. Defaults match a freshly opened menu: tab/selected/page 0,
/// not editing, empty editor, no warnings, flags clear.
pub struct MenuState {
    pub tab: usize,
    pub selected: usize,
    pub page: usize,
    pub editing: bool,
    pub editor: TextEdit,
    pub warnings: Option<String>,
    pub theme_refresh: bool,
    pub screen_redraw: bool,
}

impl std::fmt::Debug for MenuState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MenuState")
            .field("tab", &self.tab)
            .field("selected", &self.selected)
            .field("page", &self.page)
            .field("editing", &self.editing)
            .field("editor_text", &self.editor.text)
            .field("warnings", &self.warnings)
            .field("theme_refresh", &self.theme_refresh)
            .field("screen_redraw", &self.screen_redraw)
            .finish()
    }
}

impl Default for MenuState {
    fn default() -> Self {
        Self {
            tab: 0,
            selected: 0,
            page: 0,
            editing: false,
            editor: TextEdit::new(String::new(), false),
            warnings: None,
            theme_refresh: false,
            screen_redraw: false,
        }
    }
}

/// Owned option values, initialized FROM `btop-config` `Config` in P3 (tests
/// construct literally). Edits stage here; see the module-level store/sink
/// split.
#[derive(Debug, Default, Clone)]
pub struct OptionsStore {
    pub strings: HashMap<String, String>,
    pub bools: HashMap<String, bool>,
    pub ints: HashMap<String, i64>,
}

/// Per-option interaction kind. Mirrors the C++ `selPred` predispositions
/// (`:1299`, derived `:1605-1621`): `isBool` → [`OptKind::Bool`],
/// `isInt` → [`OptKind::Int`], `isString` → [`OptKind::Str`],
/// `isBrowsable` → [`OptKind::Browsable`], `isEditable` → [`OptKind::Editable`}.
/// [`OptKind::TwoD`] is the derived `is2D` marker (`!isString`, `:1615-1616`):
/// [`classify_option`] never returns it directly (bools/ints classify as
/// [`OptKind::Bool`]/[`OptKind::Int`); use [`is_2d`] to test the property,
/// which holds for [`OptKind::Bool`], [`OptKind::Int`] and [`OptKind::TwoD`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OptKind {
    Bool,
    Int,
    Str,
    TwoD,
    Browsable,
    Editable,
}

/// Options with a static value list (`optionsList`, `:1315-1340`), Apple
/// Silicon + `GPU_SUPPORT` scope (matching `tables.rs`): `freq_mode` is
/// `__linux__`-only and excluded.
pub const BROWSABLE_OPTIONS: &[&str] = &[
    "color_theme",
    "log_level",
    "temp_scale",
    "proc_sorting",
    "graph_symbol",
    "graph_symbol_cpu",
    "graph_symbol_mem",
    "graph_symbol_net",
    "graph_symbol_proc",
    "graph_symbol_gpu",
    "cpu_graph_upper",
    "cpu_graph_lower",
    "cpu_sensor",
    "selected_battery",
    "base_10_bitrate",
    "disable_presets",
    "show_gpu_info",
];

/// Classify an option per the `:1605-1621` inference: ints → [`OptKind::Int`],
/// bools → [`OptKind::Bool`], strings in [`BROWSABLE_OPTIONS`] →
/// [`OptKind::Browsable`], other known strings → [`OptKind::Editable`]
/// (`!isBrowsable && (isString || isInt)`, `:1620-1621` — ints are editable
/// too, but classify as [`OptKind::Int`]); unknown names → [`OptKind::Str`].
pub fn classify_option(name: &str, store: &OptionsStore) -> OptKind {
    if store.bools.contains_key(name) {
        OptKind::Bool
    } else if store.ints.contains_key(name) {
        OptKind::Int
    } else if store.strings.contains_key(name) {
        if BROWSABLE_OPTIONS.iter().any(|b| *b == name) {
            OptKind::Browsable
        } else {
            OptKind::Editable
        }
    } else {
        OptKind::Str
    }
}

/// Derived `is2D` property (`:1615-1616`: set whenever the option is not a
/// string — i.e. bool/int left/right stepping applies).
pub fn is_2d(kind: OptKind) -> bool {
    matches!(kind, OptKind::Bool | OptKind::Int | OptKind::TwoD)
}

/// Page count: `ceil(n_options / item_height)` (`:1597`).
/// `item_height == 0` is unreachable in C++ (`min(len, ...)` with non-empty
/// categories); guarded to 1 page rather than dividing by zero.
pub fn page_count(n_options: usize, item_height: usize) -> usize {
    if item_height == 0 {
        return 1;
    }
    n_options.div_ceil(item_height)
}

/// Visible rows per page: `min(n_options, floor((height - 4) / 2))` (`:1596`).
pub fn item_height_for(n_options: usize, height: usize) -> usize {
    n_options.min(height.saturating_sub(4) / 2)
}

/// Max selectable row on a page:
/// `min(item_height - 1, n_options - 1 - item_height * page)` (`:1599`,
/// saturating — C++ clamps `page` first at `:1598`).
pub fn select_max(n_options: usize, item_height: usize, page: usize) -> usize {
    let a = item_height.saturating_sub(1);
    let b = n_options
        .saturating_sub(1)
        .saturating_sub(item_height.saturating_mul(page));
    a.min(b)
}

/// Menu display value. Mirrors `getAsString` (`src/btop_config.cpp:670-678`)
/// with the `color_theme` stem exception (`:1669`:
/// `fs::path(getS("color_theme")).stem()`): bools as `"True"`/`"False"`,
/// ints plain, strings raw, `color_theme` as path stem, missing as `""`.
pub fn display_value(name: &str, store: &OptionsStore) -> String {
    if name == "color_theme" {
        return store
            .strings
            .get(name)
            .map(|p| path_stem(p))
            .unwrap_or_default();
    }
    if let Some(v) = store.bools.get(name) {
        return if *v { "True".into() } else { "False".into() };
    }
    if let Some(v) = store.ints.get(name) {
        return v.to_string();
    }
    store.strings.get(name).cloned().unwrap_or_default()
}

/// `fs::path(p).stem()`: file name after the last `/`, minus the last `.`
/// extension. No external crates — string ops only.
fn path_stem(p: &str) -> String {
    let file = p.rsplit('/').next().unwrap_or(p);
    match file.rfind('.') {
        Some(i) => file[..i].to_string(),
        None => file.to_string(),
    }
}

/// Browse position label for the selected browsable row: `" i/n"`
/// (`:1664-1684`; `i` defaults to `list.len()` when the current value matches
/// nothing, so an unknown theme shows `" n+1/n"`). `None` for non-browsable
/// options. `color_theme` matches full path or filename (`:1668-1680`);
/// every other option matches exactly (`:1682`).
pub fn browse_label(name: &str, value: &str, list: &[String]) -> Option<String> {
    if !BROWSABLE_OPTIONS.iter().any(|b| *b == name) {
        return None;
    }
    let pos = list_position(name, value, list).unwrap_or(list.len());
    Some(format!(" {}/{}", pos + 1, list.len()))
}

/// Position of the current value in a browsable list (`:1535-1548` cycle
/// semantics — also reused for the `:1664-1684` index label; the render path
/// additionally matches stem/legacy-absolute at `:1673-1677`, which Task 6
/// may refine). `color_theme` matches full path or filename; others exact.
/// `None` when nothing matches (C++ `i = optList.size()` fallback).
fn list_position(name: &str, value: &str, list: &[String]) -> Option<usize> {
    if name == "color_theme" {
        // :1537-1538: full-path equality OR list-entry filename == the raw
        // current string (NOT its own filename — a full-path current value
        // never filename-matches).
        list.iter()
            .position(|p| p == value || p.rsplit('/').next().unwrap_or(p.as_str()) == value)
    } else {
        list.iter().position(|p| p == value)
    }
}

/// Plain option-row data for Task 6 overlay assembly (see the module-level
/// draw split): visible window `[page * item_height, +item_height)` of
/// `CATEGORIES[tab]`, values via [`display_value`], labels via
/// [`browse_label`] for the SELECTED row only when it is browsable
/// (`:1665` gates the label on `c - 1 == selected && isBrowsable`).
/// `lists` carries the live browsable value lists (P3 supplies
/// `Theme::themes`, `Logger::log_levels`, ...); tests pass literals.
#[derive(Debug, Clone, PartialEq)]
pub struct OptionRow {
    pub name: String,
    pub value: String,
    pub index_label: Option<String>,
}

pub fn option_rows(
    tab: usize,
    store: &OptionsStore,
    page: usize,
    item_height: usize,
    selected: usize,
    lists: &HashMap<String, Vec<String>>,
) -> Vec<OptionRow> {
    use crate::tables::CATEGORIES;
    let Some(cat) = CATEGORIES.get(tab) else {
        return Vec::new();
    };
    let start = page.saturating_mul(item_height);
    cat.iter()
        .skip(start)
        .take(item_height)
        .enumerate()
        .map(|(row, entry)| {
            let name = entry[0].to_string();
            let value = display_value(&name, store);
            let index_label = if row == selected {
                lists
                    .get(&name)
                    .and_then(|list| browse_label(&name, &value_for_label(&name, store), list))
            } else {
                None
            };
            OptionRow {
                name,
                value,
                index_label,
            }
        })
        .collect()
}

/// Raw (non-stem) value for label matching: the label indexes the live list
/// (`Theme::themes` holds full paths), while [`display_value`] stems
/// `color_theme` for SHOW. C++ compares `Config::getS` (raw) at `:1669-1682`.
fn value_for_label(name: &str, store: &OptionsStore) -> String {
    if let Some(v) = store.bools.get(name) {
        return if *v { "True".into() } else { "False".into() };
    }
    if let Some(v) = store.ints.get(name) {
        return v.to_string();
    }
    store.strings.get(name).cloned().unwrap_or_default()
}

/// Initial editor text + numeric flag for the `enter`/`e` edit arm
/// (`:1431-1436`: `editor = TextEdit{getAsString(option), isInt}`).
pub fn begin_edit_text(name: &str, store: &OptionsStore) -> (String, bool) {
    (display_value(name, store), store.ints.contains_key(name))
}

/// `stoi` digit-run prefix (leading whitespace + optional sign accepted,
/// trailing garbage ignored) used to convert AFTER [`Config::int_valid`]
/// passed (`:1406`: `Config::set(option, stoi(editor.text))`). Safe: the
/// validator already enforced the `i32` range, so the prefix always parses.
fn parse_int_prefix(text: &str) -> i64 {
    let s = text.trim_start();
    let s = s.strip_prefix(['+', '-']).unwrap_or(s);
    let negative = text.trim_start().starts_with('-');
    let digits: String = s
        .bytes()
        .take_while(u8::is_ascii_digit)
        .map(char::from)
        .collect();
    // intValid already passed: a digit run exists and fits i32.
    let mut v: i64 = digits.parse().unwrap_or(0);
    if negative {
        v = -v;
    }
    v
}

/// Commit an edit (`enter` while editing, `:1385-1414`): validate via
/// `cfg`, stage into `store`, return side-effect actions.
/// - ints: `intValid` → set or `Err(valid_error)` (`:1405-1409`).
/// - `shown_boxes`: SPECIAL-CASE — accepted WITHOUT `string_valid` (see the
///   module-level decision); stages + `[RecalcLayout, ResetPreset]`
///   (`:1391-1394`).
/// - other strings: `stringValid` → set or `Err(valid_error)`, with
///   per-option actions (`:1387-1403`): `custom_cpu_name`/`custom_gpu_name*`
///   → `RecalcLayout`; `presets` → `RecalcLayout + ResetPreset`;
///   `clock_format` → `UpdateClock + RecalcLayout`; `cpu_core_map` →
///   `RefreshCoreMapping`.
/// - bools / browsables / unknown names are NOT editable (the C++ `enter`
///   arm requires `isEditable`, `:1431`): `Err`.
pub fn commit_edit(
    name: &str,
    text: &str,
    store: &mut OptionsStore,
    cfg: &mut Config,
) -> Result<Vec<Action>, String> {
    if store.ints.contains_key(name) {
        // :1405-1406, :1408-1409: intValid → set, else warnings = validError.
        if cfg.int_valid(name, text) {
            store.ints.insert(name.to_string(), parse_int_prefix(text));
            Ok(vec![])
        } else {
            Err(cfg.valid_error(name))
        }
    } else if name == "shown_boxes" {
        // Step 0b special-case (:1391-1394 minus the validator): accepted
        // WITHOUT string_valid (always-pass in Rust); validated at layout
        // time in P3, never persisted on bare `true` (no WriteConfig).
        store.strings.insert(name.to_string(), text.to_string());
        Ok(vec![Action::RecalcLayout, Action::ResetPreset])
    } else if store.strings.contains_key(name) {
        // The enter arm requires isEditable (:1431: enter/e/E only fires
        // `when selPred.test(isEditable)`): browsables commit via cycling,
        // never via the editor.
        if BROWSABLE_OPTIONS.iter().any(|b| *b == name) {
            return Err(format!("option '{name}' is not editable"));
        }
        // :1387-1389, :1408-1409: stringValid → set, else validError.
        if cfg.string_valid(name, text) {
            store.strings.insert(name.to_string(), text.to_string());
            Ok(commit_string_actions(name))
        } else {
            Err(cfg.valid_error(name))
        }
    } else if store.bools.contains_key(name) {
        Err(format!("option '{name}' is not editable"))
    } else {
        Err(format!("unknown option '{name}'"))
    }
}

/// Per-option actions for a successful string commit (:1389-1403).
fn commit_string_actions(name: &str) -> Vec<Action> {
    if name == "custom_cpu_name" || name.starts_with("custom_gpu_name") {
        vec![Action::RecalcLayout] // :1389-1390
    } else if name == "presets" {
        vec![Action::RecalcLayout, Action::ResetPreset] // :1391-1394
    } else if name == "clock_format" {
        vec![Action::UpdateClock, Action::RecalcLayout] // :1396-1398
    } else if name == "cpu_core_map" {
        vec![Action::RefreshCoreMapping] // :1400-1402
    } else {
        vec![]
    }
}

/// Left/right (or vim `h`/`l`) on the selected option (`:1482-1585`).
/// `dir > 0` = right, otherwise left. Mutates `store`, returns side-effect
/// actions (see the module-level flag mapping):
/// - ints: step ±(`update_ms` ? 100 : 1), `intValid`-gated (`:1484-1494`).
/// - bools: flip (screen_redraw ALWAYS → `RecalcLayout`, `:1497-1498`) plus
///   per-option extras (`:1501-1529`).
/// - browsables: cycle `list` with wrap (`:1547-1548`),
///   per-option actions (`:1550-1580`); `None`/empty `list` → `Err`.
/// - plain editable strings: no change, `Ok(vec![])` (`:1584-1585`
///   `retval = NoChange`).
/// - unknown names → `Err`.
pub fn flip_or_cycle(
    name: &str,
    dir: i8,
    store: &mut OptionsStore,
    cfg: &mut Config,
    list: Option<&[String]>,
) -> Result<Vec<Action>, String> {
    let right = dir > 0;
    if store.ints.contains_key(name) {
        // :1484-1494: step ±(update_ms ? 100 : 1), intValid-gated.
        let cur = store.ints[name];
        let step = if name == "update_ms" { 100 } else { 1 };
        // Saturating: staged values are unbounded i64 (unlike C++ long
        // wrap); valid inputs never saturate, hostile ones fail intValid.
        let next = if right {
            cur.saturating_add(step)
        } else {
            cur.saturating_sub(step)
        };
        if cfg.int_valid(name, &next.to_string()) {
            store.ints.insert(name.to_string(), next);
            Ok(vec![])
        } else {
            Err(cfg.valid_error(name))
        }
    } else if store.bools.contains_key(name) {
        // :1496-1498: flip (+ screen_redraw ALWAYS) then per-option extras.
        let next = !store.bools[name];
        store.bools.insert(name.to_string(), next);
        Ok(flip_bool_actions(name, next, store))
    } else if store.strings.contains_key(name) {
        if !BROWSABLE_OPTIONS.iter().any(|b| *b == name) {
            return Ok(vec![]); // plain strings: left/right is NoChange (:1584-1585)
        }
        let list = match list {
            Some(l) if !l.is_empty() => l,
            _ => return Err(format!("option '{name}' needs a value list")),
        };
        cycle_browsable(name, right, store, list)
    } else {
        Err(format!("unknown option '{name}'"))
    }
}

/// Per-option actions for a bool flip (:1496-1529). `next` is the POST-flip
/// value. Every path implies screen_redraw (`:1498`); theme-affecting paths
/// emit [`Action::ApplyTheme`] whose sink contract covers the redraw
/// (`:1720-1726` set `screen_redraw = true`), so no separate
/// `RecalcLayout` is emitted there.
fn flip_bool_actions(name: &str, next: bool, store: &mut OptionsStore) -> Vec<Action> {
    let theme = || {
        store
            .strings
            .get("color_theme")
            .cloned()
            .unwrap_or_else(|| "Default".into())
    };
    if name == "truecolor" {
        // :1501-1503: theme_refresh + lowcolor follows the flip.
        let low = !store.bools.get("lowcolor").copied().unwrap_or(false);
        store.bools.insert("lowcolor".into(), low);
        vec![Action::ApplyTheme { name: theme() }]
    } else if name == "force_tty" {
        // :1508-1511: theme_refresh + tty_mode follows the flip.
        // NOTE: the :1506 gate (no theme_refresh when current_tty is already
        // a /dev/tty*) is a P3 sink duty — the sink owns Term state and
        // downgrades this ApplyTheme to a layout recalc on dev-tty setups.
        store.bools.insert("tty_mode".into(), next);
        vec![Action::ApplyTheme { name: theme() }]
    } else if name == "rounded_corners" || name == "theme_background" {
        // :1513-1514.
        vec![Action::ApplyTheme { name: theme() }]
    } else if name == "background_update" {
        // :1515-1516: Runner::pause_output = false unconditionally.
        vec![Action::RecalcLayout, Action::PauseOutput { paused: false }]
    } else if name == "base_10_sizes" {
        // :1518-1519: recollect → run("all", false, true) (:1735-1736).
        vec![
            Action::RecalcLayout,
            Action::Run {
                target: RunTarget::All,
                no_update: false,
                redraw: true,
            },
        ]
    } else if name == "save_config_on_exit" {
        // :1521-1525: write() only on the True→False transition (i.e. when
        // the post-flip value is false).
        if next {
            vec![Action::RecalcLayout]
        } else {
            vec![Action::RecalcLayout, Action::WriteConfig]
        }
    } else if name == "disable_mouse" {
        // :1527-1529: enabled = !new disable_mouse value.
        vec![
            Action::RecalcLayout,
            Action::SetMouseEnabled { enabled: !next },
        ]
    } else {
        vec![Action::RecalcLayout]
    }
}

/// Cycle a browsable option through `list` with wrap-around (`:1547-1548`;
/// missing current value starts past the end, like C++ `i = size`, via
/// [`list_position`] → `None`). Stores the pick and returns per-option
/// actions (`:1550-1581`).
fn cycle_browsable(
    name: &str,
    right: bool,
    store: &mut OptionsStore,
    list: &[String],
) -> Result<Vec<Action>, String> {
    let cur = store.strings.get(name).cloned().unwrap_or_default();
    let mut i = list_position(name, &cur, list).unwrap_or(list.len());
    if right {
        i += 1;
        if i >= list.len() {
            i = 0;
        }
    } else if i == 0 {
        i = list.len() - 1;
    } else {
        i -= 1;
    }
    if name == "color_theme" {
        // :1550-1564: store the filename when the pick is the FIRST list
        // entry with that filename (shadowing), else the full path.
        let picked = &list[i];
        let filename = picked.rsplit('/').next().unwrap_or(picked);
        let first = list
            .iter()
            .find(|p| p.rsplit('/').next().unwrap_or(p) == filename);
        let stored = if first.map(|p| p == picked).unwrap_or(false) {
            filename.to_string()
        } else {
            picked.clone()
        };
        store.strings.insert(name.to_string(), stored.clone());
        return Ok(vec![Action::ApplyTheme { name: stored }]);
    }
    let picked = list[i].clone();
    store.strings.insert(name.to_string(), picked.clone());
    Ok(cycle_string_actions(name, &picked))
}

/// Per-option actions for a non-theme browsable cycle (`:1569-1581`).
fn cycle_string_actions(name: &str, picked: &str) -> Vec<Action> {
    if name == "log_level" {
        vec![Action::SetLogLevel {
            level: picked.to_string(),
        }] // :1569-1571
    } else if name == "base_10_bitrate" {
        vec![Action::Run {
            target: RunTarget::All,
            no_update: false,
            redraw: true,
        }] // :1573-1574
    } else if name == "proc_sorting"
        || name == "cpu_sensor"
        || name == "show_gpu_info"
        || name.starts_with("graph_symbol")
        || name.starts_with("cpu_graph_")
    {
        vec![Action::RecalcLayout] // :1576-1577
    } else if name == "disable_presets" && picked != "Off" {
        vec![Action::ResetPreset] // :1578-1580
    } else {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::CATEGORIES;

    fn store() -> OptionsStore {
        let mut s = OptionsStore::default();
        s.bools.insert("theme_background".into(), false);
        s.bools.insert("truecolor".into(), true);
        s.bools.insert("lowcolor".into(), false);
        s.bools.insert("force_tty".into(), false);
        s.bools.insert("tty_mode".into(), false);
        s.bools.insert("rounded_corners".into(), false);
        s.bools.insert("background_update".into(), true);
        s.bools.insert("base_10_sizes".into(), false);
        s.bools.insert("save_config_on_exit".into(), true);
        s.bools.insert("disable_mouse".into(), false);
        s.bools.insert("vim_keys".into(), false);
        s.ints.insert("update_ms".into(), 2000);
        s.ints.insert("proc_tree_auto_collapse".into(), 0);
        s.strings.insert("color_theme".into(), "Default".into());
        s.strings.insert("custom_cpu_name".into(), String::new());
        s.strings.insert("clock_format".into(), String::new());
        s.strings.insert("cpu_core_map".into(), String::new());
        s.strings.insert("presets".into(), "cpu:0:default".into());
        s.strings
            .insert("shown_boxes".into(), "cpu mem net proc".into());
        s.strings.insert("log_level".into(), "INFO".into());
        s.strings.insert("temp_scale".into(), "celsius".into());
        s.strings.insert("proc_sorting".into(), "cpu lazy".into());
        s.strings.insert("disable_presets".into(), "Off".into());
        s.strings.insert("base_10_bitrate".into(), "Auto".into());
        s.strings
            .insert("graph_symbol_cpu".into(), "default".into());
        s
    }

    fn themes() -> Vec<String> {
        vec!["Default".to_string(), "Gruvbox".to_string()]
    }

    //? classify_option (:1605-1621).

    #[test]
    fn classify_each_kind() {
        let s = store();
        assert_eq!(classify_option("theme_background", &s), OptKind::Bool);
        assert_eq!(classify_option("update_ms", &s), OptKind::Int);
        assert_eq!(classify_option("color_theme", &s), OptKind::Browsable);
        assert_eq!(classify_option("log_level", &s), OptKind::Browsable);
        assert_eq!(classify_option("custom_cpu_name", &s), OptKind::Editable);
        assert_eq!(classify_option("shown_boxes", &s), OptKind::Editable);
        assert_eq!(classify_option("no_such_option", &s), OptKind::Str);
    }

    #[test]
    fn twod_holds_for_bool_int_and_marker() {
        assert!(is_2d(OptKind::Bool));
        assert!(is_2d(OptKind::Int));
        assert!(is_2d(OptKind::TwoD));
        assert!(!is_2d(OptKind::Str));
        assert!(!is_2d(OptKind::Browsable));
        assert!(!is_2d(OptKind::Editable));
    }

    //? Paging math (:1596-1599).

    #[test]
    fn paging_math() {
        // General tab has 21 options (tables.rs scope check).
        assert_eq!(CATEGORIES[0].len(), 21);
        assert_eq!(page_count(21, 10), 3);
        assert_eq!(page_count(20, 10), 2);
        assert_eq!(page_count(21, 21), 1);
        assert_eq!(select_max(21, 10, 0), 9);
        assert_eq!(select_max(21, 10, 1), 9);
        assert_eq!(select_max(21, 10, 2), 0);
        assert_eq!(item_height_for(21, 40), 18);
        assert_eq!(item_height_for(5, 40), 5);
    }

    //? display_value (getAsString :670-678 + color_theme stem :1669).

    #[test]
    fn display_value_formats() {
        let s = store();
        assert_eq!(display_value("theme_background", &s), "False");
        assert_eq!(display_value("update_ms", &s), "2000");
        assert_eq!(display_value("proc_sorting", &s), "cpu lazy");
        assert_eq!(display_value("missing", &s), "");
    }

    #[test]
    fn display_value_color_theme_stem() {
        let mut s = store();
        s.strings
            .insert("color_theme".into(), "/x/themes/Gruvbox.theme".into());
        assert_eq!(display_value("color_theme", &s), "Gruvbox");
    }

    #[test]
    fn begin_edit_text_numeric_flag() {
        let s = store();
        let (text, numeric) = begin_edit_text("update_ms", &s);
        assert_eq!((text.as_str(), numeric), ("2000", true));
        let (text, numeric) = begin_edit_text("custom_cpu_name", &s);
        assert_eq!((text.as_str(), numeric), ("", false));
    }

    //? browse_label (:1664-1684).

    #[test]
    fn browse_label_index() {
        assert_eq!(
            browse_label("log_level", "INFO", &["INFO".into(), "DEBUG".into()]),
            Some(" 1/2".to_string())
        );
        // Unknown value defaults past the end (:1679-1680).
        assert_eq!(
            browse_label("log_level", "VERBOSE", &["INFO".into()]),
            Some(" 2/1".to_string())
        );
        assert_eq!(browse_label("custom_cpu_name", "x", &["x".into()]), None);
    }

    //? option_rows (draw split for Task 6).

    #[test]
    fn option_rows_window_and_label() {
        let s = store();
        let mut lists = HashMap::new();
        lists.insert("color_theme".to_string(), themes());
        let rows = option_rows(0, &s, 0, 5, 0, &lists);
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0].name, "color_theme");
        assert_eq!(rows[0].value, "Default");
        assert_eq!(rows[0].index_label, Some(" 1/2".to_string()));
        // Non-selected rows carry no label even when browsable... row 1 is
        // theme_background (bool) — value formatted, no label.
        assert_eq!(rows[1].name, "theme_background");
        assert_eq!(rows[1].value, "False");
        assert_eq!(rows[1].index_label, None);
        // Second page window.
        let rows = option_rows(0, &s, 1, 5, 0, &lists);
        assert_eq!(rows[0].name, CATEGORIES[0][5][0]);
    }

    //? commit_edit (:1385-1414).

    #[test]
    fn commit_string_paths() {
        let mut s = store();
        let mut cfg = Config::new();
        // custom_cpu_name → screen_redraw (:1389-1390).
        assert_eq!(
            commit_edit("custom_cpu_name", "Ryzen", &mut s, &mut cfg),
            Ok(vec![Action::RecalcLayout])
        );
        assert_eq!(
            s.strings.get("custom_cpu_name").map(String::as_str),
            Some("Ryzen")
        );
        // clock_format → update_clock + screen_redraw (:1396-1398).
        assert_eq!(
            commit_edit("clock_format", "%X", &mut s, &mut cfg),
            Ok(vec![Action::UpdateClock, Action::RecalcLayout])
        );
        // cpu_core_map → core remap (:1400-1402).
        assert_eq!(
            commit_edit("cpu_core_map", "0:0", &mut s, &mut cfg),
            Ok(vec![Action::RefreshCoreMapping])
        );
        // presets → screen_redraw + preset reset (:1391-1394).
        assert_eq!(
            commit_edit("presets", "cpu:0:default", &mut s, &mut cfg),
            Ok(vec![Action::RecalcLayout, Action::ResetPreset])
        );
        // Invalid presets text → validError, store untouched (:1408-1409).
        assert_eq!(
            commit_edit("presets", "bogus:0:default", &mut s, &mut cfg),
            Err("Invalid box name in config value presets!".to_string())
        );
        assert_eq!(
            s.strings.get("presets").map(String::as_str),
            Some("cpu:0:default")
        );
    }

    #[test]
    fn commit_shown_boxes_special_case() {
        // Step 0b: accepted WITHOUT string_valid (always-pass in Rust);
        // validated at layout time in P3, never persisted on bare `true`
        // (no WriteConfig). Actions mirror :1393-1397.
        let mut s = store();
        let mut cfg = Config::new();
        assert_eq!(
            commit_edit("shown_boxes", "cpu mem", &mut s, &mut cfg),
            Ok(vec![Action::RecalcLayout, Action::ResetPreset])
        );
        assert_eq!(
            s.strings.get("shown_boxes").map(String::as_str),
            Some("cpu mem")
        );
    }

    #[test]
    fn commit_int_valid_and_invalid() {
        let mut s = store();
        let mut cfg = Config::new();
        // Valid int stages with NO side-effect flags (:1405-1406).
        assert_eq!(
            commit_edit("update_ms", "2000", &mut s, &mut cfg),
            Ok(vec![])
        );
        assert_eq!(s.ints.get("update_ms"), Some(&2000));
        // stoi trailing-garbage tolerance: "2000x" stores 2000.
        assert_eq!(
            commit_edit("update_ms", "2000x", &mut s, &mut cfg),
            Ok(vec![])
        );
        assert_eq!(s.ints.get("update_ms"), Some(&2000));
        // Invalid → validError text, store untouched (:1408-1409).
        assert_eq!(
            commit_edit("update_ms", "99", &mut s, &mut cfg),
            Err("Config value update_ms set too low (<100).".to_string())
        );
        assert_eq!(s.ints.get("update_ms"), Some(&2000));
    }

    #[test]
    fn commit_rejects_non_editable() {
        // The C++ enter arm requires isEditable (:1431): bools, browsables
        // and unknown names cannot be committed.
        let mut s = store();
        let mut cfg = Config::new();
        assert!(commit_edit("theme_background", "True", &mut s, &mut cfg).is_err());
        assert!(commit_edit("log_level", "DEBUG", &mut s, &mut cfg).is_err());
        assert!(commit_edit("nope", "x", &mut s, &mut cfg).is_err());
    }

    //? flip_or_cycle ints (:1484-1494).

    #[test]
    fn flip_int_steps_with_update_ms_100() {
        let mut s = store();
        let mut cfg = Config::new();
        assert_eq!(
            flip_or_cycle("update_ms", 1, &mut s, &mut cfg, None),
            Ok(vec![])
        );
        assert_eq!(s.ints.get("update_ms"), Some(&2100));
        assert_eq!(
            flip_or_cycle("update_ms", -1, &mut s, &mut cfg, None),
            Ok(vec![])
        );
        assert_eq!(s.ints.get("update_ms"), Some(&2000));
        // Other ints step by 1.
        assert_eq!(
            flip_or_cycle("proc_tree_auto_collapse", 1, &mut s, &mut cfg, None),
            Ok(vec![])
        );
        assert_eq!(s.ints.get("proc_tree_auto_collapse"), Some(&1));
        // Failed validation → validError, store untouched (:1490-1493).
        s.ints.insert("update_ms".into(), 100);
        assert_eq!(
            flip_or_cycle("update_ms", -1, &mut s, &mut cfg, None),
            Err("Config value update_ms set too low (<100).".to_string())
        );
        assert_eq!(s.ints.get("update_ms"), Some(&100));
    }

    //? flip_or_cycle bools (:1496-1529): screen_redraw ALWAYS + extras.

    #[test]
    fn flip_plain_bool_recalcs_layout() {
        let mut s = store();
        let mut cfg = Config::new();
        assert_eq!(
            flip_or_cycle("vim_keys", 1, &mut s, &mut cfg, None),
            Ok(vec![Action::RecalcLayout])
        );
        assert_eq!(s.bools.get("vim_keys"), Some(&true));
    }

    #[test]
    fn flip_theme_bools_emit_apply_theme() {
        let mut s = store();
        let mut cfg = Config::new();
        // theme_background → theme_refresh (:1513-1514).
        assert_eq!(
            flip_or_cycle("theme_background", 1, &mut s, &mut cfg, None),
            Ok(vec![Action::ApplyTheme {
                name: "Default".to_string()
            }])
        );
        assert_eq!(s.bools.get("theme_background"), Some(&true));
        // rounded_corners → theme_refresh (:1513-1514).
        assert_eq!(
            flip_or_cycle("rounded_corners", -1, &mut s, &mut cfg, None),
            Ok(vec![Action::ApplyTheme {
                name: "Default".to_string()
            }])
        );
        assert_eq!(s.bools.get("rounded_corners"), Some(&true));
        // truecolor → theme_refresh + lowcolor flip (:1501-1503).
        assert_eq!(
            flip_or_cycle("truecolor", 1, &mut s, &mut cfg, None),
            Ok(vec![Action::ApplyTheme {
                name: "Default".to_string()
            }])
        );
        assert_eq!(s.bools.get("truecolor"), Some(&false));
        assert_eq!(s.bools.get("lowcolor"), Some(&true));
        // force_tty → theme_refresh + tty_mode follows (:1508-1511).
        assert_eq!(
            flip_or_cycle("force_tty", 1, &mut s, &mut cfg, None),
            Ok(vec![Action::ApplyTheme {
                name: "Default".to_string()
            }])
        );
        assert_eq!(s.bools.get("force_tty"), Some(&true));
        assert_eq!(s.bools.get("tty_mode"), Some(&true));
    }

    #[test]
    fn flip_special_bools() {
        let mut s = store();
        let mut cfg = Config::new();
        // background_update → pause_output=false (:1515-1516).
        assert_eq!(
            flip_or_cycle("background_update", 1, &mut s, &mut cfg, None),
            Ok(vec![
                Action::RecalcLayout,
                Action::PauseOutput { paused: false }
            ])
        );
        // base_10_sizes → recollect = run("all", false, true) (:1518-1519, :1736).
        assert_eq!(
            flip_or_cycle("base_10_sizes", 1, &mut s, &mut cfg, None),
            Ok(vec![
                Action::RecalcLayout,
                Action::Run {
                    target: RunTarget::All,
                    no_update: false,
                    redraw: true
                }
            ])
        );
        // save_config_on_exit True→False → immediate write (:1521-1525).
        assert_eq!(
            flip_or_cycle("save_config_on_exit", 1, &mut s, &mut cfg, None),
            Ok(vec![Action::RecalcLayout, Action::WriteConfig])
        );
        // ...but False→True flips with no write.
        assert_eq!(
            flip_or_cycle("save_config_on_exit", 1, &mut s, &mut cfg, None),
            Ok(vec![Action::RecalcLayout])
        );
        // disable_mouse → terminal mouse switch (:1527-1529).
        assert_eq!(
            flip_or_cycle("disable_mouse", 1, &mut s, &mut cfg, None),
            Ok(vec![
                Action::RecalcLayout,
                Action::SetMouseEnabled { enabled: false }
            ])
        );
    }

    //? flip_or_cycle browsables (:1532-1581).

    #[test]
    fn cycle_color_theme_wraps_and_applies() {
        let mut s = store();
        let mut cfg = Config::new();
        let list = themes();
        assert_eq!(
            flip_or_cycle("color_theme", 1, &mut s, &mut cfg, Some(&list)),
            Ok(vec![Action::ApplyTheme {
                name: "Gruvbox".to_string()
            }])
        );
        assert_eq!(
            s.strings.get("color_theme").map(String::as_str),
            Some("Gruvbox")
        );
        // Wrap right past the end → first (:1547-1548).
        assert_eq!(
            flip_or_cycle("color_theme", 1, &mut s, &mut cfg, Some(&list)),
            Ok(vec![Action::ApplyTheme {
                name: "Default".to_string()
            }])
        );
        // Wrap left from first → last.
        assert_eq!(
            flip_or_cycle("color_theme", -1, &mut s, &mut cfg, Some(&list)),
            Ok(vec![Action::ApplyTheme {
                name: "Gruvbox".to_string()
            }])
        );
    }

    #[test]
    fn cycle_color_theme_filename_shadowing() {
        // Filename stored when the pick is the FIRST list entry with that
        // filename, else the full path (:1550-1564).
        let mut s = store();
        let mut cfg = Config::new();
        let list = vec!["/a/Dup.theme".to_string(), "/b/Dup.theme".to_string()];
        s.strings
            .insert("color_theme".into(), "/a/Dup.theme".into());
        assert_eq!(
            flip_or_cycle("color_theme", -1, &mut s, &mut cfg, Some(&list)),
            Ok(vec![Action::ApplyTheme {
                name: "/b/Dup.theme".to_string()
            }])
        );
        assert_eq!(
            s.strings.get("color_theme").map(String::as_str),
            Some("/b/Dup.theme")
        );
        s.strings
            .insert("color_theme".into(), "/b/Dup.theme".into());
        assert_eq!(
            flip_or_cycle("color_theme", 1, &mut s, &mut cfg, Some(&list)),
            Ok(vec![Action::ApplyTheme {
                name: "Dup.theme".to_string()
            }])
        );
        assert_eq!(
            s.strings.get("color_theme").map(String::as_str),
            Some("Dup.theme")
        );
    }

    #[test]
    fn cycle_side_effect_options() {
        let mut s = store();
        let mut cfg = Config::new();
        // log_level → Logger level (:1569-1571).
        let levels = vec!["INFO".to_string(), "DEBUG".to_string()];
        assert_eq!(
            flip_or_cycle("log_level", 1, &mut s, &mut cfg, Some(&levels)),
            Ok(vec![Action::SetLogLevel {
                level: "DEBUG".to_string()
            }])
        );
        // base_10_bitrate → recollect (:1573-1574).
        let rates = vec!["Auto".to_string(), "True".to_string()];
        assert_eq!(
            flip_or_cycle("base_10_bitrate", 1, &mut s, &mut cfg, Some(&rates)),
            Ok(vec![Action::Run {
                target: RunTarget::All,
                no_update: false,
                redraw: true
            }])
        );
        // proc_sorting / graph_symbol_cpu → screen_redraw (:1576-1577).
        let sorts = vec!["cpu lazy".to_string(), "pid".to_string()];
        assert_eq!(
            flip_or_cycle("proc_sorting", 1, &mut s, &mut cfg, Some(&sorts)),
            Ok(vec![Action::RecalcLayout])
        );
        let syms = vec!["default".to_string(), "braille".to_string()];
        assert_eq!(
            flip_or_cycle("graph_symbol_cpu", 1, &mut s, &mut cfg, Some(&syms)),
            Ok(vec![Action::RecalcLayout])
        );
        // disable_presets → preset reset unless new is "Off" (:1578-1580).
        let presets = vec!["Off".to_string(), "Custom".to_string()];
        assert_eq!(
            flip_or_cycle("disable_presets", 1, &mut s, &mut cfg, Some(&presets)),
            Ok(vec![Action::ResetPreset])
        );
        assert_eq!(
            flip_or_cycle("disable_presets", 1, &mut s, &mut cfg, Some(&presets)),
            Ok(vec![])
        );
        // temp_scale cycles with no flags.
        let scales = vec!["celsius".to_string(), "fahrenheit".to_string()];
        assert_eq!(
            flip_or_cycle("temp_scale", 1, &mut s, &mut cfg, Some(&scales)),
            Ok(vec![])
        );
        // Missing list → Err (P3 must supply live lists).
        assert!(flip_or_cycle("log_level", 1, &mut s, &mut cfg, None).is_err());
    }

    #[test]
    fn flip_plain_string_and_unknown() {
        let mut s = store();
        let mut cfg = Config::new();
        // Plain strings: left/right is NoChange (:1584-1585).
        assert_eq!(
            flip_or_cycle("custom_cpu_name", 1, &mut s, &mut cfg, None),
            Ok(vec![])
        );
        assert!(flip_or_cycle("nope", 1, &mut s, &mut cfg, None).is_err());
    }

    #[test]
    fn menu_state_default() {
        let st = MenuState::default();
        assert_eq!((st.tab, st.selected, st.page), (0, 0, 0));
        assert!(!st.editing);
        assert!(st.editor.text.is_empty());
        assert_eq!(st.warnings, None);
        assert!(!st.theme_refresh);
        assert!(!st.screen_redraw);
    }
}
