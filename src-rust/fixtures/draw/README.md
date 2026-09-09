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

## Proc detail/tree/filter fixtures

`proc_detail.ans`, `proc_tree.ans`, `proc_filtered.ans` capture
`Proc::draw` at **S0 (100x30)** with the base 3-proc fixture
(`launchd` pid 1, `kernel_task` pid 777, `btop` pid 4242, pid order).
S0 suffices: proc box is h=20 there, and the detail header costs 8 rows,
leaving a 12-row list; tree/filter are list-only. Each scenario runs its
own `setup(S0, "cpu mem net proc")` pass and resets its flips afterwards,
so the menu block below still starts from defaults.

Setup deltas (all verified against `src/btop_draw.cpp` Proc::draw state):

- `proc_detail`: `show_detailed=true` + `detailed_pid=4242` **plus**
  `Proc::detailed.last_pid=4242` (draw gates on
  `show_detailed && detailed.last_pid == detailed_pid`). `detailed.entry`
  is the `btop` proc_info; `status="Running"`, `elapsed="12:34"`,
  `parent="launchd"`, `io_read="1.0M"`, `io_write="512K"`,
  `memory="64M"`, fixed 8-sample `cpu_percent`/`mem_bytes`,
  `first_mem=134217728`. At S0 widths the detail labels show
  Status/Elapsed/IO-R only (`item_fit=3`); terminate/kill/follow buttons
  are width-gated out.
- `proc_tree`: `proc_tree=true` + manual tree fields (the harness bypasses
  `collect()`, so `_tree_gen`/`_collect_prefixes` output is staged by
  hand): `ppid 0/1/1`, `depth 0/1/1`, prefixes `[-]─` / ` ├─` / ` └─`,
  `tree_index 0/1/2`, `collapsed=false`, `filtered=false`.
- `proc_filtered`: committed-filter view — `proc_filter="btop"` with
  `proc_filtering=false` (editing mode would render the TextEdit cursor),
  `plist[0..1].filtered=true`, `numpids=1`, `filter_found=2` (mirrors
  `collect()`'s `numpids = size - filter_found`).

Fidelity note: `proc_detail.ans` contains **2 embedded NUL bytes**. The
detail header calls `uresize(name, n, wide=true)`, whose wide path
(`btop_tools.cpp:269-288`) sizes the output to the wchar count including
the terminator, leaving `wcstombs`' `\0` embedded in the `std::string`.
Real btop writes size-aware so it is unaffected; the harness `emit()`
uses `fwrite` (not `printf %s`) to preserve the faithful bytes. Rust
`String` can hold U+0000, so transcription must compare NUL-inclusive.

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

## Trailing state

The menu block ends with `Config::unlock()`: `Menu::process` calls
`Runner::run("all")`, which leaves `Config::locked == true` even headless, and a
locked Config diverts later `setup()` sets into `*Tmp` staging maps. The unlock
restores the invariant so scenarios appended after menus behave.
