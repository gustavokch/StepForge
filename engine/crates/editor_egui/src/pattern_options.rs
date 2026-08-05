//! Phase 3 §T T12 — PatternOptionsSheet overlay. Port of the iOS
//! `app/StepForge/Features/Performance/PatternOptionsSheet.swift`.
//!
//! Pure UI (V4): edits a per-pattern [`FollowAction`] draft and emits a single
//! [`Command::SetFollowAction`] on Save (the WHOLE struct — `after_loops` +
//! `action` — atomically, matching the command shape). Cancel/Esc/outside-click
//! dismiss without emit. Like the T11 overlays it is a floating `egui::Area` +
//! `Frame::popup` (NOT `egui::Window`: a Window's title-bar absorbs the first
//! click), with the shared [`crate::overlay::should_dismiss`] + open-frame guard.
//!
//! iOS parity + the requested editor enhancement: the action picker exposes ALL
//! six `FollowActionType` variants incl. `PlaySpecific` (iOS omits it). The
//! draft carries a Copy [`ActionDraft`] (no `Uuid` — the model's
//! `PlaySpecific(Uuid)` is never named or constructed here); for `PlaySpecific`
//! a target-pattern picker resolves the chosen slot's `Pattern.id` to a `Uuid`
//! on Save via [`resolve_action`].

use egui::{ComboBox, Context, Id, Pos2, RichText, Slider, Ui};
use sequencer_engine::command::Command;
use sequencer_engine::models::{FollowAction, FollowActionType, Pattern, PATTERN_SLOTS};

#[cfg(test)]
use egui::Rect;

use crate::grid::{PRIMARY, SURFACE_HIGH, TEXT_MUTED, TEXT_PRIMARY};
use crate::{CommandSink, UiState};

fn pattern_options_id() -> Id {
    Id::new("stepforge.pattern_options")
}
#[cfg(test)]
fn save_rect_id() -> Id {
    Id::new("stepforge.pattern_options.save")
}
#[cfg(test)]
fn cancel_rect_id() -> Id {
    Id::new("stepforge.pattern_options.cancel")
}
#[cfg(test)]
fn window_rect_id() -> Id {
    Id::new("stepforge.pattern_options.window")
}
#[cfg(test)]
fn clip_rect_id() -> Id {
    // Vec<(ActionClip, Rect)> — the four whole-pattern clipboard buttons.
    Id::new("stepforge.pattern_options.clip")
}

/// Per-field edit-focus flags, recorded at the end of one frame and read at the
/// top of the next so the per-frame re-seed (#42) skips whichever field the user
/// is actively editing (slider / action combobox / target picker).
#[derive(Clone, Copy, Default)]
struct FocusFlags {
    loops: bool,
    action: bool,
    target: bool,
}
fn focus_id() -> Id {
    Id::new("stepforge.pattern_options.focus")
}
fn read_focus(ctx: &Context) -> FocusFlags {
    ctx.data(|d| d.get_temp::<FocusFlags>(focus_id()).unwrap_or_default())
}
fn write_focus(ctx: &Context, f: FocusFlags) {
    ctx.data_mut(|d| d.insert_temp(focus_id(), f));
}

/// Test-facing tag for the four whole-pattern clipboard buttons (rect-lookup key).
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ActionClip {
    Cut,
    Copy,
    Paste,
    Clear,
}

/// Copy mirror of [`FollowActionType`] for the draft state (the model variant is
/// not `Copy` — `PlaySpecific(Uuid)` — and `Uuid` is not re-exported from
/// `models`, so this enum never names or constructs one). Mapped to the real
/// variant on Save by [`resolve_action`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum ActionDraft {
    #[default]
    None,
    PlayNext,
    PlaySpecific,
    PlayPrevious,
    Stop,
    PlayRandom,
}

const ACTION_OPTIONS: [(ActionDraft, &str); 6] = [
    (ActionDraft::None, "None"),
    (ActionDraft::PlayNext, "Play Next"),
    (ActionDraft::PlaySpecific, "Play Specific"),
    (ActionDraft::PlayPrevious, "Play Previous"),
    (ActionDraft::Stop, "Stop"),
    (ActionDraft::PlayRandom, "Play Random"),
];

