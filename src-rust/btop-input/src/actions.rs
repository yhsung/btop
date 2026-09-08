//! Actions mirroring every Input::process() branch (btop_input.cpp:214-647).
//!
//! UNIFORM RULE (documented): emit intent Actions for every syntactically-valid
//! key; ALL Config-list legality (gpu_count, preset lists) lives in the P3 sink.
//!
//! SEMANTIC-vs-EXPLICIT RULE (proc branch): arms that flip a Config bool or
//! step `proc_sorting` emit a dedicated semantic Action (SortPrev, ToggleTree,
//! ...) whose P3 sink contract INCLUDES the Config write (documented per
//! variant below). Value-carrying writes with no semantic carrier use explicit
//! SetBool/SetInt (pid/row ints, show_detailed flag). NOTE: no SetString
//! variant exists — every string write in :297-539 already has a semantic
//! carrier (proc_filter text ⇒ SetProcFilter/ClearFilter, proc_sorting ⇒
//! SortPrev/SortNext), so a generic string setter would be dead code.
//!
//! STATE SPLIT: C++ `Input::` namespace globals (filtering flag, old_filter,
//! dragging_scroll, mouse_pos) live in InputState and are mutated directly by
//! process_key (precedent: Task 3 mouse_release). Everything else (Config,
//! Proc:: statics, Menu, Runner) is an emitted Action for the P3 sink.
//!
//! MOUSE POS: C++ Input::mouse_pos is set in get() for any parsed SGR event
//! (:163-184). Here the caller (Task 5 handle_key) threads
//! keys::decode_mouse_pos(raw) into InputState::mouse_pos; process_key reads
//! it for proc-box geometry. None (= unparseable) ⇒ mouse arms keep_going.
//!
//! RECURSION: C++ process("down"/"space"/"enter") re-entry is modelled by
//! self-calls to proc_key with the same (st, view, editor). Terminates: the
//! subkeys land in the scroll (:522), tree-expand (:491) and detail (:461)
//! arms, none of which recurse further (verified against :461-531).
//!
//! Proc::selection CONTRACT (btop_draw.cpp:1627-1705): takes a scroll key or
//! "mousey<N>", mutates proc_start/proc_selected (+follow-break sets), and
//! returns the new selected row or -1 when nothing changed. The -1 failure
//! case is P3 sink behavior: the sink calls selection() and suppresses the
//! trailing Runs on -1, so process_key ALWAYS emits ProcScroll + trailing
//! Runs. selection("mousey..") row math lives in the sink; ScrollKey::Row
//! carries N. Proc::sort_vector (btop_shared.cpp:337-346) =
//! ["pid","name","command","threads","user","memory","cpu direct","cpu lazy"];
//! left/right wrap-around (missing ⇒ len, --<0 ⇒ len-1, ++>len-1 ⇒ 0) is
//! transcribed in the P3 sink per the SortPrev/SortNext contracts below.

use crate::keys::{decode_key, decode_mouse_pos};
use crate::textedit::TextEdit;
use btop_tools::mouse::MouseMap;

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
    /// Scrollbar mousey: P3 maps to Proc::selection("mousey{N}") (:1691-1694).
    Row(i64),
    /// C++ passes "mouse_scroll_up/down" straight into selection() (:452-453,
    /// :1661-1666: start ∓ 3) — distinct from Up/Down, which move selected.
    ScrollUp,
    ScrollDown,
}

/// POSIX signals for the t/kill_key arm (:507). C++ uses SIGTERM/SIGKILL.
const SIG_TERM: i32 = 15;
const SIG_KILL: i32 = 9;

