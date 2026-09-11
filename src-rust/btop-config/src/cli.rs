//! CLI parsing mirroring Cli:: in src/btop_cli.hpp:16-35, the flag table
//! in src/btop_cli.cpp:60-190, and the printers (`version`/`build_info`/
//! `error`/`usage`/`help`/`help_hint`/`default_config`, :36-51, :201-271).
use std::path::PathBuf;

use crate::config::Config;

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

// ANSI constants (src/btop_cli.cpp:27-34); only the used ones are ported.
const BOLD: &str = "\x1b[1m";
const BOLD_UNDERLINE: &str = "\x1b[1;4m";
const BOLD_RED: &str = "\x1b[1;31m";
const BOLD_GREEN: &str = "\x1b[1;32m";
const BOLD_YELLOW: &str = "\x1b[1;33m";
const BOLD_BRIGHT_BLACK: &str = "\x1b[1;90m";
const YELLOW: &str = "\x1b[33m";
const RESET: &str = "\x1b[0m";

/// `version()` (src/btop_cli.cpp:36-42). `GIT_COMMIT` is baked by
/// `build.rs` (empty when git is unavailable, mirroring Makefile:216).
pub fn version_text() -> String {
    let commit = env!("BTOP_GIT_COMMIT");
    if commit.is_empty() {
        format!("btop version: {BOLD}{}{RESET}\n", crate::VERSION)
    } else {
        format!("btop version: {BOLD}{}+{commit}{RESET}\n", crate::VERSION)
    }
}

/// `build_info()` (src/btop_cli.cpp:44-47).
pub fn build_info_text() -> String {
    format!(
        "Compiled with: {} ({})\nConfigured with: {}\n",
        env!("BTOP_COMPILER"),
        env!("BTOP_COMPILER_VERSION"),
        env!("BTOP_CONFIGURE_COMMAND"),
    )
}

/// `error(msg)` (src/btop_cli.cpp:49-51). C++ prints to stdout
/// (`fmt::println`), so this text goes to the caller's stdout too.
pub fn error_text(msg: &str) -> String {
    format!("{BOLD_RED}error:{RESET} {msg}\n\n")
}

/// `usage()` (src/btop_cli.cpp:245-247).
pub fn usage_text() -> String {
    format!("{BOLD_UNDERLINE}Usage:{RESET} {BOLD}btop{RESET} [OPTIONS]\n\n")
}

/// `help()` (src/btop_cli.cpp:249-267).
pub fn help_text() -> String {
    format!(
        "{BOLD_UNDERLINE}Options:{RESET}\n\
         \x20 {BOLD}-c, --config{RESET} <file>     Path to a config file\n\
         \x20 {BOLD}-d, --debug{RESET}             Start in debug mode with additional logs and metrics\n\
         \x20 {BOLD}-f, --filter{RESET} <filter>   Set an initial process filter\n\
         \x20 {BOLD}    --force-utf{RESET}         Override automatic UTF locale detection\n\
         \x20 {BOLD}-l, --low-color{RESET}         Disable true color, 256 colors only\n\
         \x20 {BOLD}-p, --preset{RESET} <id>       Start with a preset (0-9)\n\
         \x20 {BOLD}-t, --tty{RESET}               Force tty mode with ANSI graph symbols and 16 colors only\n\
         \x20 {BOLD}    --themes-dir{RESET} <dir>  Path to a custom themes directory\n\
         \x20 {BOLD}    --no-tty{RESET}            Force disable tty mode\n\
         \x20 {BOLD}-u, --update{RESET} <ms>       Set an initial update rate in milliseconds\n\
         \x20 {BOLD}    --default-config{RESET}    Print default config to standard output\n\
         \x20 {BOLD}-h, --help{RESET}              Show this help message and exit\n\
         \x20 {BOLD}-V, --version{RESET}           Show a version message and exit (more with --version)\n",
    )
}

/// `help_hint()` (src/btop_cli.cpp:269-271).
pub fn help_hint_text() -> String {
    format!("For more information, try '{BOLD}--help{RESET}'\n")
}

/// `default_config()` body (src/btop_cli.cpp:201-243): dump
/// `Config::current_config()` — here `Config::defaults()` — colorized
/// per line if `colorize` (C++: `isatty(STDOUT_FILENO)`, :206), plain
/// otherwise (:239-241). Comment lines (none in the port — no
/// descriptions table, see `Config::dump`) would take
/// `BOLD_BRIGHT_BLACK`; `name` takes `BOLD_YELLOW`, `value`
/// `BOLD_GREEN`.
pub fn default_config_text(colorize: bool) -> String {
    let plain = Config::defaults().dump();
    if !colorize {
        return plain;
    }
    let mut out = String::with_capacity(plain.len() + 256);
    for line in plain.lines() {
        if line.starts_with('#') {
            out.push_str(&format!("{BOLD_BRIGHT_BLACK}{line}{RESET}\n"));
        } else if line.is_empty() {
            out.push('\n');
        } else if let Some((name, value)) = line.split_once('=') {
            out.push_str(&format!(
                "{BOLD_YELLOW}{name}{RESET}={BOLD_GREEN}{value}{RESET}\n"
            ));
        } else {
            // Unreachable: `dump` only emits header/comment/blank/`a=b`
            // lines. C++ errors (`invalid default config`, :220) instead
            // of guessing; the port keeps the line verbatim.
            out.push_str(&format!("{line}\n"));
        }
    }
    out
}

