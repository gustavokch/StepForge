//! Phase 1 §T T10c — TransportBar widget. Port of the iOS `EditingView` top
//! transport row (`app/StepForge/Features/Editing/TransportBar.swift`): play/stop,
//! BPM, read-only sync badge, and the 8/16 zoom toggle.
//!
//! Pure UI (V4): reads [`UiState`], emits [`Command`]s via a [`CommandSink`],
//! never touches the engine. Every control reflects the *actual* mirror state
//! (⊥ optimistic) — a gesture only emits a command; the engine echoes the change
//! back through `UiState::apply` (Hard Rule 2 split, ported from iOS).
//!
//! The zoom toggle shares the step-grid's `grid_id()` temp slot (T10b) — ONE zoom
//! state, owned by the grid, toggled here. The `1`/`2` keys are handled in
//! `grid::apply_zoom_input` (no scroll-wheel zoom — DAWs claim the gesture before
//! the plugin sees it; see `grid.rs`).

use egui::{Button, DragValue, Id, RichText, Ui};
use sequencer_engine::command::Command;
use sequencer_engine::models::{SyncSource, MAX_BPM, MIN_BPM};

use crate::grid::{read_grid, write_grid, Zoom, PRIMARY, SURFACE_HIGH, TEXT_MUTED, TEXT_PRIMARY};
use crate::{transport_action, CommandSink, UiState};

// ---- Pure helpers (headless oracle; ⊥ egui state) ----

/// Clamp a raw BPM to the engine range `[MIN_BPM, MAX_BPM]` (`models.rs`).
/// Mirrors the engine-side clamp in `apply_command(SetBpm)` — both sides agree,
/// so the DragValue can never request a value the engine would reject.
pub(crate) fn clamp_bpm(raw: f64) -> f64 {
    raw.clamp(MIN_BPM, MAX_BPM)
}

/// Command emitted for a BPM edit. Pure → testable without driving the widget.
pub(crate) fn bpm_edit_command(raw: f64) -> Command {
    Command::SetBpm {
        bpm: clamp_bpm(raw),
    }
}

/// Human label for a sync source (port of iOS `SyncSource.label`,
/// `Engine/Models.swift:69`). Read-only badge text — the plugin never emits
/// `SetSyncSource` (host owns transport; full sync UI lands in Phase 4
/// `SettingsSheet`).
pub(crate) fn sync_label(src: SyncSource) -> &'static str {
    match src {
        SyncSource::Free => "Free",
        SyncSource::MidiClock => "MIDI",
        SyncSource::Link => "Link",
    }
}

/// BPM shown this frame: the in-flight (user-edited, echo-pending) value wins
/// over the mirror so an active drag doesn't snap back to the stale mirror each
/// frame; once the engine echo (`BpmChanged`) lands, [`clear_inflight`] drops it.
pub(crate) fn seed_bpm(mirror: f64, inflight: Option<f64>) -> f64 {
    inflight.unwrap_or(mirror)
}

/// Whether the mirror has caught up to the pending in-flight value (echo
/// arrived) → safe to drop the override. Exact f64 eq is sound here: the value
/// round-trips `SetBpm { bpm } → BpmChanged { bpm }` unchanged (no rounding).
pub(crate) fn clear_inflight(mirror: f64, inflight: Option<f64>) -> bool {
    match inflight {
        Some(pending) => (mirror - pending).abs() < 1e-6,
        None => false,
    }
}

/// Sync-badge text: label + Link peer count (matches the iOS badge body).
fn sync_badge_text(src: SyncSource, link_peers: usize) -> String {
    let label = sync_label(src);
    if src == SyncSource::Link && link_peers > 0 {
        format!("{label} · {link_peers} peers")
    } else {
        label.to_string()
    }
}

// ---- Widget-local temp ids (`Id::new` is non-const ∴ accessors) ----

fn bpm_inflight_id() -> Id {
    Id::new("stepforge.transport.bpm_inflight")
}
#[cfg(test)]
fn play_rect_id() -> Id {
    Id::new("stepforge.transport.play_rect")
}
#[cfg(test)]
fn bpm_rect_id() -> Id {
    Id::new("stepforge.transport.bpm_rect")
}
#[cfg(test)]
fn zoom_rects_id() -> Id {
    Id::new("stepforge.transport.zoom_rects")
}
#[cfg(test)]
fn mode_rect_id() -> Id {
    Id::new("stepforge.transport.mode_rect")
}

