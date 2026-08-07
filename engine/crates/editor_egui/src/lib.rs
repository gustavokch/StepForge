//! StepForge egui editor — pure UI, no nih_plug dependency.

pub mod action_drawer;
pub mod feel;
pub mod grid;
pub mod note_picker;
pub mod overlay;
pub mod pattern_options;
pub mod performance;
pub mod settings;
#[cfg(test)]
mod test_support;
pub mod theme;
pub mod track_management;
pub mod transport;
pub mod ui_state;
pub use ui_state::UiState;

use egui::{Color32, Context, Id};
use sequencer_engine::command::Command;

/// Sink for commands emitted by UI interactions.
pub trait CommandSink {
    fn push(&self, cmd: Command);
}

// ---- Editor frame counter (open-frame guard for the T11 overlays) ----
//
// egui 0.31 exposes no frame number on `Context`, so the editor keeps its own
// monotonically-increasing counter in `ctx.data` temp storage, ticked once at
// the top of [`render`]. The ActionDrawer + NotePickerSheet record the counter
// value in their state on `open`, then suppress the outside-click dismiss on
// that same frame: `pointer.primary_clicked()` is global and not
// consumption-aware, so the header click that opened the overlay is still
// "primary_clicked" when the overlay renders one statement later this same
// frame — without the guard it self-dismisses for any track whose header lands
// outside the overlay rect. (The ratchet popover avoids this with `!alt`; a
// plain-click open has no modifier.)
pub(crate) fn frame_id() -> Id {
    Id::new("stepforge.frame")
}
pub(crate) fn tick_frame(ctx: &Context) {
    ctx.data_mut(|d| *d.get_temp_mut_or_default::<u64>(frame_id()) += 1);
}
pub(crate) fn frame_nr(ctx: &Context) -> u64 {
    ctx.data(|d| d.get_temp::<u64>(frame_id()).unwrap_or(0))
}

// ---- Editor AppMode (T12): Editing ↔ Performance ----
//
// Port of the iOS `AppMode` enum (`app/StepForge/Features/AppMode.swift:7`).
// Widget-local state in `ctx.data` temp storage (same idiom as `grid::Zoom`):
// `render` branches on it — Editing renders the step grid, Performance renders
// `performance::render_performance_view`. No engine command on switch (iOS
// `@State mode`); the toggle lives in the TransportBar (the persistent top bar
// across both modes, mirroring the iOS `appBar`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AppMode {
    #[default]
    Editing,
    Performance,
}

fn mode_id() -> Id {
    Id::new("stepforge.mode")
}
/// `pub(crate)` so the TransportBar toggle (T10c) + tests read/write the same slot.
pub(crate) fn read_mode(ctx: &Context) -> AppMode {
    ctx.data(|d| d.get_temp::<AppMode>(mode_id()).unwrap_or_default())
}
pub(crate) fn write_mode(ctx: &Context, m: AppMode) {
    ctx.data_mut(|d| d.insert_temp(mode_id(), m));
}

/// Pure: which transport command a play/stop toggle emits given current state.
pub fn transport_action(playing: bool) -> Command {
    if playing {
        Command::Stop
    } else {
        Command::Play
    }
}

/// Dark graphite theme (Phase 0 minimal; full palette in Phase 4).
pub fn apply_theme(ctx: &Context) {
    let mut v = egui::Visuals::dark();
    v.override_text_color = Some(egui::Color32::WHITE);
    ctx.set_visuals(v);
}