/// Every mutation/side effect process() triggers, as data. P3 sink executes.
/// Step-5 audit (against :214-641): every variant below is constructed by at
/// least one process_key arm — Quit/ReloadConfig/ShowMenu/ToggleBox/
/// CyclePreset (global), SetProcFilter/CommitFilter/CancelFilter/FlushConfig/
/// SortPrev/SortNext/ToggleTree/CollapseAll/TogglePause/FollowSelected/
/// FollowDetailed/Unfollow/ToggleReversed/TogglePerCore/ToggleMemBytes/
/// ClearFilter/ProcSelectRow/ProcDetailOpen/ProcDetailClose/ExpandPid/
/// CollapsePid/ToggleChildren/ProcScroll/SetDraggingScroll/SetBool/SetInt
/// (proc), SetUpdateMs (cpu), ToggleIoMode/ToggleDisks/RecalcLayout (mem),
/// CycleIface/ToggleNetSync/ToggleNetAuto/ZeroNetOffsets (net), Run
/// (everywhere). OpenFilterEditor was deleted: the f/ arm mutates the editor
/// directly and emits no intent (nothing left for the sink to intend).
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Quit,
    ReloadConfig,
    ShowMenu {
        menu: MenuKind,
    },
    ToggleBox {
        index: u8,
    },
    CyclePreset {
        dir: i8,
    },
    Run {
        target: RunTarget,
        no_update: bool,
        redraw: bool,
    },
    RecalcLayout,
    SetUpdateMs {
        ms: i64,
    },
    /// Filtering commit (:302-311). Sink: proc_filtering=false; the committed
    /// text travels in the PRECEDING SetProcFilter (symmetric with CancelFilter).
    CommitFilter {
        via_down: bool,
    },
    /// Filtering abandon (:313-316). Sink: proc_filtering=false; the restored
    /// text travels in the PRECEDING SetProcFilter{old_filter}.
    CancelFilter,
    /// proc_filter=text. Emitted for enter/down commits (editor text), live
    /// edits (:319-320), and escape/click restores (old_filter, cloned BEFORE
    /// st.old_filter is cleared — order matters, :314-316).
    SetProcFilter {
        text: String,
    },
    /// Sink: proc_sorting=prev entry of sort_vector (wrap to len-1) +
    /// update_following=true (:325-331).
    SortPrev,
    /// Sink: proc_sorting=next entry of sort_vector (wrap to 0) +
    /// update_following=true (:333-339).
    SortNext,
    /// Sink: flip proc_tree + update_following=true (:346-350).
    ToggleTree,
    /// Sink: Proc::collapse_all=1 (:351-355).
    CollapseAll,
    /// Sink: flip pause_proc_list (:356-358).
    TogglePause,
    /// Sink: follow_process=true, followed_pid=selected_pid,
    /// update_following=true (:360-364, :469-472).
    FollowSelected,
    /// Sink: follow_process=true, followed_pid=detailed_pid,
    /// update_following=true (:365-369).
    FollowDetailed,
    /// Sink: flip follow_process (true→false on every emitting path:
    /// F-maze :370-378, click follow-break :421-425/:444-448, detail close
    /// :478-482). The zeroing Sets travel alongside explicitly.
    Unfollow,
    /// Sink: flip proc_reversed + update_following=true (:380-383).
    ToggleReversed,
    /// Sink: flip proc_per_core (:384-385).
    TogglePerCore,
    /// Sink: flip proc_mem_bytes (:387-388).
    ToggleMemBytes,
    /// Sink: proc_filter="" (:390-391).
    ClearFilter,
    /// Sink: proc_selected=row (:428).
    ProcSelectRow {
        row: i64,
    },
    /// Detail open (:465-474). Value Sets carry the writes; sink also sets
    /// update_following=true (:489).
    ProcDetailOpen,
    /// Detail close (:475-488). Value Sets carry the writes; sink also sets
    /// update_following=true (:489).
    ProcDetailClose,
    /// Sink: Proc::expand=pid (:496).
    ExpandPid {
        pid: u64,
    },
    /// Sink: Proc::collapse=pid (:497). Space emits BOTH Expand+Collapse,
    /// matching C++ (:496-497), which the collector resolves as a toggle.
    CollapsePid {
        pid: u64,
    },
    /// Sink: Proc::toggle_children=pid (:498).
    ToggleChildren {
        pid: u64,
    },
    /// Sink: Proc::selection(mapped key); on -1 suppress the trailing Runs.
    /// Row(n)⇒"mousey{n}", ScrollUp/Down⇒"mouse_scroll_up/down", else verbatim.
    ProcScroll {
        key: ScrollKey,
    },
    /// Sink: Input::dragging_scroll=on (:437). process_key ALSO flips
    /// st.dragging_scroll immediately (later mouse_drag arms read it).
    SetDraggingScroll {
        on: bool,
    },
    /// Config::unlock(); Config::lock() around the filtering-down recursion
    /// (:306-310). Sink flushes staged Config values before the sub-dispatch.
    FlushConfig,
    /// Generic bool Config write (only show_detailed in :297-539; every other
    /// bool flip has a semantic carrier — see rule above).
    SetBool {
        key: String,
        value: bool,
    },
    /// Generic int Config write (proc_selected, proc_last_selected,
    /// detailed_pid, followed_pid, proc_followed, restore_detailed_pid).
    SetInt {
        key: String,
        value: i64,
    },
    /// Sink: flip io_mode (:579). Mem i arm.
    ToggleIoMode,
    /// Sink: flip show_disks (:582). The d arm ALSO emits RecalcLayout
    /// (Draw::calcSizes, :584) and Run{mem,no_update=false} — the layout
    /// reflow is structural, hence no_update=false unlike every other arm.
    ToggleDisks,
    /// Sink: selected_iface=next/prev entry of Net::interfaces with wrap
    /// (:602-610) + Net::rescale=true (:611). dir=-1 for b (prev), +1 for n
    /// (next). rescale/offset internals are sink duties. Emitted whenever
    /// b/n matches — even with an empty/missing iface list (C++ keep_going
    /// stays false, :600-613) — so the trailing Run fires regardless.
    CycleIface {
        dir: i8,
    },
    /// Sink: flip net_sync + Net::rescale=true (:615-616). y arm.
    ToggleNetSync,
    /// Sink: flip net_auto + Net::rescale=true (:619-620). a arm.
    ToggleNetAuto,
    /// Sink: zero (or re-seed, when already zero) the download/upload offsets
    /// of Net::current_net[selected_iface] (:623-632). Offset math is a sink
    /// duty. Run{net,no_update=false} — like RecalcLayout, a data re-seed.
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
    /// Threaded by the caller from keys::decode_mouse_pos(raw); mirrors the
    /// C++ Input::mouse_pos global read at :395. None ⇒ mouse arms keep_going.
    pub mouse_pos: Option<(i64, i64)>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ProcGeom {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

/// View reads for dispatch. Every field mirrors the Config key / Proc global
/// of the same name (selected_depth ⇒ selected_depth, scroll_pos ⇒
/// Proc::scroll_pos); show_detailed_geom drives the y+8/height-8 adjust (:396).
/// Box-gate flags (cpu_shown/mem_shown/net_shown, default false) mirror
/// Cpu::shown/Mem::shown/Net::shown (:542/:573/:595); existing proc tests
/// keep passing because proc_view() sets proc_shown=true explicitly while the
/// new flags default to false. update_ms mirrors Config update_ms (cpu accel
/// arms, :548-562). net_interfaces/net_selected mirror Net::interfaces /
/// Net::selected_iface (kept flat — 30 fields total stays readable without
/// sub-structs; regroup if the next plan pushes past ~30).
#[derive(Debug, Default)]
pub struct ViewState {
    pub proc_shown: bool,
    pub cpu_shown: bool,
    pub mem_shown: bool,
    pub net_shown: bool,
    pub update_ms: i64,
    pub net_interfaces: Vec<String>,
    pub net_selected: String,
    pub sorting: String,
    pub sorting_list: Vec<String>,
    pub tree: bool,
    pub selected: i64,
    pub selected_pid: u64,
    pub detailed_pid: u64,
    pub detailed_dead: bool,
    pub show_detailed: bool,
    pub followed_pid: u64,
    pub follow_process: bool,
    pub follow_detailed_cfg: bool,
    pub should_return_to_followed: bool,
    pub restore_detailed_pid: u64,
    pub proc_last_selected: i64,
    pub proc_followed: i64,
    pub filter_text: String,
    pub proc_geom: ProcGeom,
    pub show_detailed_geom: bool,
    pub scroll_pos: i64,
    pub banner_shown: bool,
    pub pause_proc_list: bool,
    /// Extra (beyond the base spec): required by the tree x-zone check
    /// `offset = selected_depth * 3` (:406-407). i64 ⇒ Default holds.
    pub selected_depth: i64,
}

fn help_key(st: &InputState) -> &str {
    if st.vim_keys {
        "H"
    } else {
        "h"
    }
}

/// The trailing `run("proc")+run("cpu")` pair (:534-536) with identical flags.
fn runs(no_update: bool, redraw: bool) -> Vec<Action> {
    vec![
        Action::Run {
            target: RunTarget::Proc,
            no_update,
            redraw,
        },
        Action::Run {
            target: RunTarget::Cpu,
            no_update,
            redraw,
        },
    ]
}

fn sint(key: &str, value: i64) -> Action {
    Action::SetInt {
        key: key.to_string(),
        value,
    }
}

fn sbool(key: &str, value: bool) -> Action {
    Action::SetBool {
        key: key.to_string(),
        value,
    }
}

fn scroll_key(key: &str) -> ScrollKey {
    match key {
        "up" | "k" => ScrollKey::Up,
        "down" | "j" => ScrollKey::Down,
        "page_up" => ScrollKey::PageUp,
        "page_down" => ScrollKey::PageDown,
        "home" | "g" => ScrollKey::Home,
        _ => ScrollKey::End, // "end" | "G" (only called for scroll keys)
    }
}

/// Shared scroll emission for the :522 arm and the mouse goto/drag paths.
/// redraw=true iff view.selected==0 — the old-side of :529-530's 0-transition
/// rule (`old==0 or new==0`); the new-side and the -1 guard are sink-refined
/// via selection()'s return value.
fn scroll_actions(sk: ScrollKey, redraw: bool) -> Vec<Action> {
    let mut out = vec![Action::ProcScroll { key: sk }];
    out.extend(runs(true, redraw));
    out
}

/// Pure dispatcher (global + proc + cpu/mem/net branches).
/// Unknown keys fall through the global branch into the proc section (C++
/// keep_going=true, :290-293); filtering skips the global branch entirely.
/// Box sections run in C++ order proc → cpu → mem → net (:297/:542/:573/:595),
/// each gated on its shown flag; the first match wins (C++ `return`s).
pub fn process_key(
    key: &str,
    st: &mut InputState,
    view: &ViewState,
    editor: &mut TextEdit,
    now_ms: u64,
) -> Vec<Action> {
    if key.is_empty() {
        return vec![];
    }
    if !st.filtering {
        let mut out = Vec::new();
        let mut global_handled = true;
        if key == "q" {
            out.push(Action::Quit);
        } else if key == "escape" || key == "m" {
            out.push(Action::ShowMenu {
                menu: MenuKind::Main,
            });
        } else if key == "f1" || key == "?" || key == help_key(st) {
            out.push(Action::ShowMenu {
                menu: MenuKind::Help,
            });
        } else if key == "f2" || key == "o" {
            out.push(Action::ShowMenu {
                menu: MenuKind::Options,
            });
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
        } else {
            global_handled = false;
        }
        if global_handled {
            return out;
        }
    }
    if view.proc_shown {
        if let Some(actions) = proc_key(key, st, view, editor, now_ms) {
            return actions;
        }
    }
    if view.cpu_shown {
        if let Some(actions) = cpu_key(key, st, view, now_ms) {
            return actions;
        }
    }
    if view.mem_shown {
        if let Some(actions) = mem_key(key) {
            return actions;
        }
    }
    if view.net_shown {
        if let Some(actions) = net_key(key) {
            return actions;
        }
    }
    vec![]
}

/// Proc-box section (btop_input.cpp:297-539). Some(vec) = handled (vec
/// includes the trailing runs; Some(vec![]) = C++ bare `return`, no runs).
/// None = keep_going (Task 5 tries the next box).
fn proc_key(
    key: &str,
    st: &mut InputState,
    view: &ViewState,
    editor: &mut TextEdit,
    now_ms: u64,
) -> Option<Vec<Action>> {
    let _ = now_ms; // Task 5 cpu branch (update_ms accel window) consumes this.
                    //? Filtering block (:301-324) — checked before every other proc arm.
    if st.filtering {
        if key == "enter" || key == "down" {
            // :302-311: commit editor text, leave filtering, clear old_filter.
            // Enter falls through to the trailing runs (keep_going stays
            // false); down additionally flushes + recurses (:306-310).
            let text = editor.text.clone();
            st.filtering = false;
            st.old_filter.clear();
            if key == "down" {
                let mut out = vec![
                    Action::SetProcFilter { text },
                    Action::CommitFilter { via_down: true },
                    Action::FlushConfig,
                ];
                out.extend(proc_key("down", st, view, editor, now_ms).unwrap_or_default());
                return Some(out);
            }
            let mut out = vec![
                Action::SetProcFilter { text },
                Action::CommitFilter { via_down: false },
            ];
            out.extend(runs(true, true));
            return Some(out);
        } else if key == "escape" || key == "mouse_click" {
            // :313-316: restore old_filter text, leave filtering.
            let old = st.old_filter.clone();
            st.filtering = false;
            st.old_filter.clear();
            let mut out = vec![Action::SetProcFilter { text: old }, Action::CancelFilter];
            out.extend(runs(true, true));
            return Some(out);
        } else if editor.command(key) {
            // :318-321: consumed edit falls through to the trailing runs.
            let mut out = Vec::new();
            if editor.text != view.filter_text {
                out.push(Action::SetProcFilter {
                    text: editor.text.clone(),
                });
            }
            out.extend(runs(true, true));
            return Some(out);
        } else {
            // :322-323: unconsumed key ⇒ bare return, no runs.
            return Some(Vec::new());
        }
    }

    let vim = st.vim_keys;
    // kill_key vim variant (:220).
    let kill_key = if vim { "K" } else { "k" };

    if key == "left" || (vim && key == "h") {
        // :325-332. Wrap math lives in the P3 sink (SortPrev contract);
        // in tree mode no_update=false (:331), else it stays true.
        let mut out = vec![Action::SortPrev];
        out.extend(runs(!view.tree, true));
        Some(out)
    } else if key == "right" || (vim && key == "l") {
        // :333-340, mirror of left.
        let mut out = vec![Action::SortNext];
        out.extend(runs(!view.tree, true));
        Some(out)
    } else if key == "f" || key == "/" {
        // :341-345: enter filtering. The &mut editor exists for this: re-init
        // from current filter text + snapshot old_filter, flip the flag.
        // Falls to trailing runs with NO OpenFilterEditor intent (nothing left
        // for the sink to intend — Task 5 deletes the unemitted variant).
        st.filtering = !st.filtering;
        *editor = TextEdit::new(view.filter_text.clone(), false);
        st.old_filter = view.filter_text.clone();
        Some(runs(true, true))
    } else if key == "e" {
        // :346-350.
        let mut out = vec![Action::ToggleTree];
        out.extend(runs(false, true));
        Some(out)
    } else if key == "E" && view.tree {
        // :351-355 (gated on tree; without tree falls through to keep_going).
        let mut out = vec![Action::CollapseAll];
        out.extend(runs(false, true));
        Some(out)
    } else if key == "u" {
        // :356-358.
        let mut out = vec![Action::TogglePause];
        out.extend(runs(true, true));
        Some(out)
    } else if key == "F" {
        // :359-379 follow maze. No inner else: with no matching sub-arm the
        // F arm still falls to the trailing runs (keep_going stays false).
        if view.selected != 0 && view.followed_pid != view.selected_pid {
            let mut out = vec![Action::FollowSelected];
            out.extend(runs(true, true));
            Some(out)
        } else if view.show_detailed && view.selected == 0 && view.followed_pid != view.detailed_pid
        {
            let mut out = vec![Action::FollowDetailed];
            out.extend(runs(true, true));
            Some(out)
        } else if view.follow_process {
            let mut out = vec![Action::Unfollow];
            if view.should_return_to_followed {
                out.push(sint("proc_selected", view.proc_followed));
            } else if view.show_detailed && view.followed_pid == view.detailed_pid {
                out.push(sint("restore_detailed_pid", view.detailed_pid as i64));
            }
            out.push(sint("followed_pid", 0));
            out.push(sint("proc_followed", 0));
            out.extend(runs(true, true));
            Some(out)
        } else {
            Some(runs(true, true))
        }
    } else if key == "r" {
        // :380-383.
        let mut out = vec![Action::ToggleReversed];
        out.extend(runs(true, true));
        Some(out)
    } else if key == "c" {
        // :384-385.
        let mut out = vec![Action::TogglePerCore];
        out.extend(runs(true, true));
        Some(out)
    } else if key == "%" {
        // :387-388.
        let mut out = vec![Action::ToggleMemBytes];
        out.extend(runs(true, true));
        Some(out)
    } else if key == "delete" && !view.filter_text.is_empty() {
        // :390-391 ("delete" while filtering never reaches here — the
        // filtering block routes it into editor.command).
        let mut out = vec![Action::ClearFilter];
        out.extend(runs(true, true));
        Some(out)
    } else if key.starts_with("mouse_") {
        // :393-460 (redraw=false for the whole block, :394).
        proc_mouse(key, st, view, editor, now_ms)
    } else if key == "enter" || key == "info_enter" {
        // :461-490. update_following (:489) rides in the Detail contracts.
        if view.selected == 0 && !view.show_detailed {
            // :462-464 guard: bare return.
            return Some(Vec::new());
        } else if view.selected > 0 && view.detailed_pid != view.selected_pid {
            // :465-474 detail open.
            let mut out = vec![
                Action::ProcDetailOpen,
                sint("detailed_pid", view.selected_pid as i64),
                sint("proc_last_selected", view.selected),
                sint("proc_selected", 0),
            ];
            if view.follow_detailed_cfg {
                out.push(Action::FollowSelected);
            }
            out.push(sbool("show_detailed", true));
            out.extend(runs(true, true));
            Some(out)
        } else if view.show_detailed {
            // :475-488 detail close.
            let mut out = vec![Action::ProcDetailClose];
            if view.follow_detailed_cfg {
                out.push(sint("restore_detailed_pid", view.detailed_pid as i64));
                if view.follow_process && view.followed_pid == view.detailed_pid {
                    out.push(Action::Unfollow);
                    out.push(sint("followed_pid", 0));
                    out.push(sint("proc_followed", 0));
                }
            } else if view.proc_last_selected > 0 {
                out.push(sint("proc_selected", view.proc_last_selected));
            }
            out.push(sint("proc_last_selected", 0));
            out.push(sint("detailed_pid", 0));
            out.push(sbool("show_detailed", false));
            out.extend(runs(true, true));
            Some(out)
        } else {
            // selected>0, detailed==selected, !show_detailed: neither open nor
            // close matches, but the arm DID match ⇒ update_following +
            // trailing runs still fire (:489, :534-536).
            Some(runs(true, true))
        }
    } else if (key == "+" || key == "-" || key == "space" || key == "C" || key == "=") && view.tree
    {
        // :491-503 tree expand/collapse.
        let is_following_detailed = view.follow_process && view.followed_pid == view.detailed_pid;
        if view.selected > 0 || is_following_detailed {
            let pid = if is_following_detailed && view.selected == 0 {
                view.followed_pid
            } else {
                view.selected_pid
            };
            let mut out = Vec::new();
            if key == "+" || key == "space" || key == "=" {
                out.push(Action::ExpandPid { pid });
            }
            if key == "-" || key == "space" {
                out.push(Action::CollapsePid { pid });
            }
            if key == "C" {
                out.push(Action::ToggleChildren { pid });
            }
            out.extend(runs(false, true));
            Some(out)
        } else {
            None
        }
    } else if (key == "t" || key == kill_key) && (view.show_detailed || view.selected_pid > 0) {
        // :504-509 signal menu (dead-detailed guard ⇒ bare return).
        if view.show_detailed && view.selected == 0 && view.detailed_dead {
            return Some(Vec::new());
        }
        let sig = if key == "t" { SIG_TERM } else { SIG_KILL };
        Some(vec![Action::ShowMenu {
            menu: MenuKind::SignalSend { sig },
        }])
    } else if key == "s" && (view.show_detailed || view.selected_pid > 0) {
        // :510-515 signal-choose menu.
        if view.show_detailed && view.selected == 0 && view.detailed_dead {
            return Some(Vec::new());
        }
        Some(vec![Action::ShowMenu {
            menu: MenuKind::SignalChoose,
        }])
    } else if key == "N" && (view.show_detailed || view.selected_pid > 0) {
        // :516-521 renice menu.
        if view.show_detailed && view.selected == 0 && view.detailed_dead {
            return Some(Vec::new());
        }
        Some(vec![Action::ShowMenu {
            menu: MenuKind::Renice,
        }])
    } else if key == "up"
        || key == "down"
        || key == "page_up"
        || key == "page_down"
        || key == "home"
        || key == "end"
        || (vim && (key == "j" || key == "k" || key == "g" || key == "G"))
    {
        // :522-531 shared scroll path (also the mouse_scroll goto target).
        Some(scroll_actions(scroll_key(key), view.selected == 0))
    } else {
        None
    }
}

/// Mouse arm of the proc section (:393-460). The caller threads
/// decode_mouse_pos(raw) into st.mouse_pos; None ⇒ keep_going (None).
fn proc_mouse(
    key: &str,
    st: &mut InputState,
    view: &ViewState,
    editor: &mut TextEdit,
    now_ms: u64,
) -> Option<Vec<Action>> {
    let (col, line) = st.mouse_pos?;
    let y = if view.show_detailed_geom {
        view.proc_geom.y + 8
    } else {
        view.proc_geom.y
    };
    let height = if view.show_detailed_geom {
        view.proc_geom.height - 8
    } else {
        view.proc_geom.height
    };
    let g = &view.proc_geom;
    // `> x` ≡ C++ `>= x + 1` for integers (transcribed from :398).
    let in_box = col > g.x && col < g.x + g.width && line > y && line < y + height - 1;
    if key == "mouse_click" {
        if in_box {
            if col < g.x + g.width - 2 {
                // :400-429 main zone.
                let row = line - y - 1;
                if view.selected == row {
                    // :403-415 same-row click: tree x-zone recurses into
                    // space, otherwise into enter. (The local redraw=true at
                    // :404 dies with the recursion — C++ returns the sub-run.)
                    if view.tree {
                        let x_pos = col - g.x;
                        let offset = view.selected_depth * 3;
                        if x_pos > offset && x_pos < 4 + offset {
                            return Some(
                                proc_key("space", st, view, editor, now_ms).unwrap_or_default(),
                            );
                        }
                    }
                    return Some(proc_key("enter", st, view, editor, now_ms).unwrap_or_default());
                } else if view.banner_shown && line == y + height - 2 {
                    // :416-417 banner row guard: bare return, no runs.
                    return Some(Vec::new());
                }
                // :418-428 row select (+follow-break :421-425).
                let mut rd = view.selected == 0 || row == 0;
                let mut out = Vec::new();
                if view.follow_process && !view.pause_proc_list {
                    out.push(Action::Unfollow);
                    out.push(sint("followed_pid", 0));
                    out.push(sint("proc_followed", 0));
                    rd = true;
                }
                out.push(Action::ProcSelectRow { row });
                out.extend(runs(true, rd));
                return Some(out);
            } else if line == y + 1 {
                // :430-432 page-up gutter (-1 ⇒ sink suppresses runs).
                return Some(scroll_actions(ScrollKey::PageUp, false));
            } else if line == y + height - 2 {
                // :433-435 page-down gutter.
                return Some(scroll_actions(ScrollKey::PageDown, false));
            } else if line == y + 2 + view.scroll_pos {
                // :436-438 drag handle.
                st.dragging_scroll = true;
                let mut out = vec![Action::SetDraggingScroll { on: true }];
                out.extend(runs(true, false));
                return Some(out);
            }
            // :439-440 scrollbar mousey (-1 ⇒ sink suppresses runs).
            let mut out = vec![Action::ProcScroll {
                key: ScrollKey::Row(line - y - 2),
            }];
            out.extend(runs(true, false));
            return Some(out);
        } else if view.selected > 0 {
            // :442-450 outside-box deselect (+follow-break).
            let mut out = vec![sint("proc_selected", 0)];
            if view.follow_process && !view.pause_proc_list {
                out.push(Action::Unfollow);
                out.push(sint("followed_pid", 0));
                out.push(sint("proc_followed", 0));
            }
            out.extend(runs(true, true));
            return Some(out);
        }
        // Outside-box click with nothing selected: the arm matched but changed
        // nothing ⇒ trailing runs with redraw=false (keep_going stays false).
        return Some(runs(true, false));
    } else if (key == "mouse_scroll_up" || key == "mouse_scroll_down") && in_box {
        // :452-453 goto proc_mouse_scroll.
        let sk = if key == "mouse_scroll_up" {
            ScrollKey::ScrollUp
        } else {
            ScrollKey::ScrollDown
        };
        return Some(scroll_actions(sk, false));
    } else if key == "mouse_drag" && st.dragging_scroll {
        // :455-457 scrollbar drag: selection result ignored, run kept.
        let mut out = vec![Action::ProcScroll {
            key: ScrollKey::Row(line - y - 2),
        }];
        out.extend(runs(true, false));
        return Some(out);
    }
    None
}

/// Cpu-box section (btop_input.cpp:542-570). Some(vec) = handled (vec carries
/// the single Run{cpu,true,true} — no_update stays true, redraw is set true
/// on every handled press, :544-545/:554/:561). None = keep_going.
/// HISTORY CONTRACT: C++ pushes the key into Input::history in get() BEFORE
/// process() runs (:193-196), so by the time this fn sees `key`, st.history
/// already ends with `key`. Unit tests must pre-fill st.history INCLUDING the
/// current key (e.g. history=["+","+","+"] for a third "+"); handle_key upholds
/// this ordering. NOTE: C++ history is a fixed 50-deque pre-filled with "";
/// here it grows from empty and caps at 50, so all_of holds after N
/// consecutive same-keys rather than 50 — accel arrives EARLIER than C++.
/// Fail-safe direction (a stray key still downgrades to the single step),
/// and the window+history shape verified by the Step-1 tests is unchanged.
/// WINDOW MATH: C++ `last_press >= time_ms() - 200` is uint64 arithmetic;
/// mirrored with wrapping_sub so now_ms<200 wraps identically. Consequence:
/// presses in the first 200ms after boot never accelerate (last=0 < huge
/// wrapped bound) — faithful, not a bug. st.last_press_ms updates on handled
/// presses ONLY (gated-out keys leave it untouched).
fn cpu_key(key: &str, st: &mut InputState, view: &ViewState, now_ms: u64) -> Option<Vec<Action>> {
    if (key == "+" || key == "=") && view.update_ms <= 86_399_900 {
        // :548-554. "=" rides the + arm but can never accelerate (history
        // all_of demands "+" while history ends with "=").
        let accel = view.update_ms <= 86_399_000
            && st.last_press_ms >= now_ms.wrapping_sub(200)
            && st.history.iter().all(|s| s == "+");
        let add = if accel { 1000 } else { 100 };
        st.last_press_ms = now_ms;
        Some(vec![
            Action::SetUpdateMs {
                ms: view.update_ms + add,
            },
            Action::Run {
                target: RunTarget::Cpu,
                no_update: true,
                redraw: true,
            },
        ])
    } else if key == "-" && view.update_ms >= 200 {
        // :556-562, mirror (accel sub-gate >= 2000).
        let accel = view.update_ms >= 2000
            && st.last_press_ms >= now_ms.wrapping_sub(200)
            && st.history.iter().all(|s| s == "-");
        let sub = if accel { 1000 } else { 100 };
        st.last_press_ms = now_ms;
        Some(vec![
            Action::SetUpdateMs {
                ms: view.update_ms - sub,
            },
            Action::Run {
                target: RunTarget::Cpu,
                no_update: true,
                redraw: true,
            },
        ])
    } else {
        None
    }
}

/// Mem-box section (:573-592). i flips io_mode (:579); d flips show_disks +
/// recalcs layout (:581-584, hence no_update=false). Else keep_going (None).
fn mem_key(key: &str) -> Option<Vec<Action>> {
    if key == "i" {
        Some(vec![
            Action::ToggleIoMode,
            Action::Run {
                target: RunTarget::Mem,
                no_update: true,
                redraw: true,
            },
        ])
    } else if key == "d" {
        Some(vec![
            Action::ToggleDisks,
            Action::RecalcLayout,
            Action::Run {
                target: RunTarget::Mem,
                no_update: false,
                redraw: true,
            },
        ])
    } else {
        None
    }
}

/// Net-box section (:595-641). b=prev(dir=-1), n=next(dir=+1) with wrap
/// (:604-609 — b decrements, n increments); y/a flip net_sync/net_auto;
/// z re-seeds offsets (no_update=false, :633). rescale/offset internals and
/// the empty-list no-op are sink duties (CycleIface contract); the Run fires
/// whenever the arm matches, even with no interfaces (C++ keep_going=false).
fn net_key(key: &str) -> Option<Vec<Action>> {
    let run = Action::Run {
        target: RunTarget::Net,
        no_update: true,
        redraw: true,
    };
    if key == "b" {
        Some(vec![Action::CycleIface { dir: -1 }, run])
    } else if key == "n" {
        Some(vec![Action::CycleIface { dir: 1 }, run])
    } else if key == "y" {
        Some(vec![Action::ToggleNetSync, run])
    } else if key == "a" {
        Some(vec![Action::ToggleNetAuto, run])
    } else if key == "z" {
        Some(vec![
            Action::ZeroNetOffsets,
            Action::Run {
                target: RunTarget::Net,
                no_update: false,
                redraw: true,
            },
        ])
    } else {
        None
    }
}

/// Full get()+process() glue for P3/P4 (C++ Input::get :123-199 + process
/// :214-647). Decodes the key, threads the mouse position ALWAYS (even None —
/// a stale pos must never linger for a later mouse arm), appends history
/// (BEFORE process, preserving the :193-196 order the cpu accel arm depends
/// on; capped at 50), then dispatches. Empty raws decode to "" and
/// process_key short-circuits to empty — but they still shift history (C++
/// skips empty pushes; fail-safe direction: accel needs all_same, so a ""
/// entry only ever downgrades accel to the single step).
pub fn handle_key(
    raw: &str,
    input_maps: &[MouseMap],
    menu_maps: &[MouseMap],
    st: &mut InputState,
    view: &ViewState,
    editor: &mut TextEdit,
    now_ms: u64,
) -> Vec<Action> {
    let key = decode_key(raw, input_maps, menu_maps, st.filtering, st.menu_active);
    st.mouse_pos = decode_mouse_pos(raw);
    st.history.push_back(key.clone());
    while st.history.len() > 50 {
        st.history.pop_front();
    }
    process_key(&key, st, view, editor, now_ms)
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
        let mut ed = TextEdit::new(String::new(), false);
        process_key(key, &mut st, &ViewState::default(), &mut ed, 0)
    }

    /// Full-literal proc ViewState (every field spelled so struct churn breaks
    /// this helper, not each test). Per-test overrides use `..proc_view()`.
    fn proc_view() -> ViewState {
        ViewState {
            proc_shown: true,
            cpu_shown: false,
            mem_shown: false,
            net_shown: false,
            update_ms: 0,
            net_interfaces: Vec::new(),
            net_selected: String::new(),
            sorting: "cpu lazy".to_string(),
            sorting_list: vec![
                "pid".to_string(),
                "name".to_string(),
                "command".to_string(),
                "threads".to_string(),
                "user".to_string(),
                "memory".to_string(),
                "cpu direct".to_string(),
                "cpu lazy".to_string(),
            ],
            tree: false,
            selected: 0,
            selected_pid: 0,
            detailed_pid: 0,
            detailed_dead: false,
            show_detailed: false,
            followed_pid: 0,
            follow_process: false,
            follow_detailed_cfg: false,
            should_return_to_followed: false,
            restore_detailed_pid: 0,
            proc_last_selected: 0,
            proc_followed: 0,
            filter_text: String::new(),
            proc_geom: ProcGeom {
                x: 1,
                y: 2,
                width: 40,
                height: 15,
            },
            show_detailed_geom: false,
            scroll_pos: 0,
            banner_shown: false,
            pause_proc_list: false,
            selected_depth: 0,
        }
    }

    fn prockey(
        key: &str,
        st: &mut InputState,
        view: &ViewState,
        editor: &mut TextEdit,
    ) -> Vec<Action> {
        process_key(key, st, view, editor, 0)
    }

    fn run_pair(no_update: bool, redraw: bool) -> Vec<Action> {
        runs(no_update, redraw)
    }

    #[test]
    fn global_keys_map() {
        assert_eq!(proced("q"), vec![Action::Quit]);
        assert_eq!(
            proced("escape"),
            vec![Action::ShowMenu {
                menu: MenuKind::Main
            }]
        );
        assert_eq!(
            proced("m"),
            vec![Action::ShowMenu {
                menu: MenuKind::Main
            }]
        );
        assert_eq!(
            proced("f1"),
            vec![Action::ShowMenu {
                menu: MenuKind::Help
            }]
        );
        assert_eq!(
            proced("?"),
            vec![Action::ShowMenu {
                menu: MenuKind::Help
            }]
        );
        assert_eq!(
            proced("f2"),
            vec![Action::ShowMenu {
                menu: MenuKind::Options
            }]
        );
        assert_eq!(
            proced("o"),
            vec![Action::ShowMenu {
                menu: MenuKind::Options
            }]
        );
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
        let mut ed = TextEdit::new(String::new(), false);
        assert_eq!(
            process_key("H", &mut st, &ViewState::default(), &mut ed, 0),
            vec![Action::ShowMenu {
                menu: MenuKind::Help
            }]
        );
        assert_eq!(
            process_key("h", &mut st, &ViewState::default(), &mut ed, 0),
            Vec::<Action>::new()
        );
    }

    #[test]
    fn mouse_release_clears_drag_state() {
        let mut st = InputState {
            dragging_scroll: true,
            ..Default::default()
        };
        let mut ed = TextEdit::new(String::new(), false);
        assert_eq!(
            process_key("mouse_release", &mut st, &ViewState::default(), &mut ed, 0),
            Vec::<Action>::new()
        );
        assert!(!st.dragging_scroll);
    }

    //? Filtering block (:301-324).

    #[test]
    fn filtering_enter_commits_and_runs() {
        // Enter falls through to the trailing proc+cpu runs (keep_going stays
        // false — verified against :301-311 + :534-536, not intuition).
        let view = proc_view();
        let mut st = InputState {
            filtering: true,
            old_filter: "old".to_string(),
            ..Default::default()
        };
        let mut ed = TextEdit::new("foo".to_string(), false);
        let mut expected = vec![
            Action::SetProcFilter {
                text: "foo".to_string(),
            },
            Action::CommitFilter { via_down: false },
        ];
        expected.extend(run_pair(true, true));
        assert_eq!(prockey("enter", &mut st, &view, &mut ed), expected);
        assert!(!st.filtering);
        assert!(st.old_filter.is_empty());
    }

    #[test]
    fn filtering_down_commits_flushes_and_recurses() {
        // Down = commit + FlushConfig (unlock/lock) + process("down") inline.
        // Recursion terminates in the scroll arm (selected==0 ⇒ redraw=true).
        let view = proc_view();
        let mut st = InputState {
            filtering: true,
            old_filter: "old".to_string(),
            ..Default::default()
        };
        let mut ed = TextEdit::new("foo".to_string(), false);
        let mut expected = vec![
            Action::SetProcFilter {
                text: "foo".to_string(),
            },
            Action::CommitFilter { via_down: true },
            Action::FlushConfig,
            Action::ProcScroll {
                key: ScrollKey::Down,
            },
        ];
        expected.extend(run_pair(true, true));
        assert_eq!(prockey("down", &mut st, &view, &mut ed), expected);
        assert!(!st.filtering);
    }

    #[test]
    fn filtering_escape_restores_old_filter() {
        // Restore value cloned BEFORE st.old_filter is cleared (order matters).
        let view = proc_view();
        let mut st = InputState {
            filtering: true,
            old_filter: "old".to_string(),
            ..Default::default()
        };
        let mut ed = TextEdit::new("foo".to_string(), false);
        let mut expected = vec![
            Action::SetProcFilter {
                text: "old".to_string(),
            },
            Action::CancelFilter,
        ];
        expected.extend(run_pair(true, true));
        assert_eq!(prockey("escape", &mut st, &view, &mut ed), expected);
        assert!(!st.filtering);
        assert!(st.old_filter.is_empty());
    }

    #[test]
    fn filtering_click_cancels_like_escape() {
        let view = proc_view();
        let mut st = InputState {
            filtering: true,
            old_filter: "old".to_string(),
            ..Default::default()
        };
        let mut ed = TextEdit::new("foo".to_string(), false);
        let mut expected = vec![
            Action::SetProcFilter {
                text: "old".to_string(),
            },
            Action::CancelFilter,
        ];
        expected.extend(run_pair(true, true));
        assert_eq!(prockey("mouse_click", &mut st, &view, &mut ed), expected);
    }

    #[test]
    fn filtering_edit_emits_text_when_different() {
        // Consumed edit falls through to trailing runs (:318-321, no return).
        let view = proc_view();
        let mut st = InputState {
            filtering: true,
            ..Default::default()
        };
        let mut ed = TextEdit::new(String::new(), false);
        let mut expected = vec![Action::SetProcFilter {
            text: "x".to_string(),
        }];
        expected.extend(run_pair(true, true));
        assert_eq!(prockey("x", &mut st, &view, &mut ed), expected);
        assert!(st.filtering); // edit does not leave filtering mode
    }

    #[test]
    fn filtering_edit_same_text_runs_only() {
        let mut st = InputState {
            filtering: true,
            ..Default::default()
        };
        // Left-arrow is consumed by the editor without changing text, so no
        // SetProcFilter — only the trailing runs (:318-321).
        let mut ed2 = TextEdit::new("ab".to_string(), false);
        let view2 = ViewState {
            filter_text: "ab".to_string(),
            ..proc_view()
        };
        assert_eq!(
            prockey("left", &mut st, &view2, &mut ed2),
            run_pair(true, true)
        );
    }

    #[test]
    fn filtering_unhandled_key_returns_empty() {
        // editor.command("f1") is false ⇒ bare return, no runs (:322-323).
        let view = proc_view();
        let mut st = InputState {
            filtering: true,
            ..Default::default()
        };
        let mut ed = TextEdit::new(String::new(), false);
        assert_eq!(prockey("f1", &mut st, &view, &mut ed), Vec::<Action>::new());
    }

    //? Sort / filter-toggle / tree-mode / pause (:325-358).

    #[test]
    fn sort_left_right_carry_tree_no_update_flag() {
        // no_update = !tree (:331/:339); redraw always true.
        let view = proc_view();
        let mut st = InputState::default();
        let mut ed = TextEdit::new(String::new(), false);
        let mut expected = vec![Action::SortPrev];
        expected.extend(run_pair(true, true));
        assert_eq!(prockey("left", &mut st, &view, &mut ed), expected);
        let mut expected = vec![Action::SortNext];
        expected.extend(run_pair(true, true));
        assert_eq!(prockey("right", &mut st, &view, &mut ed), expected);
        let tree = ViewState {
            tree: true,
            ..proc_view()
        };
        let mut expected = vec![Action::SortPrev];
        expected.extend(run_pair(false, true));
        assert_eq!(prockey("left", &mut st, &tree, &mut ed), expected);
    }

    #[test]
    fn vim_hl_sort_and_k_kill_split() {
        // vim h/l sort; k scrolls under vim but kills without it.
        let view = proc_view();
        let mut vim = InputState {
            vim_keys: true,
            ..Default::default()
        };
        let mut plain = InputState::default();
        let mut ed = TextEdit::new(String::new(), false);
        let mut expected = vec![Action::SortPrev];
        expected.extend(run_pair(true, true));
        assert_eq!(prockey("h", &mut vim, &view, &mut ed), expected);
        let mut expected = vec![Action::SortNext];
        expected.extend(run_pair(true, true));
        assert_eq!(prockey("l", &mut vim, &view, &mut ed), expected);
        // "k" under vim ⇒ scroll Up...
        assert_eq!(
            prockey("k", &mut vim, &view, &mut ed)[0],
            Action::ProcScroll { key: ScrollKey::Up }
        );
        // ...without vim ⇒ SIGKILL menu (selected_pid>0 gate).
        let armed = ViewState {
            selected_pid: 42,
            ..proc_view()
        };
        assert_eq!(
            prockey("k", &mut plain, &armed, &mut ed),
            vec![Action::ShowMenu {
                menu: MenuKind::SignalSend { sig: SIG_KILL }
            }]
        );
        // Without vim and without a target, k keeps going ⇒ empty.
        assert_eq!(
            prockey("k", &mut plain, &view, &mut ed),
            Vec::<Action>::new()
        );
    }

    #[test]
    fn filter_toggle_inits_editor_and_state() {
        // f// flips filtering, re-inits editor from filter text, snapshots
        // old_filter (:341-345), then trailing runs — no OpenFilterEditor.
        let view = ViewState {
            filter_text: "cur".to_string(),
            ..proc_view()
        };
        let mut st = InputState::default();
        let mut ed = TextEdit::new(String::new(), false);
        assert_eq!(prockey("f", &mut st, &view, &mut ed), run_pair(true, true));
        assert!(st.filtering);
        assert_eq!(ed.text, "cur");
        assert_eq!(st.old_filter, "cur");
        let mut st2 = InputState::default();
        let mut ed2 = TextEdit::new(String::new(), false);
        assert_eq!(
            prockey("/", &mut st2, &view, &mut ed2),
            run_pair(true, true)
        );
        assert!(st2.filtering);
    }

    #[test]
    fn toggle_tree_and_collapse_all_flags() {
        let view = proc_view();
        let mut st = InputState::default();
        let mut ed = TextEdit::new(String::new(), false);
        let mut expected = vec![Action::ToggleTree];
        expected.extend(run_pair(false, true));
        assert_eq!(prockey("e", &mut st, &view, &mut ed), expected);
        // E requires tree (:351): without ⇒ keep_going ⇒ empty.
        assert_eq!(prockey("E", &mut st, &view, &mut ed), Vec::<Action>::new());
        let tree = ViewState {
            tree: true,
            ..proc_view()
        };
        let mut expected = vec![Action::CollapseAll];
        expected.extend(run_pair(false, true));
        assert_eq!(prockey("E", &mut st, &tree, &mut ed), expected);
    }

    #[test]
    fn toggle_pause_reversed_percore_membytes() {
        let view = proc_view();
        let mut st = InputState::default();
        let mut ed = TextEdit::new(String::new(), false);
        for (key, action) in [
            ("u", Action::TogglePause),
            ("r", Action::ToggleReversed),
            ("c", Action::TogglePerCore),
            ("%", Action::ToggleMemBytes),
        ] {
            let mut expected = vec![action];
            expected.extend(run_pair(true, true));
            assert_eq!(prockey(key, &mut st, &view, &mut ed), expected);
        }
    }

    #[test]
    fn delete_clears_only_when_filter_nonempty() {
        let view = ViewState {
            filter_text: "x".to_string(),
            ..proc_view()
        };
        let mut st = InputState::default();
        let mut ed = TextEdit::new(String::new(), false);
        let mut expected = vec![Action::ClearFilter];
        expected.extend(run_pair(true, true));
        assert_eq!(prockey("delete", &mut st, &view, &mut ed), expected);
        // Empty filter ⇒ gate fails (:390) ⇒ keep_going ⇒ empty.
        assert_eq!(
            prockey("delete", &mut st, &proc_view(), &mut ed),
            Vec::<Action>::new()
        );
    }

    //? F follow maze (:359-379).

    #[test]
    fn follow_selects_selected_pid_first() {
        let view = ViewState {
            selected: 2,
            selected_pid: 42,
            followed_pid: 9,
            ..proc_view()
        };
        let mut st = InputState::default();
        let mut ed = TextEdit::new(String::new(), false);
        let mut expected = vec![Action::FollowSelected];
        expected.extend(run_pair(true, true));
        assert_eq!(prockey("F", &mut st, &view, &mut ed), expected);
    }

    #[test]
    fn follow_selects_detailed_pid_second() {
        let view = ViewState {
            show_detailed: true,
            show_detailed_geom: true,
            selected: 0,
            detailed_pid: 7,
            followed_pid: 5,
            ..proc_view()
        };
        let mut st = InputState::default();
        let mut ed = TextEdit::new(String::new(), false);
        let mut expected = vec![Action::FollowDetailed];
        expected.extend(run_pair(true, true));
        assert_eq!(prockey("F", &mut st, &view, &mut ed), expected);
    }

    #[test]
    fn unfollow_returns_to_followed_row() {
        // Arm 3a: should_return_to_followed ⇒ proc_selected=proc_followed.
        let view = ViewState {
            follow_process: true,
            followed_pid: 42,
            selected_pid: 42, // arm 1 fails: followed == selected
            selected: 2,
            should_return_to_followed: true,
            proc_followed: 4,
            ..proc_view()
        };
        let mut st = InputState::default();
        let mut ed = TextEdit::new(String::new(), false);
        let mut expected = vec![
            Action::Unfollow,
            sint("proc_selected", 4),
            sint("followed_pid", 0),
            sint("proc_followed", 0),
        ];
        expected.extend(run_pair(true, true));
        assert_eq!(prockey("F", &mut st, &view, &mut ed), expected);
    }

    #[test]
    fn unfollow_restores_detailed_pid() {
        // Arm 3b: show_detailed + followed==detailed ⇒ restore_detailed_pid.
        let view = ViewState {
            follow_process: true,
            followed_pid: 7,
            detailed_pid: 7,
            selected_pid: 7, // arm 1 fails
            selected: 0,
            show_detailed: true,
            show_detailed_geom: true,
            follow_detailed_cfg: true, // arm 2 fails: followed == detailed
            ..proc_view()
        };
        let mut st = InputState::default();
        let mut ed = TextEdit::new(String::new(), false);
        let mut expected = vec![
            Action::Unfollow,
            sint("restore_detailed_pid", 7),
            sint("followed_pid", 0),
            sint("proc_followed", 0),
        ];
        expected.extend(run_pair(true, true));
        assert_eq!(prockey("F", &mut st, &view, &mut ed), expected);
    }

    #[test]
    fn follow_with_no_arm_still_runs() {
        // No inner else in the F arm: no match ⇒ trailing runs anyway.
        let view = proc_view();
        let mut st = InputState::default();
        let mut ed = TextEdit::new(String::new(), false);
        assert_eq!(prockey("F", &mut st, &view, &mut ed), run_pair(true, true));
    }

    //? Mouse geometry (:393-460). Geom x=1,y=2,w=40,h=15 ⇒ y=2; in-box is
    // col∈[2,41), line∈[3,16); main zone col<39; page-up line=3, page-down
    // line=15; drag row line=2+2+scroll_pos.

    fn moused(col: i64, line: i64) -> InputState {
        InputState {
            mouse_pos: Some((col, line)),
            ..Default::default()
        }
    }

    #[test]
    fn click_selects_row_with_redraw() {
        // Row 2 selected from 0 ⇒ redraw=true (:418-419).
        let view = proc_view();
        let mut st = moused(5, 5);
        let mut ed = TextEdit::new(String::new(), false);
        let mut expected = vec![Action::ProcSelectRow { row: 2 }];
        expected.extend(run_pair(true, true));
        assert_eq!(prockey("mouse_click", &mut st, &view, &mut ed), expected);
    }

    #[test]
    fn click_breaks_follow_before_selecting() {
        // follow && !pause ⇒ Unfollow + zero Sets first (:421-425), then row.
        let view = ViewState {
            selected: 3,
            follow_process: true,
            ..proc_view()
        };
        let mut st = moused(5, 5);
        let mut ed = TextEdit::new(String::new(), false);
        let mut expected = vec![
            Action::Unfollow,
            sint("followed_pid", 0),
            sint("proc_followed", 0),
            Action::ProcSelectRow { row: 2 },
        ];
        expected.extend(run_pair(true, true));
        assert_eq!(prockey("mouse_click", &mut st, &view, &mut ed), expected);
    }

    #[test]
    fn click_same_row_recurses_into_enter() {
        // selected==row ⇒ process("enter") inline (:403-414); result equals a
        // direct enter dispatch (detail open for selected_pid=42).
        let view = ViewState {
            selected: 2,
            selected_pid: 42,
            ..proc_view()
        };
        let mut st = moused(5, 5);
        let mut ed = TextEdit::new(String::new(), false);
        let via_click = prockey("mouse_click", &mut st, &view, &mut ed);
        let mut st2 = InputState::default();
        let mut ed2 = TextEdit::new(String::new(), false);
        assert_eq!(via_click, prockey("enter", &mut st2, &view, &mut ed2));
        assert!(matches!(via_click[0], Action::ProcDetailOpen));
    }

    #[test]
    fn click_tree_xzone_recurses_into_space() {
        // tree + x_pos in (offset,4+offset) ⇒ process("space") (:405-411).
        // selected_depth=1 ⇒ offset=3; col=6,x=1 ⇒ x_pos=5 ∈ (3,7).
        let view = ViewState {
            tree: true,
            selected: 2,
            selected_pid: 42,
            selected_depth: 1,
            ..proc_view()
        };
        let mut st = moused(6, 5);
        let mut ed = TextEdit::new(String::new(), false);
        let mut expected = vec![
            Action::ExpandPid { pid: 42 },
            Action::CollapsePid { pid: 42 },
        ];
        expected.extend(run_pair(false, true));
        assert_eq!(prockey("mouse_click", &mut st, &view, &mut ed), expected);
    }

    #[test]
    fn click_banner_row_is_swallowed() {
        // banner row with a different row selected ⇒ bare return (:416-417).
        let view = ViewState {
            selected: 5,
            banner_shown: true,
            ..proc_view()
        };
        let mut st = moused(5, 15); // line == y+height-2
        let mut ed = TextEdit::new(String::new(), false);
        assert_eq!(
            prockey("mouse_click", &mut st, &view, &mut ed),
            Vec::<Action>::new()
        );
    }

    #[test]
    fn click_gutter_page_buttons_scroll() {
        let view = proc_view();
        let mut ed = TextEdit::new(String::new(), false);
        // Top gutter (y+1) ⇒ page_up; bottom (y+height-2) ⇒ page_down.
        let mut st = moused(39, 3);
        assert_eq!(
            prockey("mouse_click", &mut st, &view, &mut ed),
            scroll_actions(ScrollKey::PageUp, false)
        );
        let mut st = moused(39, 15);
        assert_eq!(
            prockey("mouse_click", &mut st, &view, &mut ed),
            scroll_actions(ScrollKey::PageDown, false)
        );
    }

    #[test]
    fn click_drag_row_arms_scroll_flag() {
        // line == y+2+scroll_pos ⇒ dragging_scroll + trailing runs (:436-438).
        let view = ViewState {
            scroll_pos: 4,
            ..proc_view()
        };
        let mut st = moused(39, 8);
        let mut ed = TextEdit::new(String::new(), false);
        let mut expected = vec![Action::SetDraggingScroll { on: true }];
        expected.extend(run_pair(true, false));
        assert_eq!(prockey("mouse_click", &mut st, &view, &mut ed), expected);
        assert!(st.dragging_scroll);
    }

    #[test]
    fn click_scrollbar_mousey_selects_start() {
        // Other gutter rows ⇒ selection("mousey{N}"), N=line-y-2 (:439-440).
        let view = proc_view();
        let mut st = moused(39, 10);
        let mut ed = TextEdit::new(String::new(), false);
        let mut expected = vec![Action::ProcScroll {
            key: ScrollKey::Row(6),
        }];
        expected.extend(run_pair(true, false));
        assert_eq!(prockey("mouse_click", &mut st, &view, &mut ed), expected);
    }

    #[test]
    fn click_outside_deselects_and_breaks_follow() {
        let view = ViewState {
            selected: 3,
            follow_process: true,
            ..proc_view()
        };
        let mut st = moused(100, 100);
        let mut ed = TextEdit::new(String::new(), false);
        let mut expected = vec![
            sint("proc_selected", 0),
            Action::Unfollow,
            sint("followed_pid", 0),
            sint("proc_followed", 0),
        ];
        expected.extend(run_pair(true, true));
        assert_eq!(prockey("mouse_click", &mut st, &view, &mut ed), expected);
    }

    #[test]
    fn click_outside_with_nothing_selected_runs_quiet() {
        // Arm matched but changed nothing ⇒ trailing runs, redraw=false.
        let view = proc_view();
        let mut st = moused(100, 100);
        let mut ed = TextEdit::new(String::new(), false);
        assert_eq!(
            prockey("mouse_click", &mut st, &view, &mut ed),
            run_pair(true, false)
        );
    }

    #[test]
    fn click_without_pos_keeps_going() {
        let view = proc_view();
        let mut st = InputState::default(); // mouse_pos None
        let mut ed = TextEdit::new(String::new(), false);
        assert_eq!(
            prockey("mouse_click", &mut st, &view, &mut ed),
            Vec::<Action>::new()
        );
    }

    //? Scroll keys + mouse scroll/drag (:452-457, :522-531).

    #[test]
    fn scroll_redraw_only_from_unselected() {
        // :529-530 old-side: selected==0 ⇒ redraw=true.
        let view = proc_view();
        let mut st = InputState::default();
        let mut ed = TextEdit::new(String::new(), false);
        assert_eq!(
            prockey("down", &mut st, &view, &mut ed),
            scroll_actions(ScrollKey::Down, true)
        );
        let sel = ViewState {
            selected: 3,
            ..proc_view()
        };
        assert_eq!(
            prockey("up", &mut st, &sel, &mut ed),
            scroll_actions(ScrollKey::Up, false)
        );
        assert_eq!(
            prockey("page_up", &mut st, &sel, &mut ed)[0],
            Action::ProcScroll {
                key: ScrollKey::PageUp
            }
        );
        assert_eq!(
            prockey("home", &mut st, &sel, &mut ed)[0],
            Action::ProcScroll {
                key: ScrollKey::Home
            }
        );
        assert_eq!(
            prockey("end", &mut st, &sel, &mut ed)[0],
            Action::ProcScroll {
                key: ScrollKey::End
            }
        );
    }

    #[test]
    fn vim_scroll_aliases_map() {
        let sel = ViewState {
            selected: 3,
            ..proc_view()
        };
        let mut st = InputState {
            vim_keys: true,
            ..Default::default()
        };
        let mut ed = TextEdit::new(String::new(), false);
        for (key, sk) in [
            ("j", ScrollKey::Down),
            ("k", ScrollKey::Up),
            ("g", ScrollKey::Home),
            ("G", ScrollKey::End),
        ] {
            assert_eq!(
                prockey(key, &mut st, &sel, &mut ed)[0],
                Action::ProcScroll { key: sk }
            );
        }
    }

    #[test]
    fn mouse_scroll_needs_in_box() {
        let view = proc_view();
        let mut ed = TextEdit::new(String::new(), false);
        let mut st = moused(5, 5);
        assert_eq!(
            prockey("mouse_scroll_up", &mut st, &view, &mut ed),
            scroll_actions(ScrollKey::ScrollUp, false)
        );
        let mut st = moused(5, 5);
        assert_eq!(
            prockey("mouse_scroll_down", &mut st, &view, &mut ed),
            scroll_actions(ScrollKey::ScrollDown, false)
        );
        // Outside the box ⇒ keep_going ⇒ empty.
        let mut st = moused(100, 100);
        assert_eq!(
            prockey("mouse_scroll_up", &mut st, &view, &mut ed),
            Vec::<Action>::new()
        );
    }

    #[test]
    fn mouse_drag_scrolls_only_when_armed() {
        let view = proc_view();
        let mut ed = TextEdit::new(String::new(), false);
        let mut st = InputState {
            mouse_pos: Some((39, 10)),
            dragging_scroll: true,
            ..Default::default()
        };
        let mut expected = vec![Action::ProcScroll {
            key: ScrollKey::Row(6),
        }];
        expected.extend(run_pair(true, false));
        assert_eq!(prockey("mouse_drag", &mut st, &view, &mut ed), expected);
        let mut st = moused(39, 10);
        assert_eq!(
            prockey("mouse_drag", &mut st, &view, &mut ed),
            Vec::<Action>::new()
        );
    }

    //? Detail open/close (:461-490).

    #[test]
    fn enter_guards_empty_selection() {
        let view = proc_view();
        let mut st = InputState::default();
        let mut ed = TextEdit::new(String::new(), false);
        assert_eq!(
            prockey("enter", &mut st, &view, &mut ed),
            Vec::<Action>::new()
        );
        assert_eq!(
            prockey("info_enter", &mut st, &view, &mut ed),
            Vec::<Action>::new()
        );
    }

    #[test]
    fn enter_opens_detail_for_new_pid() {
        let view = ViewState {
            selected: 2,
            selected_pid: 42,
            detailed_pid: 0,
            ..proc_view()
        };
        let mut st = InputState::default();
        let mut ed = TextEdit::new(String::new(), false);
        let mut expected = vec![
            Action::ProcDetailOpen,
            sint("detailed_pid", 42),
            sint("proc_last_selected", 2),
            sint("proc_selected", 0),
            sbool("show_detailed", true),
        ];
        expected.extend(run_pair(true, true));
        assert_eq!(prockey("enter", &mut st, &view, &mut ed), expected);
    }

    #[test]
    fn enter_opens_detail_and_follows_when_configured() {
        let view = ViewState {
            selected: 2,
            selected_pid: 42,
            follow_detailed_cfg: true,
            ..proc_view()
        };
        let mut st = InputState::default();
        let mut ed = TextEdit::new(String::new(), false);
        let mut expected = vec![
            Action::ProcDetailOpen,
            sint("detailed_pid", 42),
            sint("proc_last_selected", 2),
            sint("proc_selected", 0),
            Action::FollowSelected,
            sbool("show_detailed", true),
        ];
        expected.extend(run_pair(true, true));
        assert_eq!(prockey("info_enter", &mut st, &view, &mut ed), expected);
    }

    #[test]
    fn enter_closes_detail_restoring_last() {
        let view = ViewState {
            selected: 0,
            show_detailed: true,
            show_detailed_geom: true,
            detailed_pid: 7,
            proc_last_selected: 3,
            ..proc_view()
        };
        let mut st = InputState::default();
        let mut ed = TextEdit::new(String::new(), false);
        let mut expected = vec![
            Action::ProcDetailClose,
            sint("proc_selected", 3),
            sint("proc_last_selected", 0),
            sint("detailed_pid", 0),
            sbool("show_detailed", false),
        ];
        expected.extend(run_pair(true, true));
        assert_eq!(prockey("enter", &mut st, &view, &mut ed), expected);
    }

    #[test]
    fn enter_closes_detail_breaking_follow() {
        // follow_detailed_cfg + followed==detailed ⇒ restore + Unfollow.
        let view = ViewState {
            selected: 0,
            show_detailed: true,
            show_detailed_geom: true,
            detailed_pid: 7,
            follow_detailed_cfg: true,
            follow_process: true,
            followed_pid: 7,
            ..proc_view()
        };
        let mut st = InputState::default();
        let mut ed = TextEdit::new(String::new(), false);
        let mut expected = vec![
            Action::ProcDetailClose,
            sint("restore_detailed_pid", 7),
            Action::Unfollow,
            sint("followed_pid", 0),
            sint("proc_followed", 0),
            sint("proc_last_selected", 0),
            sint("detailed_pid", 0),
            sbool("show_detailed", false),
        ];
        expected.extend(run_pair(true, true));
        assert_eq!(prockey("enter", &mut st, &view, &mut ed), expected);
    }

    //? Tree expand/collapse (:491-503) and signal menus (:504-521).

    #[test]
    fn tree_space_expands_and_collapses() {
        // Space sets BOTH expand and collapse (:496-497); the collector
        // resolves collapse==expand as a toggle.
        let view = ViewState {
            tree: true,
            selected: 2,
            selected_pid: 42,
            ..proc_view()
        };
        let mut st = InputState::default();
        let mut ed = TextEdit::new(String::new(), false);
        let mut expected = vec![
            Action::ExpandPid { pid: 42 },
            Action::CollapsePid { pid: 42 },
        ];
        expected.extend(run_pair(false, true));
        assert_eq!(prockey("space", &mut st, &view, &mut ed), expected);
        assert_eq!(prockey("+", &mut st, &view, &mut ed), {
            let mut v = vec![Action::ExpandPid { pid: 42 }];
            v.extend(run_pair(false, true));
            v
        });
        assert_eq!(prockey("-", &mut st, &view, &mut ed), {
            let mut v = vec![Action::CollapsePid { pid: 42 }];
            v.extend(run_pair(false, true));
            v
        });
        assert_eq!(prockey("C", &mut st, &view, &mut ed), {
            let mut v = vec![Action::ToggleChildren { pid: 42 }];
            v.extend(run_pair(false, true));
            v
        });
        // Without tree ⇒ keep_going ⇒ empty; with tree but nothing
        // selected and not following detailed ⇒ empty.
        assert_eq!(
            prockey("+", &mut st, &proc_view(), &mut ed),
            Vec::<Action>::new()
        );
        let tree_idle = ViewState {
            tree: true,
            ..proc_view()
        };
        assert_eq!(
            prockey("+", &mut st, &tree_idle, &mut ed),
            Vec::<Action>::new()
        );
    }

    #[test]
    fn tree_expand_uses_followed_pid_for_detail() {
        // selected==0 + following detailed ⇒ pid=followed_pid (:495).
        let view = ViewState {
            tree: true,
            selected: 0,
            selected_pid: 1,
            follow_process: true,
            followed_pid: 7,
            detailed_pid: 7,
            show_detailed: true,
            show_detailed_geom: true,
            ..proc_view()
        };
        let mut st = InputState::default();
        let mut ed = TextEdit::new(String::new(), false);
        let mut expected = vec![Action::ExpandPid { pid: 7 }];
        expected.extend(run_pair(false, true));
        assert_eq!(prockey("+", &mut st, &view, &mut ed), expected);
    }

    #[test]
    fn signal_menus_gate_on_target_and_liveness() {
        // t ⇒ SIGTERM menu; s ⇒ choose; N ⇒ renice.
        let armed = ViewState {
            selected_pid: 42,
            ..proc_view()
        };
        let mut st = InputState::default();
        let mut ed = TextEdit::new(String::new(), false);
        assert_eq!(
            prockey("t", &mut st, &armed, &mut ed),
            vec![Action::ShowMenu {
                menu: MenuKind::SignalSend { sig: SIG_TERM }
            }]
        );
        assert_eq!(
            prockey("s", &mut st, &armed, &mut ed),
            vec![Action::ShowMenu {
                menu: MenuKind::SignalChoose
            }]
        );
        assert_eq!(
            prockey("N", &mut st, &armed, &mut ed),
            vec![Action::ShowMenu {
                menu: MenuKind::Renice
            }]
        );
        // Dead detailed with nothing selected ⇒ bare return (no menus).
        let dead = ViewState {
            show_detailed: true,
            show_detailed_geom: true,
            selected: 0,
            detailed_dead: true,
            ..proc_view()
        };
        assert_eq!(prockey("t", &mut st, &dead, &mut ed), Vec::<Action>::new());
        assert_eq!(prockey("s", &mut st, &dead, &mut ed), Vec::<Action>::new());
        assert_eq!(prockey("N", &mut st, &dead, &mut ed), Vec::<Action>::new());
        // No target at all ⇒ keep_going ⇒ empty.
        assert_eq!(
            prockey("t", &mut st, &proc_view(), &mut ed),
            Vec::<Action>::new()
        );
        // Vim K ⇒ SIGKILL (covered with kill split, re-assert menu shape).
        let mut vim = InputState {
            vim_keys: true,
            ..Default::default()
        };
        assert_eq!(
            prockey("K", &mut vim, &armed, &mut ed),
            vec![Action::ShowMenu {
                menu: MenuKind::SignalSend { sig: SIG_KILL }
            }]
        );
    }

    #[test]
    fn proc_hidden_skips_section() {
        // proc_shown=false ⇒ unknown keys yield nothing (Task 5 appends boxes).
        let mut st = InputState::default();
        let mut ed = TextEdit::new(String::new(), false);
        assert_eq!(
            process_key("e", &mut st, &ViewState::default(), &mut ed, 0),
            Vec::<Action>::new()
        );
    }

    //? Cpu branch (:542-570). HISTORY CONTRACT (see cpu_key docs): get()
    // pushes the key BEFORE process (:193-196), so st.history must already
    // INCLUDE the current key when calling process_key directly — every test
    // below pre-fills history ending with the pressed key. handle_key upholds
    // this ordering (covered by the integration test at the bottom).

    fn cpu_view(update_ms: i64) -> ViewState {
        ViewState {
            proc_shown: false,
            cpu_shown: true,
            update_ms,
            ..proc_view()
        }
    }

    fn histed(keys: &[&str], last_press_ms: u64) -> InputState {
        InputState {
            history: keys.iter().map(|s| s.to_string()).collect(),
            last_press_ms,
            ..Default::default()
        }
    }

    fn cpu_run() -> Action {
        Action::Run {
            target: RunTarget::Cpu,
            no_update: true,
            redraw: true,
        }
    }

    #[test]
    fn cpu_accel_up_triple_plus() {
        // update 2000 (<= 86399000) + inside 200ms window + history all "+"
        // ⇒ +1000 (:549-552). last_press advances to now.
        let view = cpu_view(2000);
        let mut st = histed(&["+", "+", "+"], 1000);
        let mut ed = TextEdit::new(String::new(), false);
        assert_eq!(
            process_key("+", &mut st, &view, &mut ed, 1100),
            vec![Action::SetUpdateMs { ms: 3000 }, cpu_run()]
        );
        assert_eq!(st.last_press_ms, 1100);
    }

    #[test]
    fn cpu_single_up_without_window_or_history() {
        // Fresh press (last=0, far outside the window) ⇒ +100 even with a
        // clean single-"+" history.
        let view = cpu_view(2000);
        let mut st = histed(&["+"], 0);
        let mut ed = TextEdit::new(String::new(), false);
        assert_eq!(
            process_key("+", &mut st, &view, &mut ed, 1000),
            vec![Action::SetUpdateMs { ms: 2100 }, cpu_run()]
        );
        // Polluted history (a stray key) ⇒ +100 even inside the window.
        let mut st = histed(&["x", "+"], 950);
        assert_eq!(
            process_key("+", &mut st, &view, &mut ed, 1000),
            vec![Action::SetUpdateMs { ms: 2100 }, cpu_run()]
        );
    }

    #[test]
    fn cpu_equals_rides_plus_arm_without_accel() {
        // "=" shares the + arm (:548) but history ends with "=" ⇒ all_of("+")
        // fails ⇒ always +100, even inside the window.
        let view = cpu_view(2000);
        let mut st = histed(&["=", "=", "="], 950);
        let mut ed = TextEdit::new(String::new(), false);
        assert_eq!(
            process_key("=", &mut st, &view, &mut ed, 1000),
            vec![Action::SetUpdateMs { ms: 2100 }, cpu_run()]
        );
        assert_eq!(st.last_press_ms, 1000); // handled ⇒ press recorded
    }

    #[test]
    fn cpu_accel_down_and_single_down() {
        let view = cpu_view(5000);
        let mut st = histed(&["-", "-"], 1000);
        let mut ed = TextEdit::new(String::new(), false);
        assert_eq!(
            process_key("-", &mut st, &view, &mut ed, 1100),
            vec![Action::SetUpdateMs { ms: 4000 }, cpu_run()]
        );
        // Below the 2000 accel sub-gate (:557) ⇒ single step even with
        // window + clean history.
        let low = cpu_view(500);
        let mut st = histed(&["-", "-"], 1000);
        assert_eq!(
            process_key("-", &mut st, &low, &mut ed, 1100),
            vec![Action::SetUpdateMs { ms: 400 }, cpu_run()]
        );
    }

    #[test]
    fn cpu_gates_and_bounds() {
        let mut ed = TextEdit::new(String::new(), false);
        // Below floor: update 100 (< 200, :556) ⇒ keep_going ⇒ empty, and the
        // rejected press must NOT touch last_press_ms.
        let floor = cpu_view(100);
        let mut st = histed(&["-"], 777);
        assert_eq!(
            process_key("-", &mut st, &floor, &mut ed, 1000),
            Vec::<Action>::new()
        );
        assert_eq!(st.last_press_ms, 777);
        // Above ceiling: update 86400000 (> 86399900, :548) ⇒ empty.
        let ceil = cpu_view(86_400_000);
        let mut st = histed(&["+"], 777);
        assert_eq!(
            process_key("+", &mut st, &ceil, &mut ed, 1000),
            Vec::<Action>::new()
        );
        assert_eq!(st.last_press_ms, 777);
        // Upper edge: 86399900 still handled, but past the accel sub-gate
        // (> 86399000) ⇒ +100 lands exactly on 86400000.
        let edge = cpu_view(86_399_900);
        let mut st = histed(&["+", "+", "+"], 950);
        assert_eq!(
            process_key("+", &mut st, &edge, &mut ed, 1000),
            vec![Action::SetUpdateMs { ms: 86_400_000 }, cpu_run()]
        );
        // Hidden box ⇒ empty (gate :542).
        let mut st = InputState::default();
        assert_eq!(
            process_key("+", &mut st, &ViewState::default(), &mut ed, 1000),
            Vec::<Action>::new()
        );
    }

    //? Mem branch (:573-592).

    fn mem_view() -> ViewState {
        ViewState {
            proc_shown: false,
            mem_shown: true,
            ..proc_view()
        }
    }

    #[test]
    fn mem_io_and_disks_arms() {
        let view = mem_view();
        let mut st = InputState::default();
        let mut ed = TextEdit::new(String::new(), false);
        // i ⇒ flip io_mode + Run{mem,true,true} (:578-579).
        assert_eq!(
            process_key("i", &mut st, &view, &mut ed, 0),
            vec![
                Action::ToggleIoMode,
                Action::Run {
                    target: RunTarget::Mem,
                    no_update: true,
                    redraw: true,
                },
            ]
        );
        // d ⇒ flip show_disks + calcSizes + Run{mem,false,true} (:581-584).
        assert_eq!(
            process_key("d", &mut st, &view, &mut ed, 0),
            vec![
                Action::ToggleDisks,
                Action::RecalcLayout,
                Action::Run {
                    target: RunTarget::Mem,
                    no_update: false,
                    redraw: true,
                },
            ]
        );
        // Anything else ⇒ keep_going ⇒ empty.
        assert_eq!(
            process_key("x", &mut st, &view, &mut ed, 0),
            Vec::<Action>::new()
        );
        // Hidden box ⇒ empty (gate :573).
        assert_eq!(
            process_key("i", &mut st, &ViewState::default(), &mut ed, 0),
            Vec::<Action>::new()
        );
    }

    //? Net branch (:595-641).

    fn net_view() -> ViewState {
        ViewState {
            proc_shown: false,
            net_shown: true,
            net_interfaces: vec!["eth0".to_string(), "wlan0".to_string()],
            net_selected: "eth0".to_string(),
            ..proc_view()
        }
    }

    fn net_run() -> Action {
        Action::Run {
            target: RunTarget::Net,
            no_update: true,
            redraw: true,
        }
    }

    #[test]
    fn net_cycle_dirs_split_prev_next() {
        // b=prev(-1), n=next(+1) per :604-609 (b decrements, n increments).
        // Index wrap math (v_index miss, ±1 wrap) is transcribed in the P3
        // sink per the CycleIface contract — here only the direction split
        // and the trailing Run are pinned.
        let view = net_view();
        let mut st = InputState::default();
        let mut ed = TextEdit::new(String::new(), false);
        assert_eq!(
            process_key("b", &mut st, &view, &mut ed, 0),
            vec![Action::CycleIface { dir: -1 }, net_run()]
        );
        assert_eq!(
            process_key("n", &mut st, &view, &mut ed, 0),
            vec![Action::CycleIface { dir: 1 }, net_run()]
        );
    }

    #[test]
    fn net_sync_auto_zero_arms() {
        let view = net_view();
        let mut st = InputState::default();
        let mut ed = TextEdit::new(String::new(), false);
        // y/a ⇒ flip + Run{net,true,true} (:614-621).
        assert_eq!(
            process_key("y", &mut st, &view, &mut ed, 0),
            vec![Action::ToggleNetSync, net_run()]
        );
        assert_eq!(
            process_key("a", &mut st, &view, &mut ed, 0),
            vec![Action::ToggleNetAuto, net_run()]
        );
        // z ⇒ offsets re-seed + Run{net,false,true} (:622-633). Offset math
        // is a sink duty (ZeroNetOffsets contract).
        assert_eq!(
            process_key("z", &mut st, &view, &mut ed, 0),
            vec![
                Action::ZeroNetOffsets,
                Action::Run {
                    target: RunTarget::Net,
                    no_update: false,
                    redraw: true,
                },
            ]
        );
        // Anything else ⇒ keep_going ⇒ empty; hidden box ⇒ empty (:595).
        assert_eq!(
            process_key("x", &mut st, &view, &mut ed, 0),
            Vec::<Action>::new()
        );
        assert_eq!(
            process_key("n", &mut st, &ViewState::default(), &mut ed, 0),
            Vec::<Action>::new()
        );
    }

    //? handle_key glue (get :123-199 + process :214-647).

    #[test]
    fn handle_key_caps_history_at_50() {
        // 55 single-char pushes ⇒ len 50, front is the 6th push ("5").
        let mut st = InputState::default();
        let mut ed = TextEdit::new(String::new(), false);
        let view = ViewState::default();
        let nomaps: Vec<MouseMap> = vec![];
        for i in 0..55 {
            handle_key(
                &(i % 10).to_string(),
                &nomaps,
                &nomaps,
                &mut st,
                &view,
                &mut ed,
                0,
            );
        }
        assert_eq!(st.history.len(), 50);
        assert_eq!(st.history.front().unwrap(), "5");
        assert_eq!(st.history.back().unwrap(), "4");
    }

    #[test]
    fn handle_key_empty_raw_yields_empty() {
        let mut st = InputState::default();
        let mut ed = TextEdit::new(String::new(), false);
        let nomaps: Vec<MouseMap> = vec![];
        assert_eq!(
            handle_key(
                "",
                &nomaps,
                &nomaps,
                &mut st,
                &ViewState::default(),
                &mut ed,
                0
            ),
            Vec::<Action>::new()
        );
    }

    #[test]
    fn handle_key_threads_mouse_pos_always() {
        // Click raw ⇒ Some; plain key ⇒ None (stale pos must never linger).
        let mut st = InputState::default();
        let mut ed = TextEdit::new(String::new(), false);
        let nomaps: Vec<MouseMap> = vec![];
        handle_key(
            "\x1b[<0;12;6M",
            &nomaps,
            &nomaps,
            &mut st,
            &ViewState::default(),
            &mut ed,
            0,
        );
        assert_eq!(st.mouse_pos, Some((12, 6)));
        handle_key(
            "a",
            &nomaps,
            &nomaps,
            &mut st,
            &ViewState::default(),
            &mut ed,
            0,
        );
        assert_eq!(st.mouse_pos, None);
    }

    #[test]
    fn handle_key_accel_integration_orders_history_first() {
        // Three rapid "+" through handle_key with controlled now_ms: the
        // third sees history ["+","+","+"] (current INCLUDED, :193-196
        // ordering) + window hit ⇒ SetUpdateMs(+1000). Proves the glue
        // preserves the order cpu_key depends on.
        let view = cpu_view(2000);
        let mut st = InputState::default();
        let mut ed = TextEdit::new(String::new(), false);
        let nomaps: Vec<MouseMap> = vec![];
        handle_key("+", &nomaps, &nomaps, &mut st, &view, &mut ed, 1000);
        handle_key("+", &nomaps, &nomaps, &mut st, &view, &mut ed, 1100);
        let third = handle_key("+", &nomaps, &nomaps, &mut st, &view, &mut ed, 1200);
        assert_eq!(third, vec![Action::SetUpdateMs { ms: 3000 }, cpu_run()]);
        assert_eq!(st.last_press_ms, 1200);
    }
}