/// Render the transport bar. `state` is the live mirror; gestures emit via
/// `sink`. Read-only over session ground truth except for the explicit emits
/// (play/stop, SetBpm, zoom).
pub fn render_transport_bar(ui: &mut Ui, state: &UiState, sink: &impl CommandSink) {
    let ctx = ui.ctx().clone();

    ui.horizontal(|ui| {
        // ---- play / stop: reflect ACTUAL state.playing (⊥ optimistic toggle) ----
        let playing = state.playing;
        let play_resp = ui.add(play_button(playing));
        #[cfg(test)]
        ctx.data_mut(|d| d.insert_temp(play_rect_id(), play_resp.rect));
        if play_resp.clicked() {
            sink.push(transport_action(playing));
        }

        ui.separator();

        // ---- BPM: read snapshot; edit → SetBpm (clamped). An in-flight override
        //      keeps a drag continuous until the engine echo lands (⊥ snap-back). ----
        ui.label(RichText::new("BPM").color(TEXT_MUTED));
        let mirror_bpm = state.bpm();
        // Hoist the temp read OUT of any later data_mut closure (⊥ re-entrant
        // Context lock — parking_lot is non-reentrant; see grid.rs drag notes).
        let inflight: Option<f64> = ctx
            .data(|d| d.get_temp::<Option<f64>>(bpm_inflight_id()))
            .flatten();
        let mut v = seed_bpm(mirror_bpm, inflight);
        let dv = ui.add(DragValue::new(&mut v).range(MIN_BPM..=MAX_BPM).speed(0.5));
        #[cfg(test)]
        ctx.data_mut(|d| d.insert_temp(bpm_rect_id(), dv.rect));
        if dv.changed() {
            let clamped = clamp_bpm(v);
            sink.push(bpm_edit_command(clamped));
            ctx.data_mut(|d| d.insert_temp(bpm_inflight_id(), Some(clamped)));
        }
        // Drop the override once the mirror catches up (echo arrived).
        if clear_inflight(mirror_bpm, inflight) {
            ctx.data_mut(|d| d.insert_temp(bpm_inflight_id(), None::<f64>));
        }

        ui.separator();

        // ---- sync badge: READ-ONLY (host owns transport in the plugin). ----
        let src = state.sync_source();
        ui.label(
            RichText::new(sync_badge_text(src, state.link_peers))
                .color(TEXT_MUTED)
                .strong(),
        );

        ui.separator();

        // ---- zoom toggle: shares the grid's `grid_id()` slot (ONE state). ----
        let mut z = read_grid(&ctx).zoom;
        let before = z;
        // Underscore-prefixed: only read for test rect-recording below.
        let _r8 = ui.radio_value(&mut z, Zoom::Eight, "8");
        let _r16 = ui.radio_value(&mut z, Zoom::Sixteen, "16");
        #[cfg(test)]
        ctx.data_mut(|d| {
            d.insert_temp(
                zoom_rects_id(),
                vec![(Zoom::Eight, _r8.rect), (Zoom::Sixteen, _r16.rect)],
            );
        });
        if z != before {
            write_grid(&ctx, |g| g.zoom = z);
        }

        ui.separator();

        // Phase 3 §T T12 — AppMode toggle (Editing ↔ Performance). Widget-local
        // (`stepforge.mode`); no engine command on switch (iOS `@State mode`).
        // The button label names the mode it switches TO. Switching mode closes
        // the other mode's track-level overlays so nothing dangles over the
        // newly-shown view (a drawer opened in Editing would otherwise float
        // over the PerformanceView, and vice-versa for the PatternOptionsSheet).
        let mode = crate::read_mode(&ctx);
        let (next_mode, label) = match mode {
            crate::AppMode::Editing => (crate::AppMode::Performance, "Performance"),
            crate::AppMode::Performance => (crate::AppMode::Editing, "Editing"),
        };
        let m_resp = ui.button(RichText::new(label).color(TEXT_PRIMARY).strong());
        #[cfg(test)]
        ctx.data_mut(|d| d.insert_temp(mode_rect_id(), m_resp.rect));
        if m_resp.clicked() {
            crate::write_mode(&ctx, next_mode);
            match next_mode {
                crate::AppMode::Performance => {
                    crate::note_picker::close(&ctx);
                    crate::action_drawer::close(&ctx);
                }
                crate::AppMode::Editing => {
                    crate::pattern_options::close(&ctx);
                }
            }
        }
    });
}

