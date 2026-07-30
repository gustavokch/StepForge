//! StepForge egui editor — pure UI, no nih_plug dependency.

pub mod ui_state;
pub use ui_state::UiState;

use egui::Context;
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

/// Render the Phase 0 editor: BPM readout (from snapshot) + play/stop toggle.
pub fn render(ctx: &Context, ui_state: &UiState, sink: &impl CommandSink) {
    egui::CentralPanel::default().show(ctx, |ui| {
        let bpm = ui_state.session.as_ref().map(|s| s.bpm).unwrap_or(120.0);
        ui.heading(format!("StepForge — {:.1} BPM", bpm));
        if ui
            .button(if ui_state.playing {
                "■ Stop"
            } else {
                "▶ Play"
            })
            .clicked()
        {
            sink.push(transport_action(ui_state.playing));
        }

        // Surface engine errors (rolling last-N from UiState). Phase 0: read-only.
        if !ui_state.errors.is_empty() {
            ui.separator();
            ui.label(egui::RichText::new("engine errors:").color(egui::Color32::LIGHT_RED));
            for msg in &ui_state.errors {
                ui.label(format!("• {msg}"));
            }
        }
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
