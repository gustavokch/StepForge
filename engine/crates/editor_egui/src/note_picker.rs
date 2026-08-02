//! Phase 2 §T T11 — `NotePickerSheet` overlay.
//! Port of `app/StepForge/Features/Editing/NotePickerSheet.swift`.
//!
//! Pure UI (V4): reads [`UiState`] (the open track's current note), emits
//! [`Command::SetTrackNote`], never mutates the engine. The sheet reflects the
//! *actual* mirror note — selection only emits a command and the engine echoes
//! it back via `FullSnapshot` (⊥ optimistic mutation, the iOS Hard Rule 2
//! split restated for the editor).
//!
//! Open/close + the GM/Piano-roll mode toggle are widget-local `ctx.data` temp
//! state (like the grid zoom / ratchet target) — NOT engine mirror state. The
//! sheet is a floating `egui::Area` + `Frame::popup` (the ratchet-popover idiom,
//! NOT `egui::Window`: a `Window`'s title-bar absorbs the first click to acquire
//! focus, which breaks single-click selection both headlessly and in a host),
//! rendered last from [`crate::render`] so it floats above the whole editor.

use egui::{Color32, Context, Id, Key, Pos2, Rect, RichText, Vec2};
use sequencer_engine::command::Command;

use crate::grid::{drum_name, BORDER_WEAK, PRIMARY, SURFACE_HIGH};
use crate::{CommandSink, UiState};

// ---- ctx.data temp slot (open target + GM/Piano mode) ----
fn note_picker_id() -> Id {
    Id::new("stepforge.note_picker")
}
#[cfg(test)]
fn cell_rects_id() -> Id {
    Id::new("stepforge.note_picker.cells")
}
#[cfg(test)]
fn key_rects_id() -> Id {
    Id::new("stepforge.note_picker.keys")
}
#[cfg(test)]
fn highlight_id() -> Id {
    Id::new("stepforge.note_picker.highlight")
}
#[cfg(test)]
fn window_rect_id() -> Id {
    Id::new("stepforge.note_picker.window_rect")
}

/// Segmented mode: GM drum grid (notes 35..=50) or piano roll (36..=60).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum NoteMode {
    #[default]
    Gm,
    PianoRoll,
}

/// Widget-local open/mode state. UI-only — NOT engine mirror state. `Copy` so
/// it round-trips through `ctx.data` temp storage without borrow-threading.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NotePickerState {
    /// Track index the sheet is open for; `None` = closed.
    pub(crate) target: Option<usize>,
    pub(crate) mode: NoteMode,
    /// The frame [`open`] was last called on. Suppresses the outside-click
    /// dismiss on the *opening* frame: `pointer.primary_clicked()` is a global,
    /// non-consumption-aware signal, so the click that opened the sheet (the
    /// header drum-name) is still "primary_clicked" when the sheet renders one
    /// statement later this same frame — without this guard, that click (which
    /// lands outside the sheet's rect for tracks below it) closes the sheet
    /// immediately. The ratchet popover avoids this with its `!alt` guard; a
    /// plain-click open has no modifier, so it guards on the frame number.
    pub(crate) opened_at: u64,
}

pub(crate) fn read(ctx: &Context) -> NotePickerState {
    ctx.data(|d| {
        d.get_temp::<NotePickerState>(note_picker_id())
            .unwrap_or_default()
    })
}
pub(crate) fn write(ctx: &Context, f: impl FnOnce(&mut NotePickerState)) {
    ctx.data_mut(|d| f(d.get_temp_mut_or_default(note_picker_id())));
}

/// Open the sheet for `track_idx`. Closes the ActionDrawer — only one
/// track-level overlay may be open at a time (mutual exclusion).
pub(crate) fn open(ctx: &Context, track_idx: usize) {
    let frame = crate::frame_nr(ctx);
    write(ctx, |s| {
        s.target = Some(track_idx);
        s.opened_at = frame;
    });
    crate::action_drawer::close(ctx);
}
pub(crate) fn close(ctx: &Context) {
    write(ctx, |s| s.target = None);
}