/// Play/stop button. ■ = stop (playing), ▶ = play (stopped); accent only while
/// playing (mirrors the iOS `playStop` symbol + tint).
fn play_button(playing: bool) -> Button<'static> {
    let (glyph, color) = if playing {
        ("■", PRIMARY)
    } else {
        ("▶", TEXT_PRIMARY)
    };
    Button::new(RichText::new(glyph).color(color).strong()).fill(SURFACE_HIGH)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use sequencer_engine::models::Session;

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
                    render_transport_bar(ui, &self.state, &self.sink);
                });
            });
        }
        fn idle(&self) {
            self.frame(raw_input());
        }
        fn play_center(&self) -> egui::Pos2 {
            self.idle();
            self.ctx
                .data(|d| d.get_temp::<egui::Rect>(play_rect_id()))
                .expect("play rect recorded")
                .center()
        }
        fn bpm_center(&self) -> egui::Pos2 {
            self.idle();
            self.ctx
                .data(|d| d.get_temp::<egui::Rect>(bpm_rect_id()))
                .expect("bpm rect recorded")
                .center()
        }
        fn zoom_center(&self, want: Zoom) -> egui::Pos2 {
            self.idle();
            self.ctx
                .data(|d| d.get_temp::<Vec<(Zoom, egui::Rect)>>(zoom_rects_id()))
                .expect("zoom rects recorded")
                .into_iter()
                .find(|(z, _)| *z == want)
                .map(|(_, r)| r.center())
                .expect("zoom variant recorded")
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
        /// Horizontal drag (BPM DragValue): down at `from`, move by `dx`, up.
        fn drag_horizontal(&self, from: egui::Pos2, dx: f32) {
            let to = egui::Pos2::new(from.x + dx, from.y);
            let mods = egui::Modifiers::default();
            let mut a = raw_input();
            a.events.push(egui::Event::PointerButton {
                pos: from,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: mods,
            });
            self.frame(a);
            let mut m = raw_input();
            m.events.push(egui::Event::PointerMoved(to));
            self.frame(m);
            let mut rel = raw_input();
            rel.events.push(egui::Event::PointerButton {
                pos: to,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: mods,
            });
            self.frame(rel);
        }
        fn cmds(&self) -> Vec<Command> {
            self.sink.0.lock().unwrap().clone()
        }
    }

    // ---- pure oracle tests ----

    #[test]
    fn clamp_bpm_bounds() {
        assert_eq!(clamp_bpm(120.0), 120.0);
        assert_eq!(clamp_bpm(0.0), MIN_BPM);
        assert_eq!(clamp_bpm(-5.0), MIN_BPM);
        assert_eq!(clamp_bpm(999.0), MAX_BPM);
    }

    #[test]
    fn bpm_edit_command_clamps() {
        assert!(matches!(
            bpm_edit_command(999.0),
            Command::SetBpm { bpm: 400.0 }
        ));
        assert!(matches!(
            bpm_edit_command(5.0),
            Command::SetBpm { bpm: 20.0 }
        ));
        assert!(matches!(
            bpm_edit_command(130.0),
            Command::SetBpm { bpm: 130.0 }
        ));
    }

    #[test]
    fn sync_label_variants() {
        assert_eq!(sync_label(SyncSource::Free), "Free");
        assert_eq!(sync_label(SyncSource::MidiClock), "MIDI");
        assert_eq!(sync_label(SyncSource::Link), "Link");
    }

    #[test]
    fn seed_and_clear_inflight() {
        // in-flight wins over mirror while echo is pending
        assert_eq!(seed_bpm(120.0, None), 120.0);
        assert_eq!(seed_bpm(140.0, Some(150.0)), 150.0);
        // drop the override once the mirror catches up
        assert!(clear_inflight(140.0, Some(140.0)));
        assert!(!clear_inflight(140.0, Some(150.0)));
        assert!(!clear_inflight(140.0, None));
    }

    #[test]
    fn uistate_bpm_sync_accessors() {
        let mut st = UiState::default();
        assert_eq!(st.bpm(), 120.0); // pre-snapshot default
        assert_eq!(st.sync_source(), SyncSource::Free);

        let s = Session {
            bpm: 88.0,
            sync_source: SyncSource::Link,
            ..Default::default()
        };
        st.session = Some(Arc::new(s));
        assert_eq!(st.bpm(), 88.0);
        assert_eq!(st.sync_source(), SyncSource::Link);
    }

    // ---- headless harness tests (e2e wiring) ----

    #[test]
    fn transport_play_stop_reflects_actual_state() {
        // stopped → click emits Play (⊥ optimistic: button reads state.playing)
        let h = Harness::new(UiState::default()); // playing = false
        h.click(h.play_center());
        assert!(matches!(h.cmds().as_slice(), [Command::Play]));

        // engine echoes PlayStateChanged{true} → state.playing = true → Stop
        let playing = UiState {
            playing: true,
            ..UiState::default()
        };
        let h2 = Harness::new(playing);
        h2.click(h2.play_center());
        assert!(matches!(h2.cmds().as_slice(), [Command::Stop]));
    }

    #[test]
    fn transport_bpm_drag_emits_clamped_setbpm() {
        let h = Harness::new(UiState::default()); // bpm 120
        let pos = h.bpm_center();
        h.drag_horizontal(pos, 5000.0); // large right drag → range-clamps to MAX
        let cmds = h.cmds();
        assert!(!cmds.is_empty(), "drag must emit SetBpm");
        assert!(
            cmds.iter()
                .all(|c| matches!(c, Command::SetBpm { bpm } if *bpm > 120.0 && *bpm <= MAX_BPM)),
            "all emitted bpms must be in (120, {MAX_BPM}]"
        );
        // the large drag reaches the clamp ceiling
        let max_bpm = cmds
            .iter()
            .filter_map(|c| match c {
                Command::SetBpm { bpm } => Some(*bpm),
                _ => None,
            })
            .fold(0.0_f64, f64::max);
        assert_eq!(max_bpm, MAX_BPM, "drag should reach MAX_BPM");
    }

    #[test]
    fn transport_zoom_toggle_writes_shared_grid_state() {
        let h = Harness::new(UiState::default());
        h.idle();
        assert_eq!(read_grid(&h.ctx).zoom, Zoom::Sixteen); // default
        h.click(h.zoom_center(Zoom::Eight));
        assert_eq!(read_grid(&h.ctx).zoom, Zoom::Eight);
        h.click(h.zoom_center(Zoom::Sixteen));
        assert_eq!(read_grid(&h.ctx).zoom, Zoom::Sixteen);
    }

    #[test]
    fn transport_sync_badge_emits_no_setsyncsource() {
        let s = Session {
            sync_source: SyncSource::Link,
            ..Default::default()
        };
        let st = UiState {
            session: Some(Arc::new(s)),
            link_peers: 3,
            ..UiState::default()
        };
        let h = Harness::new(st);
        h.idle();
        // exercise every transport control — none may emit SetSyncSource
        h.click(h.play_center());
        h.drag_horizontal(h.bpm_center(), 200.0);
        h.click(h.zoom_center(Zoom::Eight));
        assert!(
            h.cmds()
                .iter()
                .all(|c| !matches!(c, Command::SetSyncSource { .. })),
            "sync badge is read-only"
        );
    }

    #[test]
    fn transport_render_no_session_no_panic() {
        let h = Harness::new(UiState::default()); // no session
        h.idle();
        h.click(h.zoom_center(Zoom::Eight)); // zoom toggle works w/o session
        assert_eq!(read_grid(&h.ctx).zoom, Zoom::Eight);
    }
}
