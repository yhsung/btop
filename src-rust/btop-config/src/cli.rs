//! CLI parsing mirroring Cli:: in src/btop_cli.hpp:16-35 and the flag table
//! in src/btop_cli.cpp:60-190.
use std::path::PathBuf;

#[derive(Debug, Default, PartialEq)]
pub struct Cli {
    pub config_file: Option<PathBuf>,
    pub debug: bool,
    pub filter: Option<String>,
    pub force_tty: Option<bool>,
    pub force_utf: bool,
    pub low_color: bool,
    pub preset: Option<u32>,
    pub themes_dir: Option<PathBuf>,
    pub updates: Option<u32>,
}

/// Parse argv (excluding argv[0]). `Err(exit_code)` mirrors the C++
/// `std::expected<Cli, int>` error channel; `--help`/`--version` and
/// `--default-config` return `Err(0)`.
///
/// Deferred to M4 CLI wiring (see src/btop_cli.cpp): filesystem validation
/// (config-is-directory check, cpp:117; themes-dir-must-exist, cpp:166) and
/// printing side effects (`usage()`/`help()`/`version()`/`default_config()`).
pub fn parse(args: &[String]) -> Result<Cli, i32> {
    let mut cli = Cli::default();
    let mut it = args.iter().peekable();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--default-config" | "-h" | "--help" | "--version" | "-v" | "-V" => return Err(0),
            "-d" | "--debug" => cli.debug = true,
            "--force-utf" => cli.force_utf = true,
            "-l" | "--low-color" => cli.low_color = true,
            "-t" | "--tty" => {
                if cli.force_tty.is_some() {
                    return Err(1);
                }
                cli.force_tty = Some(true)
            }
            "--no-tty" => {
                if cli.force_tty.is_some() {
                    return Err(1);
                }
                cli.force_tty = Some(false)
            }
            "-c" | "--config" => cli.config_file = Some(PathBuf::from(next_value(&mut it, arg)?)),
            "-f" | "--filter" => cli.filter = Some(next_value(&mut it, arg)?),
            "-p" | "--preset" => {
                let raw = next_value(&mut it, arg)?;
                let v = stoi_prefix(&raw).map_err(|_| 1)?;
                cli.preset = Some(v.clamp(0, 9) as u32)
            }
            "--themes-dir" => cli.themes_dir = Some(PathBuf::from(next_value(&mut it, arg)?)),
            "-u" | "--update" => {
                let raw = next_value(&mut it, arg)?;
                let v = stoi_prefix(&raw).map_err(|_| 1)?;
                cli.updates = Some(v.max(100) as u32)
            }
            _ => return Err(1),
        }
    }
    Ok(cli)
}

fn next_value(
    it: &mut std::iter::Peekable<std::slice::Iter<String>>,
    flag: &str,
) -> Result<String, i32> {
    it.next().cloned().ok_or_else(|| {
        eprintln!("{flag} requires a value");
        1
    })
}

/// Mirror C++ `std::stoi`: skip leading ASCII spaces, optional single
/// `+`/`-` sign, then a non-empty run of ASCII digits (stop at first
/// non-digit). Accumulate in i64 and range-check to i32.
fn stoi_prefix(s: &str) -> Result<i32, ()> {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    let mut neg = false;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        neg = b[i] == b'-';
        i += 1;
    }
    let start = i;
    let mut acc: i64 = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        acc = acc
            .checked_mul(10)
            .and_then(|a| a.checked_add((b[i] - b'0') as i64))
            .ok_or(())?;
        i += 1;
    }
    if i == start {
        return Err(());
    }
    if neg {
        acc = -acc;
    }
    if acc < i32::MIN as i64 || acc > i32::MAX as i64 {
        return Err(());
    }
    Ok(acc as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_all_value_flags() {
        let c = parse(&args(&[
            "-c",
            "/tmp/x.conf",
            "-f",
            "ssh",
            "-p",
            "2",
            "--themes-dir",
            "/tmp/t",
            "-u",
            "500",
        ]))
        .unwrap();
        assert_eq!(
            c.config_file.as_deref(),
            Some(std::path::Path::new("/tmp/x.conf"))
        );
        assert_eq!(c.filter.as_deref(), Some("ssh"));
        assert_eq!(c.preset, Some(2));
        assert_eq!(
            c.themes_dir.as_deref(),
            Some(std::path::Path::new("/tmp/t"))
        );
        assert_eq!(c.updates, Some(500));
    }

    #[test]
    fn parses_bool_switches() {
        let c = parse(&args(&["-d", "--force-utf", "-l", "-t"])).unwrap();
        assert!(c.debug && c.force_utf && c.low_color);
        assert_eq!(c.force_tty, Some(true));
        let c = parse(&args(&["--no-tty"])).unwrap();
        assert_eq!(c.force_tty, Some(false));
    }

    #[test]
    fn rejects_missing_value() {
        assert!(parse(&args(&["-p"])).is_err());
    }

    #[test]
    fn version_shorts_exit_zero() {
        assert_eq!(parse(&args(&["-v"])), Err(0));
        assert_eq!(parse(&args(&["-V"])), Err(0));
    }

    #[test]
    fn tty_set_twice_errors() {
        assert_eq!(parse(&args(&["-t", "-t"])), Err(1));
        assert_eq!(parse(&args(&["-t", "--no-tty"])), Err(1));
        assert_eq!(parse(&args(&["--no-tty", "--tty"])), Err(1));
    }

    #[test]
    fn preset_clamps_and_updates_floors() {
        assert_eq!(parse(&args(&["-p", "12"])).unwrap().preset, Some(9));
        assert_eq!(parse(&args(&["-p", "-5"])).unwrap().preset, Some(0));
        assert!(parse(&args(&["-p", "abc"])).is_err());
        assert_eq!(parse(&args(&["-u", "50"])).unwrap().updates, Some(100));
        assert_eq!(parse(&args(&["-u", "500"])).unwrap().updates, Some(500));
    }

    #[test]
    fn stoi_rejects_overflow_and_weird_spacing() {
        assert!(parse(&args(&["-p", "9999999999999999999999999"])).is_err());
        assert_eq!(parse(&args(&["-p", "\t5"])).unwrap().preset, Some(5));
        assert!(parse(&args(&["--bogus"])).is_err());
    }
}