fn draft_label(d: ActionDraft) -> &'static str {
    ACTION_OPTIONS
        .iter()
        .find(|(v, _)| *v == d)
        .map(|(_, l)| *l)
        .unwrap_or("None")
}

/// A fixed-width whole-pattern clipboard button. Returns its [`egui::Response`]
/// — the caller records the (tagged) rect + dispatches the command, so this
/// helper carries no `cfg(test)`-only params (mirrors `action_drawer::action_btn`).
fn clip_btn(ui: &mut Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).color(TEXT_PRIMARY))
            .fill(SURFACE_HIGH)
            .min_size(egui::Vec2::new(60.0, 0.0)),
    )
}

/// Resolve the draft to a real [`FollowActionType`].
///
/// `PlaySpecific` reads the chosen target slot's `Pattern.id` (a `Uuid`).
/// `specific_target` uses [`PATTERN_SLOTS`] as a sentinel meaning "no target
/// chosen"; a sentinel, out-of-range, or empty slot cannot yield a valid id,
/// so the action collapses to [`FollowActionType::None`]. This never emits
/// `PlaySpecific(Uuid::nil())` — the engine searches for a nil-id slot, finds
/// none, and falls back to active, which would make the saved action a silent
/// no-op.
fn resolve_action(
    draft: ActionDraft,
    specific_target: usize,
    patterns: &[Option<Pattern>; PATTERN_SLOTS],
) -> FollowActionType {
    match draft {
        ActionDraft::None => FollowActionType::None,
        ActionDraft::PlayNext => FollowActionType::PlayNext,
        ActionDraft::PlaySpecific => match patterns
            .get(specific_target)
            .and_then(|p| p.as_ref())
            .map(|p| p.id)
        {
            Some(id) => FollowActionType::PlaySpecific(id),
            // Sentinel / out-of-range / empty slot → no resolvable target.
            // Emit None rather than PlaySpecific(nil) (silent engine no-op).
            None => FollowActionType::None,
        },
        ActionDraft::PlayPrevious => FollowActionType::PlayPrevious,
        ActionDraft::Stop => FollowActionType::Stop,
        ActionDraft::PlayRandom => FollowActionType::PlayRandom,
    }
}

/// Map a real action back to the draft + seed the `PlaySpecific` target index.
///
/// For `PlaySpecific(id)`, returns the slot index whose `Pattern.id` matches
/// `id`. If no slot matches (the slot was cleared or the id never existed),
/// returns [`PATTERN_SLOTS`] — the sentinel "no target" value — rather than
/// silently seeding slot 0 (which would redirect a Save to P1 with no warning).
/// The sentinel renders as a placeholder in the target ComboBox and resolves
/// to [`FollowActionType::None`] on Save via [`resolve_action`].
fn action_to_draft(
    action: &FollowActionType,
    patterns: &[Option<Pattern>; PATTERN_SLOTS],
) -> (ActionDraft, usize) {
    match action {
        FollowActionType::None => (ActionDraft::None, 0),
        FollowActionType::PlayNext => (ActionDraft::PlayNext, 0),
        FollowActionType::PlayPrevious => (ActionDraft::PlayPrevious, 0),
        FollowActionType::Stop => (ActionDraft::Stop, 0),
        FollowActionType::PlayRandom => (ActionDraft::PlayRandom, 0),
        FollowActionType::PlaySpecific(id) => {
            let target = (0..PATTERN_SLOTS)
                .find(|&i| patterns[i].as_ref().is_some_and(|p| p.id == *id))
                .unwrap_or(PATTERN_SLOTS); // sentinel: no resolvable target
            (ActionDraft::PlaySpecific, target)
        }
    }
}

/// Widget-local draft state. `Clone + Copy` (all fields are `Copy` — the `Uuid`
/// is NOT carried here, only an [`ActionDraft`] + a target slot index). Seeded
/// from the target pattern's current follow_action on [`open`].
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PatternOptionsState {
    pub(crate) target: Option<usize>,
    pub(crate) opened_at: u64,
    after_loops: u32,
    action: ActionDraft,
    /// For `PlaySpecific`: the chosen target slot index, or [`PATTERN_SLOTS`]
    /// as a sentinel meaning "no target chosen / target unresolvable." Resolved
    /// to its `Pattern.id` on Save by [`resolve_action`] (sentinel →
    /// [`FollowActionType::None`], never `PlaySpecific(nil)`).
    specific_target: usize,
}

