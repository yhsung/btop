#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# capture.sh — run draw_golden twice, prove determinism, split .ans fixtures.
#
# Usage: capture.sh <path-to-draw_golden>
#
# 1. Runs the harness twice, normalizes volatile spans (wall-clock HH:MM:SS,
#    ISO dates, uptime strings) and requires an EMPTY diff (determinism proof).
#    Box scenarios are configured clock/uptime/battery-free, so normalization
#    is a safety net; any diff failure means a real nondeterminism bug.
# 2. Splits the normalized output at @@BEGIN:<name>@@ / @@END:<name>@@ markers
#    into <name>.ans files next to this script.
#
# NOTE: mem/proc fixtures embed this machine's total RAM (Mem::get_totalMem()
# is called inside Mem::draw/Proc::draw and reads sysctl hw.memsize). Stable
# run-to-run on one machine, but host-specific — do not compare across hosts.

set -eu

if [ "$#" -ne 1 ]; then
	echo "usage: $0 <path-to-draw_golden>" >&2
	exit 2
fi
BIN="$1"
OUT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"

normalize() {
	sed -E \
		-e 's/[0-9]{2}:[0-9]{2}:[0-9]{2}/HH:MM:SS/g' \
		-e 's/[0-9]{4}-[0-9]{2}-[0-9]{2}/YYYY-MM-DD/g' \
		-e 's/up [0-9]+ days?, [0-9]+:[0-9]+/up N days, HH:MM/g'
}

"$BIN" > "$OUT_DIR/.run1.raw"
"$BIN" > "$OUT_DIR/.run2.raw"
normalize < "$OUT_DIR/.run1.raw" > "$OUT_DIR/.run1.norm"
normalize < "$OUT_DIR/.run2.raw" > "$OUT_DIR/.run2.norm"

if ! diff -u "$OUT_DIR/.run1.norm" "$OUT_DIR/.run2.norm"; then
	echo "capture.sh: FAIL — output differs between runs (non-deterministic)" >&2
	exit 1
fi
echo "capture.sh: determinism proof OK (double-run diff empty)"

python3 - "$OUT_DIR/.run1.norm" "$OUT_DIR" <<'EOF'
import re, sys

norm_path, out_dir = sys.argv[1], sys.argv[2]
text = open(norm_path).read()
blocks = re.findall(r'@@BEGIN:(.+?)@@\n(.*?)\n@@END:\1@@\n?', text, re.DOTALL)
opens = len(re.findall(r'@@BEGIN:', text))
if not blocks or len(blocks) != opens:
    print(f'splitter: FAIL — {opens} BEGIN markers, {len(blocks)} complete blocks')
    sys.exit(1)
names = []
for name, body in blocks:
    if '/' in name or name in ('.', '..'):
        print(f'splitter: FAIL — bad block name {name!r}')
        sys.exit(1)
    with open(f'{out_dir}/{name}.ans', 'w') as f:
        f.write(body + '\n')
    names.append(name)
print(f'splitter: wrote {len(names)} fixtures: {", ".join(names)}')
EOF

rm -f "$OUT_DIR/.run1.raw" "$OUT_DIR/.run2.raw" "$OUT_DIR/.run1.norm" "$OUT_DIR/.run2.norm"
