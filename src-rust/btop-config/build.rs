//! Bake `btop --version` build info at compile time, mirroring the
//! `config.h` substitution (Makefile:339-340): `GIT_COMMIT` (short hash,
//! empty when unavailable), `COMPILER`/`COMPILER_VERSION`, and
//! `CONFIGURE_COMMAND` (the make invocation upstream; here the canonical
//! cargo build command for the active profile).

use std::process::Command;

fn cmd_output(prog: &str, args: &[&str]) -> String {
    Command::new(prog)
        .args(args)
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok()
            } else {
                None
            }
        })
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn main() {
    // Re-run when the commit changes (best effort; harmless if absent).
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs/heads/");

    // `git rev-parse --short HEAD`, empty on failure (mirrors
    // `Makefile:216`, where a missing git yields an empty `GIT_COMMIT`
    // and `version()` omits the `+hash` suffix).
    let commit = cmd_output("git", &["rev-parse", "--short", "HEAD"]);
    println!("cargo:rustc-env=BTOP_GIT_COMMIT={commit}");

    // `rustc --version` → "rustc 1.xx.y (...)"; keep the middle token so
    // the printer can emit `Compiled with: rustc (1.xx.y)`.
    let rustc = cmd_output("rustc", &["--version"]);
    let version = rustc.split_whitespace().nth(1).unwrap_or("0").to_string();
    println!("cargo:rustc-env=BTOP_COMPILER=rustc");
    println!("cargo:rustc-env=BTOP_COMPILER_VERSION={version}");

    // No configure step under cargo; the canonical build command for the
    // active profile is the honest equivalent of `CONFIGURE_COMMAND`.
    let profile = std::env::var("PROFILE").unwrap_or_default();
    let configure = if profile == "release" {
        "cargo build --release -p btop-app"
    } else {
        "cargo build -p btop-app"
    };
    println!("cargo:rustc-env=BTOP_CONFIGURE_COMMAND={configure}");
}