pub(crate) fn read(ctx: &Context) -> PatternOptionsState {
    ctx.data(|d| {
        d.get_temp::<PatternOptionsState>(pattern_options_id())
            .unwrap_or_default()
    })
}
fn write(ctx: &Context, f: impl FnOnce(&mut PatternOptionsState)) {
    ctx.data_mut(|d| f(d.get_temp_mut_or_default(pattern_options_id())));
}

/// Open the sheet for `pattern_idx`, seeding the draft from that pattern's
/// current follow_action (V4: read-only over `state` here).
pub(crate) fn open(ctx: &Context, pattern_idx: usize, state: &UiState) {
    let frame = crate::frame_nr(ctx);
    // Reset edit-focus so the first-frame re-seed (#42) isn't skipped by stale
    // focus left from a previously open sheet (`close()` clears the target, not
    // focus). `open()` already seeds the draft from the live pattern below, so
    // this is insurance against a one-frame stale display if an external
    // SetFollowAction lands between `open()` and the first render.
    write_focus(ctx, FocusFlags::default());
    let (after_loops, action, specific_target) = match state.session.as_deref() {
        Some(s) => match s.patterns.get(pattern_idx).and_then(Option::as_ref) {
            Some(p) => {
                let (draft, tgt) = action_to_draft(&p.follow_action.action, &s.patterns);
                (p.follow_action.after_loops, draft, tgt)
            }
            None => (1, ActionDraft::None, 0),
        },
        None => (1, ActionDraft::None, 0),
    };
    write(ctx, |s| {
        s.target = Some(pattern_idx);
        s.opened_at = frame;
        s.after_loops = after_loops;
        s.action = action;
        s.specific_target = specific_target;
    });
}

pub(crate) fn close(ctx: &Context) {
    write(ctx, |s| s.target = None);
}

