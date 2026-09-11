//! Seven menus + dispatcher, transcribed from `src/btop_menu.cpp`.
//!
//! SCOPE (Task 5): pure state transitions + [`Action`] emission. Overlay
//! ASSEMBLY (ANSI boxes, banners, mouse zones) is Task 6: this module keeps
//! an [`MenuSystem::overlay`] string field (cleared on the empty-mask path
//! like `Global::overlay`, `:1908`) but menus do not render into it — Task 6
//! builds bytes from the state kept here plus [`crate::tables`]/
//! [`crate::options::option_rows`] data. The split is documented per menu
//! below.
//!
//! OWNERSHIP (C++ statics → owned fields): every `static` inside the C++ menu
//! fns lives in [`MenuSystem`] (`signal_selected`, `main_selected`,
//! `help_page`, `renice_nice`/`renice_edit`, `msg_box`, [`MenuState`]); the
//! C++ `bg.empty()` open-reset checks become [`MenuSystem::reset_for`],
//! called when the dispatcher activates a menu. `s_pid` (the kill/renice
//! target, `:1008`, `:1143`, `:1804`) is NOT stored: P3 resolves
//! selected/detailed pid and passes it as the explicit `target_pid` param
//! (documented per menu).
//!
//! DISPATCHER MAPPING (`:1905-1962`, verified against source):
//! - empty mask → `active=false`, overlay/mouse cleared, `current=None`,
//!   `[PauseOutput{false}, Run{All,true,true}]` (`:1906-1916` sets
//!   `pause_output=false` and runs `("all", true, true)`).
//! - new current → `active=true`, `redraw=true`, size-coerce, highest set bit
//!   wins (`:1919-1933`; the loop overwrites so the HIGHEST bit stays).
//! - menu `Closed` → reset bit, clear mouse, `PauseOutput{false}`, recurse
//!   with `""` (`:1936-1942`).
//! - `redraw` flag set (by activation or `Switch`) → clear it,
//!   `Run{All,true,true}` (`:1943-1946`). Checked BEFORE `Changed`/`Switch`,
//!   mirroring the `else if` order.
//! - `Changed` → `Run{Overlay,false,false}` (`:1947-1948`: `run("overlay")`
//!   uses the declared defaults `no_update=false, force_redraw=false`,
//!   `src/btop_shared.hpp:86`).
//! - `Switch` → `PauseOutput{false}`, `redraw=true`, clear mouse, recurse
//!   (`:1949-1955`).
//! - `NoChange` → no `Run` (falls through all arms).
//! - `show(menu, signal)` sets the mask bit + `signal_to_send`, then
//!   `process("")` (`:1958-1962`).
//!
//! SIZE RULE (`:1922-1927`, transcribed exactly — note the precedence
//! `((big-family) AND (w<80 OR h<24)) OR (w<50 OR h<20))`):
//! `Main`/`Options`/`Help`/`SignalChoose` need 80x24, everything else 50x20;
//! violation resets the mask to `SizeError` only.
//!
//! OUTCOME CODES (`:994-999`): `NoChange=0`, `Changed=1`, `Closed=2`,
//! `Switch=3` — mirrored as [`MenuOutcome`] discriminants.
//!
//! QUIT REUSE: C++ `mainMenu` calls `clean_quit(0)` directly (`:1266`); here
//! that path returns [`Action::Quit`] (the P1 global-`q` variant, reused —
//! no new variant) for the P3 sink to execute.
//!
//! KILL ERRORS: C++ calls `kill()` synchronously and sets the `SignalReturn`
//! mask bit on failure (`:1036-1041`, `:1162-1165`), then falls into
//! `MenuClosing` (returns `Closed`, NOT `Switch` — the pending bit drives the
//! next dispatch). This module mirrors that: the synchronous `pid < 1`
//! (`ESRCH`) check stores [`MenuSystem::kill_errno`] + sets the bit and
//! returns `Closed`. Async kill failures (non-zero return with a valid pid)
//! are a P3 sink duty: the sink records `errno` and shows `SignalReturn`.
//! Hence the success path emits only [`Action::Kill`] + `Closed`.

use std::collections::HashMap;

use btop_config::config::Config;
use btop_input::actions::{Action, RunTarget};
use btop_input::textedit::TextEdit;
use btop_tools::mouse::MouseMap;

use crate::msgbox::{BoxKind, MsgBox};
use crate::options::{
    self, begin_edit_text, commit_edit, flip_or_cycle, MenuState, OptKind, OptionsStore,
};
use crate::tables::{CATEGORIES, HELP_TEXT};

/// `Menus` (`src/btop_menu.hpp:82-91`). Discriminants MUST match the C++
/// `menuFunc` index order (`:1893-1902`): SizeError=0 … Main=7 — the
/// dispatcher picks the highest set bit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Menus {
    SizeError = 0,
    SignalChoose = 1,
    SignalSend = 2,
    SignalReturn = 3,
    Options = 4,
    Help = 5,
    Renice = 6,
    Main = 7,
}

impl Menus {
    fn bit(self) -> u8 {
        1 << (self as u8)
    }

    fn all() -> [Menus; 8] {
        use Menus::*;
        [
            SizeError,
            SignalChoose,
            SignalSend,
            SignalReturn,
            Options,
            Help,
            Renice,
            Main,
        ]
    }
}

/// Return codes (`:994-999`): `NoChange=0`, `Changed=1`, `Closed=2`,
/// `Switch=3`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MenuOutcome {
    NoChange = 0,
    Changed = 1,
    Closed = 2,
    Switch = 3,
}

/// `errno` values for [`signal_return_text`] (POSIX; no libc crate —
/// transcribed literals, used only for text selection).
pub const ESRCH: i32 = 3;
pub const EINVAL: i32 = 22;
pub const EPERM: i32 = 1;

/// Caller-supplied context per `process` call. `target_pid` is the P3-resolved
/// kill/renice pid (C++ `s_pid`: `show_detailed && selected_pid == 0 ?
/// detailed_pid : selected_pid`, `:1008`); `target_name` is the matching
/// process name for menu headers (C++ `selected_name` / detailed entry
/// name, `:1148`). Signal menus stash both on `show` (mirroring the
/// `s_pid` static) so later navigation keys keep the target.
pub struct MenuCtx {
    pub term_w: usize,
    pub term_h: usize,
    pub target_pid: u64,
    pub target_name: String,
}

/// Owned menu system: mask + current + C++ globals/statics.
#[derive(Debug, Default)]
pub struct MenuSystem {
    pub mask: u8,
    pub current: Option<Menus>,
    pub signal_to_send: i32,
    /// C++ `Menu::redraw` global: set on activation/`Switch`/theme-refresh,
    /// consumed by the dispatcher (`:1943-1946`).
    pub redraw: bool,
    pub active: bool,
    pub overlay: String,
    pub mouse_maps: Vec<MouseMap>,
    // Per-menu episodic state (C++ fn statics):
    pub options: MenuState,
    pub msg_box: MsgBox,
    /// `selected_signal` (`:1005`): -1 = none, else 0..=64 typed/grid value.
    pub signal_selected: i32,
    /// `selected` (`:1222`): 0 = Options, 1 = Help, 2 = Quit.
    pub main_selected: usize,
    /// `page` (`:1747`): separate from [`MenuState::page`] (options owns its
    /// paging) — help paging is an independent counter over [`HELP_TEXT`].
    pub help_page: usize,
    /// `selected_nice` (`:1800`) + `nice_edit` (`:1801`).
    pub renice_nice: i32,
    pub renice_edit: String,
    /// Stashed signal target (C++ `s_pid` static, `:1140-1144`): set on
    /// `show` for the signal family, reused by navigation keys.
    pub signal_pid: u64,
    /// Process name for signal headers (`selected_name` / detailed entry
    /// name, `:1148`).
    pub signal_pname: String,
    /// C++ `signalKillRet`: errno stored on the synchronous `pid < 1` path
    /// for [`signal_return_text`]; the sink overwrites it on async failures.
    pub kill_errno: i32,
}