/// Parse argv (excluding argv[0]). `Err(exit_code)` mirrors the C++
/// `std::expected<Cli, int>` error channel. Like C++ (`Cli::parse`,
/// src/btop_cli.cpp:54-199), printers and error messages print to stdout
/// inside the parser; the caller (`bin/btop.rs`, mirroring
/// src/btop.cpp:847-857) adds `usage()` + `help_hint()` on `Err != 0`.
pub fn parse(args: &[String]) -> Result<Cli, i32> {
    use std::io::IsTerminal as _;
    let mut cli = Cli::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--default-config" => {
                let colorize = std::io::stdout().is_terminal();
                print!("{}", default_config_text(colorize));
                return Err(0);
            }
            "-h" | "--help" => {
                print!("{}{}", usage_text(), help_text());
                return Err(0);
            }
            "-v" | "-V" => {
                print!("{}", version_text());
                return Err(0);
            }
            "--version" => {
                print!("{}{}", version_text(), build_info_text());
                return Err(0);
            }
            "-d" | "--debug" => cli.debug = true,
            "--force-utf" => cli.force_utf = true,
            "-l" | "--low-color" => cli.low_color = true,
            "-t" | "--tty" => {
                if cli.force_tty.is_some() {
                    print!("{}", error_text("tty mode can't be set twice"));
                    return Err(1);
                }
                cli.force_tty = Some(true)
            }
            "--no-tty" => {
                if cli.force_tty.is_some() {
                    print!("{}", error_text("tty mode can't be set twice"));
                    return Err(1);
                }
                cli.force_tty = Some(false)
            }
            "-c" | "--config" => {
                let Some(raw) = it.next() else {
                    print!("{}", error_text("Config requires an argument"));
                    return Err(1);
                };
                let path = PathBuf::from(raw);
                // Filesystem validation (src/btop_cli.cpp:117).
                if path.is_dir() {
                    print!("{}", error_text("Config file can't be a directory"));
                    return Err(1);
                }
                cli.config_file = Some(path)
            }
            "-f" | "--filter" => {
                let Some(raw) = it.next() else {
                    print!("{}", error_text("Filter requires an argument"));
                    return Err(1);
                };
                cli.filter = Some(raw.clone())
            }
            "-p" | "--preset" => {
                let Some(raw) = it.next() else {
                    print!("{}", error_text("Preset requires an argument"));
                    return Err(1);
                };
                match crate::config::stoi_for_cli(raw) {
                    Ok(v) => cli.preset = Some(v.clamp(0, 9) as u32),
                    Err(true) => {
                        print!("{}", error_text("Preset must be a positive number"));
                        return Err(1);
                    }
                    Err(false) => {
                        print!(
                            "{}",
                            error_text(&format!("Preset argument is out of range: {raw}"))
                        );
                        return Err(1);
                    }
                }
            }
            "--themes-dir" => {
                let Some(raw) = it.next() else {
                    print!("{}", error_text("Themes directory requires an argument"));
                    return Err(1);
                };
                let path = PathBuf::from(raw);
                // Filesystem validation (src/btop_cli.cpp:166).
                if !path.is_dir() {
                    print!(
                        "{}",
                        error_text("Themes directory does not exist or is not a directory")
                    );
                    return Err(1);
                }
                cli.themes_dir = Some(path)
            }
            "-u" | "--update" => {
                let Some(raw) = it.next() else {
                    print!("{}", error_text("Update requires an argument"));
                    return Err(1);
                };
                match crate::config::stoi_for_cli(raw) {
                    Ok(v) => cli.updates = Some(v.max(100) as u32),
                    Err(true) => {
                        print!("{}", error_text("Update must be a positive number"));
                        return Err(1);
                    }
                    Err(false) => {
                        print!(
                            "{}",
                            error_text(&format!("Update argument is out of range: {raw}"))
                        );
                        return Err(1);
                    }
                }
            }
            _ => {
                print!(
                    "{}",
                    error_text(&format!("Unknown argument '{YELLOW}{arg}{RESET}'"))
                );
                return Err(1);
            }
        }
    }
    Ok(cli)
}

