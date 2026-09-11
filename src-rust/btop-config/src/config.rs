//! Config maps mirroring Config:: in src/btop_config.hpp:37-42.
use std::collections::HashMap;
use std::path::Path;

/// Milliseconds in one day. C++ `ONE_DAY_MILLIS` (src/btop_config.hpp:64):
/// `1000 * 60 * 60 * 24`.
pub const ONE_DAY_MILLIS: i64 = 86_400_000;

/// Valid `log_level` values (src/btop_log.cpp:160).
const LOG_LEVELS: &[&str] = &["DISABLED", "ERROR", "WARNING", "INFO", "DEBUG"];
/// Valid `graph_symbol` values (src/btop_config.cpp:50).
const GRAPH_SYMBOLS: &[&str] = &["braille", "block", "tty"];
/// Valid `graph_symbol_*` values including `"default"` (`:51`).
const GRAPH_SYMBOLS_DEF: &[&str] = &["default", "braille", "block", "tty"];
/// Valid `show_gpu_info` values (`:63`).
const SHOW_GPU_VALUES: &[&str] = &["Auto", "On", "Off"];

/// ASCII-digits-only check. C++ `isint` (src/btop_tools.hpp:278-280), also
/// relied on by `Config::load` above.
fn is_int(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

enum Stoi {
    Ok(i64),
    Invalid,
    OutOfRange,
}

/// C++ `stoi` semantics relied on by `intValid` (src/btop_config.cpp:559-575):
/// skip leading whitespace, accept an optional sign, parse the digit run
/// (trailing garbage ignored); no digits → `invalid_argument`, overflow of
/// the `int` (`i32`) range → `out_of_range`.
fn parse_stoi(value: &str) -> Stoi {
    let s = value.trim_start();
    let s = match s.strip_prefix(['+', '-']) {
        Some(rest) => rest,
        None => s,
    };
    let negative = value.trim_start().starts_with('-');
    let digits: String = s
        .bytes()
        .take_while(u8::is_ascii_digit)
        .map(char::from)
        .collect();
    if digits.is_empty() {
        return Stoi::Invalid;
    }
    match digits.parse::<i64>() {
        Ok(mut v) => {
            if negative {
                v = -v;
            }
            if v < i32::MIN as i64 || v > i32::MAX as i64 {
                Stoi::OutOfRange
            } else {
                Stoi::Ok(v)
            }
        }
        Err(_) => Stoi::OutOfRange,
    }
}

#[derive(Debug, Default)]
pub struct Config {
    pub strings: HashMap<String, String>,
    pub strings_tmp: HashMap<String, String>,
    pub bools: HashMap<String, bool>,
    pub bools_tmp: HashMap<String, bool>,
    pub ints: HashMap<String, i64>,
    pub ints_tmp: HashMap<String, i64>,
    locked: bool,
    last_error: String,
}

impl Config {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_s(&self, name: &str) -> Option<&str> {
        self.strings.get(name).map(String::as_str)
    }

    pub fn get_b(&self, name: &str) -> Option<bool> {
        self.bools.get(name).copied()
    }

    pub fn get_i(&self, name: &str) -> Option<i64> {
        self.ints.get(name).copied()
    }

    pub fn set_s(&mut self, name: &str, value: String) -> bool {
        if self.strings.contains_key(name) {
            if self.locked {
                self.strings_tmp.insert(name.to_string(), value);
            } else {
                self.strings.insert(name.to_string(), value);
            }
            true
        } else {
            false
        }
    }

    pub fn set_b(&mut self, name: &str, value: bool) -> bool {
        if self.bools.contains_key(name) {
            if self.locked {
                self.bools_tmp.insert(name.to_string(), value);
            } else {
                self.bools.insert(name.to_string(), value);
            }
            true
        } else {
            false
        }
    }

    pub fn set_i(&mut self, name: &str, value: i64) -> bool {
        if self.ints.contains_key(name) {
            if self.locked {
                self.ints_tmp.insert(name.to_string(), value);
            } else {
                self.ints.insert(name.to_string(), value);
            }
            true
        } else {
            false
        }
    }

    /// Toggle a bool, staging into tmp when locked. Mirrors Config::flip.
    pub fn flip(&mut self, name: &str) {
        if let Some(v) = self.bools.get(name).copied() {
            self.set_b(name, !v);
        }
    }

    /// Toggle a box name in `shown_boxes`. Transcribes `Config::toggle_box`
    /// (src/btop_config.cpp:739-761): remove the name when present, append
    /// it when absent, then `set("shown_boxes", joined)`.
    ///
    /// DEVIATION (structural): C++ mutates the `current_boxes` vector, then
    /// reverts when `Term::width/height < get_min_size(new_boxes)` (:752-756)
    /// and only then writes the string. This port has no `current_boxes`
    /// vector (the string is the single source of truth) and no `Term`
    /// access, so the size gate lives with the caller
    /// (`btop_app::box_labels::min_size_loop` re-checks the terminal size
    /// every iteration and simply keeps looping when the toggled layout
    /// still does not fit). Always returns `true` (`set_s` targets the
    /// known `shown_boxes` key; `false` only when the key was never seeded,
    /// mirroring `set_s` semantics).
    pub fn toggle_box(&mut self, name: &str) -> bool {
        let cur = self.strings.get("shown_boxes").cloned().unwrap_or_default();
        let mut boxes: Vec<&str> = cur.split_whitespace().collect();
        match boxes.iter().position(|b| *b == name) {
            Some(pos) => {
                boxes.remove(pos);
            }
            None => boxes.push(name),
        }
        let new_boxes = boxes.join(" ");
        self.set_s("shown_boxes", new_boxes)
    }

    pub fn lock(&mut self) {
        self.locked = true;
    }

    /// Persist the live maps to `path`, mirroring `Config::write` +
    /// `current_config` (src/btop_config.cpp:836-844, :877-899): header
    /// `#? Config file for btop v.<VERSION>`, then one `name = value` line
    /// per key (`"quoted"` strings, lowercase `true`/`false` bools, plain
    /// ints). Keys sort alphabetically (C++ iterates its `descriptions`
    /// table order instead — DEVIATION: the port keeps no descriptions
    /// table, so description comments are omitted and the order is sorted
    /// rather than table order; `load` accepts either order). The caller
    /// (P4 `clean_quit`) gates on `save_config_on_exit` and passes
    /// `World::conf_file`; an empty path is a no-op error, mirroring the
    /// `conf_file.empty()` early return (:837).
    ///
    /// DEVIATION (documented gap): C++ additionally gates on `write_new`
    /// (src/btop_config.cpp:837 — skips the write when nothing changed),
    /// which is set in four places (`_locked` on any known-key write at
    /// :462-463, missing conf file at :769, version-header mismatch at
    /// :783, non-empty load warnings at :832). The port always rewrites:
    /// every quit reformats the file and drops the description comments
    /// (see the table-order note above) instead of leaving an untouched
    /// file alone. A faithful gate would thread a dirty flag through
    /// `set_*`/`load`/`write` plus seed the missing-file case — kept as a
    /// doc note until a caller needs the leave-untouched behavior.
    pub fn write(&self, path: &Path) -> Result<(), String> {
        if path.as_os_str().is_empty() {
            return Err("empty config path".to_string());
        }
        let mut out = format!("#? Config file for btop v.{}\n", crate::VERSION);
        let mut names: Vec<&str> = self
            .strings
            .keys()
            .chain(self.ints.keys())
            .chain(self.bools.keys())
            .map(String::as_str)
            .collect();
        names.sort_unstable();
        names.dedup();
        for name in names {
            out.push('\n');
            if let Some(v) = self.strings.get(name) {
                out.push_str(&format!("{name} = \"{v}\"\n"));
            } else if let Some(v) = self.ints.get(name) {
                out.push_str(&format!("{name} = {v}\n"));
            } else if let Some(v) = self.bools.get(name) {
                out.push_str(&format!("{name} = {}\n", if *v { "true" } else { "false" }));
            }
        }
        std::fs::write(path, out).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Unlock and flush staged tmp values into the live maps.
    /// Mirrors Config::unlock (src/btop_config.cpp:701-714).
    pub fn unlock(&mut self) {
        self.locked = false;
        for (k, v) in std::mem::take(&mut self.strings_tmp) {
            self.strings.insert(k, v);
        }
        for (k, v) in std::mem::take(&mut self.ints_tmp) {
            self.ints.insert(k, v);
        }
        for (k, v) in std::mem::take(&mut self.bools_tmp) {
            self.bools.insert(k, v);
        }
    }

    /// Load `key="value"` lines. Returns one warning string per unknown or
    /// malformed line. Mirrors Config::load dispatch (src/btop_config.cpp:799-830).
    /// Bool accepts exactly `true`/`false`/`True`/`False` (C++ `isbool`,
    /// src/btop_tools.hpp:268-270); int accepts ASCII digits only, no sign
    /// (C++ `isint`, src/btop_tools.hpp:278-280), with `i64` range enforced
    /// via parse. Quotes are stripped only in the strings branch, exactly one
    /// surrounding pair (C++ cpp:817-821).
    /// Deliberate deviations that remain: unknown keys and malformed lines
    /// produce warnings here (mandated by the plan), while C++ silently
    /// ignores unknown names (cpp:793-796).
    pub fn load(&mut self, path: &Path) -> Vec<String> {
        let mut warnings = Vec::new();
        let Ok(text) = std::fs::read_to_string(path) else {
            warnings.push(format!("cannot read {}", path.display()));
            return warnings;
        };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((name, value)) = line.split_once('=') else {
                warnings.push(format!("malformed line: {line}"));
                continue;
            };
            let name = name.trim();
            let token = value.trim();
            if self.bools.contains_key(name) {
                match token {
                    "true" | "True" => {
                        self.set_b(name, true);
                    }
                    "false" | "False" => {
                        self.set_b(name, false);
                    }
                    _ => warnings.push(format!("invalid bool {name}={token}")),
                }
            } else if self.ints.contains_key(name) {
                let shape_ok = !token.is_empty() && token.bytes().all(|b| b.is_ascii_digit());
                let parsed = if shape_ok {
                    token.parse::<i64>().ok()
                } else {
                    None
                };
                match parsed {
                    Some(v) => {
                        self.set_i(name, v);
                    }
                    None => warnings.push(format!("invalid int {name}={token}")),
                }
            } else if self.strings.contains_key(name) {
                let unquoted = if token.len() >= 2 && token.starts_with('"') && token.ends_with('"')
                {
                    &token[1..token.len() - 1]
                } else {
                    token
                };
                self.set_s(name, unquoted.to_string());
            } else {
                warnings.push(format!("unknown key: {name}"));
            }
        }
        warnings
    }

    /// Format a value as the options menu shows it. Mirrors `getAsString`
    /// (src/btop_config.cpp:670-678): bools as `"True"`/`"False"`, ints plain
    /// (`to_string`, no separators), strings raw (NOT quoted — verified:
    /// `:675-676` returns `it->second` directly). Missing names yield `None`
    /// (C++ returns `""`, `:677`).
    pub fn get_as_string(&self, name: &str) -> Option<String> {
        if let Some(v) = self.bools.get(name) {
            return Some(if *v { "True".into() } else { "False".into() });
        }
        if let Some(v) = self.ints.get(name) {
            return Some(v.to_string());
        }
        self.strings.get(name).cloned()
    }

    /// Last validation failure text. Mirrors `Config::validError` state
    /// (src/btop_config.cpp:557): set as a side effect of [`Config::int_valid`]
    /// / [`Config::string_valid`], read after a `false` return
    /// (src/btop_menu.cpp:1409,1493). `name` is accepted for call-site
    /// symmetry with the validators; the stored last error is returned.
    /// Starts empty (like C++); a passing validation leaves it untouched
    /// (C++ only assigns on failure arms).
    pub fn valid_error(&self, _name: &str) -> String {
        self.last_error.clone()
    }

    /// Validate an int option's text form. Mirrors `intValid`
    /// (src/btop_config.cpp:559-593): `stoi` parsing (leading whitespace and
    /// an optional sign accepted, trailing garbage ignored, `i32` range —
    /// see `parse_stoi`), then the per-key bounds below. Unknown names pass
    /// (`:589-590` fall through to `return true`).
    ///
    /// Rules transcribed: `update_ms` in `[100, ONE_DAY_MILLIS]` (`:577-581`;
    /// `ONE_DAY_MILLIS = 1000*60*60*24 = 86400000`, src/btop_config.hpp:64),
    /// `proc_tree_auto_collapse` in `[0, 10000]` (`:583-587`).
    pub fn int_valid(&mut self, name: &str, value: &str) -> bool {
        let i_value = match parse_stoi(value) {
            Stoi::Ok(v) => v,
            Stoi::Invalid => {
                self.last_error = "Invalid numerical value!".into();
                return false;
            }
            Stoi::OutOfRange => {
                self.last_error = "Value out of range!".into();
                return false;
            }
        };
        if name == "update_ms" && i_value < 100 {
            self.last_error = "Config value update_ms set too low (<100).".into();
        } else if name == "update_ms" && i_value > ONE_DAY_MILLIS {
            self.last_error = format!("Config value update_ms set too high (>{ONE_DAY_MILLIS}).");
        } else if name == "proc_tree_auto_collapse" && i_value < 0 {
            self.last_error = "Config value proc_tree_auto_collapse must be >= 0.".into();
        } else if name == "proc_tree_auto_collapse" && i_value > 10_000 {
            self.last_error = "Config value proc_tree_auto_collapse set too high (>10000).".into();
        } else {
            return true;
        }
        false
    }

    /// Validate a string option's text form. Mirrors `stringValid`
    /// (src/btop_config.cpp:600-668). Rules transcribed:
    /// - `log_level` in `{"DISABLED","ERROR","WARNING","INFO","DEBUG"}`
    ///   (src/btop_log.cpp:160; error at `:601-602`),
    /// - `graph_symbol` in `{"braille","block","tty"}`
    ///   (src/btop_config.cpp:50; error at `:604-605`),
    /// - `graph_symbol_*` in `{"default","braille","block","tty"}`
    ///   (`:51`; error at `:607-608`),
    /// - `show_gpu_info` in `{"Auto","On","Off"}` (`:63`; error at `:622-623`;
    ///   C++ gates this arm on `GPU_SUPPORT`, the port applies it
    ///   unconditionally),
    /// - `presets` via `presetsValid` (`:476-512`; errors at `:481-504`),
    /// - `cpu_core_map` (`:629-645`) and `io_graph_speeds` (`:646-662`).
    ///
    /// Unknown names pass (`:664-665` fall through to `return true`).
    ///
    /// Deliberate deviations: `shown_boxes` always passes — C++ checks
    /// terminal size and live box state (`:610-619`, needs `Term` +
    /// `set_boxes`); `presetsValid` validates only (C++ also stores the
    /// parsed list, `:510`). NOTE: `temp_scale` has NO `stringValid` rule in
    /// C++ (verified by reading `:600-668`): its value set (`temp_scales`,
    /// `:58`) is enforced only via menu option lists
    /// (src/btop_menu.cpp:1318), so any string passes here too.
    pub fn string_valid(&mut self, name: &str, value: &str) -> bool {
        if name == "log_level" && !LOG_LEVELS.contains(&value) {
            self.last_error = format!("Invalid log_level: {value}");
        } else if name == "graph_symbol" && !GRAPH_SYMBOLS.contains(&value) {
            self.last_error = format!("Invalid graph symbol identifier: {value}");
        } else if name.starts_with("graph_symbol_")
            && value != "default"
            && !GRAPH_SYMBOLS.contains(&value)
        {
            self.last_error = format!("Invalid graph symbol identifier for {name}: {value}");
        } else if name == "show_gpu_info" && !SHOW_GPU_VALUES.contains(&value) {
            self.last_error = format!("Invalid value for show_gpu_info: {value}");
        } else if name == "presets" && !self.presets_valid(value) {
            return false;
        } else if name == "cpu_core_map" {
            for map in value.split(' ').filter(|s| !s.is_empty()) {
                let parts: Vec<&str> = map.split(':').filter(|s| !s.is_empty()).collect();
                if parts.len() != 2 || !is_int(parts[0]) || !is_int(parts[1]) {
                    self.last_error = "Invalid formatting of cpu_core_map!".into();
                    return false;
                }
            }
            return true;
        } else if name == "io_graph_speeds" {
            for map in value.split(' ').filter(|s| !s.is_empty()) {
                let parts: Vec<&str> = map.split(':').filter(|s| !s.is_empty()).collect();
                if parts.len() != 2 || parts[0].is_empty() || !is_int(parts[1]) {
                    self.last_error = "Invalid formatting of io_graph_speeds!".into();
                    return false;
                }
            }
            return true;
        } else {
            return true;
        }
        false
    }

    /// Validate a `presets` string. Mirrors `presetsValid`
    /// (src/btop_config.cpp:476-512): at most 9 space-separated presets
    /// (`:479-483`), at most 4 comma boxes each (`:484-488`), each box three
    /// colon fields (`:489-493`), box name in
    /// `cpu/mem/net/proc/gpu0..gpu5` (`:494-497`), position `0`/`1`
    /// (`:498-501`), graph in `valid_graph_symbols_def` (`:502-505`).
    /// Validation only — C++ additionally stores the parsed list (`:510`),
    /// which has no Rust counterpart yet.
    fn presets_valid(&mut self, value: &str) -> bool {
        let mut count = 0;
        for preset in value.split(' ').filter(|s| !s.is_empty()) {
            count += 1;
            if count > 9 {
                self.last_error = "Too many presets entered!".into();
                return false;
            }
            let mut boxes = 0;
            for b in preset.split(',').filter(|s| !s.is_empty()) {
                boxes += 1;
                if boxes > 4 {
                    self.last_error = "Too many boxes entered for preset!".into();
                    return false;
                }
                let vals: Vec<&str> = b.split(':').filter(|s| !s.is_empty()).collect();
                if vals.len() != 3 {
                    self.last_error = "Malformatted preset in config value presets!".into();
                    return false;
                }
                if !matches!(
                    vals[0],
                    "cpu"
                        | "mem"
                        | "net"
                        | "proc"
                        | "gpu0"
                        | "gpu1"
                        | "gpu2"
                        | "gpu3"
                        | "gpu4"
                        | "gpu5"
                ) {
                    self.last_error = "Invalid box name in config value presets!".into();
                    return false;
                }
                if vals[1] != "0" && vals[1] != "1" {
                    self.last_error = "Invalid position value in config value presets!".into();
                    return false;
                }
                if !GRAPH_SYMBOLS_DEF.contains(&vals[2]) {
                    self.last_error = "Invalid graph name in config value presets!".into();
                    return false;
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Config {
        let mut c = Config::new();
        c.strings.insert("color_theme".into(), "Default".into());
        c.bools.insert("theme_background".into(), true);
        c.ints.insert("update_ms".into(), 2000);
        c
    }

    #[test]
    fn typed_getters_read_defaults() {
        let c = sample();
        assert_eq!(c.get_s("color_theme"), Some("Default"));
        assert_eq!(c.get_b("theme_background"), Some(true));
        assert_eq!(c.get_i("update_ms"), Some(2000));
        assert_eq!(c.get_s("nope"), None);
    }

    #[test]
    fn set_writes_directly_when_unlocked() {
        let mut c = sample();
        assert!(c.set_b("theme_background", false));
        assert_eq!(c.get_b("theme_background"), Some(false));
        assert!(!c.set_b("nope", true));
    }

    #[test]
    fn set_stages_into_tmp_when_locked() {
        let mut c = sample();
        c.lock();
        assert!(c.set_i("update_ms", 1000));
        assert_eq!(c.get_i("update_ms"), Some(2000));
        c.unlock();
        assert_eq!(c.get_i("update_ms"), Some(1000));
    }

    #[test]
    fn flip_toggles_bool() {
        let mut c = sample();
        c.flip("theme_background");
        assert_eq!(c.get_b("theme_background"), Some(false));
    }

    #[test]
    fn toggle_box_removes_then_reappends() {
        let mut c = sample();
        c.strings
            .insert("shown_boxes".into(), "cpu mem net proc".into());
        assert!(c.toggle_box("mem"));
        assert_eq!(c.get_s("shown_boxes"), Some("cpu net proc"));
        assert!(c.toggle_box("mem"));
        assert_eq!(c.get_s("shown_boxes"), Some("cpu net proc mem"));
    }

    #[test]
    fn write_round_trips_through_load() {
        // `write` header + `name = value` lines must re-parse via `load`
        // with no warnings (P4 T7 `clean_quit` persistence).
        let c = sample();
        let dir = std::env::temp_dir().join("btop-t7-write-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("btop.conf");
        c.write(&path).expect("write must succeed");
        let text = std::fs::read_to_string(&path).expect("written file must read");
        assert!(
            text.starts_with("#? Config file for btop v.1.4.7\n"),
            "{text:?}"
        );
        assert!(text.contains("color_theme = \"Default\"\n"), "{text:?}");
        assert!(text.contains("update_ms = 2000\n"), "{text:?}");
        // `load` only fills pre-registered keys (unknown keys warn,
        // mirroring C++), so register the sample's keys first — key
        // seeding itself lives in the runner `World`, not in `Config`.
        let mut c2 = Config::new();
        c2.strings.insert("color_theme".into(), String::new());
        c2.bools.insert("theme_background".into(), false);
        c2.ints.insert("update_ms".into(), 0);
        assert!(c2.load(&path).is_empty(), "write output must load cleanly");
        assert_eq!(c2.get_s("color_theme"), Some("Default"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn get_as_string_formats_per_type() {
        // Mirrors getAsString (src/btop_config.cpp:670-678): bools as
        // "True"/"False", ints plain, strings raw, missing as "".
        let mut c = sample();
        c.set_b("theme_background", false);
        assert_eq!(
            c.get_as_string("theme_background").as_deref(),
            Some("False")
        );
        c.set_b("theme_background", true);
        assert_eq!(c.get_as_string("theme_background").as_deref(), Some("True"));
        assert_eq!(c.get_as_string("update_ms").as_deref(), Some("2000"));
        assert_eq!(c.get_as_string("color_theme").as_deref(), Some("Default"));
        assert_eq!(c.get_as_string("nope"), None);
    }

    #[test]
    fn int_valid_update_ms_bounds() {
        // Bounds from intValid (src/btop_config.cpp:577-581).
        let mut c = sample();
        assert!(c.int_valid("update_ms", "100"));
        assert!(c.int_valid("update_ms", "86400000"));
        assert!(!c.int_valid("update_ms", "99"));
        assert_eq!(
            c.valid_error("update_ms"),
            "Config value update_ms set too low (<100)."
        );
        assert!(!c.int_valid("update_ms", "86400001"));
        assert_eq!(
            c.valid_error("update_ms"),
            "Config value update_ms set too high (>86400000)."
        );
    }

    #[test]
    fn int_valid_proc_tree_auto_collapse_bounds() {
        // Bounds from intValid (src/btop_config.cpp:583-587).
        let mut c = sample();
        c.ints.insert("proc_tree_auto_collapse".into(), 0);
        assert!(c.int_valid("proc_tree_auto_collapse", "0"));
        assert!(c.int_valid("proc_tree_auto_collapse", "10000"));
        assert!(!c.int_valid("proc_tree_auto_collapse", "-1"));
        assert_eq!(
            c.valid_error("proc_tree_auto_collapse"),
            "Config value proc_tree_auto_collapse must be >= 0."
        );
        assert!(!c.int_valid("proc_tree_auto_collapse", "10001"));
        assert_eq!(
            c.valid_error("proc_tree_auto_collapse"),
            "Config value proc_tree_auto_collapse set too high (>10000)."
        );
    }

    #[test]
    fn int_valid_rejects_non_numeric() {
        // stoi failure arms (src/btop_config.cpp:559-575).
        let mut c = sample();
        assert!(!c.int_valid("update_ms", "abc"));
        assert_eq!(c.valid_error("update_ms"), "Invalid numerical value!");
        assert!(!c.int_valid("update_ms", "9999999999999999999999"));
        assert_eq!(c.valid_error("update_ms"), "Value out of range!");
        // Unknown names pass (C++ falls through to `return true`, :589-590).
        assert!(c.int_valid("whatever", "12x"));
    }

    #[test]
    fn valid_error_contract() {
        // Contract pinned (mirrors Config::validError, src/btop_config.cpp:557):
        // the stored error is meaningful ONLY after a `false` validation;
        // a passing validation leaves it untouched; it starts empty.
        let mut c = sample();
        assert_eq!(c.valid_error("update_ms"), "");
        // Failing validation stores the error...
        assert!(!c.int_valid("update_ms", "99"));
        assert_eq!(
            c.valid_error("update_ms"),
            "Config value update_ms set too low (<100)."
        );
        // ...and a passing validation leaves it untouched (C++ only assigns
        // on the failure arms, never on success).
        assert!(c.int_valid("update_ms", "2000"));
        assert_eq!(
            c.valid_error("update_ms"),
            "Config value update_ms set too low (<100)."
        );
        // Same for the string validator: pass leaves the stale error alone,
        // fail overwrites it.
        assert!(c.string_valid("log_level", "DEBUG"));
        assert_eq!(
            c.valid_error("log_level"),
            "Config value update_ms set too low (<100)."
        );
        assert!(!c.string_valid("log_level", "VERBOSE"));
        assert_eq!(c.valid_error("log_level"), "Invalid log_level: VERBOSE");
    }

    #[test]
    fn string_valid_log_level_and_graph_symbols() {
        // Set-membership arms (src/btop_config.cpp:600-608).
        let mut c = sample();
        assert!(c.string_valid("log_level", "DEBUG"));
        assert!(!c.string_valid("log_level", "VERBOSE"));
        assert_eq!(c.valid_error("log_level"), "Invalid log_level: VERBOSE");
        assert!(c.string_valid("graph_symbol", "braille"));
        assert!(!c.string_valid("graph_symbol", "emoji"));
        assert_eq!(
            c.valid_error("graph_symbol"),
            "Invalid graph symbol identifier: emoji"
        );
        assert!(c.string_valid("graph_symbol_cpu", "default"));
        assert!(!c.string_valid("graph_symbol_cpu", "emoji"));
        assert_eq!(
            c.valid_error("graph_symbol_cpu"),
            "Invalid graph symbol identifier for graph_symbol_cpu: emoji"
        );
    }

    #[test]
    fn string_valid_maps_and_presets() {
        // cpu_core_map / io_graph_speeds (src/btop_config.cpp:629-662) and
        // presetsValid (src/btop_config.cpp:476-512).
        let mut c = sample();
        assert!(c.string_valid("cpu_core_map", "0:0 1:2"));
        assert!(!c.string_valid("cpu_core_map", "0-x"));
        assert_eq!(
            c.valid_error("cpu_core_map"),
            "Invalid formatting of cpu_core_map!"
        );
        assert!(c.string_valid("io_graph_speeds", "eth0:100"));
        assert!(!c.string_valid("io_graph_speeds", "eth0:x"));
        assert_eq!(
            c.valid_error("io_graph_speeds"),
            "Invalid formatting of io_graph_speeds!"
        );
        assert!(c.string_valid("presets", "cpu:0:default,mem:0:braille"));
        assert!(!c.string_valid("presets", "bogus:0:default"));
        assert_eq!(
            c.valid_error("presets"),
            "Invalid box name in config value presets!"
        );
        // 10 presets exceeds the 9-preset cap (:479-483).
        let many = ["cpu:0:default"; 10].join(" ");
        assert!(!c.string_valid("presets", &many));
        assert_eq!(c.valid_error("presets"), "Too many presets entered!");
        // Unknown names pass (C++ falls through to `return true`, :664-665).
        assert!(c.string_valid("color_theme", "anything"));
    }
}
