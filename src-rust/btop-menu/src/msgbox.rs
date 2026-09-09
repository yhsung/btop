//! Message-box input state machine, transcribed branch-for-branch from
//! `msgBox::input` (`src/btop_menu.cpp:950-981`).
//!
//! Source semantics (`src/btop_menu.hpp:44-77`, `src/btop_menu.cpp:909-931`):
//! - `boxtype`: 0 = `OK` button | 1 = YES and NO with YES selected |
//!   2 = same as 1 but with NO selected (constructor sets `selected = 1`
//!   only when `boxtype == 2`; see `:921`).
//! - `selected`: focused button — `0` = left/first button (Ok/Yes),
//!   `1` = right button (No). Modelled here as `bool`
//!   (`false` = first button, `true` = No button).
//! - Return enum (`msgReturn`): `Invalid = 0`, `Ok_Yes = 1`, `No_Esc = 2`,
//!   `Select = 3`. `enter`/`space` return `selected + 1`, i.e. `OkYes`
//!   when the first button is focused, `NoEsc` when No is focused.
//! - Key order matters (transcribed top-to-bottom from `:951-980`):
//!   empty -> `Invalid`; `escape`/`backspace`/`q`/`button2` -> `NoEsc`
//!   (any box kind, even `Ok`); `button1`, or `o`/`O` on an `Ok` box ->
//!   `OkYes`; `enter`/`space` -> `selected + 1`; anything else on an `Ok`
//!   box -> `Invalid` (so `y`/`n` and `left`/`right`/`tab` toggles only
//!   exist on two-button boxes); `y`/`Y` -> `OkYes`; `n`/`N` -> `NoEsc`;
//!   `right`/`tab` and `left`/`shift_tab` toggle `selected` and return
//!   `Select`; anything else -> `Invalid`.

/// Box kind. Maps to C++ `boxtype`: `Ok = 0`, `YesNo = 1`, `NoYes = 2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BoxKind {
    /// Single `Ok` button (`boxtype == 0`).
    #[default]
    Ok,
    /// `Yes` and `No` with `Yes` selected (`boxtype == 1`).
    YesNo,
    /// `Yes` and `No` with `No` selected (`boxtype == 2`).
    NoYes,
}

/// Result of feeding a key to [`MsgBox::input`]. Maps to C++ `msgReturn`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgReturn {
    /// No action (`Invalid = 0`).
    Invalid,
    /// Confirm (`Ok_Yes = 1`).
    OkYes,
    /// Cancel (`No_Esc = 2`).
    NoEsc,
    /// Focus moved between buttons (`Select = 3`).
    Select,
}

/// Message-box focus state. `selected == false` focuses the first
/// (Ok/Yes) button, `selected == true` focuses the No button.
#[derive(Debug, Default)]
pub struct MsgBox {
    pub kind: BoxKind,
    pub selected: bool,
}

impl MsgBox {
    /// Create a box of `kind`. Like the C++ constructor (`:921`), a
    /// `NoYes` box starts with the No button focused; all other kinds
    /// start on the first button.
    pub fn new(kind: BoxKind) -> Self {
        Self {
            kind,
            selected: matches!(kind, BoxKind::NoYes),
        }
    }