/// Mirror C++ `std::stoi`: skip leading ASCII spaces, optional single
/// `+`/`-` sign, then a non-empty run of ASCII digits (stop at first
/// non-digit). Accumulate in i64 and range-check to i32.
///
/// Shared with `btop-input` for SGR mouse coordinates (btop_input.cpp).
/// Unit error is intentional: callers only branch on ok/err (C++ throws).
#[allow(clippy::result_unit_err)]
pub fn stoi_prefix(s: &str) -> Result<i32, ()> {
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
            "/tmp",
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
        assert_eq!(c.themes_dir.as_deref(), Some(std::path::Path::new("/tmp")));
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
        assert_eq!(parse(&args(&["--version"])), Err(0));
        assert_eq!(parse(&args(&["-h"])), Err(0));
        assert_eq!(parse(&args(&["--help"])), Err(0));
        assert_eq!(parse(&args(&["--default-config"])), Err(0));
    }

    #[test]
    fn version_text_shape() {
        let v = version_text();
        assert!(v.starts_with("btop version: \x1b[1m1.4.7"), "{v:?}");
        assert!(v.ends_with("\x1b[0m\n"), "{v:?}");
        if env!("BTOP_GIT_COMMIT").is_empty() {
            assert_eq!(v, "btop version: \x1b[1m1.4.7\x1b[0m\n");
        } else {
            assert!(
                v.contains(&format!("1.4.7+{}", env!("BTOP_GIT_COMMIT"))),
                "{v:?}"
            );
        }
    }

    #[test]
    fn build_info_text_shape() {
        let b = build_info_text();
        assert!(b.starts_with("Compiled with: rustc ("), "{b:?}");
        assert!(b.contains("Configured with: cargo build"), "{b:?}");
    }

    #[test]
    fn usage_and_hint_golden() {
        assert_eq!(
            usage_text(),
            "\x1b[1;4mUsage:\x1b[0m \x1b[1mbtop\x1b[0m [OPTIONS]\n\n"
        );
        assert_eq!(
            help_hint_text(),
            "For more information, try '\x1b[1m--help\x1b[0m'\n"
        );
    }

    #[test]
    fn help_golden() {
        let b = "\x1b[1m";
        let r = "\x1b[0m";
        let expected = format!(
            "\x1b[1;4mOptions:{r}\n\
             \x20 {b}-c, --config{r} <file>     Path to a config file\n\
             \x20 {b}-d, --debug{r}             Start in debug mode with additional logs and metrics\n\
             \x20 {b}-f, --filter{r} <filter>   Set an initial process filter\n\
             \x20 {b}    --force-utf{r}         Override automatic UTF locale detection\n\
             \x20 {b}-l, --low-color{r}         Disable true color, 256 colors only\n\
             \x20 {b}-p, --preset{r} <id>       Start with a preset (0-9)\n\
             \x20 {b}-t, --tty{r}               Force tty mode with ANSI graph symbols and 16 colors only\n\
             \x20 {b}    --themes-dir{r} <dir>  Path to a custom themes directory\n\
             \x20 {b}    --no-tty{r}            Force disable tty mode\n\
             \x20 {b}-u, --update{r} <ms>       Set an initial update rate in milliseconds\n\
             \x20 {b}    --default-config{r}    Print default config to standard output\n\
             \x20 {b}-h, --help{r}              Show this help message and exit\n\
             \x20 {b}-V, --version{r}           Show a version message and exit (more with --version)\n"
        );
        assert_eq!(help_text(), expected);
    }

    #[test]
    fn error_text_golden() {
        assert_eq!(
            error_text("tty mode can't be set twice"),
            "\x1b[1;31merror:\x1b[0m tty mode can't be set twice\n\n"
        );
    }

    #[test]
    fn default_config_plain_and_colorized() {
        let plain = default_config_text(false);
        assert!(
            plain.starts_with("#? Config file for btop v.1.4.7\n"),
            "{:?}",
            &plain[..60]
        );
        assert!(
            plain.contains("shown_boxes = \"cpu mem net proc\"\n"),
            "{}",
            &plain[..400]
        );
        assert!(!plain.contains('\x1b'), "plain must carry no escapes");
        let color = default_config_text(true);
        assert!(
            color.contains("\x1b[1;90m#? Config file for btop v.1.4.7\x1b[0m\n"),
            "{:?}",
            &color[..80]
        );
        assert!(
            color
                .contains("\x1b[1;33mshown_boxes \x1b[0m=\x1b[1;32m \"cpu mem net proc\"\x1b[0m\n"),
            "{}",
            &color[..600]
        );
    }

    #[test]
    fn numeric_errors_split_invalid_vs_range() {
        assert_eq!(parse(&args(&["-p", "abc"])), Err(1));
        assert_eq!(parse(&args(&["-p", "99999999999999999999"])), Err(1));
        assert_eq!(parse(&args(&["-u", "abc"])), Err(1));
        assert_eq!(parse(&args(&["-u", "99999999999999999999"])), Err(1));
        assert_eq!(parse(&args(&["-c", "/tmp"])), Err(1));
        assert_eq!(
            parse(&args(&["--themes-dir", "/nonexistent-dir-xyz"])),
            Err(1)
        );
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
