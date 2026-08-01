//! StepForge egui editor — pure UI, no nih_plug dependency.

pub mod feel;
pub mod grid;
pub mod track_management;
pub mod transport;
pub mod ui_state;
pub use ui_state::UiState;

use egui::{Color32, Context};
use sequencer_engine::command::Command;

/// Sink for commands emitted by UI interactions.
pub trait CommandSink {
    fn push(&self, cmd: Command);
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
        // Phase 1 §T T10b — step grid (pinned headers + step cells + gestures).
        grid::render_step_grid(ui, ui_state, sink);
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
