# AGENTS.md — btop (C++ upstream + Rust port)

Fork of aristocratos/btop with a from-scratch Rust port in `src-rust/`
(workspace: btop-app, btop-cli, btop-collect, btop-config, btop-draw,
btop-input, btop-menu, btop-runner, btop-tools). C++ truth lives in
`src/`, `osx/`, `linux/`, `include/`. The Rust binary is the goal;
`btop` on PATH (`/opt/homebrew/bin/btop`) is the reference oracle.

## Build / test gates (run from `src-rust/`)

```bash
cargo test --workspace --locked --offline        # 526 tests / 32 suites
cargo clippy --workspace --locked --offline --lib --bins -- -D warnings
cargo fmt --check
cargo build --release -p btop-app --locked --offline  # → target/release/btop
```

- Pinned local toolchain is nightly-2025-02-16; CI uses latest stable.
  New code must satisfy BOTH: stable clippy has extra lints
  (`chunks_exact_to_as_chunks`, `manual_is_multiple_of`, `contains`-over-
  `iter().any`, `sort_by_key`, …). MSRV-sensitive rewrites only:
  no `as_chunks` (1.88+), no `is_multiple_of` (1.87+) — see
  `real.rs` chunking comment and `menus.rs` bitwise-oddness comment.
- `cargo clippy --all-targets` has pre-existing hits in golden test
  files (snake_case test names) — out of scope, do not churn test names.
- Linux CI builds examples: `btop-collect/examples/record.rs` must keep
  its non-macOS `main` stub. Live smoke tests must be degrade-tolerant
  (no swap / no thermal / no GPU on CI runners — assert the call, not
  the hardware).

## E2E parity harness (`scripts/parity/`)

tmux dual-pane: original (left) vs Rust (right), shared fresh HOME
(default config both sides), fixed geometry, `capture-pane` diff.

```bash
scripts/parity/capture.sh [--wait SECS] [--out DIR] [--width COLS] [--height ROWS]
# → $OUT/orig.txt $OUT/rust.txt
scripts/parity/keys.sh   # manpage CLI flags (headless) + interactive key matrix
```

Env overrides: `BTOP_ORIG`, `BTOP_RUST`, `PARITY_HOME`.
Rules: capture mid-run (post-`q` the alt-screen is gone — shell only);
same wall-clock for both panes; initial triage ignores absolute values
(speeds, temps, pids) and compares structure (boxes, titles, columns,
tabs, counts within sampling noise).

## Parity workflow

1. `capture.sh` → diff orig vs rust → file divergences (data bug /
   missing draw / missing input).
2. Fix C++-truth-first (`src/osx/btop_collect.cpp`, `src/btop_draw.cpp`,
   `src/btop.cpp` + cpp line cites in comments and commit messages).
3. **One commit per fix** (`fix(rust): <area> — <what>`). Never batch.
4. Re-run gates + `capture.sh` to close the loop; extend `keys.sh`
   markers when a new behavior becomes testable.

Known-acceptable deferrals (do not re-litigate): GPU panels, Theme-file
port, header clock/menu interactivity, mouse maps, `--help` printers.
