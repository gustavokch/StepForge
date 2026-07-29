//! StepForge CLAP plugin (Phase 0 skeleton).

mod editor;
mod midi;
mod params;
mod transport;

use nih_plug::prelude::*;
use parking_lot::{Mutex, RwLock};
use std::sync::Arc;

use params::StepForgeParams;
use sequencer_engine::engine::Engine;
use sequencer_engine::host::HostRenderState;
use stepforge_editor_egui::UiState;

pub struct StepForge {
    engine: Arc<Engine>,
    host_render_state: HostRenderState,
    sample_rate: f32,
    params: Arc<StepForgeParams>,
    /// State-worker JoinHandle, spawned in `initialize`, joined in `deactivate`.
    worker_handle: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// GUI mirror; cloned into the editor closure in Task 8.
    ui_state: Arc<RwLock<UiState>>,
}

impl Default for StepForge {
    fn default() -> Self {
        Self {
            engine: Arc::new(Engine::new_host_driven()),
            host_render_state: HostRenderState::new(),
            sample_rate: 48000.0,
            params: Arc::new(StepForgeParams::default()),
            worker_handle: Mutex::new(None),
            ui_state: Arc::new(RwLock::new(UiState::default())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_constructs() {
        let p = StepForge::default();
        assert_eq!(p.sample_rate, 48000.0);
        let _ = p.params.clone();
    }
}

impl Plugin for StepForge {
    const NAME: &'static str = "StepForge";
    const VENDOR: &'static str = "StepForge";
    const URL: &'static str = "https://github.com/gus/StepForge";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[]; // MIDI-only (Phase 0)
    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::Basic;
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn initialize(
        &mut self,
        _layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;

        // Host-driven mode does NOT auto-spawn the state worker — do it once here.
        let mut wh = self.worker_handle.lock();
        if wh.is_none() {
            let e = Arc::clone(&self.engine);
            *wh = Some(
                std::thread::Builder::new()
                    .name("stepforge-worker".into())
                    .spawn(move || e.run_worker_loop())
                    .expect("spawn state-worker"),
            );
        }
        drop(wh);

        // State is deserialized into params.session BEFORE initialize; restore it.
        let bytes = self.params.session.read().clone();
        if !bytes.is_empty() {
            let _ = sequencer_engine::midi_out::push_drop_oldest(
                &self.engine.commands,
                sequencer_engine::command::Command::LoadSession { bytes },
            );
        }
        true
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let tr = context.transport();
        let transport = transport::map_transport(
            tr.tempo,
            tr.playing,
            tr.pos_beats(),
            tr.bar_start_pos_beats(),
            self.sample_rate,
            buffer.samples() as u32,
        );

        // Stack-allocated, fixed-size output buffer. MidiEvent: Copy.
        let mut midi_out: [sequencer_engine::host::MidiEvent; 1024] =
            [sequencer_engine::host::MidiEvent::zero(); 1024];

        let n =
            self.engine
                .render_host(&mut self.host_render_state, &transport, &[], &mut midi_out);

        for ev in &midi_out[..n] {
            if let Some(note) = midi::midi_event_to_note(ev) {
                context.send_event(note);
            }
        }

        ProcessStatus::Normal
    }

    fn deactivate(&mut self) {
        self.engine
            .shutdown
            .store(true, std::sync::atomic::Ordering::Release);
        if let Some(handle) = self.worker_handle.lock().take() {
            let _ = handle.join();
        }
        // Reset render state so a possible re-initialize starts clean.
        self.host_render_state = HostRenderState::new();
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::spawn_editor(
            Arc::clone(&self.engine),
            Arc::clone(&self.ui_state),
            Arc::clone(&self.params.session),
            self.params.editor_state.clone(),
        )
    }
}

impl ClapPlugin for StepForge {
    const CLAP_ID: &'static str = "org.stepforge.clap";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("MIDI drum sequencer");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::NoteEffect];
}

nih_export_clap!(StepForge);
