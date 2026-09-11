//! CLI parsing for `btop`.
//!
//! P4 T1: skeleton only — the actual `Cli::` parser still lives in
//! `btop-config/src/cli.rs` (P1 port, src/btop_cli.cpp:60-190). The split
//! into its own crate is pending a follow-up refactor (plan P4 spec §3).
//! Consumers (P4 `btop-app`) depend on this crate so the move is a
//! drop-in import swap when the refactor lands.
