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

## Menu overlay fixtures

`menu_main.ans`, `menu_options.ans`, `menu_help.ans` capture
`Global::overlay` after `Menu::show(Main|Options|Help)` at **100x30,
Default theme**, with the same `setup()` Config overrides as the box
scenarios (clock/uptime/battery/watts/freq off, fixed boxes
`"cpu mem net proc"` + `Draw::calcSizes()`). `show()` alone populates
the string: it sets the menuMask bit and calls `Menu::process("")`,
whose menu body fills `Global::overlay` synchronously — no
`Runner::run("overlay")` needed to populate (the trailing
`Runner::run("all", ...)` inside `process()` is a harmless headless
no-op). Between scenarios the harness closes the menu with
`Menu::process("escape")` so the next `show()` starts fresh.

`msgbox_ok.ans` / `msgbox_yesno.ans` capture standalone
`Menu::msgBox(45, OK|YES_NO, {"Golden msgbox line one",
"Golden msgbox line two"}, "golden ok|yesno")()` — width 45, fixed
ASCII content + titles.

Determinism notes: theme list headless is `[Default, TTY]` only
(`Theme::updateThemes()` skips empty theme dirs), so the options page
(`color_theme` idx `1/2`) is stable; options shows the general-category
first page, help shows page 1 of 3. Proven by the same double-run diff
gate in `capture.sh` (no extra normalization needed).

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