    /// Process one input `key` (`btop_menu.cpp:950-981`). Key names follow
    /// the `Input` key mapping (`"escape"`, `"enter"`, `"space"`, `"tab"`,
    /// `"shift_tab"`, `"left"`, `"right"`, `"backspace"`, `"button1"`,
    /// `"button2"`, single characters, ...).
    pub fn input(&mut self, key: &str) -> MsgReturn {
        if key.is_empty() {
            return MsgReturn::Invalid;
        }
        if matches!(key, "escape" | "backspace" | "q" | "button2") {
            return MsgReturn::NoEsc;
        } else if key == "button1" || (self.kind == BoxKind::Ok && key.eq_ignore_ascii_case("o")) {
            return MsgReturn::OkYes;
        } else if matches!(key, "enter" | "space") {
            // C++ returns `selected + 1`, i.e. Invalid(0)+1 = Ok_Yes,
            // Ok_Yes(1)+1 = No_Esc for selected in {0, 1}.
            return if self.selected {
                MsgReturn::NoEsc
            } else {
                MsgReturn::OkYes
            };
        } else if self.kind == BoxKind::Ok {
            return MsgReturn::Invalid;
        } else if key.eq_ignore_ascii_case("y") {
            return MsgReturn::OkYes;
        } else if key.eq_ignore_ascii_case("n") {
            return MsgReturn::NoEsc;
        } else if matches!(key, "right" | "tab") {
            // C++: `if (++selected > 1) selected = 0;` — a 0 <-> 1 toggle.
            self.selected = !self.selected;
            return MsgReturn::Select;
        } else if matches!(key, "left" | "shift_tab") {
            // C++: `if (--selected < 0) selected = 1;` — same toggle.
            self.selected = !self.selected;
            return MsgReturn::Select;
        }
        MsgReturn::Invalid
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_selects_first_button_except_no_yes() {
        assert!(!MsgBox::new(BoxKind::Ok).selected);
        assert!(!MsgBox::new(BoxKind::YesNo).selected);
        assert!(MsgBox::new(BoxKind::NoYes).selected);
    }

    #[test]
    fn empty_key_is_invalid() {
        assert_eq!(MsgBox::new(BoxKind::YesNo).input(""), MsgReturn::Invalid);
        assert_eq!(MsgBox::new(BoxKind::Ok).input(""), MsgReturn::Invalid);
    }

    #[test]
    fn cancel_keys_return_no_esc_on_any_kind() {
        for kind in [BoxKind::Ok, BoxKind::YesNo, BoxKind::NoYes] {
            for key in ["escape", "backspace", "q", "button2"] {
                assert_eq!(
                    MsgBox::new(kind).input(key),
                    MsgReturn::NoEsc,
                    "{kind:?} {key}"
                );
            }
        }
    }

    #[test]
    fn button1_confirms_on_any_kind() {
        for kind in [BoxKind::Ok, BoxKind::YesNo, BoxKind::NoYes] {
            assert_eq!(MsgBox::new(kind).input("button1"), MsgReturn::OkYes);
        }
    }

    #[test]
    fn ok_box_accepts_o_key() {
        let mut b = MsgBox::new(BoxKind::Ok);
        assert_eq!(b.input("o"), MsgReturn::OkYes);
        assert_eq!(b.input("O"), MsgReturn::OkYes);
    }

    #[test]
    fn enter_and_space_follow_focus() {
        let mut yes = MsgBox::new(BoxKind::YesNo);
        assert_eq!(yes.input("enter"), MsgReturn::OkYes);
        assert_eq!(yes.input("space"), MsgReturn::OkYes);
        let mut no = MsgBox::new(BoxKind::NoYes);
        assert_eq!(no.input("enter"), MsgReturn::NoEsc);
        assert_eq!(no.input("space"), MsgReturn::NoEsc);
        let mut ok = MsgBox::new(BoxKind::Ok);
        assert_eq!(ok.input("enter"), MsgReturn::OkYes);
    }

    #[test]
    fn y_and_n_shortcuts_only_on_two_button_boxes() {
        for key in ["y", "Y"] {
            assert_eq!(MsgBox::new(BoxKind::YesNo).input(key), MsgReturn::OkYes);
            assert_eq!(MsgBox::new(BoxKind::NoYes).input(key), MsgReturn::OkYes);
            assert_eq!(MsgBox::new(BoxKind::Ok).input(key), MsgReturn::Invalid);
        }
        for key in ["n", "N"] {
            assert_eq!(MsgBox::new(BoxKind::YesNo).input(key), MsgReturn::NoEsc);
            assert_eq!(MsgBox::new(BoxKind::NoYes).input(key), MsgReturn::NoEsc);
            assert_eq!(MsgBox::new(BoxKind::Ok).input(key), MsgReturn::Invalid);
        }
        // `o` is an Ok-box-only shortcut: Invalid on two-button boxes.
        assert_eq!(MsgBox::new(BoxKind::YesNo).input("o"), MsgReturn::Invalid);
    }

    #[test]
    fn toggle_keys_move_focus_and_return_select() {
        let mut b = MsgBox::new(BoxKind::YesNo);
        assert_eq!(b.input("right"), MsgReturn::Select);
        assert!(b.selected);
        assert_eq!(b.input("enter"), MsgReturn::NoEsc);
        assert_eq!(b.input("tab"), MsgReturn::Select);
        assert!(!b.selected);
        assert_eq!(b.input("left"), MsgReturn::Select);
        assert!(b.selected);
        assert_eq!(b.input("shift_tab"), MsgReturn::Select);
        assert!(!b.selected);
    }

    #[test]
    fn toggle_keys_wrap_around() {
        let mut b = MsgBox::new(BoxKind::NoYes);
        assert_eq!(b.input("right"), MsgReturn::Select);
        assert!(!b.selected);
        assert_eq!(b.input("left"), MsgReturn::Select);
        assert!(b.selected);
    }

    #[test]
    fn ok_box_has_no_toggle_keys() {
        let mut b = MsgBox::new(BoxKind::Ok);
        for key in ["right", "tab", "left", "shift_tab"] {
            assert_eq!(b.input(key), MsgReturn::Invalid);
        }
        assert!(!b.selected);
    }

    #[test]
    fn unknown_keys_are_invalid() {
        for kind in [BoxKind::Ok, BoxKind::YesNo, BoxKind::NoYes] {
            for key in ["x", "F1", "up", "mouse", "button3"] {
                assert_eq!(
                    MsgBox::new(kind).input(key),
                    MsgReturn::Invalid,
                    "{kind:?} {key}"
                );
            }
        }
    }
}