/// GM drum kit, MIDI 35..=50 — verbatim port of `NotePickerSheet.swift`
/// `gmSoundNames`. This is the long table; the grid header's short `drum_name`
/// map is a different, smaller set (Kick/Snare/Hat…).
const GM_DRUMS: [(u8, &str); 16] = [
    (35, "Acoustic Bass Drum"),
    (36, "Bass Drum 1 (Kick)"),
    (37, "Side Stick"),
    (38, "Acoustic Snare"),
    (39, "Hand Clap"),
    (40, "Electric Snare"),
    (41, "Low Floor Tom"),
    (42, "Closed Hi-Hat"),
    (43, "High Floor Tom"),
    (44, "Pedal Hi-Hat"),
    (45, "Low Tom"),
    (46, "Open Hi-Hat"),
    (47, "Low-Mid Tom"),
    (48, "Hi-Mid Tom"),
    (49, "Crash Cymbal 1"),
    (50, "High Tom"),
];

/// Pitch classes (note % 12) that are black keys: C#/D#/F#/G#/A#.
const BLACK_PC: [u8; 5] = [1, 3, 6, 8, 10];

/// Render the sheet if open. No-op (no panic) when closed or the session is
/// missing — mirrors the no-session guards across the other widgets.
pub(crate) fn render_note_picker(ctx: &Context, ui_state: &UiState, sink: &impl CommandSink) {
    let mut st = read(ctx);
    let Some(track_idx) = st.target else {
        return;
    };
    let current = ui_state.tracks().get(track_idx).map(|t| t.midi_note);

    // Reset test probes each frame (a frame after a select-and-close must not
    // leak the previous frame's rects/highlight into a harness read).
    #[cfg(test)]
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Vec<(u8, Rect)>>(cell_rects_id())
            .clear();
        d.get_temp_mut_or_default::<Vec<(u8, Rect)>>(key_rects_id())
            .clear();
        *d.get_temp_mut_or_default::<Option<u8>>(highlight_id()) = current;
        *d.get_temp_mut_or_default::<Option<Rect>>(window_rect_id()) = None;
    });

    let title = match current {
        Some(n) => format!("Track {} — note ({})", track_idx + 1, drum_name(n)),
        None => format!("Track {} — note", track_idx + 1),
    };

    let area = egui::Area::new(Id::new("stepforge.note_picker"))
        .order(egui::Order::Foreground)
        .current_pos(Pos2::new(40.0, 60.0))
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(312.0);
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(title).strong());
                    ui.separator();
                    // Segmented GM / Piano-roll toggle.
                    ui.horizontal(|ui| {
                        if ui
                            .selectable_label(st.mode == NoteMode::Gm, "GM Drums")
                            .clicked()
                        {
                            st.mode = NoteMode::Gm;
                        }
                        if ui
                            .selectable_label(st.mode == NoteMode::PianoRoll, "Piano Roll")
                            .clicked()
                        {
                            st.mode = NoteMode::PianoRoll;
                        }
                    });
                    ui.separator();
                    match st.mode {
                        NoteMode::Gm => render_gm(ctx, ui, track_idx, current, sink),
                        NoteMode::PianoRoll => render_piano(ctx, ui, track_idx, current, sink),
                    }
                });
            });
        });

    // Persist any mode toggle back to temp storage.
    write(ctx, |s| s.mode = st.mode);

    let rect = area.response.rect;
    #[cfg(test)]
    ctx.data_mut(|d| {
        *d.get_temp_mut_or_default::<Option<Rect>>(window_rect_id()) = Some(rect);
    });
    // `opened_at == frame_nr` only on the frame `open` was called — the opening
    // header click is still "primary_clicked" this frame, so skip the
    // outside-click dismiss branch then (Esc still dismisses).
    let is_open_frame = crate::frame_nr(ctx) == st.opened_at;
    dismiss_outside_or_esc(ctx, rect, is_open_frame);
}

