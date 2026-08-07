//! Phase 2 §T T11 — `ActionDrawer` overlay.
//! Port of `app/StepForge/Features/Editing/ActionDrawer.swift`.
//!
//! Pure UI (V4): reads [`UiState`] (the open track's drum name + the engine's
//! `undo_available` set to gate the Undo button), emits Roll/Vary/Cut/Copy/
//! Paste/Trash/Undo [`Command`]s, never mutates the engine. Strengths are
//! widget-local slider state — there is **no** `SetRollStrength` command; the
//! strength rides inline on each Roll/Vary. The drawer reflects the mirror that
//! comes back via `FullSnapshot` (⊥ optimistic mutation).
//!
//! Dismiss parity with iOS: only the header Keep and the Undo button close the
//! drawer; the six action buttons (incl. Clear/Trash) stay open so the user can
//! chain ops. Undo is additionally gated on `undo_available` — an *improvement*
//! over iOS (which leaves it always enabled): the editor has the engine's
//! `UndoAvailable` echo, so disabling it when no snapshot exists avoids a no-op.
//!
//! Like the NotePickerSheet, the drawer is a floating `egui::Area` +
//! `Frame::popup` (the ratchet-popover idiom, NOT `egui::Window`: a `Window`'s
//! title-bar absorbs the first click to acquire focus), rendered last from
//! [`crate::render`].

#[cfg(test)]
use egui::Rect;
use egui::{Context, Id, Pos2, Response, RichText, Vec2};
use sequencer_engine::command::Command;

use crate::grid::{drum_name, TEXT_MUTED, TEXT_PRIMARY};
use crate::{CommandSink, UiState};

// ---- ctx.data temp slot (open target + slider-backed strengths) ----
fn action_drawer_id() -> Id {
    Id::new("stepforge.action_drawer")
}
#[cfg(test)]
fn btn_rects_id() -> Id {
    Id::new("stepforge.action_drawer.btns")
}
#[cfg(test)]
fn undo_rect_id() -> Id {
    Id::new("stepforge.action_drawer.undo")
}
#[cfg(test)]
fn keep_rect_id() -> Id {
    Id::new("stepforge.action_drawer.keep")
}
#[cfg(test)]
fn window_rect_id() -> Id {
    Id::new("stepforge.action_drawer.window_rect")
}

/// Slider-backed strengths. Defaults match iOS (`varyStrength 0.5`,
/// `rollStrength 0.6` — roll is the higher default). Only one drawer is open at
/// a time, so strengths are shared across tracks; the engine never sees them —
/// they ride inline on each Roll/Vary command. `Copy` + hand-rolled `Default`
/// (f32 defaults are 0.0, not the iOS values) so it round-trips through
/// `ctx.data` temp storage with the right starting strengths.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ActionDrawerState {
    pub(crate) target: Option<usize>,
    pub(crate) vary: f32,
    pub(crate) roll: f32,
    /// The frame [`open`] was last called on — suppresses the outside-click
    /// dismiss on the opening frame. See [`crate::note_picker::NotePickerState`]
    /// for the full rationale (the header `…` click that opens the drawer is
    /// still "primary_clicked" the same frame the drawer first renders).
    pub(crate) opened_at: u64,
}

impl Default for ActionDrawerState {
    fn default() -> Self {
        Self {
            target: None,
            vary: 0.5,
            roll: 0.6,
            opened_at: 0,
        }
    }
}

pub(crate) fn read(ctx: &Context) -> ActionDrawerState {
    ctx.data(|d| {
        d.get_temp::<ActionDrawerState>(action_drawer_id())
            .unwrap_or_default()
    })
}
pub(crate) fn write(ctx: &Context, f: impl FnOnce(&mut ActionDrawerState)) {
    ctx.data_mut(|d| f(d.get_temp_mut_or_default(action_drawer_id())));
}

/// Open the drawer for `track_idx`. Closes the NotePickerSheet and the
/// SettingsSheet — only one floating overlay may be open at a time (mutual
/// exclusion).
pub(crate) fn open(ctx: &Context, track_idx: usize) {
    let frame = crate::frame_nr(ctx);
    write(ctx, |s| {
        s.target = Some(track_idx);
        s.opened_at = frame;
    });
    crate::note_picker::close(ctx);
    crate::settings::close(ctx);
}
pub(crate) fn close(ctx: &Context) {
    write(ctx, |s| s.target = None);
}

