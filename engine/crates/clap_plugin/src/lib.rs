//! StepForge CLAP plugin (Phase 0 skeleton).

mod midi;
mod params;
mod transport;
// `editor` module added in Task 8.

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
        _buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        // Filled in Task 6.
        true
    }

    fn process(
        &mut self,
        _buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // Filled in Task 6.
        ProcessStatus::Normal
    }

    fn deactivate(&mut self) {
        // Filled in Task 6.
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        None // Filled in Task 8.
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
