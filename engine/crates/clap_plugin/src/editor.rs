//! Editor spawn + per-frame GUI tick: drain RT→GUI events into UiState,
//! throttle-refresh the authoritative snapshot + serialize-for-save, then render.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, EguiState};
use parking_lot::{RwLock, RwLockReadGuard};

use sequencer_engine::command::Command;
use sequencer_engine::engine::Engine;
use sequencer_engine::event::EngineEvent;
use sequencer_engine::midi_out::push_drop_oldest;
use sequencer_engine::serde_ext::{SessionEnvelope, SESSION_FORMAT_VERSION};
use stepforge_editor_egui::{apply_theme, render, CommandSink, UiState};

pub struct EngineCommandSink {
    pub engine: Arc<Engine>,
}
impl CommandSink for EngineCommandSink {
    fn push(&self, cmd: Command) {
        let _ = push_drop_oldest(&self.engine.commands, cmd);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_editor(
    engine: Arc<Engine>,
    ui_state: Arc<RwLock<UiState>>,
    session: Arc<RwLock<Vec<u8>>>,
    egui_state: Arc<EguiState>,
) -> Option<Box<dyn Editor>> {
    let frame = Arc::new(AtomicU64::new(0));
    create_egui_editor(
        egui_state,
        (),
        |_ctx, _user_state: &mut ()| {
            // build closure runs once before the first frame.
        },
        move |ctx, _setter, _user_state: &mut ()| {
            let n = frame.fetch_add(1, Ordering::Relaxed) + 1;
            tick(&engine, &ui_state, &session, n);
            apply_theme(ctx);
            let st: RwLockReadGuard<'_, UiState> = ui_state.read();
            render(
                ctx,
                &st,
                &EngineCommandSink {
                    engine: engine.clone(),
                },
            );
        },
    )
}

fn tick(
    engine: &Arc<Engine>,
    ui_state: &Arc<RwLock<UiState>>,
    session: &Arc<RwLock<Vec<u8>>>,
    frame: u64,
) {
    {
        let mut st = ui_state.write();
        // Hot channel: small fixed-slot events (Phase 0: just PlayStateChanged).
        while let Some(slot) = engine.hot_events.dequeue() {
            if let Ok(EngineEvent::PlayStateChanged { playing }) =
                postcard::from_bytes::<EngineEvent>(&slot.bytes[..slot.len as usize])
            {
                st.playing = playing;
                // Remaining variants are ported in Phase 1.
            }
        }
        // Large channel: discard for Phase 0 (Serialized/FullSnapshot handled via snapshot_arc below).
        while engine.large_events.dequeue().is_some() {}

        // Throttled authoritative snapshot refresh + serialize-for-save (~1 Hz at 60 fps).
        if frame.is_multiple_of(60) {
            let snap = engine.snapshot_arc();
            st.session = Some(snap.clone());
            let env = SessionEnvelope {
                version: SESSION_FORMAT_VERSION,
                session: (*snap).clone(),
            };
            if let Ok(bytes) = postcard::to_allocvec(&env) {
                *session.write() = bytes;
            }
        }
    }
}
