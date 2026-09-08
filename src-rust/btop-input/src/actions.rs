//! Actions mirroring every Input::process() branch (btop_input.cpp:214-647).

#[derive(Debug, Clone, PartialEq)]
pub enum MenuKind {
    Main,
    Help,
    Options,
    SizeError,
    SignalSend { sig: i32 },
    SignalChoose,
    Renice,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RunTarget {
    All,
    Cpu,
    Mem,
    Net,
    Proc,
    Clock,
    Overlay,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScrollKey {
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
}

/// Every mutation/side effect process() triggers, as data. P3 sink executes.
/// Proc/cpu/mem/net variants activate in Tasks 4-5.
#[allow(dead_code)] // Tasks 4-5 construct the rest; Task 5 removes this.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Quit,
    ReloadConfig,
    ShowMenu { menu: MenuKind },
    ToggleBox { index: u8 },
    CyclePreset { dir: i8 },
    Run {
        target: RunTarget,
        no_update: bool,
        redraw: bool,
    },
    RecalcLayout,
    SetUpdateMs { ms: i64 },
    CommitFilter { via_down: bool },
    CancelFilter,
    SetProcFilter { text: String },
    OpenFilterEditor { current: String },
    SortPrev,
    SortNext,
    ToggleTree,
    CollapseAll,
    TogglePause,
    FollowSelected,
    FollowDetailed,
    Unfollow,
    ToggleReversed,
    TogglePerCore,
    ToggleMemBytes,
    ClearFilter,
    ProcSelectRow { row: i64 },
    ProcDetailOpen,
    ProcDetailClose,
    ExpandPid { pid: u64 },
    CollapsePid { pid: u64 },
    ToggleChildren { pid: u64 },
    ProcScroll { key: ScrollKey },
    SetDraggingScroll { on: bool },
    ToggleIoMode,
    ToggleDisks,
    CycleIface { dir: i8 },
    ToggleNetSync,
    ToggleNetAuto,
    ZeroNetOffsets,
}

pub trait ActionSink {
    fn emit(&mut self, action: Action);
}

#[derive(Debug, Default)]
pub struct InputState {
    pub filtering: bool,
    pub vim_keys: bool,
    pub menu_active: bool,
    pub history: std::collections::VecDeque<String>,
    pub last_press_ms: u64,
    pub old_filter: String,
    pub dragging_scroll: bool,
}

/// View reads for dispatch. Task 4 extends with proc/net/cpu view fields.
#[derive(Debug, Default)]
pub struct ViewState;

fn help_key(st: &InputState) -> &str {
    if st.vim_keys { "H" } else { "h" }
}

/// Pure dispatcher (global branch only; Tasks 4-5 append sections).
/// UNIFORM RULE (documented): emit intent Actions for every syntactically-valid
/// key; ALL Config-list legality (gpu_count, preset lists) lives in the P3 sink.
pub fn process_key(key: &str, st: &mut InputState, _view: &ViewState, _now_ms: u64) -> Vec<Action> {
    if key.is_empty() {
        return vec![];
    }
    let mut out = Vec::new();
    if !st.filtering {
        if key == "q" {
            out.push(Action::Quit);
        } else if key == "escape" || key == "m" {
            out.push(Action::ShowMenu { menu: MenuKind::Main });
        } else if key == "f1" || key == "?" || key == help_key(st) {
            out.push(Action::ShowMenu { menu: MenuKind::Help });
        } else if key == "f2" || key == "o" {
            out.push(Action::ShowMenu { menu: MenuKind::Options });
        } else if key.len() == 1 && key.as_bytes()[0].is_ascii_digit() {
            out.push(Action::ToggleBox {
                index: key.as_bytes()[0] - b'0',
            });
        } else if key == "p" {
            out.push(Action::CyclePreset { dir: 1 });
        } else if key == "P" {
            out.push(Action::CyclePreset { dir: -1 });
        } else if key == "ctrl_r" {
            out.push(Action::ReloadConfig);
        } else if key == "mouse_release" {
            st.dragging_scroll = false;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Rec {
        actions: Vec<Action>,
    }
    impl ActionSink for Rec {
        fn emit(&mut self, a: Action) {
            self.actions.push(a);
        }
    }

    #[test]
    fn sink_records_in_order() {
        let mut r = Rec::default();
        r.emit(Action::Quit);
        r.emit(Action::Run {
            target: RunTarget::Cpu,
            no_update: true,
            redraw: true,
        });
        assert_eq!(r.actions.len(), 2);
    }

    fn proced(key: &str) -> Vec<Action> {
        let mut st = InputState::default();
        process_key(key, &mut st, &ViewState, 0)
    }

    #[test]
    fn global_keys_map() {
        assert_eq!(proced("q"), vec![Action::Quit]);
        assert_eq!(
            proced("escape"),
            vec![Action::ShowMenu { menu: MenuKind::Main }]
        );
        assert_eq!(proced("m"), vec![Action::ShowMenu { menu: MenuKind::Main }]);
        assert_eq!(proced("f1"), vec![Action::ShowMenu { menu: MenuKind::Help }]);
        assert_eq!(proced("?"), vec![Action::ShowMenu { menu: MenuKind::Help }]);
        assert_eq!(
            proced("f2"),
            vec![Action::ShowMenu {
                menu: MenuKind::Options
            }]
        );
        assert_eq!(proced("o"), vec![Action::ShowMenu { menu: MenuKind::Options }]);
        assert_eq!(proced("3"), vec![Action::ToggleBox { index: 3 }]);
        assert_eq!(proced("p"), vec![Action::CyclePreset { dir: 1 }]);
        assert_eq!(proced("P"), vec![Action::CyclePreset { dir: -1 }]);
        assert_eq!(proced("ctrl_r"), vec![Action::ReloadConfig]);
        assert_eq!(proced(""), Vec::<Action>::new());
    }

    #[test]
    fn vim_keys_switch_help() {
        let mut st = InputState {
            vim_keys: true,
            ..Default::default()
        };
        assert_eq!(
            process_key("H", &mut st, &ViewState, 0),
            vec![Action::ShowMenu { menu: MenuKind::Help }]
        );
        assert_eq!(
            process_key("h", &mut st, &ViewState, 0),
            Vec::<Action>::new()
        );
    }

    #[test]
    fn mouse_release_clears_drag_state() {
        let mut st = InputState {
            dragging_scroll: true,
            ..Default::default()
        };
        assert_eq!(
            process_key("mouse_release", &mut st, &ViewState, 0),
            Vec::<Action>::new()
        );
        assert!(!st.dragging_scroll);
    }
}
