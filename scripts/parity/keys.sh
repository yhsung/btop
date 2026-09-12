#!/bin/bash
# tmux key-matrix checker: drive the rust binary through manpage CLI flags
# (headless where possible) and interactive keys, capture screens, and
# assert screen markers. Each case prints PASS/FAIL.
#
# Usage: scripts/parity/keys.sh [--bin PATH] [--out DIR]
# Env: BTOP_RUST (default src-rust/target/release/btop)
set -u
BIN="${1:-${BTOP_RUST:-$PWD/src-rust/target/release/btop}}"
if [ "${1:-}" = "--bin" ]; then BIN="$2"; shift 2; fi
OUT="${OUT:-/tmp/parity-keys}"
SESSION="keys$$"
PASS=0; FAIL=0
mkdir -p "$OUT"
cleanup() { tmux kill-session -t "$SESSION" 2>/dev/null || true; }
trap cleanup EXIT

check() { # check <name> <needle> <file>
  if grep -qF "$2" "$3"; then echo "PASS: $1"; PASS=$((PASS+1));
  else echo "FAIL: $1 (missing [$2])"; FAIL=$((FAIL+1)); fi
}

# ── headless CLI flags (manpage OPTIONS) ──────────────────────────────
"$BIN" --version > "$OUT/version.txt" 2>&1
check "cli --version" "1.4.7" "$OUT/version.txt"
"$BIN" --help > "$OUT/help.txt" 2>&1
check "cli --help" "Usage" "$OUT/help.txt"
"$BIN" --default-config > "$OUT/defconf.txt" 2>&1
check "cli --default-config" "shown_boxes" "$OUT/defconf.txt"

# ── interactive keys (fresh HOME each, 120x40, -u 500 for fast ticks) ─
run_case() { # run_case <name> <extra-args> <keys...> ; keys sent 1.2s apart
  local name="$1"; local args="$2"; shift 2
  local home; home="$(mktemp -d /tmp/parity-key-XXXXXX)"
  tmux new-session -d -s "$SESSION" -x 120 -y 40
  tmux send-keys -t "$SESSION" "HOME=$home TERM=xterm-256color $BIN -u 500 $args" Enter
  sleep 3
  for k in "$@"; do tmux send-keys -t "$SESSION" "$k"; sleep 1.2; done
  sleep 1.5
  tmux capture-pane -p -t "$SESSION" > "$OUT/$name.txt"
  tmux kill-session -t "$SESSION" 2>/dev/null || true
  rm -rf "$home"
}
run_case boot ""
check "boot cpu box" "¹cpu" "$OUT/boot.txt"
check "boot proc count" "/" "$OUT/boot.txt"
if grep -Eq '[0-9]{2}:[0-9]{2}:[0-9]{2}' "$OUT/boot.txt"; then echo "PASS: header clock"; PASS=$((PASS+1));
else echo "FAIL: header clock (no HH:MM:SS)"; FAIL=$((FAIL+1)); fi
run_case quit-q "" q
check "quit-q exits" "Quitting! Runtime:" "$OUT/quit-q.txt"
run_case preset0 "-p 0"
check "preset0 boots" "¹cpu" "$OUT/preset0.txt"
run_case filter "-f opencode"
check "filter keeps match" "opencode" "$OUT/filter.txt"
run_case help-key "" h
check "help overlay" "Description:" "$OUT/help-key.txt"
run_case toggle-mem "" 2
check "toggle mem off" "¹cpu" "$OUT/toggle-mem.txt"
run_case update-plus "" +
check "update-plus survives" "¹cpu" "$OUT/update-plus.txt"
run_case sort-m "" m
check "sort mode cycles" "←" "$OUT/sort-m.txt"
run_case tree-e "" e
check "tree toggle" "[-]─1" "$OUT/tree-e.txt"
run_case tree-collapse "" e E
check "tree collapse-all" "[+]─" "$OUT/tree-collapse.txt"
run_case detail-enter "" Down Enter
check "detail pane" "Status:" "$OUT/detail-enter.txt"
# mouse click: SGR press+release on proc row 3 (col 70, line 18), then 't'
# opens the signal menu iff the click selected the row.
mouse_click_case() {
  local home; home="$(mktemp -d /tmp/parity-key-XXXXXX)"
  tmux new-session -d -s "$SESSION" -x 120 -y 40
  tmux send-keys -t "$SESSION" "HOME=$home TERM=xterm-256color $BIN -u 500" Enter
  sleep 3
  tmux send-keys -t "$SESSION" $'\x1b[<0;70;18M'; sleep 1
  tmux send-keys -t "$SESSION" $'\x1b[<0;70;18m'; sleep 1
  tmux send-keys -t "$SESSION" t; sleep 2
  tmux capture-pane -p -t "$SESSION" > "$OUT/mouse-click.txt"
  tmux kill-session -t "$SESSION" 2>/dev/null || true
  rm -rf "$home"
}
mouse_click_case
check "mouse click selects" "Send signal:" "$OUT/mouse-click.txt"
# options outside-click (1,1) closes the menu back to boxes.
run_case options-close "" o
check "options opens" "Color theme" "$OUT/options-close.txt"
options_click_case() {
  local home; home="$(mktemp -d /tmp/parity-key-XXXXXX)"
  tmux new-session -d -s "$SESSION" -x 120 -y 40
  tmux send-keys -t "$SESSION" "HOME=$home TERM=xterm-256color $BIN -u 500" Enter
  sleep 3
  tmux send-keys -t "$SESSION" o; sleep 2
  tmux send-keys -t "$SESSION" $'\x1b[<0;1;1M'; sleep 1
  tmux send-keys -t "$SESSION" $'\x1b[<0;1;1m'; sleep 2
  tmux capture-pane -p -t "$SESSION" > "$OUT/options-click.txt"
  tmux kill-session -t "$SESSION" 2>/dev/null || true
  rm -rf "$home"
}
options_click_case
if grep -qF "Color theme" "$OUT/options-click.txt"; then echo "FAIL: options outside-click close (menu still open)"; FAIL=$((FAIL+1));
else echo "PASS: options outside-click close"; PASS=$((PASS+1)); fi

echo "== $PASS passed, $FAIL failed =="
[ "$FAIL" -eq 0 ]
