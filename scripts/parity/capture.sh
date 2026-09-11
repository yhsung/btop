#!/bin/bash
# tmux dual-pane parity harness: run original btop (left) and btop-rust
# (right) side by side, capture both screens, quit cleanly.
#
# Usage:
#   scripts/parity/capture.sh [--wait SECS] [--out DIR] [--width COLS] [--height ROWS]
#   Env: BTOP_ORIG (default /opt/homebrew/bin/btop),
#        BTOP_RUST (default src-rust/target/release/btop),
#        PARITY_HOME (default mktemp-fresh; shared so both run default config)
#
# Output: $OUT/orig.txt, $OUT/rust.txt (plain capture-pane text, 1 file per pane)
# Exit: 0 on clean capture (both processes quit with q).
set -u
WAIT=9; OUT=/tmp/parity; WIDTH=242; HEIGHT=42
while [ $# -gt 0 ]; do
  case "$1" in
    --wait) WAIT="$2"; shift 2;;
    --out) OUT="$2"; shift 2;;
    --width) WIDTH="$2"; shift 2;;
    --height) HEIGHT="$2"; shift 2;;
    *) echo "unknown arg: $1" >&2; exit 2;;
  esac
done
ORIG="${BTOP_ORIG:-/opt/homebrew/bin/btop}"
RUST="${BTOP_RUST:-$PWD/src-rust/target/release/btop}"
HOME_DIR="${PARITY_HOME:-$(mktemp -d /tmp/parity-home-XXXXXX)}"
SESSION="parity$$"
mkdir -p "$OUT" "$HOME_DIR"
cleanup() { tmux kill-session -t "$SESSION" 2>/dev/null || true; }
trap cleanup EXIT
tmux new-session -d -s "$SESSION" -x "$WIDTH" -y "$HEIGHT" || exit 1
tmux split-window -h -t "$SESSION"
tmux send-keys -t "$SESSION:0.0" "HOME=$HOME_DIR TERM=xterm-256color $ORIG" Enter
tmux send-keys -t "$SESSION:0.1" "HOME=$HOME_DIR TERM=xterm-256color $RUST" Enter
sleep "$WAIT"
tmux capture-pane -p -t "$SESSION:0.0" > "$OUT/orig.txt"
tmux capture-pane -p -t "$SESSION:0.1" > "$OUT/rust.txt"
tmux send-keys -t "$SESSION:0.0" q
tmux send-keys -t "$SESSION:0.1" q
sleep 1
echo "captured: $OUT/orig.txt $OUT/rust.txt (home: $HOME_DIR)"
