//! Config maps mirroring Config:: in src/btop_config.hpp:37-42.
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Default)]
pub struct Config {
    pub strings: HashMap<String, String>,
    pub strings_tmp: HashMap<String, String>,
    pub bools: HashMap<String, bool>,
    pub bools_tmp: HashMap<String, bool>,
    pub ints: HashMap<String, i64>,
    pub ints_tmp: HashMap<String, i64>,
    locked: bool,
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

    pub fn lock(&mut self) {
        self.locked = true;
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
            let Some((name, mut value)) = line.split_once('=') else {
                warnings.push(format!("malformed line: {line}"));
                continue;
            };
            let name = name.trim();
            value = value.trim().trim_matches('"');
            if self.bools.contains_key(name) {
                match value.to_ascii_lowercase().as_str() {
                    "true" => {
                        self.set_b(name, true);
                    }
                    "false" => {
                        self.set_b(name, false);
                    }
                    _ => warnings.push(format!("invalid bool {name}={value}")),
                }
            } else if self.ints.contains_key(name) {
                match value.parse::<i64>() {
                    Ok(v) => {
                        self.set_i(name, v);
                    }
                    Err(_) => warnings.push(format!("invalid int {name}={value}")),
                }
            } else if self.strings.contains_key(name) {
                self.set_s(name, value.to_string());
            } else {
                warnings.push(format!("unknown key: {name}"));
            }
        }
        warnings
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
}