/// Render the editor: the Phase 1 §T T10c `TransportBar` (play/stop, BPM,
/// read-only sync badge, zoom toggle), the Phase 1 §T T10d `FeelBar` (swing,
/// humanize, quantize grain, pattern switcher — Row 2), the Phase 1 §T T10e
/// `TrackManagementBar` (track count + add/remove — Row 3), the engine-error
/// surface, and the Phase 1 §T T10b step grid.
pub fn render(ctx: &Context, ui_state: &UiState, sink: &impl CommandSink) {
    tick_frame(ctx); // advance the editor frame counter (overlay open-frame guard)
    egui::CentralPanel::default().show(ctx, |ui| {
        transport::render_transport_bar(ui, ui_state, sink);

        ui.separator();
        // Phase 1 §T T10d — FeelBar (Row 2): swing / humanize / grain / patterns.
        feel::render_feel_bar(ui, ui_state, sink);

        ui.separator();
        // Phase 1 §T T10e — TrackManagementBar (Row 3): track count + add/remove.
        track_management::render_track_management_bar(ui, ui_state, sink);

        // Surface the latest engine error (read-only) — T8.
        if let Some(err) = &ui_state.last_error {
            ui.separator();
            ui.label(
                egui::RichText::new(format!("engine error [{}]: {}", err.code, err.message))
                    .color(Color32::LIGHT_RED),
            );
        }

        ui.separator();
        // Phase 3 §T T12 — AppMode branch: Editing renders the step grid,
        // Performance renders the PerformanceView (iOS `AppMode` parity — the
        // view is fully replaced; only the persistent top bars stay). Default
        // is Editing, so all existing grid/overlay tests see the grid unchanged.
        match read_mode(ui.ctx()) {
            AppMode::Editing => {
                // Phase 1 §T T10b — step grid (pinned headers + step cells + gestures).
                grid::render_step_grid(ui, ui_state, sink);
            }
            AppMode::Performance => {
                // Phase 3 §T T12 — PerformanceView: large play/stop, 3×3 pattern
                // grid, track LEDs/mutes, quantize selector.
                performance::render_performance_view(ui, ui_state, sink);
            }
        }

        // Phase 2 §T T11 — track-level overlays. Rendered last as floating
        // `egui::Area`s so they float above the whole editor (not just the
        // grid). Each is a no-op unless its widget-local target is set; the two
        // are mutually exclusive (opening one clears the other's target). The
        // grid header drum-name tap opens the NotePicker; the `…` button opens
        // the ActionDrawer. (Editing-only by nature — their targets are set only
        // from grid header gestures; the AppMode toggle closes them on the
        // switch to Performance so nothing dangles over the PerformanceView.)
        note_picker::render_note_picker(ui.ctx(), ui_state, sink);
        action_drawer::render_action_drawer(ui.ctx(), ui_state, sink);
        // Phase 3 §T T12 — PatternOptionsSheet overlay (Performance-only trigger:
        // a pattern cell's `…` gear). Floating `egui::Area`; no-op when its
        // target is None. The AppMode toggle closes it on the switch to Editing.
        pattern_options::render_pattern_options(ui.ctx(), ui_state, sink);
        // Phase 4 §T T13a — SettingsSheet overlay (mode-agnostic; gear in the
        // TransportBar opens it). Floating `egui::Area`; no-op when closed.
        settings::render_settings(ui.ctx(), ui_state, sink);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[derive(Default)]
    struct RecordingSink(std::sync::Mutex<Vec<Command>>);
    impl CommandSink for RecordingSink {
        fn push(&self, cmd: Command) {
            self.0.lock().unwrap().push(cmd);
        }
    }

    #[test]
    fn transport_action_toggles() {
        assert!(matches!(transport_action(false), Command::Play));
        assert!(matches!(transport_action(true), Command::Stop));
    }

    #[test]
    fn render_does_not_panic() {
        let ctx = Context::default();
        let sink = RecordingSink::default();
        let mut state = UiState::default();
        // egui requires panels to be laid out inside a frame run; calling
        // `render` on a bare `Context::default()` panics with
        // "Called `available_rect()` before `Context::run()`".
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            render(ctx, &state, &sink);
        });
        state.session = Some(Arc::new(sequencer_engine::models::Session::default()));
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            render(ctx, &state, &sink);
        });
    }
}