/// Test-facing tag for the six action buttons (rect-lookup key). `cfg(test)`:
/// every construction site is a `#[cfg(test)]` rect probe, so the enum is dead
/// in release builds by design.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ActionTag {
    Vary,
    Roll,
    Copy,
    Cut,
    Paste,
    Clear,
}

/// Render the drawer if open. No-op (no panic) when closed or session missing.
pub(crate) fn render_action_drawer(ctx: &Context, ui_state: &UiState, sink: &impl CommandSink) {
    let mut st = read(ctx);
    let Some(track_idx) = st.target else {
        return;
    };
    // Stale-target guard: `RemoveTrack` can shrink the session under an open
    // drawer, leaving `target` past the end. Close instead of rendering a
    // dangling "Actions · Track" whose buttons emit out-of-range `track_idx`.
    if track_idx >= ui_state.tracks().len() {
        close(ctx);
        return;
    }
    let drum = ui_state
        .tracks()
        .get(track_idx)
        .map(|t| drum_name(t.midi_note));
    let undo_ok = ui_state.undo_available.contains(&track_idx);

    #[cfg(test)]
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Vec<(ActionTag, Rect)>>(btn_rects_id())
            .clear();
        *d.get_temp_mut_or_default::<Option<Rect>>(window_rect_id()) = None;
    });

    let title = format!("Actions · {}", drum.unwrap_or("Track"));
    let area = egui::Area::new(Id::new("stepforge.action_drawer"))
        .order(egui::Order::Foreground)
        .current_pos(Pos2::new(40.0, 60.0))
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(300.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(title).strong());
                    ui.separator();
                    // Strength sliders with a live % readout (iOS parity).
                    strength_slider(ui, "VARY", &mut st.vary);
                    strength_slider(ui, "ROLL", &mut st.roll);
                    ui.separator();
                    // Six action buttons (iOS HStack order/labels).
                    ui.horizontal(|ui| {
                        let r = action_btn(ui, "Vary");
                        #[cfg(test)]
                        ctx.data_mut(|d| {
                            d.get_temp_mut_or_default::<Vec<(ActionTag, Rect)>>(btn_rects_id())
                                .push((ActionTag::Vary, r.rect));
                        });
                        if r.clicked() {
                            sink.push(Command::Vary {
                                track_idx,
                                strength: st.vary,
                            });
                        }
                        let r = action_btn(ui, "Roll");
                        #[cfg(test)]
                        ctx.data_mut(|d| {
                            d.get_temp_mut_or_default::<Vec<(ActionTag, Rect)>>(btn_rects_id())
                                .push((ActionTag::Roll, r.rect));
                        });
                        if r.clicked() {
                            sink.push(Command::Roll {
                                track_idx,
                                strength: st.roll,
                            });
                        }
                        let r = action_btn(ui, "Copy");
                        #[cfg(test)]
                        ctx.data_mut(|d| {
                            d.get_temp_mut_or_default::<Vec<(ActionTag, Rect)>>(btn_rects_id())
                                .push((ActionTag::Copy, r.rect));
                        });
                        if r.clicked() {
                            sink.push(Command::Copy { track_idx });
                        }
                        let r = action_btn(ui, "Cut");
                        #[cfg(test)]
                        ctx.data_mut(|d| {
                            d.get_temp_mut_or_default::<Vec<(ActionTag, Rect)>>(btn_rects_id())
                                .push((ActionTag::Cut, r.rect));
                        });
                        if r.clicked() {
                            sink.push(Command::Cut { track_idx });
                        }
                        let r = action_btn(ui, "Paste");
                        #[cfg(test)]
                        ctx.data_mut(|d| {
                            d.get_temp_mut_or_default::<Vec<(ActionTag, Rect)>>(btn_rects_id())
                                .push((ActionTag::Paste, r.rect));
                        });
                        if r.clicked() {
                            sink.push(Command::Paste { track_idx });
                        }
                        let r = action_btn(ui, "Clear");
                        #[cfg(test)]
                        ctx.data_mut(|d| {
                            d.get_temp_mut_or_default::<Vec<(ActionTag, Rect)>>(btn_rects_id())
                                .push((ActionTag::Clear, r.rect));
                        });
                        if r.clicked() {
                            sink.push(Command::Trash { track_idx });
                        }
                    });
                    ui.separator();
                    // Undo (gated on undo_available) + Keep (dismiss, no emit).
                    ui.horizontal(|ui| {
                        let undo_resp = ui.add_enabled(
                            undo_ok,
                            egui::Button::new("Undo").min_size(Vec2::new(72.0, 0.0)),
                        );
                        #[cfg(test)]
                        ctx.data_mut(|d| {
                            *d.get_temp_mut_or_default::<Option<Rect>>(undo_rect_id()) =
                                Some(undo_resp.rect);
                        });
                        if undo_resp.clicked() {
                            sink.push(Command::Undo { track_idx });
                            close(ctx);
                        }
                        let keep_resp = ui.button("Keep");
                        #[cfg(test)]
                        ctx.data_mut(|d| {
                            *d.get_temp_mut_or_default::<Option<Rect>>(keep_rect_id()) =
                                Some(keep_resp.rect);
                        });
                        if keep_resp.clicked() {
                            close(ctx);
                        }
                    });
                });
            });
        });

    // Persist slider values back to temp storage.
    write(ctx, |s| {
        s.vary = st.vary;
        s.roll = st.roll;
    });

    let rect = area.response.rect;
    #[cfg(test)]
    ctx.data_mut(|d| {
        *d.get_temp_mut_or_default::<Option<Rect>>(window_rect_id()) = Some(rect);
    });
    // `opened_at == frame_nr` only on the opening frame; the header `…` click
    // that opened the drawer is still "primary_clicked" then, so skip the
    // outside-click dismiss branch that frame (Esc still dismisses).
    let is_open_frame = crate::frame_nr(ctx) == st.opened_at;
    if crate::overlay::should_dismiss(ctx, rect, is_open_frame) {
        close(ctx);
    }
}

