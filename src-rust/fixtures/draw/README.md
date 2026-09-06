# draw fixtures

Single-line `.ans` blobs captured from `tests/draw_golden.cpp` via
`capture.sh`. Each file holds one scenario's raw terminal output
(ANSI escapes included, trailing newline appended by the splitter).

## Machine-specific inputs (do not cross-compare across hosts)

`Mem::get_totalMem()` reads host RAM (via sysctl `hw.memsize` on macOS)
and is called inside `Mem::draw`/`Proc::draw`, so `mem_*.ans` and
`proc_*.ans` embed this machine's total memory. They are stable run to
run on ONE machine (proven by the double-run diff in `capture.sh`) but
are NOT portable across hosts.

## cpuHz gating rationale

The harness sets `show_cpu_freq=false` and clears `Cpu::cpuHz` before
the first draw. Reason: `btop_draw.cpp:2366` declares
`static const bool hasCpuHz` inside the cpu-title code, initialized ONCE
on the first `Cpu::draw` call — clearing before the first draw pins it
`false` process-wide (see the `hasCpuHz trap` comment in
`tests/draw_golden.cpp`). Any future scenario populating `cpuHz` after
the first draw would silently still observe `false`, so keep the
clear-first ordering.

## Regenerating

`capture.sh` takes ONE argument — the harness binary — and derives the
output directory from its own location (NOT `<bin> <outdir>`):

```sh
capture.sh <path-to-draw_golden>
```

It runs the harness twice, requires an empty normalized diff
(determinism proof), then splits stdout at `@@BEGIN:<name>@@` /
`@@END:<name>@@` markers into `<name>.ans` files next to the script.

## Diffing

`.ans` files are single-line blobs: terminal diffs wrap them into noise.
Diff with wrapping disabled or visible escapes, e.g. `diff --width=...`,
`cat -v`, or compare per-escape with `sed 's/\x1b/\nESC/g'`.