impl MenuSystem {
    /// Open-reset for one menu (C++ `bg.empty()` inits: `:1011`, `:1226`,
    /// `:1346-1350`, `:1750`, `:1807-1810`; msgBox ctor `:921`).
    fn reset_for(&mut self, menu: Menus) {
        match menu {
            Menus::SignalChoose => self.signal_selected = -1,
            Menus::SignalSend => self.msg_box = MsgBox::new(BoxKind::YesNo),
            Menus::SignalReturn | Menus::SizeError => {
                self.msg_box = MsgBox::new(BoxKind::Ok);
            }
            Menus::Options => self.options = MenuState::default(),
            Menus::Help => self.help_page = 0,
            Menus::Renice => {
                self.renice_nice = 0;
                self.renice_edit.clear();
            }
            Menus::Main => self.main_selected = 0,
        }
    }

    /// Size coercion (`:1922-1927`): big-family menus need 80x24, all menus
    /// need 50x20; violation resets the mask to `SizeError` only.
    fn coerce_size(&mut self, w: usize, h: usize) {
        let big = Menus::Main.bit()
            | Menus::Options.bit()
            | Menus::Help.bit()
            | Menus::SignalChoose.bit();
        if ((self.mask & big) != 0 && (w < 80 || h < 24)) || w < 50 || h < 20 {
            self.mask = Menus::SizeError.bit();
        }
    }

    /// `show(menu, signal)` (`:1958-1962`): set bit + signal, `process("")`.
    /// Signal-family menus stash the target (`s_pid` static, `:1140-1144`)
    /// for navigation keys and rendering.
    pub fn show(
        &mut self,
        menu: Menus,
        signal: i32,
        ctx: &MenuCtx,
        store: &mut OptionsStore,
        cfg: &mut Config,
        lists: &HashMap<String, Vec<String>>,
    ) -> Vec<Action> {
        self.mask |= menu.bit();
        self.signal_to_send = signal;
        if matches!(
            menu,
            Menus::SignalChoose | Menus::SignalSend | Menus::Renice
        ) {
            self.signal_pid = ctx.target_pid;
            self.signal_pname = ctx.target_name.clone();
        }
        self.process("", ctx, store, cfg, lists)
    }

    /// Dispatcher (`:1905-1956`). See the module-level outcome mapping.
    pub fn process(
        &mut self,
        key: &str,
        ctx: &MenuCtx,
        store: &mut OptionsStore,
        cfg: &mut Config,
        lists: &HashMap<String, Vec<String>>,
    ) -> Vec<Action> {
        if self.mask == 0 {
            self.active = false;
            self.overlay.clear();
            self.current = None;
            self.mouse_maps.clear();
            return vec![
                Action::PauseOutput { paused: false },
                Action::Run {
                    target: RunTarget::All,
                    no_update: true,
                    redraw: true,
                },
            ];
        }

        if self.current.is_none_or(|c| self.mask & c.bit() == 0) {
            self.active = true;
            self.redraw = true;
            self.coerce_size(ctx.term_w, ctx.term_h);
            let mut pick = Menus::SizeError;
            for m in Menus::all() {
                if self.mask & m.bit() != 0 {
                    pick = m;
                }
            }
            if self.current != Some(pick) {
                self.reset_for(pick);
            }
            self.current = Some(pick);
        }

        let cur = self.current.expect("mask non-empty implies a pick");
        let was_redraw = self.redraw;
        let (outcome, mut acts) = self.dispatch(cur, key, ctx, store, cfg, lists);
        if outcome == MenuOutcome::Closed {
            self.mask &= !cur.bit();
            self.mouse_maps.clear();
            acts.push(Action::PauseOutput { paused: false });
            let mut rest = self.process("", ctx, store, cfg, lists);
            acts.append(&mut rest);
        } else if was_redraw || self.redraw {
            // `else if (redraw)` (:1943-1946): fires for ANY outcome when
            // the flag is set (activation/first-draw or Switch), taking
            // precedence over Changed/Switch handling below.
            self.redraw = false;
            acts.push(Action::Run {
                target: RunTarget::All,
                no_update: true,
                redraw: true,
            });
        } else {
            match outcome {
                MenuOutcome::Changed => acts.push(Action::Run {
                    target: RunTarget::Overlay,
                    no_update: false,
                    redraw: false,
                }),
                MenuOutcome::Switch => {
                    self.mouse_maps.clear();
                    acts.push(Action::PauseOutput { paused: false });
                    self.redraw = true;
                    let mut rest = self.process("", ctx, store, cfg, lists);
                    acts.append(&mut rest);
                }
                MenuOutcome::NoChange | MenuOutcome::Closed => {}
            }
        }
        acts
    }

    fn dispatch(
        &mut self,
        menu: Menus,
        key: &str,
        ctx: &MenuCtx,
        store: &mut OptionsStore,
        cfg: &mut Config,
        lists: &HashMap<String, Vec<String>>,
    ) -> (MenuOutcome, Vec<Action>) {
        // First call after activation only draws (C++ `if (redraw)` arm runs
        // setup and skips input); episodic state was set by `reset_for`.
        if self.redraw {
            return (MenuOutcome::Changed, vec![]);
        }
        match menu {
            Menus::SizeError => self.menu_size_error(key),
            Menus::SignalChoose => self.menu_signal_choose(key, self.signal_pid),
            Menus::SignalSend => self.menu_signal_send(key, self.signal_pid),
            Menus::SignalReturn => self.menu_signal_return(key),
            Menus::Options => self.menu_options(key, ctx, store, cfg, lists),
            Menus::Help => self.menu_help(key, ctx.term_h),
            Menus::Renice => self.menu_renice(key, self.signal_pid),
            Menus::Main => self.menu_main(key),
        }
    }

    /// Shared kill path (`signalChoose` enter `:1032-1042`, `signalSend`
    /// confirm `:1160-1167`): `pid < 1` is the synchronous `ESRCH`
    /// (`:1034-1037`) — store errno, set `SignalReturn`, `Closed`. Valid pids
    /// emit [`Action::Kill`] + `Closed`; async failures are a P3 sink duty.
    fn kill_path(&mut self, pid: u64, sig: i32) -> (MenuOutcome, Vec<Action>) {
        if pid < 1 {
            self.kill_errno = ESRCH;
            self.mask |= Menus::SignalReturn.bit();
            (MenuOutcome::Closed, vec![])
        } else {
            (MenuOutcome::Closed, vec![Action::Kill { pid, sig }])
        }
    }

    /// `sizeError` (`:1117-1137`): static content (Task 6 renders it); any
    /// confirm/cancel msgBox code closes, else `NoChange`.
    fn menu_size_error(&mut self, key: &str) -> (MenuOutcome, Vec<Action>) {
        use crate::msgbox::MsgReturn::*;
        match self.msg_box.input(key) {
            OkYes | NoEsc => (MenuOutcome::Closed, vec![]),
            _ => (MenuOutcome::NoChange, vec![]),
        }
    }

    /// `signalChoose` (`:1001-1115`): digits/backspace + grid nav skipping 16
    /// (`1..31` over `P_Signals`; 0/16 have no grid cell, `:1089`), `button_N`
    /// select/confirm, enter → kill path, esc/q → `Closed`.
    fn menu_signal_choose(&mut self, key: &str, pid: u64) -> (MenuOutcome, Vec<Action>) {
        if matches!(key, "escape" | "q") {
            return (MenuOutcome::Closed, vec![]);
        }
        if let Some(n) = key
            .strip_prefix("button_")
            .and_then(|s| s.parse::<i32>().ok())
        {
            if n == self.signal_selected {
                let sig = self.signal_selected;
                return self.kill_path(pid, sig);
            }
            self.signal_selected = n;
            return (MenuOutcome::Changed, vec![]);
        }
        if matches!(key, "enter" | "space") && self.signal_selected >= 0 {
            let sig = self.signal_selected;
            return self.kill_path(pid, sig);
        }
        if key.len() == 1 && key.as_bytes()[0].is_ascii_digit() && self.signal_selected < 10 {
            let typed: i32 = if self.signal_selected < 1 {
                key.parse().unwrap_or(0)
            } else {
                format!("{}{}", self.signal_selected, key)
                    .parse()
                    .unwrap_or(64)
            };
            self.signal_selected = typed.min(64);
            return (MenuOutcome::Changed, vec![]);
        }
        if key == "backspace" && self.signal_selected != -1 {
            self.signal_selected = if self.signal_selected < 10 {
                -1
            } else {
                self.signal_selected / 10
            };
            return (MenuOutcome::Changed, vec![]);
        }
        let s = self.signal_selected;
        if matches!(key, "up" | "k") && s != 16 {
            self.signal_selected = if s == 1 {
                31
            } else if s < 6 {
                s + 25
            } else {
                let offset = s > 16;
                let mut v = s - 5;
                if v <= 16 && offset {
                    v -= 1;
                }
                v
            };
            return (MenuOutcome::Changed, vec![]);
        }
        if matches!(key, "down" | "j") {
            self.signal_selected = if s == 31 || s < 1 || s == 16 {
                1
            } else if s > 26 {
                s - 25
            } else {
                let offset = s < 16;
                let mut v = s + 5;
                if v >= 16 && offset {
                    v += 1;
                }
                if v > 31 {
                    31
                } else {
                    v
                }
            };
            return (MenuOutcome::Changed, vec![]);
        }
        if matches!(key, "left" | "h") && s > 0 && s != 16 {
            let mut v = s - 1;
            if v < 1 {
                v = 31;
            } else if v == 16 {
                v -= 1;
            }
            self.signal_selected = v;
            return (MenuOutcome::Changed, vec![]);
        }
        if matches!(key, "right" | "l") && s <= 31 && s != 16 {
            let mut v = s + 1;
            if v > 31 {
                v = 1;
            } else if v == 16 {
                v += 1;
            }
            self.signal_selected = v;
            return (MenuOutcome::Changed, vec![]);
        }
        (MenuOutcome::NoChange, vec![])
    }