/// A strength slider with a leading label and trailing `%` readout (iOS parity).
fn strength_slider(ui: &mut egui::Ui, label: &str, val: &mut f32) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(TEXT_MUTED).strong());
        ui.add(egui::Slider::new(val, 0.0..=1.0).show_value(false));
        ui.label(RichText::new(format!("{}%", (*val * 100.0).round() as i32)).color(TEXT_PRIMARY));
    });
}

/// A fixed-width action button. Returns its [`Response`] — the caller records
/// the (tagged) rect and dispatches the command, so this helper carries no
/// `cfg(test)`-only params that would be unused in release builds.
fn action_btn(ui: &mut egui::Ui, label: &str) -> Response {
    ui.add(egui::Button::new(label).min_size(Vec2::new(64.0, 0.0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Harness;
    use egui::{Key, Pos2};
    use sequencer_engine::models::Session;
    use std::sync::Arc;

    /// Track 0 = Snare (midi 38) with a couple of active steps; no undo slot.
    fn fixture() -> UiState {
        use sequencer_engine::models::{Step, VelocityZone};
        let mut st = UiState {
            session: Some(Arc::new(Session::default())),
            ..Default::default()
        };
        {
            let s = Arc::make_mut(st.session.as_mut().unwrap());
            let p = s.patterns[0].as_mut().unwrap();
            p.tracks[0].midi_note = 38;
            p.tracks[0].steps[0] = Step {
                active: true,
                velocity_zone: VelocityZone::Mid,
                ..Default::default()
            };
            p.tracks[0].steps[4] = Step {
                active: true,
                velocity_zone: VelocityZone::Accent,
                ..Default::default()
            };
        }
        st
    }

    fn fixture_with_undo() -> UiState {
        let mut st = fixture();
        st.undo_available.insert(0);
        st
    }

    /// 8 tracks so the lower headers sit below the drawer's fixed rect. The
    /// open-frame self-dismiss regression only bites when the opening click
    /// lands outside the overlay rect (track 7's `…` is ~y=280, below the
    /// drawer's ~y=235 bottom edge).
    fn fixture_eight() -> UiState {
        use sequencer_engine::models::{Session, Track};
        let mut st = UiState {
            session: Some(Arc::new(Session::default())),
            ..Default::default()
        };
        {
            let s = Arc::make_mut(st.session.as_mut().unwrap());
            let p = s.patterns[0].as_mut().unwrap();
            for n in [46u8, 41, 49, 51] {
                p.tracks.push(Track::with_note(n));
            }
        }
        st
    }

    fn btn_center(ctx: &egui::Context, tag: ActionTag) -> Pos2 {
        ctx.data(|d| d.get_temp::<Vec<(ActionTag, Rect)>>(btn_rects_id()))
            .unwrap_or_default()
            .into_iter()
            .find(|(t, _)| *t == tag)
            .map(|(_, r)| r.center())
            .expect("action button rect recorded")
    }
    fn undo_center(ctx: &egui::Context) -> Pos2 {
        ctx.data(|d| d.get_temp::<Option<Rect>>(undo_rect_id()))
            .unwrap_or_default()
            .map(|r| r.center())
            .expect("undo button rect recorded")
    }
    fn keep_center(ctx: &egui::Context) -> Pos2 {
        ctx.data(|d| d.get_temp::<Option<Rect>>(keep_rect_id()))
            .unwrap_or_default()
            .map(|r| r.center())
            .expect("keep button rect recorded")
    }
    fn window_rect(ctx: &egui::Context) -> Option<Rect> {
        ctx.data(|d| d.get_temp::<Option<Rect>>(window_rect_id()))
            .unwrap_or_default()
    }

    #[test]
    fn six_action_buttons_emit_expected_commands_and_stay_open() {
        let h = Harness::new(fixture());
        write(&h.ctx, |s| {
            s.target = Some(0);
            s.vary = 0.5;
            s.roll = 0.6;
        });
        h.settle();
        h.click_primary(btn_center(&h.ctx, ActionTag::Vary));
        h.click_primary(btn_center(&h.ctx, ActionTag::Roll));
        h.click_primary(btn_center(&h.ctx, ActionTag::Copy));
        h.click_primary(btn_center(&h.ctx, ActionTag::Cut));
        h.click_primary(btn_center(&h.ctx, ActionTag::Paste));
        h.click_primary(btn_center(&h.ctx, ActionTag::Clear));

        let cmds = h.cmds();
        assert_eq!(cmds.len(), 6);
        assert!(matches!(
            cmds[0],
            Command::Vary {
                track_idx: 0,
                strength: 0.5
            }
        ));
        assert!(matches!(
            cmds[1],
            Command::Roll {
                track_idx: 0,
                strength: 0.6
            }
        ));
        assert!(matches!(cmds[2], Command::Copy { track_idx: 0 }));
        assert!(matches!(cmds[3], Command::Cut { track_idx: 0 }));
        assert!(matches!(cmds[4], Command::Paste { track_idx: 0 }));
        assert!(matches!(cmds[5], Command::Trash { track_idx: 0 }));
        // iOS parity: action buttons keep the drawer open.
        assert_eq!(read(&h.ctx).target, Some(0));
    }

    #[test]
    fn vary_carries_seeded_slider_strength() {
        let h = Harness::new(fixture());
        write(&h.ctx, |s| {
            s.target = Some(0);
            s.vary = 0.9;
        });
        h.settle();
        h.click_primary(btn_center(&h.ctx, ActionTag::Vary));
        let cmds = h.cmds();
        assert_eq!(cmds.len(), 1);
        assert!(matches!(
            cmds[0],
            Command::Vary {
                track_idx: 0,
                strength
            } if (strength - 0.9).abs() < 1e-6
        ));
    }

    #[test]
    fn roll_carries_seeded_slider_strength() {
        let h = Harness::new(fixture());
        write(&h.ctx, |s| {
            s.target = Some(0);
            s.roll = 0.25;
        });
        h.settle();
        h.click_primary(btn_center(&h.ctx, ActionTag::Roll));
        let cmds = h.cmds();
        assert_eq!(cmds.len(), 1);
        assert!(matches!(
            cmds[0],
            Command::Roll {
                track_idx: 0,
                strength
            } if (strength - 0.25).abs() < 1e-6
        ));
    }

    #[test]
    fn undo_disabled_when_not_available() {
        let h = Harness::new(fixture()); // undo_available empty
        write(&h.ctx, |s| s.target = Some(0));
        h.settle();
        h.click_primary(undo_center(&h.ctx));
        // Disabled button emits nothing and does not close the drawer.
        assert!(h.cmds().is_empty());
        assert_eq!(read(&h.ctx).target, Some(0));
    }

    #[test]
    fn undo_enabled_emits_and_closes() {
        let h = Harness::new(fixture_with_undo());
        write(&h.ctx, |s| s.target = Some(0));
        h.settle();
        h.click_primary(undo_center(&h.ctx));
        let cmds = h.cmds();
        assert_eq!(cmds.len(), 1);
        assert!(matches!(cmds[0], Command::Undo { track_idx: 0 }));
        assert_eq!(read(&h.ctx).target, None);
    }

    #[test]
    fn keep_closes_without_emit() {
        let h = Harness::new(fixture());
        write(&h.ctx, |s| s.target = Some(0));
        h.settle();
        h.click_primary(keep_center(&h.ctx));
        assert_eq!(read(&h.ctx).target, None);
        assert!(h.cmds().is_empty());
    }

    #[test]
    fn esc_dismisses_without_emit() {
        let h = Harness::new(fixture());
        write(&h.ctx, |s| s.target = Some(0));
        h.settle();
        h.press_key(Key::Escape);
        assert_eq!(read(&h.ctx).target, None);
        assert!(h.cmds().is_empty());
    }

    #[test]
    fn outside_click_dismisses() {
        let h = Harness::new(fixture());
        write(&h.ctx, |s| s.target = Some(0));
        h.settle();
        let outside = window_rect(&h.ctx)
            .map(|r| Pos2::new(r.max.x + 25.0, r.center().y))
            .expect("window rect recorded");
        h.click_primary(outside);
        assert_eq!(read(&h.ctx).target, None);
    }

    #[test]
    fn closed_renders_no_panic() {
        let h = Harness::new(fixture());
        h.idle(); // target None → no drawer
        assert!(read(&h.ctx).target.is_none());
        assert!(h.cmds().is_empty());
    }

    #[test]
    fn auto_closes_when_target_track_out_of_range() {
        // Stale-target edge: `RemoveTrack` can shrink the session while the
        // drawer is open, leaving `target` past the end. Buttons would then
        // emit an out-of-range `track_idx` (engine bounds-checks → no panic,
        // but the drawer dangles as "Actions · Track"). render must auto-close.
        // (Default session has 4 tracks → idx 4 is just past the end.)
        let h = Harness::new(fixture());
        write(&h.ctx, |s| s.target = Some(4)); // OOB (valid idx 0..=3)
        h.idle();
        assert_eq!(read(&h.ctx).target, None, "OOB target must auto-close");
    }

    #[test]
    fn header_dots_click_low_track_stays_open() {
        // Regression (adversarial-review find): the header `…` click that opens
        // the drawer is the SAME primary click the drawer's outside-click
        // dismiss sees on its opening frame (`pointer.primary_clicked()` is
        // global, not consumption-aware). For a track whose `…` sits below the
        // drawer's rect that click used to self-dismiss the drawer in one frame
        // — open+close, unreachable. The `opened_at` open-frame guard prevents
        // it. Track 7's `…` (~y=280) is below the drawer's ~y=235 bottom edge.
        let h = Harness::new(fixture_eight());
        h.settle();
        let pos = h
            .ctx
            .data(|d| d.get_temp::<Vec<(usize, Rect)>>(crate::grid::more_btn_rects_id()))
            .unwrap_or_default()
            .into_iter()
            .find(|(t, _)| *t == 7)
            .map(|(_, r)| r.center())
            .expect("track 7 … rect recorded");
        h.click_primary(pos);
        assert_eq!(
            read(&h.ctx).target,
            Some(7),
            "low-track … click must open the drawer, not self-dismiss on the opening frame"
        );
    }
}
