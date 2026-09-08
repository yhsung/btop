# P1 btop-input — Design (Route A enum+sink)

Date: 2026-09-06
Status: approved S1–S3, pending spec review
Route: A — `process_key` pure fn returning `Vec<Action>`; P3 implements the sink.

## 1. Background

`src/btop_input.cpp` (648) + `.hpp` (78): `poll(timeout)` → `pselect` + non-blocking
`read` → `get()` strips leading `\x1b`, matches `Key_escapes` (~30 entries) or SGR mouse
(`[<0;…M/m`, `[<32;`, `[<64/65;`), hit-tests `Menu::mouse_mappings` (if active) else
`Input::mouse_mappings` (`Mouse_loc{line,col,height,width}` map; Rust twin is
`MouseMap{x,y,w,h,action}` with col→x, line→y). `proc_filtering` short-circuits to click-only.
`process(key)` dispatches global → proc → cpu → mem → net branches, each ending in
`Runner::run(box,no_update,redraw)`. M4 decomposition: P1 input → P2 menu → P3
runner+wiring → P4 app; P1 is the dependency leaf.

## 2. Decisions (user-confirmed)

- Full key→Action mapping with seam (not parse-only).
- Full TextEdit (cursor/blink/mouse), shared by filter + options editor.
- Pure mapping only; fd reading (`poll`/`wait`), `pselect`, SIGUSR1 `interrupt` stay in P4
  with terminal setup.
- Seam route A: pure `process_key` + `ActionSink` trait (not closures, not C++ globals).

## 3. Architecture (S1 approved)

- New workspace member `btop-input`, zero third-party deps.
- `keys.rs`: escape-table decode + SGR mouse decode + rect hit-test over `&[MouseMap]`
  (transcribed tables verbatim; `proc_filtering` short-circuit kept).
- `actions.rs`: `enum Action` + `trait ActionSink { fn emit(&mut self, a: Action) }` +
  pure `process_key(key: &str, st: &InputState) -> Vec<Action>`.
- `textedit.rs`: full editor (text, cursor, blink state machine with caller-injected
  clock, mouse positioning); M3's `filter: Option<String>` seam is its text view.
- `poll`/`wait`/`interrupt`/`clear` NOT ported (P4).

## 4. Action enum + testing (S2 approved)

- `Action` covers every `process()` branch: Quit, ShowMenu(Options/Help/Msg Variants),
  RunBox{target, no_update, redraw}, ProcEdit/Sort/Tree/Select/Signal/Renice/Scroll,
  CpuFreqUp/Down, Mem/Net toggles, Noop. Multi-effect keys yield multiple Actions in
  C++ order.
- Tests: recording fake sink (`Vec<Action>`); key→Action golden vectors transcribed
  from `process()` branch order; mouse hit-test vectors (click/drag/scroll, mapped and
  unmapped rects, menu-active vs input mappings).

## 5. TextEdit + boundaries (S3 approved, CORRECTED 2026-09-06)

- TextEdit = `{text, pos (bytes), upos (chars), numeric}`; `command()` key handling
  verbatim (incl. UTF-8 multibyte paths via uresize/ulen); `render(limit)` verbatim
  (window + underline cursor block, always visible).
- CORRECTION: C++ has NO blink phase and NO mouse positioning (verified: zero
  `blink` matches in src/; TextEdit API is command/render/clear only). The earlier
  "blink state machine / mouse定位" wording was wrong — struck. No clock injection
  needed; no cursor field beyond pos/upos for M4.
- `proc_filtering==true` short-circuit preserved and tested.
- fd/pselect/SIGUSR1 explicitly out (P4); no terminal syscalls in this crate.

## 6. Non-goals

- No Runner/Config mutation (P3 sink), no menu content (P2), no TextEdit rendering
  (draw already renders the filter row; M4 editing view if needed beyond M3 bytes).

## 7. Self-review

- Placeholders: none; modules, enum coverage, seam, and test shapes all concrete.
- Consistency: S1 modules map to S2/S3 behaviors; pure-mapping scope matches the P4
  deferral (no fd code paths left dangling — `poll`/`wait` simply absent).
- Scope: single-crate spec; P2–P4 untouched except named seams (ActionSink,
  TextEdit text view, MouseMap already shipped).
- Ambiguity resolved: "full mapping" = every branch yields Actions (no silent drops);
  "pure" = `&str` in, `Vec<Action>` out, state via explicit `InputState` param.