fn render_gm(
    ctx: &Context,
    ui: &mut egui::Ui,
    track_idx: usize,
    current: Option<u8>,
    sink: &impl CommandSink,
) {
    egui::Grid::new("stepforge.note_picker.gm")
        .num_columns(2)
        .show(ui, |ui| {
            for &(midi, name) in GM_DRUMS.iter() {
                let sel = Some(midi) == current;
                let btn = egui::Button::new(format!("{}  ({})", name, midi))
                    .min_size(Vec2::new(150.0, 0.0))
                    .fill(if sel { PRIMARY } else { SURFACE_HIGH })
                    .stroke(if sel {
                        egui::Stroke::NONE
                    } else {
                        egui::Stroke::new(1.0_f32, BORDER_WEAK)
                    });
                let resp = ui.add(btn);
                #[cfg(test)]
                ctx.data_mut(|d| {
                    d.get_temp_mut_or_default::<Vec<(u8, Rect)>>(cell_rects_id())
                        .push((midi, resp.rect));
                });
                if resp.clicked() {
                    sink.push(Command::SetTrackNote {
                        track_idx,
                        midi_note: midi,
                    });
                    close(ctx);
                }
                ui.end_row();
            }
        });
}

fn render_piano(
    ctx: &Context,
    ui: &mut egui::Ui,
    track_idx: usize,
    current: Option<u8>,
    sink: &impl CommandSink,
) {
    egui::ScrollArea::horizontal().show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.horizontal(|ui| {
            for note in 36..=60u8 {
                let black = BLACK_PC.contains(&note.rem_euclid(12));
                let sel = Some(note) == current;
                let bg = if sel {
                    PRIMARY
                } else if black {
                    Color32::BLACK
                } else {
                    Color32::WHITE
                };
                let fg = if black && !sel {
                    Color32::WHITE
                } else {
                    Color32::BLACK
                };
                let resp = ui.add_sized(
                    Vec2::new(30.0, 120.0),
                    egui::Button::new(RichText::new(format!("{}", note)).color(fg)).fill(bg),
                );
                #[cfg(test)]
                ctx.data_mut(|d| {
                    d.get_temp_mut_or_default::<Vec<(u8, Rect)>>(key_rects_id())
                        .push((note, resp.rect));
                });
                if resp.clicked() {
                    sink.push(Command::SetTrackNote {
                        track_idx,
                        midi_note: note,
                    });
                    close(ctx);
                }
            }
        });
    });
}

