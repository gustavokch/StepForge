//! Phase 1 §T T10e — TrackManagementBar widget. Port of the iOS `EditingView`
//! "Row 3" (`app/StepForge/Features/Editing/TrackManagementBar.swift`): a track
//! count label plus add (`+`) / remove (`−`) controls.
//!
//! Pure UI (V4): reads [`UiState`], emits [`Command`]s via a [`CommandSink`],
//! and never mutates the engine. Each control reflects the *actual* mirror
//! state (⊥ optimistic) — a gesture only emits a command; the engine echoes the
//! add/remove back through `UiState::apply` (`TrackAdded` / `TrackRemoved`),
//! the Hard Rule 2 split ported from iOS.
//!
//! Both [`Command`]s are unit variants — the engine decides which track
//! (`add_track()` appends one up to `MAX_TRACKS`, `remove_track()` drops the
//! bottom-most down to `MIN_TRACKS`); the bounds are enforced in the engine and
//! mirrored here as disabled-button floors, so the editor never computes a
//! track index. Desktop `−` click removes directly — the `MIN_TRACKS` disabled
//! floor prevents over-removal and Undo (T11) recovers mistakes, so the iOS
//! hold-to-remove safeguard (a mobile-only tap-precision aid) is not ported.

use egui::{Button, Layout, RichText, Ui, Vec2};
use sequencer_engine::command::Command;
use sequencer_engine::models::{MAX_TRACKS, MIN_TRACKS};

use crate::theme::{SURFACE_HIGHEST, TEXT_MUTED, TEXT_PRIMARY};
use crate::{CommandSink, UiState};

// ---- Pure helpers (headless oracle; ⊥ egui state) ----

/// Whether the `+` (add) control is enabled at a given live track count.
/// Disabled at `MAX_TRACKS` (`TrackManagementBar.swift:44`) — the engine bound
/// is the backstop; this disabled state is the primary guard (a disabled `+`
/// emits nothing).
pub(crate) fn add_enabled(len: usize) -> bool {
    len < MAX_TRACKS
}

/// Whether the `−` (remove) control is enabled at a given live track count.
/// Disabled at `MIN_TRACKS` (`TrackManagementBar.swift:60`).
pub(crate) fn remove_enabled(len: usize) -> bool {
    len > MIN_TRACKS
}

// ---- Widget-local temp ids (`Id::new` is non-const ∴ accessors) ----

#[cfg(test)]
fn add_btn_rect_id() -> egui::Id {
    egui::Id::new("stepforge.track_management.add_btn_rect")
}
#[cfg(test)]
fn remove_btn_rect_id() -> egui::Id {
    egui::Id::new("stepforge.track_management.remove_btn_rect")
}

/// Render the TrackManagementBar (Row 3). `state` is the live mirror; gestures
/// emit via `sink`. Read-only over session ground truth except for the explicit
/// emits: `AddTrack` (`+`), `RemoveTrack` (`−`).
pub fn render_track_management_bar(ui: &mut Ui, state: &UiState, sink: &impl CommandSink) {
    let len = state.tracks().len();

    ui.horizontal(|ui| {
        // ---- label (left): "{n} / 8 TRACKS" (TrackManagementBar.swift:14). ----
        ui.label(RichText::new(format!("{len} / {MAX_TRACKS} TRACKS")).color(TEXT_MUTED));

        // ---- + / − (right): right-to-left packs − to the far edge, + beside it
        //      (mirrors the iOS `HStack { … Spacer(); addButton; removeButton }`).
        //      Disabled buttons emit nothing; the engine bounds are a backstop. ----
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            let can_remove = remove_enabled(len);
            let rm = ui.add_enabled(can_remove, remove_button(can_remove));
            #[cfg(test)]
            ui.ctx()
                .data_mut(|d| d.insert_temp(remove_btn_rect_id(), rm.rect));
            if rm.clicked() {
                sink.push(Command::RemoveTrack);
            }

            let can_add = add_enabled(len);
            let ad = ui.add_enabled(can_add, add_button(can_add));
            #[cfg(test)]
            ui.ctx()
                .data_mut(|d| d.insert_temp(add_btn_rect_id(), ad.rect));
            if ad.clicked() {
                sink.push(Command::AddTrack);
            }
        });
    });
}

// ---- Buttons ----

/// `+` add button. Muted when disabled (faithful to `TrackManagementBar.swift:38`).
fn add_button(enabled: bool) -> Button<'static> {
    let color = if enabled { TEXT_PRIMARY } else { TEXT_MUTED };
    Button::new(RichText::new("+").color(color).strong())
        .fill(SURFACE_HIGHEST)
        .min_size(Vec2::new(30.0, 24.0))
}

