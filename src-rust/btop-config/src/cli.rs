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
pub fn parse(args: &[String]) -> Result<Cli, i32> {
    let mut cli = Cli::default();
    let mut it = args.iter().peekable();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--default-config" | "-h" | "--help" | "--version" => return Err(0),
            "-d" | "--debug" => cli.debug = true,
            "--force-utf" => cli.force_utf = true,
            "-l" | "--low-color" => cli.low_color = true,
            "-t" | "--tty" => cli.force_tty = Some(true),
            "--no-tty" => cli.force_tty = Some(false),
            "-c" | "--config" => {
                cli.config_file = Some(PathBuf::from(next_value(&mut it, arg)?))
            }
            "-f" | "--filter" => cli.filter = Some(next_value(&mut it, arg)?),
            "-p" | "--preset" => {
                cli.preset = Some(next_value(&mut it, arg)?.parse().map_err(|_| 1)?)
            }
            "--themes-dir" => {
                cli.themes_dir = Some(PathBuf::from(next_value(&mut it, arg)?))
            }
            "-u" | "--update" => {
                cli.updates = Some(next_value(&mut it, arg)?.parse().map_err(|_| 1)?)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_all_value_flags() {
        let c = parse(&args(&[
            "-c", "/tmp/x.conf", "-f", "ssh",
            "-p", "2", "--themes-dir", "/tmp/t",
            "-u", "500",
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
}