    /// `signalSend` (`:1139-1185`): preset `signal_to_send`, YES_NO box;
    /// Ok → kill path, No → `Closed`, Select → `Changed`.
    fn menu_signal_send(&mut self, key: &str, pid: u64) -> (MenuOutcome, Vec<Action>) {
        use crate::msgbox::MsgReturn::*;
        match self.msg_box.input(key) {
            OkYes => self.kill_path(pid, self.signal_to_send),
            NoEsc => (MenuOutcome::Closed, vec![]),
            Select => (MenuOutcome::Changed, vec![]),
            Invalid => (MenuOutcome::NoChange, vec![]),
        }
    }

    /// `signalReturn` (`:1187-1217`): errno text box (see
    /// [`signal_return_text`]); confirm/cancel → `Closed`.
    fn menu_signal_return(&mut self, key: &str) -> (MenuOutcome, Vec<Action>) {
        use crate::msgbox::MsgReturn::*;
        match self.msg_box.input(key) {
            OkYes | NoEsc => (MenuOutcome::Closed, vec![]),
            _ => (MenuOutcome::NoChange, vec![]),
        }
    }

    /// `mainMenu` (`:1219-1296`): `selected` 0..2 (Options/Help/Quit,
    /// banners from `MENU_BANNERS` rendered by Task 6); nav incl j/k/scroll;
    /// enter/space → `Switch` (mask + forced `current`, `:1257-1264`) or
    /// [`Action::Quit`]; esc/q/m/click → `Closed`.
    fn menu_main(&mut self, key: &str) -> (MenuOutcome, Vec<Action>) {
        if matches!(key, "escape" | "q" | "m" | "mouse_click") {
            return (MenuOutcome::Closed, vec![]);
        }
        // C++ takes `key.back() - '0'` unchecked (`:1249`); out-of-range
        // button keys (never emitted — only 3 main buttons exist) are
        // NoChange here rather than an out-of-bounds select.
        if let Some(n) = key
            .strip_prefix("button_")
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|n| *n <= 2)
        {
            if n == self.main_selected {
                return self.main_enter();
            }
            self.main_selected = n;
            return (MenuOutcome::Changed, vec![]);
        }
        if matches!(key, "enter" | "space") {
            return self.main_enter();
        }
        if matches!(key, "down" | "tab" | "mouse_scroll_down" | "j") {
            self.main_selected = (self.main_selected + 1) % 3;
            return (MenuOutcome::Changed, vec![]);
        }
        if matches!(key, "up" | "shift_tab" | "mouse_scroll_up" | "k") {
            self.main_selected = (self.main_selected + 2) % 3;
            return (MenuOutcome::Changed, vec![]);
        }
        (MenuOutcome::NoChange, vec![])
    }

    /// `MainEntering` (`:1255-1267`): Options/Help set the mask, FORCE
    /// `current` (the new bit is LOWER than Main=7, so highest-bit picking
    /// alone would re-pick Main — the forced `current` is what carries the
    /// transition), reset the target, `Switch`; Quit emits [`Action::Quit`].
    fn main_enter(&mut self) -> (MenuOutcome, Vec<Action>) {
        match self.main_selected {
            0 => {
                self.mask |= Menus::Options.bit();
                self.current = Some(Menus::Options);
                self.reset_for(Menus::Options);
                (MenuOutcome::Switch, vec![])
            }
            1 => {
                self.mask |= Menus::Help.bit();
                self.current = Some(Menus::Help);
                self.reset_for(Menus::Help);
                (MenuOutcome::Switch, vec![])
            }
            _ => (MenuOutcome::Closed, vec![Action::Quit]),
        }
    }

    /// `helpMenu` (`:1743-1794`): paging over [`HELP_TEXT`]; `height =
    /// min(term_h - 6, len + 3)`, `pages = ceil(len / (height - 3))`
    /// (`:1754-1757`); close keys → `Closed`; up/down family wraps.
    /// `term_h >= 20` is guaranteed post-coerce (smaller coerces to
    /// `SizeError`), so `height - 3 >= 1`; still guarded.
    fn menu_help(&mut self, key: &str, term_h: usize) -> (MenuOutcome, Vec<Action>) {
        let len = HELP_TEXT.len();
        let height = (term_h.saturating_sub(6)).min(len + 3);
        let per = height.saturating_sub(3).max(1);
        let pages = len.div_ceil(per).max(1);
        if matches!(
            key,
            "escape" | "q" | "h" | "backspace" | "space" | "enter" | "mouse_click"
        ) {
            return (MenuOutcome::Closed, vec![]);
        }
        if pages > 1
            && matches!(
                key,
                "down" | "j" | "page_down" | "tab" | "mouse_scroll_down"
            )
        {
            self.help_page = (self.help_page + 1) % pages;
            return (MenuOutcome::Changed, vec![]);
        }
        if pages > 1
            && matches!(
                key,
                "up" | "k" | "page_up" | "shift_tab" | "mouse_scroll_up"
            )
        {
            self.help_page = (self.help_page + pages - 1) % pages;
            return (MenuOutcome::Changed, vec![]);
        }
        (MenuOutcome::NoChange, vec![])
    }

    /// `reniceMenu` (`:1796-1889`): `selected_nice` -20..=19 with wrap
    /// (up/down ±1 wrap, left/right ±5 wrap), typed `nice_edit` string, enter
    /// → [`Action::SetPriority`]. `pid <= 0` still closes with no action
    /// (`:1825` gate). The `:1866-1871` draw-sync (`selected_nice =
    /// stoi(nice_edit)` whenever the edit is non-empty) is folded into the
    /// digit/backspace arms — Task 5 has no separate draw pass.
    fn menu_renice(&mut self, key: &str, pid: u64) -> (MenuOutcome, Vec<Action>) {
        if matches!(key, "escape" | "q") {
            return (MenuOutcome::Closed, vec![]);
        }
        if matches!(key, "enter" | "space") {
            if pid > 0 {
                let nice = if self.renice_edit.is_empty() {
                    self.renice_nice
                } else {
                    stoi_or_zero(&self.renice_edit)
                };
                return (
                    MenuOutcome::Closed,
                    vec![Action::SetPriority {
                        pid,
                        nice: nice as i64,
                    }],
                );
            }
            return (MenuOutcome::Closed, vec![]);
        }
        if key.len() == 1
            && (key.as_bytes()[0].is_ascii_digit() || (key == "-" && self.renice_edit.is_empty()))
        {
            self.renice_edit.push_str(key);
            self.renice_nice = stoi_or_zero(&self.renice_edit);
            return (MenuOutcome::Changed, vec![]);
        }
        if key == "backspace" && !self.renice_edit.is_empty() {
            self.renice_edit.pop();
            self.renice_nice = stoi_or_zero(&self.renice_edit);
            return (MenuOutcome::Changed, vec![]);
        }
        if matches!(key, "up" | "k") {
            self.renice_nice += 1;
            if self.renice_nice > 19 {
                self.renice_nice = -20;
            }
            self.renice_edit.clear();
            return (MenuOutcome::Changed, vec![]);
        }
        if matches!(key, "down" | "j") {
            self.renice_nice -= 1;
            if self.renice_nice < -20 {
                self.renice_nice = 19;
            }
            self.renice_edit.clear();
            return (MenuOutcome::Changed, vec![]);
        }
        if matches!(key, "left" | "h") {
            self.renice_nice -= 5;
            if self.renice_nice < -20 {
                self.renice_nice += 40;
            }
            self.renice_edit.clear();
            return (MenuOutcome::Changed, vec![]);
        }
        if matches!(key, "right" | "l") {
            self.renice_nice += 5;
            if self.renice_nice > 19 {
                self.renice_nice -= 40;
            }
            self.renice_edit.clear();
            return (MenuOutcome::Changed, vec![]);
        }
        (MenuOutcome::NoChange, vec![])
    }

    /// Options-menu input section (`:1373-1589`) over [`MenuState`] + Task 4
    /// primitives. DRAW SPLIT (Task 6 owns bytes): paging geometry needed for
    /// NAV is recomputed here from `term_h` (`height = min(term_h - 7,
    /// max*2+4)`, even-adjusted; `item_height`/`pages`/`select_max` via
    /// [`options`] helpers, `:1596-1602`); Task 6 reuses the same helpers for
    /// assembly. `selPred` is recomputed per call via
    /// [`classify_option`](options::classify_option) (`:1605-1621`:
    /// `isEditable = !isBrowsable && (isString || isInt)` — ints ARE editable).
    /// `mouse_click` zone hits are a Task 6/P3 duty (no geometry kept here):
    /// returns `NoChange` (documented deviation — C++ `:1417-1429`
    /// selects/edits on inside clicks and closes on outside ones).
    /// `theme_refresh`/`screen_redraw` flags are maintained from the returned
    /// actions (per the Task 4 store/sink split): `ApplyTheme` sets both
    /// (`:1720-1726` forces `screen_redraw`), `RecalcLayout`/`UpdateClock`
    /// set `screen_redraw`.
    #[allow(clippy::too_many_arguments)]
    fn menu_options(
        &mut self,
        key: &str,
        ctx: &MenuCtx,
        store: &mut OptionsStore,
        cfg: &mut Config,
        lists: &HashMap<String, Vec<String>>,
    ) -> (MenuOutcome, Vec<Action>) {
        let max_items = CATEGORIES.iter().map(|c| c.len()).max().unwrap_or(0);
        let mut height = (ctx.term_h.saturating_sub(7)).min(max_items * 2 + 4);
        // Even height (bitwise oddness test: is_multiple_of needs Rust 1.87+,
        // above the pinned nightly-2025-02-16).
        if (height & 1) == 1 {
            height = height.saturating_sub(1);
        }
        let tab = self.options.tab.min(CATEGORIES.len().saturating_sub(1));
        self.options.tab = tab;
        let n = CATEGORIES[tab].len();
        let ih = options::item_height_for(n, height);
        let pages = options::page_count(n, ih).max(1);
        self.options.page = self.options.page.min(pages - 1);
        let smax = options::select_max(n, ih, self.options.page);
        self.options.selected = self.options.selected.min(smax);

        // Warnings box (`:1373-1379`): any key feeds the shared Ok box;
        // confirm/cancel dismisses, else the box stays (Changed either way —
        // C++ falls through to draw with retval=Changed).
        if self.options.warnings.is_some() && !key.is_empty() {
            let mut b = MsgBox::new(BoxKind::Ok);
            use crate::msgbox::MsgReturn::*;
            if matches!(b.input(key), OkYes | NoEsc) {
                self.options.warnings = None;
            }
            return (MenuOutcome::Changed, vec![]);
        }

        // Editing arm (`:1380-1416`).
        if self.options.editing && !key.is_empty() {
            if matches!(key, "escape" | "mouse_click") {
                self.options.editor.clear();
                self.options.editing = false;
                return (MenuOutcome::Changed, vec![]);
            }
            if key == "enter" {
                let name = CATEGORIES[tab][ih * self.options.page + self.options.selected][0];
                let text = self.options.editor.text.clone();
                let res = commit_edit(name, &text, store, cfg);
                self.options.editor.clear();
                self.options.editing = false;
                match res {
                    Ok(acts) => {
                        self.options.warnings = None;
                        self.note_flags(&acts);
                        return (MenuOutcome::Changed, acts);
                    }
                    Err(e) => {
                        self.options.warnings = Some(e);
                        return (MenuOutcome::Changed, vec![]);
                    }
                }
            }
            if self.options.editor.command(key) {
                return (MenuOutcome::Changed, vec![]);
            }
            return (MenuOutcome::NoChange, vec![]);
        }

        if key == "mouse_click" {
            return (MenuOutcome::NoChange, vec![]);
        }

        let cur_name: Option<&str> = CATEGORIES[tab]
            .get(ih * self.options.page + self.options.selected)
            .map(|e| e[0]);
        let editable = cur_name.is_some_and(|nm| {
            matches!(
                options::classify_option(nm, store),
                OptKind::Editable | OptKind::Int | OptKind::Str
            )
        });

        // Begin edit (`:1431-1437`).
        if matches!(key, "enter" | "e" | "E") && editable {
            let name = cur_name.expect("editable implies a current option");
            let (text, numeric) = begin_edit_text(name, store);
            self.options.editor = TextEdit::new(text, numeric);
            self.options.editing = true;
            return (MenuOutcome::Changed, vec![]);
        }
        if matches!(key, "escape" | "q" | "o" | "backspace") {
            return (MenuOutcome::Closed, vec![]);
        }
        let vim = store.bools.get("vim_keys").copied().unwrap_or(false);
        if matches!(key, "down" | "mouse_scroll_down") || (vim && key == "j") {
            self.options.selected += 1;
            if self.options.selected > smax || self.options.selected >= ih {
                if self.options.page < pages - 1 {
                    self.options.page += 1;
                } else if pages > 1 {
                    self.options.page = 0;
                }
                self.options.selected = 0;
            }
            return (MenuOutcome::Changed, vec![]);
        }
        if matches!(key, "up" | "mouse_scroll_up") || (vim && key == "k") {
            if self.options.selected == 0 {
                if self.options.page > 0 {
                    self.options.page -= 1;
                } else if pages > 1 {
                    self.options.page = pages - 1;
                }
                self.options.selected = ih.saturating_sub(1);
            } else {
                self.options.selected -= 1;
            }
            return (MenuOutcome::Changed, vec![]);
        }
        if pages > 1 && key == "page_down" {
            self.options.page = (self.options.page + 1) % pages;
            self.options.selected = 0;
            return (MenuOutcome::Changed, vec![]);
        }
        if pages > 1 && key == "page_up" {
            self.options.page = (self.options.page + pages - 1) % pages;
            self.options.selected = 0;
            return (MenuOutcome::Changed, vec![]);
        }
        if key == "tab" {
            self.options.tab = (tab + 1) % CATEGORIES.len();
            self.options.page = 0;
            self.options.selected = 0;
            return (MenuOutcome::Changed, vec![]);
        }
        if key == "shift_tab" {
            self.options.tab = (tab + CATEGORIES.len() - 1) % CATEGORIES.len();
            self.options.page = 0;
            self.options.selected = 0;
            return (MenuOutcome::Changed, vec![]);
        }
        if (key.len() == 1 && matches!(key, "1" | "2" | "3" | "4" | "5" | "6"))
            || key.starts_with("select_cat_")
        {
            let idx = (key.as_bytes()[key.len() - 1] - b'0') as usize;
            if idx >= 1 && idx <= CATEGORIES.len() {
                self.options.tab = idx - 1;
                self.options.page = 0;
                self.options.selected = 0;
                return (MenuOutcome::Changed, vec![]);
            }
            return (MenuOutcome::NoChange, vec![]);
        }
        if matches!(key, "left" | "right") || (vim && matches!(key, "h" | "l")) {
            let dir: i8 = if matches!(key, "right" | "l") { 1 } else { -1 };
            if let Some(name) = cur_name {
                let list = lists.get(name).map(Vec::as_slice);
                // `list` borrows `lists`, not `store`/`cfg` — no aliasing.
                match flip_or_cycle(name, dir, store, cfg, list) {
                    Ok(acts) => {
                        self.note_flags(&acts);
                        return (MenuOutcome::Changed, acts);
                    }
                    Err(e) => {
                        self.options.warnings = Some(e);
                        return (MenuOutcome::Changed, vec![]);
                    }
                }
            }
            return (MenuOutcome::NoChange, vec![]);
        }
        (MenuOutcome::NoChange, vec![])
    }

    /// Maintain `theme_refresh`/`screen_redraw` from emitted actions (Task 4
    /// flag mapping): `ApplyTheme` ⇒ both (`:1720-1726` forces redraw);
    /// `RecalcLayout`/`UpdateClock` ⇒ `screen_redraw`.
    fn note_flags(&mut self, acts: &[Action]) {
        for a in acts {
            match a {
                Action::ApplyTheme { .. } => {
                    self.options.theme_refresh = true;
                    self.options.screen_redraw = true;
                }
                Action::RecalcLayout | Action::UpdateClock => {
                    self.options.screen_redraw = true;
                }
                _ => {}
            }
        }
    }

    /// Rebuild [`MenuSystem::overlay`] + [`MenuSystem::mouse_maps`] for the
    /// currently active menu (Task 6; see [`crate::overlay`]).
    ///
    /// Mirrors the C++ draw-on-`Changed` arms: call after `show`/`process`
    /// drove the LOGIC. `term_w`/`term_h` are the terminal size (golden:
    /// 100x30); `theme` is the complete theme map (golden: Default).
    /// `SizeError`/`Signal*` menus render nothing (out of golden scope).
    pub fn render_overlay(
        &mut self,
        store: &OptionsStore,
        lists: &HashMap<String, Vec<String>>,
        theme: &HashMap<String, String>,
        term_w: i64,
        term_h: i64,
    ) {
        let Some(cur) = self.current else {
            self.overlay.clear();
            self.mouse_maps.clear();
            return;
        };
        let (out, maps) = crate::overlay::render_into(
            cur,
            self.main_selected,
            self.options.tab,
            self.options.page,
            self.options.selected,
            self.help_page,
            self.signal_pid,
            &self.signal_pname,
            self.signal_to_send,
            i32::from(self.msg_box.selected),
            store,
            lists,
            theme,
            term_w,
            term_h,
        );
        self.overlay = out;
        self.mouse_maps = maps;
    }
}