/// Esc (any frame) or a primary click outside the sheet rect (any frame EXCEPT
/// the opening one) closes the sheet without emitting — mirrors the
/// ratchet-popover dismiss (`grid.rs`), with the open-frame guard standing in
/// for the ratchet's `!alt` modifier (a plain-click open has no modifier).
fn dismiss_outside_or_esc(ctx: &Context, rect: Rect, is_open_frame: bool) {
    let dismiss = ctx.input(|i| i.key_pressed(Key::Escape))
        || (!is_open_frame
            && ctx.input(|i| {
                i.pointer.primary_clicked()
                    && i.pointer.latest_pos().is_none_or(|p| !rect.contains(p))
            }));
    if dismiss {
        close(ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Modifiers, PointerButton, Pos2};
    use sequencer_engine::models::Session;
    use std::sync::{Arc, Mutex};

    #[derive(Default, Clone)]
    struct Rec(Arc<Mutex<Vec<Command>>>);
    impl CommandSink for Rec {
        fn push(&self, c: Command) {
            self.0.lock().unwrap().push(c);
        }
    }

    fn raw_input() -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                Pos2::new(0.0, 0.0),
                Vec2::new(1400.0, 800.0),
            )),
            ..Default::default()
        }
    }

    struct Harness {
        ctx: egui::Context,
        state: UiState,
        sink: Rec,
    }
    impl Harness {
        fn new(state: UiState) -> Self {
            Self {
                ctx: egui::Context::default(),
                state,
                sink: Rec::default(),
            }
        }
        fn frame(&self, raw: egui::RawInput) {
            let _ = self
                .ctx
                .run(raw, |ctx| crate::render(ctx, &self.state, &self.sink));
        }
        fn idle(&self) {
            self.frame(raw_input());
        }
        /// A floating `egui::Area`'s widgets don't receive clicks until the
        /// Area's interaction state has settled across a few frames (its rect
        /// must age into `memory.areas` so the press-target lookup resolves to
        /// the Area's layer). At 60 fps this is ~50 ms — invisible in a host —
        /// but a headless harness drives one frame per call, so an overlay just
        /// opened via `write` needs a few idle frames before its buttons click.
        /// The grid's ratchet gets this for free (its Alt+click open gesture is
        /// already two frames); a cold-open does not, so we settle explicitly.
        fn settle(&self) {
            for _ in 0..4 {
                self.idle();
            }
        }
        fn click_primary(&self, pos: Pos2) {
            let mods = Modifiers::default();
            let mut a = raw_input();
            a.events.push(egui::Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: mods,
            });
            self.frame(a);
            let mut b = raw_input();
            b.events.push(egui::Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: mods,
            });
            self.frame(b);
        }
        fn press_key(&self, key: Key) {
            let mut r = raw_input();
            r.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            });
            self.frame(r);
        }
        fn cmds(&self) -> Vec<Command> {
            self.sink.0.lock().unwrap().clone()
        }
    }

    /// Track 0 = Snare (midi 38) so the highlight probe is a known value and
    /// the GM cell for 38 is the current-note cell.
    fn fixture() -> UiState {
        let mut st = UiState {
            session: Some(Arc::new(Session::default())),
            ..Default::default()
        };
        {
            let s = Arc::make_mut(st.session.as_mut().unwrap());
            let p = s.patterns[0].as_mut().unwrap();
            p.tracks[0].midi_note = 38;
        }
        st
    }

    /// 8 tracks so the lower headers sit below the sheet's fixed rect (PianoRoll
    /// mode is the shorter sheet — ~y=240 bottom — so track 7's drum-name
    /// ~y=280 lands outside it). Used by the open-frame self-dismiss regression.
    fn fixture_eight() -> UiState {
        use sequencer_engine::models::{Session, Track};
        let mut st = UiState {
            session: Some(Arc::new(Session::default())),
            ..Default::default()
        };
        {
            let s = Arc::make_mut(st.session.as_mut().unwrap());
            let p = s.patterns[0].as_mut().unwrap();
            p.tracks[0].midi_note = 38;
            for n in [46u8, 41, 49, 51] {
                p.tracks.push(Track::with_note(n));
            }
        }
        st
    }

    fn gm_cell_center(ctx: &egui::Context, midi: u8) -> Pos2 {
        ctx.data(|d| d.get_temp::<Vec<(u8, Rect)>>(cell_rects_id()))
            .unwrap_or_default()
            .into_iter()
            .find(|(m, _)| *m == midi)
            .map(|(_, r)| r.center())
            .expect("GM cell rect recorded")
    }
    fn piano_key_center(ctx: &egui::Context, note: u8) -> Pos2 {
        ctx.data(|d| d.get_temp::<Vec<(u8, Rect)>>(key_rects_id()))
            .unwrap_or_default()
            .into_iter()
            .find(|(n, _)| *n == note)
            .map(|(_, r)| r.center())
            .expect("piano key rect recorded")
    }
    fn window_rect(ctx: &egui::Context) -> Option<Rect> {
        ctx.data(|d| d.get_temp::<Option<Rect>>(window_rect_id()))
            .unwrap_or_default()
    }

    #[test]
    fn gm_cell_click_emits_set_track_note_and_closes() {
        let h = Harness::new(fixture());
        write(&h.ctx, |s| s.target = Some(0));
        h.settle();
        let pos = gm_cell_center(&h.ctx, 42); // Closed Hi-Hat
        h.click_primary(pos);
        let cmds = h.cmds();
        assert_eq!(cmds.len(), 1);
        assert!(matches!(
            cmds[0],
            Command::SetTrackNote {
                track_idx: 0,
                midi_note: 42
            }
        ));
        // Selection closes the sheet.
        assert_eq!(read(&h.ctx).target, None);
    }

    #[test]
    fn piano_key_click_emits_set_track_note() {
        let h = Harness::new(fixture());
        write(&h.ctx, |s| {
            s.target = Some(0);
            s.mode = NoteMode::PianoRoll;
        });
        h.settle();
        // A key inside the ScrollArea's initial viewport (the roll is ~800 px
        // wide in a ~312 px frame, so the rightmost keys are scrolled out of
        // the clip rect and not interactable until scrolled — click a visible
        // one near the left).
        let pos = piano_key_center(&h.ctx, 40);
        h.click_primary(pos);
        let cmds = h.cmds();
        assert_eq!(cmds.len(), 1);
        assert!(matches!(
            cmds[0],
            Command::SetTrackNote {
                track_idx: 0,
                midi_note: 40
            }
        ));
    }

    #[test]
    fn current_note_is_highlighted() {
        let h = Harness::new(fixture()); // track 0 midi_note = 38
        write(&h.ctx, |s| s.target = Some(0));
        h.settle();
        let hl = h
            .ctx
            .data(|d| d.get_temp::<Option<u8>>(highlight_id()))
            .unwrap_or_default();
        assert_eq!(hl, Some(38));
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
        // Click just outside the sheet's right edge — guaranteed off-sheet.
        let outside = window_rect(&h.ctx)
            .map(|r| Pos2::new(r.max.x + 25.0, r.center().y))
            .expect("window rect recorded");
        h.click_primary(outside);
        assert_eq!(read(&h.ctx).target, None);
    }

    #[test]
    fn closed_renders_no_panic() {
        let h = Harness::new(fixture());
        h.idle(); // target None → no sheet
        assert!(read(&h.ctx).target.is_none());
        assert!(h.cmds().is_empty());
    }

    #[test]
    fn no_session_renders_no_panic() {
        let h = Harness::new(UiState::default()); // no session
        write(&h.ctx, |s| s.target = Some(0));
        h.settle();
        // No crash; track absent → nothing to select, no emit.
        assert!(h.cmds().is_empty());
    }

    #[test]
    fn header_drum_name_click_low_track_stays_open() {
        // Regression (adversarial-review find): the drum-name click that opens
        // the sheet is the SAME primary click the sheet's outside-click dismiss
        // sees on its opening frame. For a low track whose drum-name sits below
        // the sheet rect, that click used to self-dismiss the sheet in one
        // frame. The `opened_at` open-frame guard prevents it. PianoRoll mode
        // (shorter sheet) + track 7 (~y=280, below the ~y=240 bottom) bites.
        let h = Harness::new(fixture_eight());
        write(&h.ctx, |s| s.mode = NoteMode::PianoRoll); // shorter sheet
        h.settle();
        let pos = h
            .ctx
            .data(|d| d.get_temp::<Vec<(usize, Rect)>>(crate::grid::note_btn_rects_id()))
            .unwrap_or_default()
            .into_iter()
            .find(|(t, _)| *t == 7)
            .map(|(_, r)| r.center())
            .expect("track 7 drum-name rect recorded");
        h.click_primary(pos);
        assert_eq!(
            read(&h.ctx).target,
            Some(7),
            "low-track drum-name click must open the sheet, not self-dismiss on the opening frame"
        );
    }
}
