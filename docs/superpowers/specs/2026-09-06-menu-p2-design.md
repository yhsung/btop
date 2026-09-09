# P2 btop-menu — Design (Route A: single Action + harness parity)

Date: 2026-09-06
Status: approved S1–S3, pending spec review
Route: A — extend P1 `Action`, overlay byte-parity via `draw_golden` scenarios.

## 1. Background

`src/btop_menu.cpp` (1963) + `.hpp` (99): `categories[5]` option tables (general 21,
cpu 15, mem 13, net 7, proc 13 + gpu 6 under GPU_SUPPORT), `optionsMenu` state machine
(~443 lines: paging/browse/edit/validate + Config/Theme/Runner side effects),
`msgBox` (OK/YES_NO/NO_YES → Invalid/Ok_Yes/No_Esc/Select), 7 menus
(signalChoose/SignalSend/signalReturn/sizeError/mainMenu/optionsMenu/helpMenu/reniceMenu),
`menuFunc[]` + `menuMask` dispatcher writing `Global::overlay`. Reads Config/Theme,
writes overlay + mouse zones, drives `Runner::run`. M4 order: P1 (done) → P2 → P3 → P4.

## 2. Decisions (user-confirmed)

- Overlay output: byte parity via extended `draw_golden` menu scenarios (not structural-only).
- Side effects: extend P1 `Action` enum (not traits, not C++-globals transliteration).
- Route A over B (weaker verification) and C (statics debt to P3).

## 3. Architecture (S1 approved)

- New workspace member `btop-menu`, zero third-party deps; deps: `btop-input`
  (Action/sink), `btop-config`, `btop-draw` (createBox/banner/ansi).
- `tables.rs`: help_text/categories/P_Signals static tables transcribed verbatim.
- `options.rs`: options state machine with OWNED `MenuState` (editing, warnings,
  theme_refresh, screen_redraw, selection, page) — no statics.
- `menus.rs`: 7 menus + `menuFunc`/`menuMask` dispatcher + `Global::overlay`
  equivalent (owned `Overlay` output struct).
- `msgbox.rs`: msgBox state machine (kind + selected + input→return code).

## 4. Action seam + parity (S2 approved)

- Extend P1 `Action` with menu effects: SetTheme, CalcSizes, WriteConfig, Kill{pid,sig},
  SetPriority{pid,nice}, PauseOutput, SetLogLevel (and whatever optionsMenu side
  effects surface — enumerated in plan, P3 single sink executes all).
- Overlay strings: `draw_golden` gains menu scenarios (main/options/help/msgBox at
  fixed sizes + themes); Rust tests byte-compare `.ans` fixtures.

## 5. State + testing (S3 approved)

- optionsMenu statics converge into owned `MenuState`; validation errors via
  `Config::intValid/stringValid` + `validError` text (transcribed, no logic change).
- msgBox unit tests cover all four return codes + toggle/select paths.
- signal/renice tested via recording fake `Action` sequences (kill/errno branches
  incl. dead-detailed guards); theme key-filtering/space-tolerance deferral
  (M1 item) lands here as documented behavior with tests.

## 6. Non-goals

- No Runner execution (P3 sink), no fd/terminal (P4), no TextEdit widget changes
  (P1 as-is; options editor reuses it).

## 7. Self-review

- Placeholders: none; modules, seam, and test shapes concrete.
- Consistency: single-Action language matches P1 precedent; harness parity matches
  M3 precedent; owned-state matches P1 InputState precedent.
- Scope: single-crate spec; P3/P4 seams named (sink executes; P4 owns terminal).
- Ambiguity resolved: "byte parity" = overlay `.ans` equality at fixed geometries;
  "extend Action" = new variants on the P1 enum (no second enum).