/// Render the sheet if open. No-op (no panic) when closed, no session, or the
/// target slot was cleared under an open sheet (stale-target auto-close).
pub(crate) fn render_pattern_options(ctx: &Context, ui_state: &UiState, sink: &impl CommandSink) {
    let mut st = read(ctx);
    let Some(pattern_idx) = st.target else {
        return;
    };
    let patterns = match ui_state.session.as_deref() {
        Some(s) => &s.patterns,
        None => {
            close(ctx);
            return;
        }
    };
    if pattern_idx >= PATTERN_SLOTS || patterns[pattern_idx].is_none() {
        close(ctx);
        return;
    }

    // #42: re-sync the draft from the live target pattern every frame so an
    // external SetFollowAction (preset/automation/undo swap) is reflected
    // instead of leaving the sheet lying. Skip the field that held edit focus
    // last frame so an in-progress slider/combobox edit isn't clobbered.
    // #36: clamp after_loops to the slider's 1..=16 range on seed so an
    // out-of-range session normalizes instead of desyncing (slider=16,label=32).
    let focused = read_focus(ctx);
    if let Some(p) = patterns.get(pattern_idx).and_then(Option::as_ref) {
        let (live_action, live_target) = action_to_draft(&p.follow_action.action, patterns);
        let live_loops = p.follow_action.after_loops.clamp(1, 16);
        if !focused.loops {
            st.after_loops = live_loops;
        }
        if !focused.action {
            st.action = live_action;
        }
        if !focused.target {
            st.specific_target = live_target;
        }
    }

    #[cfg(test)]
    ctx.data_mut(|d| {
        *d.get_temp_mut_or_default::<Option<Rect>>(window_rect_id()) = None;
        *d.get_temp_mut_or_default::<Option<Rect>>(save_rect_id()) = None;
        *d.get_temp_mut_or_default::<Option<Rect>>(cancel_rect_id()) = None;
    });

    let mut focus = FocusFlags::default(); // assigned inside the closure below

    let area = egui::Area::new(Id::new("stepforge.pattern_options"))
        .order(egui::Order::Foreground)
        .current_pos(Pos2::new(40.0, 60.0))
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(320.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(format!("Pattern P{}", pattern_idx + 1)).strong());
                    ui.separator();

                    // Whole-pattern clipboard: Cut/Copy/Paste/Clear. Each emits
                    // its command; the sheet STAYS OPEN — the per-frame re-seed
                    // (#42) repairs the draft next frame, so close-on-emit is no
                    // longer needed (and was the cause of #39: an empty Paste
                    // silently dismissed the sheet). The clipboard is engine-side
                    // (the editor cannot see whether it is empty), so Paste is
                    // always enabled and no-ops when the clipboard is empty.
                    #[cfg(test)]
                    ctx.data_mut(|d| {
                        d.get_temp_mut_or_default::<Vec<(ActionClip, Rect)>>(clip_rect_id())
                            .clear();
                    });
                    ui.horizontal(|ui| {
                        let cut = clip_btn(ui, "Cut");
                        #[cfg(test)]
                        ctx.data_mut(|d| {
                            d.get_temp_mut_or_default::<Vec<(ActionClip, Rect)>>(clip_rect_id())
                                .push((ActionClip::Cut, cut.rect))
                        });
                        if cut.clicked() {
                            sink.push(Command::CutPattern { index: pattern_idx });
                        }
                        let copy = clip_btn(ui, "Copy");
                        #[cfg(test)]
                        ctx.data_mut(|d| {
                            d.get_temp_mut_or_default::<Vec<(ActionClip, Rect)>>(clip_rect_id())
                                .push((ActionClip::Copy, copy.rect))
                        });
                        if copy.clicked() {
                            sink.push(Command::CopyPattern { index: pattern_idx });
                        }
                        let paste = clip_btn(ui, "Paste");
                        #[cfg(test)]
                        ctx.data_mut(|d| {
                            d.get_temp_mut_or_default::<Vec<(ActionClip, Rect)>>(clip_rect_id())
                                .push((ActionClip::Paste, paste.rect))
                        });
                        if paste.clicked() {
                            sink.push(Command::PastePattern { index: pattern_idx });
                        }
                        let clear = clip_btn(ui, "Clear");
                        #[cfg(test)]
                        ctx.data_mut(|d| {
                            d.get_temp_mut_or_default::<Vec<(ActionClip, Rect)>>(clip_rect_id())
                                .push((ActionClip::Clear, clear.rect))
                        });
                        if clear.clicked() {
                            sink.push(Command::ClearPattern { index: pattern_idx });
                        }
                    });
                    ui.separator();

                    // Follow Action — After Loops (1..=16, iOS Stepper parity).
                    let loops_resp = ui.horizontal(|ui| {
                        ui.label(RichText::new("After Loops").color(TEXT_MUTED));
                        let mut v = st.after_loops as i32;
                        // Range-bounded; `changed()` clamps to 1..=16 on edit.
                        let sr = ui.add(Slider::new(&mut v, 1..=16).text("loops"));
                        if sr.changed() {
                            st.after_loops = v.clamp(1, 16) as u32;
                        }
                        ui.label(RichText::new(format!("{}", st.after_loops)).color(TEXT_PRIMARY));
                        sr
                    })
                    .inner;

                    // Action type — all six variants (incl PlaySpecific; editor enhancement).
                    let action_resp = ui.horizontal(|ui| {
                        ui.label(RichText::new("Action").color(TEXT_MUTED));
                        ComboBox::from_id_salt("stepforge.pattern_options.action")
                            .selected_text(draft_label(st.action))
                            .show_ui(ui, |ui| {
                                for (variant, label) in ACTION_OPTIONS {
                                    ui.selectable_value(&mut st.action, variant, label);
                                }
                            })
                    })
                    .inner
                    .response;

                    // PlaySpecific target picker (only when PlaySpecific selected).
                    // `specific_target == PATTERN_SLOTS` is the sentinel "no
                    // target" value; an empty slot is also not a valid pick, so
                    // either renders the "Select target…" placeholder. The
                    // dropdown lists only filled slots; egui's `selectable_value`
                    // writes `tgt` only on a real selection, so assigning back
                    // unconditionally preserves the sentinel when the user
                    // hasn't picked — do NOT clamp (`.min(PATTERN_SLOTS - 1)`
                    // would silently turn the sentinel into the last valid slot).
                    let target_resp = if st.action == ActionDraft::PlaySpecific {
                        let r = ui.horizontal(|ui| {
                            ui.label(RichText::new("Target").color(TEXT_MUTED));
                            let mut tgt = st.specific_target;
                            let selected_text = patterns
                                .get(tgt)
                                .and_then(|p| p.as_ref())
                                .map(|_| format!("P{}", tgt + 1))
                                .unwrap_or_else(|| "Select target…".to_string());
                            let resp = ComboBox::from_id_salt("stepforge.pattern_options.target")
                                .selected_text(selected_text)
                                .show_ui(ui, |ui| {
                                    for (i, opt) in patterns.iter().enumerate() {
                                        if opt.is_some() {
                                            ui.selectable_value(&mut tgt, i, format!("P{}", i + 1));
                                        }
                                    }
                                });
                            st.specific_target = tgt;
                            resp
                        })
                        .inner
                        .response;
                        Some(r)
                    } else {
                        None
                    };

                    ui.separator();
                    ui.horizontal(|ui| {
                        let save = ui.add(
                            egui::Button::new(RichText::new("Save").strong().color(PRIMARY))
                                .fill(SURFACE_HIGH)
                                .min_size(egui::Vec2::new(80.0, 0.0)),
                        );
                        #[cfg(test)]
                        ctx.data_mut(|d| {
                            *d.get_temp_mut_or_default::<Option<Rect>>(save_rect_id()) =
                                Some(save.rect)
                        });
                        if save.clicked() {
                            let action = resolve_action(st.action, st.specific_target, patterns);
                            sink.push(Command::SetFollowAction {
                                pattern_idx,
                                action: FollowAction {
                                    after_loops: st.after_loops,
                                    action,
                                },
                            });
                            close(ctx);
                        }
                        let cancel = ui.button("Cancel");
                        #[cfg(test)]
                        ctx.data_mut(|d| {
                            *d.get_temp_mut_or_default::<Option<Rect>>(cancel_rect_id()) =
                                Some(cancel.rect)
                        });
                        if cancel.clicked() {
                            close(ctx);
                        }
                    });

                    // Record this frame's edit-focus so next frame's re-seed
                    // (#42) skips whichever field the user is editing.
                    focus = FocusFlags {
                        loops: loops_resp.has_focus(),
                        action: action_resp.has_focus(),
                        target: target_resp.map(|r| r.has_focus()).unwrap_or(false),
                    };
                });
            });
        });

    // Persist the draft back (slider/combobox edits survive to next frame).
    write(ctx, |s| {
        s.after_loops = st.after_loops;
        s.action = st.action;
        s.specific_target = st.specific_target;
    });

    let rect = area.response.rect;
    #[cfg(test)]
    ctx.data_mut(|d| *d.get_temp_mut_or_default::<Option<Rect>>(window_rect_id()) = Some(rect));
    // Persist this frame's edit-focus for next frame's re-seed (#42).
    write_focus(ctx, focus);
    // `opened_at == frame_nr` only on the opening frame; the gear `…` click that
    // opened the sheet is still "primary_clicked" then, so skip the outside-click
    // dismiss branch that frame (Esc still dismisses). Same guard as the T11
    // overlays.
    let is_open_frame = crate::frame_nr(ctx) == st.opened_at;
    if crate::overlay::should_dismiss(ctx, rect, is_open_frame) {
        close(ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Harness;
    use sequencer_engine::models::{FollowAction, FollowActionType, Session};
    use std::sync::Arc;

    /// A session whose pattern 1 already carries a seeded follow_action so the
    /// sheet's draft-seed path is exercised.
    fn fixture_with_follow(idx: usize, fa: FollowAction) -> UiState {
        let mut st = UiState {
            session: Some(Arc::new(Session::default())),
            ..Default::default()
        };
        {
            let s = Arc::make_mut(st.session.as_mut().unwrap());
            if let Some(Some(p)) = s.patterns.get_mut(idx) {
                p.follow_action = fa;
            }
        }
        st
    }

    fn open_for(idx: usize, state: UiState) -> Harness {
        let h = Harness::new(state);
        crate::write_mode(&h.ctx, crate::AppMode::Performance);
        // open() seeds the draft from the live mirror.
        crate::pattern_options::open(&h.ctx, idx, &h.state);
        h
    }

    fn save_center(ctx: &Context) -> egui::Pos2 {
        ctx.data(|d| d.get_temp::<Option<Rect>>(save_rect_id()))
            .unwrap_or_default()
            .map(|r| r.center())
            .expect("save rect recorded")
    }
    fn cancel_center(ctx: &Context) -> egui::Pos2 {
        ctx.data(|d| d.get_temp::<Option<Rect>>(cancel_rect_id()))
            .unwrap_or_default()
            .map(|r| r.center())
            .expect("cancel rect recorded")
    }
    fn window_rect(ctx: &Context) -> Option<Rect> {
        ctx.data(|d| d.get_temp::<Option<Rect>>(window_rect_id()))
            .unwrap_or_default()
    }

    // ---- pure oracle tests ----

    #[test]
    fn resolve_action_all_drafts() {
        let p: [Option<Pattern>; PATTERN_SLOTS] = std::array::from_fn(|_| Some(Pattern::default()));
        assert!(matches!(
            resolve_action(ActionDraft::None, 0, &p),
            FollowActionType::None
        ));
        assert!(matches!(
            resolve_action(ActionDraft::PlayNext, 0, &p),
            FollowActionType::PlayNext
        ));
        assert!(matches!(
            resolve_action(ActionDraft::PlayPrevious, 0, &p),
            FollowActionType::PlayPrevious
        ));
        assert!(matches!(
            resolve_action(ActionDraft::Stop, 0, &p),
            FollowActionType::Stop
        ));
        assert!(matches!(
            resolve_action(ActionDraft::PlayRandom, 0, &p),
            FollowActionType::PlayRandom
        ));
        // PlaySpecific carries the chosen slot's id.
        let target_id = p[5].as_ref().unwrap().id;
        match resolve_action(ActionDraft::PlaySpecific, 5, &p) {
            FollowActionType::PlaySpecific(id) => assert_eq!(id, target_id),
            other => panic!("expected PlaySpecific, got {other:?}"),
        }
    }

    #[test]
    fn action_to_draft_round_trip_play_specific() {
        let mut p: [Option<Pattern>; PATTERN_SLOTS] =
            std::array::from_fn(|_| Some(Pattern::default()));
        let id3 = p[3].as_ref().unwrap().id;
        let (draft, tgt) = action_to_draft(&FollowActionType::PlaySpecific(id3), &p);
        assert_eq!(draft, ActionDraft::PlaySpecific);
        assert_eq!(tgt, 3);
        // A cleared target slot → id no longer found → sentinel (not slot 0,
        // which would silently redirect a Save to P1).
        p[3] = None;
        let (draft, tgt) = action_to_draft(&FollowActionType::PlaySpecific(id3), &p);
        assert_eq!(draft, ActionDraft::PlaySpecific);
        assert_eq!(tgt, PATTERN_SLOTS); // sentinel: not found
    }

    #[test]
    fn action_to_draft_play_specific_unknown_id_is_sentinel() {
        // An id that matches no slot → sentinel (PATTERN_SLOTS), never slot 0.
        let p: [Option<Pattern>; PATTERN_SLOTS] = std::array::from_fn(|_| Some(Pattern::default()));
        let foreign_id = Pattern::default().id; // fresh id not in any slot
        let (draft, tgt) = action_to_draft(&FollowActionType::PlaySpecific(foreign_id), &p);
        assert_eq!(draft, ActionDraft::PlaySpecific);
        assert_eq!(tgt, PATTERN_SLOTS);
    }

    #[test]
    fn resolve_action_play_specific_sentinel_is_none() {
        // Sentinel specific_target (PATTERN_SLOTS) must resolve to None, never
        // to PlaySpecific(Uuid::nil()) — the engine treats nil as "not found"
        // and falls back to active, making the saved action a silent no-op.
        let p: [Option<Pattern>; PATTERN_SLOTS] = std::array::from_fn(|_| Some(Pattern::default()));
        assert!(matches!(
            resolve_action(ActionDraft::PlaySpecific, PATTERN_SLOTS, &p),
            FollowActionType::None
        ));
    }

    #[test]
    fn resolve_action_play_specific_empty_slot_is_none() {
        // A real slot index whose pattern is None must also resolve to None —
        // no nil Uuid is ever emitted across this boundary.
        let mut p: [Option<Pattern>; PATTERN_SLOTS] =
            std::array::from_fn(|_| Some(Pattern::default()));
        p[4] = None;
        assert!(matches!(
            resolve_action(ActionDraft::PlaySpecific, 4, &p),
            FollowActionType::None
        ));
    }

    // ---- headless harness tests (e2e wiring) ----

    #[test]
    fn save_emits_set_follow_action_and_closes() {
        // Pattern 1 starts with default follow_action (after_loops=1, None).
        let h = open_for(
            1,
            UiState {
                session: Some(Arc::new(Session::default())),
                ..Default::default()
            },
        );
        h.settle();
        h.click_primary(save_center(&h.ctx));
        let cmds = h.cmds();
        assert_eq!(cmds.len(), 1);
        assert!(matches!(
            cmds[0],
            Command::SetFollowAction {
                pattern_idx: 1,
                action: FollowAction {
                    after_loops: 1,
                    action: FollowActionType::None,
                },
            }
        ));
        assert_eq!(read(&h.ctx).target, None); // closed on save
    }

    #[test]
    fn seeded_draft_round_trips_existing_follow_action() {
        // Pattern 2 already has after_loops=4, PlayNext → draft seeds from it.
        let st = fixture_with_follow(
            2,
            FollowAction {
                after_loops: 4,
                action: FollowActionType::PlayNext,
            },
        );
        let h = open_for(2, st);
        h.settle();
        h.click_primary(save_center(&h.ctx));
        assert!(matches!(
            h.cmds().as_slice(),
            [Command::SetFollowAction {
                pattern_idx: 2,
                action: FollowAction {
                    after_loops: 4,
                    action: FollowActionType::PlayNext,
                },
            }]
        ));
    }

    #[test]
    fn after_loops_out_of_range_clamps_on_seed() {
        // #36: an out-of-range after_loops (u32) must normalize to the slider's
        // 1..=16 range when the sheet is open, instead of desyncing
        // (slider=16, label=32, state=32).
        let mut st = UiState {
            session: Some(Arc::new(Session::default())),
            ..Default::default()
        };
        {
            let s = Arc::make_mut(st.session.as_mut().unwrap());
            s.patterns[1].as_mut().unwrap().follow_action.after_loops = 32;
        }
        let h = open_for(1, st);
        h.settle(); // re-seed runs each idle frame
        let draft = read(&h.ctx);
        assert_eq!(draft.after_loops, 16, "out-of-range after_loops must clamp to 16");
    }

    #[test]
    fn open_sheet_re_syncs_draft_after_external_follow_action_change() {
        // #42: while the sheet is open, an external follow_action change
        // (preset / automation / undo swap delivered as a mirror update) must
        // be reflected in the draft, not masked by a one-shot seed.
        let mut h = open_for(
            2,
            UiState {
                session: Some(Arc::new(Session::default())),
                ..Default::default()
            },
        );
        h.settle();
        // Externally mutate the target pattern's follow_action (no edit focus
        // held this frame → re-seed applies).
        {
            let s = Arc::make_mut(h.state.session.as_mut().unwrap());
            s.patterns[2].as_mut().unwrap().follow_action = FollowAction {
                after_loops: 9,
                action: FollowActionType::PlayNext,
            };
        }
        h.idle(); // one frame: render runs + re-seed applies
        let st = read(&h.ctx);
        assert_eq!(st.after_loops, 9, "draft must re-sync after_loops to the live pattern");
        assert_eq!(st.action, ActionDraft::PlayNext, "draft must re-sync action");
    }

    #[test]
    fn cancel_dismisses_without_emit() {
        let h = open_for(
            0,
            UiState {
                session: Some(Arc::new(Session::default())),
                ..Default::default()
            },
        );
        h.settle();
        h.click_primary(cancel_center(&h.ctx));
        assert_eq!(read(&h.ctx).target, None);
        assert!(h.cmds().is_empty());
    }

    #[test]
    fn esc_dismisses_without_emit() {
        let h = open_for(
            0,
            UiState {
                session: Some(Arc::new(Session::default())),
                ..Default::default()
            },
        );
        h.settle();
        h.press_key(egui::Key::Escape);
        assert_eq!(read(&h.ctx).target, None);
        assert!(h.cmds().is_empty());
    }

    #[test]
    fn outside_click_dismisses() {
        let h = open_for(
            0,
            UiState {
                session: Some(Arc::new(Session::default())),
                ..Default::default()
            },
        );
        h.settle();
        let outside = window_rect(&h.ctx)
            .map(|r| egui::Pos2::new(r.max.x + 25.0, r.center().y))
            .expect("window rect recorded");
        h.click_primary(outside);
        assert_eq!(read(&h.ctx).target, None);
    }

    #[test]
    fn closed_renders_no_panic() {
        let h = Harness::new(UiState {
            session: Some(Arc::new(Session::default())),
            ..Default::default()
        });
        h.idle(); // target None → no sheet
        assert!(read(&h.ctx).target.is_none());
        assert!(h.cmds().is_empty());
    }

    #[test]
    fn auto_closes_when_target_slot_cleared() {
        // Stale-target: the slot under an open sheet can be nulled (engine
        // PatternCleared). render must auto-close rather than read a None pattern.
        let mut st = UiState {
            session: Some(Arc::new(Session::default())),
            ..Default::default()
        };
        {
            let s = Arc::make_mut(st.session.as_mut().unwrap());
            s.patterns[2] = None;
        }
        let h = Harness::new(st);
        write(&h.ctx, |s| s.target = Some(2)); // open on a now-empty slot
        h.idle();
        assert_eq!(read(&h.ctx).target, None, "cleared target must auto-close");
    }

    fn clip_center(ctx: &Context, want: ActionClip) -> egui::Pos2 {
        ctx.data(|d| d.get_temp::<Vec<(ActionClip, Rect)>>(clip_rect_id()))
            .unwrap_or_default()
            .into_iter()
            .find(|(c, _)| *c == want)
            .map(|(_, r)| r.center())
            .unwrap_or_else(|| panic!("clip {want:?} rect recorded"))
    }

    #[test]
    fn clipboard_buttons_emit_expected_commands_and_stay_open() {
        // #39: with per-frame re-seed (#42), clipboard buttons no longer need to
        // close(ctx) — the draft self-heals next frame. Staying open fixes the
        // empty-paste silent-dismiss and enables Copy → Paste without reopening.
        // (Diverges from iOS, which dismisses — accepted in design review.)
        let st = || UiState {
            session: Some(Arc::new(Session::default())),
            ..Default::default()
        };
        for clip in [
            ActionClip::Cut,
            ActionClip::Copy,
            ActionClip::Paste,
            ActionClip::Clear,
        ] {
            let h = open_for(2, st());
            h.settle();
            h.click_primary(clip_center(&h.ctx, clip));
            let cmds = h.cmds();
            assert_eq!(
                cmds.len(),
                1,
                "{clip:?}: expected one command, got {cmds:?}"
            );
            let ok = match (clip, &cmds[0]) {
                (ActionClip::Cut, Command::CutPattern { index }) => *index == 2,
                (ActionClip::Copy, Command::CopyPattern { index }) => *index == 2,
                (ActionClip::Paste, Command::PastePattern { index }) => *index == 2,
                (ActionClip::Clear, Command::ClearPattern { index }) => *index == 2,
                _ => false,
            };
            assert!(ok, "{clip:?}: wrong command {:?}", cmds[0]);
            assert_eq!(
                read(&h.ctx).target,
                Some(2),
                "{clip:?}: sheet must STAY OPEN after emit (no close-on-emit)"
            );
        }
    }
}