/// `stoi` with the C++ try/catch folded in (`reniceMenu` `:1827-1830`,
/// `:1867-1870`): leading whitespace + optional sign accepted, digit-run
/// prefix parsed; no digits or `i32` overflow → 0.
fn stoi_or_zero(text: &str) -> i32 {
    let s = text.trim_start();
    let (neg, s) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let digits: String = s
        .bytes()
        .take_while(u8::is_ascii_digit)
        .map(char::from)
        .collect();
    if digits.is_empty() {
        return 0;
    }
    let v: i64 = digits.parse().unwrap_or(i64::MAX);
    let v = if neg { -v } else { v };
    if v < i32::MIN as i64 || v > i32::MAX as i64 {
        0
    } else {
        v as i32
    }
}

/// `signalReturn` texts (`:1191-1202`), transcribed verbatim (user-visible;
/// tests assert). Unknown errnos render `Unknown error! (errno: N)`.
pub fn signal_return_text(errno: i32) -> String {
    if errno == EINVAL {
        "Unsupported signal!".to_string()
    } else if errno == EPERM {
        "Insufficient permissions to send signal!".to_string()
    } else if errno == ESRCH {
        "Process not found!".to_string()
    } else {
        format!("Unknown error! (errno: {errno})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> MenuCtx {
        MenuCtx {
            term_w: 100,
            term_h: 30,
            target_pid: 1234,
            target_name: String::new(),
        }
    }

    fn store() -> OptionsStore {
        let mut s = OptionsStore::default();
        s.bools.insert("vim_keys".into(), false);
        s.bools.insert("theme_background".into(), false);
        s.ints.insert("update_ms".into(), 2000);
        s.strings.insert("color_theme".into(), "Default".into());
        s.strings.insert("custom_cpu_name".into(), String::new());
        s.strings.insert("log_level".into(), "INFO".into());
        s.strings.insert("proc_sorting".into(), "cpu lazy".into());
        s
    }

    fn lists() -> HashMap<String, Vec<String>> {
        let mut l = HashMap::new();
        l.insert(
            "color_theme".to_string(),
            vec!["Default".to_string(), "Gruvbox".to_string()],
        );
        l.insert(
            "log_level".to_string(),
            vec!["INFO".to_string(), "DEBUG".to_string()],
        );
        l.insert(
            "proc_sorting".to_string(),
            vec!["cpu lazy".to_string(), "pid".to_string()],
        );
        l
    }

    /// Open `menu` on a fresh system and consume the activation draw, so the
    /// following keys hit input arms (mirrors show() → process("") → redraw
    /// arm → Run{All}).
    fn open(
        sys: &mut MenuSystem,
        menu: Menus,
        signal: i32,
        ctx: &MenuCtx,
        store: &mut OptionsStore,
        cfg: &mut Config,
        lists: &HashMap<String, Vec<String>>,
    ) -> Vec<Action> {
        sys.show(menu, signal, ctx, store, cfg, lists)
    }

    fn proc(
        sys: &mut MenuSystem,
        key: &str,
        ctx: &MenuCtx,
        store: &mut OptionsStore,
        cfg: &mut Config,
        lists: &HashMap<String, Vec<String>>,
    ) -> Vec<Action> {
        sys.process(key, ctx, store, cfg, lists)
    }

    //? Outcome discriminants (:994-999).

    #[test]
    fn outcome_codes_match_cpp() {
        assert_eq!(MenuOutcome::NoChange as u8, 0);
        assert_eq!(MenuOutcome::Changed as u8, 1);
        assert_eq!(MenuOutcome::Closed as u8, 2);
        assert_eq!(MenuOutcome::Switch as u8, 3);
    }

    #[test]
    fn menu_bits_match_menufunc_order() {
        // Bit order MUST match menuFunc index order (:1893-1902).
        assert_eq!(Menus::SizeError as u8, 0);
        assert_eq!(Menus::SignalChoose as u8, 1);
        assert_eq!(Menus::SignalSend as u8, 2);
        assert_eq!(Menus::SignalReturn as u8, 3);
        assert_eq!(Menus::Options as u8, 4);
        assert_eq!(Menus::Help as u8, 5);
        assert_eq!(Menus::Renice as u8, 6);
        assert_eq!(Menus::Main as u8, 7);
    }

    //? Dispatcher: empty mask, min-size coerce, priority.

    #[test]
    fn empty_mask_deactivates_and_runs_all() {
        let mut sys = MenuSystem::default();
        let c = ctx();
        let mut s = store();
        let mut cfg = Config::new();
        let l = lists();
        // Empty path: active=false + Run{All,true,true} (:1906-1916).
        let acts = proc(&mut sys, "", &c, &mut s, &mut cfg, &l);
        assert!(!sys.active);
        assert!(sys.overlay.is_empty());
        assert_eq!(sys.current, None);
        assert!(acts.contains(&Action::PauseOutput { paused: false }));
        assert!(acts.contains(&Action::Run {
            target: RunTarget::All,
            no_update: true,
            redraw: true
        }));
    }

    #[test]
    fn signal_send_renders_confirmation_with_pid() {
        // `signalSend` redraw block (:1146-1158): confirmation box naming
        // the signal and target pid; button zones ride along for mouse.
        use crate::overlay::OverlayTheme;
        let mut sys = MenuSystem::default();
        let mut s = store();
        let mut cfg = Config::new();
        let l = lists();
        let c = MenuCtx {
            term_w: 120,
            term_h: 40,
            target_pid: 74310,
            target_name: "opencode".to_string(),
        };
        let _ = open(&mut sys, Menus::SignalSend, 15, &c, &mut s, &mut cfg, &l);
        assert!(sys.active);
        assert_eq!(sys.signal_pid, 74310);
        sys.render_overlay(&s, &l, &btop_config::theme::default_theme(), 120, 40);
        assert!(
            sys.overlay.contains("Send signal:"),
            "{}",
            &sys.overlay[..200.min(sys.overlay.len())]
        );
        assert!(sys.overlay.contains("74310"), "pid in body");
        assert!(sys.overlay.contains("opencode"), "name in body");
        assert!(sys.overlay.contains("SIGTERM"), "signal name");
        assert!(
            sys.mouse_maps.iter().any(|m| m.action == "button1"),
            "Yes button clickable"
        );
        assert!(
            sys.mouse_maps.iter().any(|m| m.action == "button2"),
            "No button clickable"
        );
    }

    #[test]
    fn show_activates_and_first_draw_runs_all() {
        let mut sys = MenuSystem::default();
        let c = ctx();
        let mut s = store();
        let mut cfg = Config::new();
        let l = lists();
        let acts = open(&mut sys, Menus::Main, -1, &c, &mut s, &mut cfg, &l);
        assert!(sys.active);
        assert_eq!(sys.current, Some(Menus::Main));
        assert_eq!(sys.main_selected, 0);
        assert!(acts.contains(&Action::Run {
            target: RunTarget::All,
            no_update: true,
            redraw: true
        }));
    }

    #[test]
    fn min_size_coerces_big_family_to_size_error() {
        // Main needs 80x24 (:1922-1924); 70-wide coerces.
        let mut sys = MenuSystem::default();
        let mut s = store();
        let mut cfg = Config::new();
        let l = lists();
        let small = MenuCtx {
            term_w: 70,
            term_h: 30,
            target_pid: 1,
            target_name: String::new(),
        };
        open(&mut sys, Menus::Main, -1, &small, &mut s, &mut cfg, &l);
        assert_eq!(sys.current, Some(Menus::SizeError));
        assert_eq!(sys.mask, Menus::SizeError.bit());
    }

    #[test]
    fn tiny_terminal_coerces_any_menu() {
        // w<50 OR h<20 coerces even non-big menus (:1924).
        let mut sys = MenuSystem::default();
        let mut s = store();
        let mut cfg = Config::new();
        let l = lists();
        let tiny = MenuCtx {
            term_w: 40,
            term_h: 30,
            target_pid: 1,
            target_name: String::new(),
        };
        open(&mut sys, Menus::Renice, -1, &tiny, &mut s, &mut cfg, &l);
        assert_eq!(sys.current, Some(Menus::SizeError));
    }

    #[test]
    fn exact_thresholds_do_not_coerce() {
        let mut sys = MenuSystem::default();
        let mut s = store();
        let mut cfg = Config::new();
        let l = lists();
        // 80x24 passes the big-family gate; 50x20 passes the small gate.
        let edge = MenuCtx {
            term_w: 80,
            term_h: 24,
            target_pid: 1,
            target_name: String::new(),
        };
        open(&mut sys, Menus::Main, -1, &edge, &mut s, &mut cfg, &l);
        assert_eq!(sys.current, Some(Menus::Main));
        let mut sys2 = MenuSystem::default();
        let edge2 = MenuCtx {
            term_w: 50,
            term_h: 20,
            target_pid: 1,
            target_name: String::new(),
        };
        open(&mut sys2, Menus::Renice, -1, &edge2, &mut s, &mut cfg, &l);
        assert_eq!(sys2.current, Some(Menus::Renice));
    }

    #[test]
    fn highest_set_bit_wins() {
        let mut sys = MenuSystem::default();
        let c = ctx();
        let mut s = store();
        let mut cfg = Config::new();
        let l = lists();
        sys.mask = Menus::Options.bit() | Menus::Help.bit();
        let acts = proc(&mut sys, "", &c, &mut s, &mut cfg, &l);
        // Help(5) > Options(4).
        assert_eq!(sys.current, Some(Menus::Help));
        assert!(acts.contains(&Action::Run {
            target: RunTarget::All,
            no_update: true,
            redraw: true
        }));
    }

    #[test]
    fn changed_runs_overlay() {
        let mut sys = MenuSystem::default();
        let c = ctx();
        let mut s = store();
        let mut cfg = Config::new();
        let l = lists();
        open(&mut sys, Menus::Main, -1, &c, &mut s, &mut cfg, &l);
        // down = Changed (redraw already consumed) → Run{Overlay,false,false}.
        let acts = proc(&mut sys, "down", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.main_selected, 1);
        assert!(acts.contains(&Action::Run {
            target: RunTarget::Overlay,
            no_update: false,
            redraw: false
        }));
    }

    #[test]
    fn close_recurses_into_empty_with_all_run() {
        let mut sys = MenuSystem::default();
        let c = ctx();
        let mut s = store();
        let mut cfg = Config::new();
        let l = lists();
        open(&mut sys, Menus::Main, -1, &c, &mut s, &mut cfg, &l);
        // escape → Closed → bit cleared → recurse hits empty mask.
        let acts = proc(&mut sys, "escape", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.mask, 0);
        assert!(!sys.active);
        assert!(acts.contains(&Action::PauseOutput { paused: false }));
        assert!(acts.contains(&Action::Run {
            target: RunTarget::All,
            no_update: true,
            redraw: true
        }));
    }

    //? signalChoose.

    #[test]
    fn signal_digits_backspace_and_kill() {
        let mut sys = MenuSystem::default();
        let c = ctx();
        let mut s = store();
        let mut cfg = Config::new();
        let l = lists();
        open(&mut sys, Menus::SignalChoose, -1, &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.signal_selected, -1);
        proc(&mut sys, "1", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.signal_selected, 1);
        proc(&mut sys, "5", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.signal_selected, 15);
        proc(&mut sys, "backspace", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.signal_selected, 1);
        proc(&mut sys, "backspace", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.signal_selected, -1);
        // enter with nothing selected → NoChange, no Kill.
        let acts = proc(&mut sys, "enter", &c, &mut s, &mut cfg, &l);
        assert!(!acts.iter().any(|a| matches!(a, Action::Kill { .. })));
        proc(&mut sys, "9", &c, &mut s, &mut cfg, &l);
        let acts = proc(&mut sys, "enter", &c, &mut s, &mut cfg, &l);
        assert!(acts.contains(&Action::Kill { pid: 1234, sig: 9 }));
        // Kill closes the menu; SignalReturn NOT set on the success path.
        assert_eq!(sys.mask & Menus::SignalReturn.bit(), 0);
    }

    #[test]
    fn signal_grid_nav_skips_16() {
        let mut sys = MenuSystem::default();
        let c = ctx();
        let mut s = store();
        let mut cfg = Config::new();
        let l = lists();
        open(&mut sys, Menus::SignalChoose, -1, &c, &mut s, &mut cfg, &l);
        // Seed 15 via digits, right skips 16 → 17.
        proc(&mut sys, "1", &c, &mut s, &mut cfg, &l);
        proc(&mut sys, "5", &c, &mut s, &mut cfg, &l);
        proc(&mut sys, "right", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.signal_selected, 17);
        proc(&mut sys, "left", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.signal_selected, 15);
        // down from 11 → 17 (5+1 skip), up from 17 → 11... use 12: down → 18?
        // 12 < 16 → +5 = 17, no skip needed (17 >= 16 with offset → +1 = 18)?
        // C++: v = 12+5 = 17; 17 >= 16 && offset → v = 18.
        sys.signal_selected = 12;
        proc(&mut sys, "down", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.signal_selected, 18);
        // up from 18: > 16 → v = 13, 13 <= 16 && offset → 12.
        proc(&mut sys, "up", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.signal_selected, 12);
        // 1 → up wraps to 31; 31 → down wraps to 1.
        sys.signal_selected = 1;
        proc(&mut sys, "up", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.signal_selected, 31);
        proc(&mut sys, "down", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.signal_selected, 1);
    }

    #[test]
    fn signal_zero_pid_takes_esrch_path() {
        // pid < 1: no Kill; ESRCH stored, SignalReturn bit set, menu closes
        // (:1034-1037 + MenuClosing → Closed).
        let mut sys = MenuSystem::default();
        let mut s = store();
        let mut cfg = Config::new();
        let l = lists();
        let dead = MenuCtx {
            term_w: 100,
            term_h: 30,
            target_pid: 0,
            target_name: String::new(),
        };
        open(
            &mut sys,
            Menus::SignalChoose,
            -1,
            &dead,
            &mut s,
            &mut cfg,
            &l,
        );
        proc(&mut sys, "9", &dead, &mut s, &mut cfg, &l);
        let acts = proc(&mut sys, "enter", &dead, &mut s, &mut cfg, &l);
        assert!(!acts.iter().any(|a| matches!(a, Action::Kill { .. })));
        assert_eq!(sys.kill_errno, ESRCH);
        // Closed recursed into the pending SignalReturn menu.
        assert_eq!(sys.current, Some(Menus::SignalReturn));
        assert_eq!(signal_return_text(sys.kill_errno), "Process not found!");
    }

    #[test]
    fn signal_button_keys_select_then_confirm() {
        let mut sys = MenuSystem::default();
        let c = ctx();
        let mut s = store();
        let mut cfg = Config::new();
        let l = lists();
        open(&mut sys, Menus::SignalChoose, -1, &c, &mut s, &mut cfg, &l);
        proc(&mut sys, "button_9", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.signal_selected, 9);
        let acts = proc(&mut sys, "button_9", &c, &mut s, &mut cfg, &l);
        assert!(acts.contains(&Action::Kill { pid: 1234, sig: 9 }));
    }

    #[test]
    fn signal_escape_closes() {
        let mut sys = MenuSystem::default();
        let c = ctx();
        let mut s = store();
        let mut cfg = Config::new();
        let l = lists();
        open(&mut sys, Menus::SignalChoose, -1, &c, &mut s, &mut cfg, &l);
        let acts = proc(&mut sys, "q", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.mask, 0);
        assert!(!acts.iter().any(|a| matches!(a, Action::Kill { .. })));
    }

    //? signalSend / signalReturn / sizeError.

    #[test]
    fn signal_send_yes_kills_no_closes() {
        let mut sys = MenuSystem::default();
        let c = ctx();
        let mut s = store();
        let mut cfg = Config::new();
        let l = lists();
        open(&mut sys, Menus::SignalSend, 15, &c, &mut s, &mut cfg, &l);
        assert!(!sys.msg_box.selected);
        let acts = proc(&mut sys, "enter", &c, &mut s, &mut cfg, &l);
        assert!(acts.contains(&Action::Kill { pid: 1234, sig: 15 }));

        let mut sys = MenuSystem::default();
        open(&mut sys, Menus::SignalSend, 9, &c, &mut s, &mut cfg, &l);
        let acts = proc(&mut sys, "n", &c, &mut s, &mut cfg, &l);
        assert!(!acts.iter().any(|a| matches!(a, Action::Kill { .. })));
        assert_eq!(sys.mask, 0);
    }

    #[test]
    fn signal_send_toggle_is_changed() {
        let mut sys = MenuSystem::default();
        let c = ctx();
        let mut s = store();
        let mut cfg = Config::new();
        let l = lists();
        open(&mut sys, Menus::SignalSend, 15, &c, &mut s, &mut cfg, &l);
        let acts = proc(&mut sys, "right", &c, &mut s, &mut cfg, &l);
        assert!(sys.msg_box.selected);
        assert!(acts.contains(&Action::Run {
            target: RunTarget::Overlay,
            no_update: false,
            redraw: false
        }));
        // enter now hits No → Closed, no Kill.
        let acts = proc(&mut sys, "enter", &c, &mut s, &mut cfg, &l);
        assert!(!acts.iter().any(|a| matches!(a, Action::Kill { .. })));
    }

    #[test]
    fn signal_return_texts_verbatim() {
        // Transcribed verbatim from :1191-1202 (user-visible).
        assert_eq!(signal_return_text(EINVAL), "Unsupported signal!");
        assert_eq!(
            signal_return_text(EPERM),
            "Insufficient permissions to send signal!"
        );
        assert_eq!(signal_return_text(ESRCH), "Process not found!");
        assert_eq!(signal_return_text(99), "Unknown error! (errno: 99)");
    }

    #[test]
    fn signal_return_confirm_closes() {
        let mut sys = MenuSystem::default();
        let c = ctx();
        let mut s = store();
        let mut cfg = Config::new();
        let l = lists();
        sys.mask = Menus::SignalReturn.bit();
        sys.kill_errno = EPERM;
        proc(&mut sys, "", &c, &mut s, &mut cfg, &l);
        let acts = proc(&mut sys, "enter", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.mask, 0);
        assert!(!acts.iter().any(|a| matches!(a, Action::Kill { .. })));
    }

    #[test]
    fn size_error_confirm_closes() {
        let mut sys = MenuSystem::default();
        let c = ctx();
        let mut s = store();
        let mut cfg = Config::new();
        let l = lists();
        open(&mut sys, Menus::SizeError, -1, &c, &mut s, &mut cfg, &l);
        let acts = proc(&mut sys, "o", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.mask, 0);
        assert!(acts.contains(&Action::Run {
            target: RunTarget::All,
            no_update: true,
            redraw: true
        }));
    }

    //? mainMenu.

    #[test]
    fn main_nav_wraps() {
        let mut sys = MenuSystem::default();
        let c = ctx();
        let mut s = store();
        let mut cfg = Config::new();
        let l = lists();
        open(&mut sys, Menus::Main, -1, &c, &mut s, &mut cfg, &l);
        proc(&mut sys, "down", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.main_selected, 1);
        proc(&mut sys, "down", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.main_selected, 2);
        proc(&mut sys, "down", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.main_selected, 0);
        proc(&mut sys, "up", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.main_selected, 2);
    }

    #[test]
    fn main_enter_options_switches() {
        let mut sys = MenuSystem::default();
        let c = ctx();
        let mut s = store();
        let mut cfg = Config::new();
        let l = lists();
        open(&mut sys, Menus::Main, -1, &c, &mut s, &mut cfg, &l);
        // selected=0 (Options) → Switch with forced current + both bits set.
        let acts = proc(&mut sys, "enter", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.current, Some(Menus::Options));
        assert_ne!(sys.mask & Menus::Options.bit(), 0);
        assert_ne!(sys.mask & Menus::Main.bit(), 0);
        assert!(acts.contains(&Action::PauseOutput { paused: false }));
        // Recurse drew Options (redraw arm).
        assert!(acts.contains(&Action::Run {
            target: RunTarget::All,
            no_update: true,
            redraw: true
        }));
        // Options state freshly reset.
        assert_eq!((sys.options.tab, sys.options.selected), (0, 0));
    }

    #[test]
    fn main_enter_help_switches() {
        let mut sys = MenuSystem::default();
        let c = ctx();
        let mut s = store();
        let mut cfg = Config::new();
        let l = lists();
        open(&mut sys, Menus::Main, -1, &c, &mut s, &mut cfg, &l);
        proc(&mut sys, "down", &c, &mut s, &mut cfg, &l);
        let _ = proc(&mut sys, "enter", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.current, Some(Menus::Help));
        assert_eq!(sys.help_page, 0);
    }

    #[test]
    fn main_enter_quit_reuses_quit_action() {
        // C++ clean_quit(0) (:1266) → reused Action::Quit for the P3 sink.
        let mut sys = MenuSystem::default();
        let c = ctx();
        let mut s = store();
        let mut cfg = Config::new();
        let l = lists();
        open(&mut sys, Menus::Main, -1, &c, &mut s, &mut cfg, &l);
        proc(&mut sys, "button_2", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.main_selected, 2);
        let acts = proc(&mut sys, "enter", &c, &mut s, &mut cfg, &l);
        assert!(acts.contains(&Action::Quit));
    }

    //? helpMenu.

    #[test]
    fn help_paging_wraps() {
        let mut sys = MenuSystem::default();
        let c = ctx();
        let mut s = store();
        let mut cfg = Config::new();
        let l = lists();
        open(&mut sys, Menus::Help, -1, &c, &mut s, &mut cfg, &l);
        // 30-high term: height = min(24, 49) = 24, per = 21, pages = 3.
        proc(&mut sys, "down", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.help_page, 1);
        proc(&mut sys, "down", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.help_page, 2);
        proc(&mut sys, "down", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.help_page, 0);
        proc(&mut sys, "up", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.help_page, 2);
        let acts = proc(&mut sys, "q", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.mask, 0);
        assert!(!acts.contains(&Action::Quit));
    }

    //? reniceMenu.

    #[test]
    fn renice_steps_and_wraps() {
        let mut sys = MenuSystem::default();
        let c = ctx();
        let mut s = store();
        let mut cfg = Config::new();
        let l = lists();
        open(&mut sys, Menus::Renice, -1, &c, &mut s, &mut cfg, &l);
        proc(&mut sys, "up", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.renice_nice, 1);
        proc(&mut sys, "right", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.renice_nice, 6);
        proc(&mut sys, "left", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.renice_nice, 1);
        proc(&mut sys, "down", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.renice_nice, 0);
        // Edges: 19 +up → -20; -20 +down → 19.
        sys.renice_nice = 19;
        proc(&mut sys, "up", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.renice_nice, -20);
        proc(&mut sys, "down", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.renice_nice, 19);
        // ±5 wraps: 17 +right → -18; -18 +left → 17.
        sys.renice_nice = 17;
        proc(&mut sys, "right", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.renice_nice, -18);
        proc(&mut sys, "left", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.renice_nice, 17);
    }

    #[test]
    fn renice_typing_and_set_priority() {
        let mut sys = MenuSystem::default();
        let c = ctx();
        let mut s = store();
        let mut cfg = Config::new();
        let l = lists();
        open(&mut sys, Menus::Renice, -1, &c, &mut s, &mut cfg, &l);
        proc(&mut sys, "1", &c, &mut s, &mut cfg, &l);
        proc(&mut sys, "0", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.renice_edit, "10");
        // Draw-sync folded into input (:1866-1871).
        assert_eq!(sys.renice_nice, 10);
        let acts = proc(&mut sys, "enter", &c, &mut s, &mut cfg, &l);
        assert!(acts.contains(&Action::SetPriority {
            pid: 1234,
            nice: 10
        }));
        assert_eq!(sys.mask, 0);
    }

    #[test]
    fn renice_negative_typing_and_garbage_zero() {
        let mut sys = MenuSystem::default();
        let c = ctx();
        let mut s = store();
        let mut cfg = Config::new();
        let l = lists();
        open(&mut sys, Menus::Renice, -1, &c, &mut s, &mut cfg, &l);
        proc(&mut sys, "-", &c, &mut s, &mut cfg, &l);
        proc(&mut sys, "5", &c, &mut s, &mut cfg, &l);
        let acts = proc(&mut sys, "enter", &c, &mut s, &mut cfg, &l);
        assert!(acts.contains(&Action::SetPriority {
            pid: 1234,
            nice: -5
        }));
        // stoi failure → 0 (:1827-1830 catch): "-" alone parses to 0.
        let mut sys = MenuSystem::default();
        open(&mut sys, Menus::Renice, -1, &c, &mut s, &mut cfg, &l);
        proc(&mut sys, "-", &c, &mut s, &mut cfg, &l);
        let acts = proc(&mut sys, "enter", &c, &mut s, &mut cfg, &l);
        assert!(acts.contains(&Action::SetPriority { pid: 1234, nice: 0 }));
    }

    #[test]
    fn renice_zero_pid_closes_silently() {
        // :1825 gate — closes with no action.
        let mut sys = MenuSystem::default();
        let mut s = store();
        let mut cfg = Config::new();
        let l = lists();
        let dead = MenuCtx {
            term_w: 100,
            term_h: 30,
            target_pid: 0,
            target_name: String::new(),
        };
        open(&mut sys, Menus::Renice, -1, &dead, &mut s, &mut cfg, &l);
        let acts = proc(&mut sys, "enter", &dead, &mut s, &mut cfg, &l);
        assert!(!acts.iter().any(|a| matches!(a, Action::SetPriority { .. })));
        assert_eq!(sys.mask, 0);
    }

    #[test]
    fn stoi_or_zero_edges() {
        assert_eq!(stoi_or_zero(""), 0);
        assert_eq!(stoi_or_zero("-"), 0);
        assert_eq!(stoi_or_zero("  -12x"), -12);
        assert_eq!(stoi_or_zero("+7"), 7);
        // i32 overflow → 0 (C++ out_of_range → catch).
        assert_eq!(stoi_or_zero("9999999999"), 0);
        assert_eq!(stoi_or_zero("-9999999999"), 0);
    }

    //? optionsMenu (input section over Task 4 primitives).

    #[test]
    fn options_nav_tab_and_close() {
        let mut sys = MenuSystem::default();
        let c = ctx();
        let mut s = store();
        let mut cfg = Config::new();
        let l = lists();
        open(&mut sys, Menus::Options, -1, &c, &mut s, &mut cfg, &l);
        proc(&mut sys, "down", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.options.selected, 1);
        proc(&mut sys, "tab", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.options.tab, 1);
        assert_eq!(sys.options.selected, 0);
        proc(&mut sys, "2", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.options.tab, 1);
        let acts = proc(&mut sys, "o", &c, &mut s, &mut cfg, &l);
        assert_eq!(sys.mask, 0);
        assert!(!acts.contains(&Action::Quit));
    }

    #[test]
    fn options_flip_bool_sets_flag_and_action() {
        let mut sys = MenuSystem::default();
        let c = ctx();
        let mut s = store();
        let mut cfg = Config::new();
        let l = lists();
        open(&mut sys, Menus::Options, -1, &c, &mut s, &mut cfg, &l);
        // Row 0 = color_theme (browsable); down → theme_background (bool).
        proc(&mut sys, "down", &c, &mut s, &mut cfg, &l);
        let acts = proc(&mut sys, "right", &c, &mut s, &mut cfg, &l);
        assert_eq!(s.bools.get("theme_background"), Some(&true));
        assert!(sys.options.screen_redraw);
        assert!(acts.iter().any(|a| matches!(a, Action::ApplyTheme { .. })));
    }

    #[test]
    fn options_edit_commit_and_warnings() {
        let mut sys = MenuSystem::default();
        let c = ctx();
        let mut s = store();
        let mut cfg = Config::new();
        let l = lists();
        open(&mut sys, Menus::Options, -1, &c, &mut s, &mut cfg, &l);
        // Move to custom_cpu_name: general tab order is color_theme(0),
        // theme_background(1), truecolor(2), force_tty(3), vim_keys(4),
        // disable_mouse(5), disable_presets(6), presets(7), shown_boxes(8),
        // update_ms(9), rounded_corners(10), terminal_sync(11),
        // graph_symbol(12), clock_format(13), base_10_sizes(14),
        // background_update(15), show_battery(16), selected_battery(17),
        // show_battery_watts(18), log_level(19), save_config_on_exit(20).
        // Page 1 (item_height 13 at h=30: height=min(23,46)=22, ih=9;
        // pages=ceil(21/9)=3): rows 0..8 visible first.
        for _ in 0..8 {
            proc(&mut sys, "down", &c, &mut s, &mut cfg, &l);
        }
        // selected=8 → shown_boxes (editable): begin edit, type, commit.
        let acts = proc(&mut sys, "enter", &c, &mut s, &mut cfg, &l);
        assert!(sys.options.editing);
        assert!(acts.is_empty() || !acts.contains(&Action::Quit));
        for k in ["c", "p", "u"] {
            proc(&mut sys, k, &c, &mut s, &mut cfg, &l);
        }
        let acts = proc(&mut sys, "enter", &c, &mut s, &mut cfg, &l);
        assert!(!sys.options.editing);
        assert!(acts.contains(&Action::RecalcLayout));
        // Invalid int commit → warnings, dismissed by any key.
        sys.options.tab = 0;
        sys.options.page = 1;
        sys.options.selected = 0; // update_ms on page 1 (index 9).
        let _ = proc(&mut sys, "enter", &c, &mut s, &mut cfg, &l);
        assert!(sys.options.editing);
        sys.options.editor = TextEdit::new("99".into(), true);
        let _ = proc(&mut sys, "enter", &c, &mut s, &mut cfg, &l);
        assert!(sys.options.warnings.is_some());
        let _ = proc(&mut sys, "enter", &c, &mut s, &mut cfg, &l);
        assert!(sys.options.warnings.is_none());
    }

    //? MenuKind::SignalReturn is showable by the sink on async kill failure.

    #[test]
    fn signal_return_kind_constructible() {
        let m = btop_input::actions::MenuKind::SignalReturn;
        assert_eq!(
            btop_input::actions::Action::ShowMenu { menu: m },
            btop_input::actions::Action::ShowMenu {
                menu: btop_input::actions::MenuKind::SignalReturn
            }
        );
    }
}