/// `−` remove button. Muted when disabled (faithful to `TrackManagementBar.swift:54`).
fn remove_button(enabled: bool) -> Button<'static> {
    let color = if enabled { TEXT_PRIMARY } else { TEXT_MUTED };
    Button::new(RichText::new("\u{2212}").color(color).strong())
        .fill(SURFACE_HIGHEST)
        .min_size(Vec2::new(30.0, 24.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use sequencer_engine::models::{Session, Track};

    #[derive(Default, Clone)]
    struct Rec(Arc<Mutex<Vec<Command>>>);
    impl CommandSink for Rec {
        fn push(&self, c: Command) {
            self.0.lock().unwrap().push(c);
        }
    }

    fn raw_input() -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::new(0.0, 0.0),
                egui::Vec2::new(1400.0, 800.0),
            )),
            ..Default::default()
        }
    }

    /// Session whose active pattern (index 0) has exactly `n` default tracks.
    /// Used to drive the bar at the MIN/MAX bounds (4..=8) without the engine.
    fn session_with_tracks(n: usize) -> Session {
        let mut s = Session::default();
        if let Some(p) = s.patterns[0].as_mut() {
            p.tracks = (0..n).map(|_| Track::default()).collect();
        }
        s
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
            let _ = self.ctx.run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    render_track_management_bar(ui, &self.state, &self.sink);
                });
            });
        }
        fn idle(&self) {
            self.frame(raw_input());
        }
        fn click(&self, pos: egui::Pos2) {
            for pressed in [true, false] {
                let mut r = raw_input();
                r.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::default(),
                });
                self.frame(r);
            }
        }
        fn center(&self, id: egui::Id) -> egui::Pos2 {
            self.idle();
            self.ctx
                .data(|d| d.get_temp::<egui::Rect>(id))
                .expect("rect recorded")
                .center()
        }
        fn cmds(&self) -> Vec<Command> {
            self.sink.0.lock().unwrap().clone()
        }
    }

    // ---- pure oracle tests ----

    #[test]
    fn add_remove_enabled_bounds() {
        // add enabled strictly below MAX, disabled at MAX
        assert!(add_enabled(0));
        assert!(add_enabled(MAX_TRACKS - 1));
        assert!(!add_enabled(MAX_TRACKS));
        // remove enabled strictly above MIN, disabled at MIN
        assert!(remove_enabled(MIN_TRACKS + 1));
        assert!(!remove_enabled(MIN_TRACKS));
        assert!(!remove_enabled(0));
    }

    #[test]
    fn uistate_tracks_count() {
        // default session → MIN_TRACKS (4); no session → empty (no panic)
        let mut st = UiState::default();
        assert_eq!(st.tracks().len(), 0); // pre-snapshot

        st.session = Some(Arc::new(session_with_tracks(6)));
        assert_eq!(st.tracks().len(), 6);

        st.session = Some(Arc::new(session_with_tracks(MAX_TRACKS)));
        assert_eq!(st.tracks().len(), MAX_TRACKS);
    }

    // ---- headless harness tests (e2e wiring) ----

    #[test]
    fn add_click_emits_addtrack() {
        // 5 tracks → + enabled → click emits AddTrack (unit).
        let st = UiState {
            session: Some(Arc::new(session_with_tracks(5))),
            ..UiState::default()
        };
        let h = Harness::new(st);
        h.click(h.center(add_btn_rect_id()));
        assert!(matches!(h.cmds().as_slice(), [Command::AddTrack]));
    }

    #[test]
    fn remove_click_emits_removetrack() {
        // 5 tracks → − enabled → click emits RemoveTrack (unit).
        let st = UiState {
            session: Some(Arc::new(session_with_tracks(5))),
            ..UiState::default()
        };
        let h = Harness::new(st);
        h.click(h.center(remove_btn_rect_id()));
        assert!(matches!(h.cmds().as_slice(), [Command::RemoveTrack]));
    }

    #[test]
    fn add_disabled_at_max_emits_nothing() {
        // at MAX_TRACKS the + is disabled → a click in its rect emits nothing.
        let st = UiState {
            session: Some(Arc::new(session_with_tracks(MAX_TRACKS))),
            ..UiState::default()
        };
        let h = Harness::new(st);
        h.click(h.center(add_btn_rect_id()));
        assert!(h.cmds().is_empty(), "disabled + must not emit");
    }

    #[test]
    fn remove_disabled_at_min_emits_nothing() {
        // at MIN_TRACKS (default 4) the − is disabled → a click emits nothing.
        let st = UiState {
            session: Some(Arc::new(Session::default())),
            ..UiState::default()
        };
        let h = Harness::new(st);
        h.click(h.center(remove_btn_rect_id()));
        assert!(h.cmds().is_empty(), "disabled − must not emit");
    }

    #[test]
    fn track_management_render_no_session_no_panic() {
        // no session → tracks() empty → "0 / 8 TRACKS", + enabled, − disabled.
        let h = Harness::new(UiState::default());
        h.idle();
        // + is enabled at 0 < MAX → emits; − is disabled at 0 <= MIN → no emit.
        h.click(h.center(add_btn_rect_id()));
        assert!(matches!(h.cmds().as_slice(), [Command::AddTrack]));
        h.click(h.center(remove_btn_rect_id()));
        assert!(
            h.cmds().iter().all(|c| !matches!(c, Command::RemoveTrack)),
            "disabled − must not emit even without a session"
        );
    }
}
