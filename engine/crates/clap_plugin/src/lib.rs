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
use sequencer_engine::host::{HostRenderState, MidiEvent};
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
    /// Reused RT output buffer (allocated once off-RT) — avoids zeroing 8 KB on
    /// the audio thread every block. `render_host` writes `[..n]`, we read that.
    midi_buf: Box<[MidiEvent; 1024]>,
    /// Last transport play state, for emitting all-notes-off on the stop edge.
    was_playing: bool,
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
            midi_buf: Box::new([MidiEvent::zero(); 1024]),
            was_playing: false,
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

    /// Regression for the PR #12 re-activation bug: `deactivate` latches
    /// `shutdown=true`; the next `initialize`/`ensure_worker` must clear it or
    /// the freshly spawned worker exits immediately and drains nothing. We
    /// prove the worker is alive by observing a drained `SetBpm` publish to the
    /// snapshot. A dead worker never publishes, so the poll times out → fail.
    #[test]
    fn reactivation_keeps_worker_alive() {
        use sequencer_engine::command::Command;
        use sequencer_engine::midi_out::push_drop_oldest;
        use std::time::{Duration, Instant};

        fn bpm_reaches(p: &StepForge, bpm: f64) -> bool {
            let deadline = Instant::now() + Duration::from_secs(1);
            while Instant::now() < deadline {
                if (p.engine.snapshot_arc().bpm - bpm).abs() < 1e-9 {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            false
        }

        let mut p = StepForge::default();

        p.ensure_worker();
        let _ = push_drop_oldest(&p.engine.commands, Command::SetBpm { bpm: 177.0 });
        assert!(
            bpm_reaches(&p, 177.0),
            "first activation: worker must drain SetBpm"
        );

        // deactivate then re-activate on the SAME instance.
        p.teardown();
        p.ensure_worker();
        let _ = push_drop_oldest(&p.engine.commands, Command::SetBpm { bpm: 99.0 });
        assert!(
            bpm_reaches(&p, 99.0),
            "re-activation: worker must drain SetBpm (shutdown latch regression)"
        );

        p.teardown();
    }

    /// V11: `reset()` runs on the RT thread after `initialize()` / host resets
    /// (no intervening `deactivate`), so it must clear stale render state on its
    /// own. Dirty the fields `reset` owns first; a no-op `reset()` would leave
    /// them dirty → fail.
    #[allow(clippy::field_reassign_with_default)] // intentional precondition setup
    #[test]
    fn reset_clears_render_state() {
        let mut p = StepForge::default();
        // Dirty the state `reset()` owns (mimic a playhead mid-flight).
        p.was_playing = true;
        p.host_render_state.initialized = true;
        p.host_render_state.sample_time = 123_456;
        p.host_render_state.was_playing = true;
        p.host_render_state.next_step_beat = 7.5;

        p.reset();

        assert!(!p.was_playing, "reset clears wrapper was_playing");
        assert!(
            !p.host_render_state.initialized,
            "reset re-inits HostRenderState"
        );
        assert_eq!(
            p.host_render_state.sample_time, 0,
            "reset realigns sample_time"
        );
        assert!(
            !p.host_render_state.was_playing,
            "reset clears internal was_playing"
        );
        assert_eq!(
            p.host_render_state.next_step_beat, 0.0,
            "reset realigns next_step_beat"
        );
    }
}

impl StepForge {
    /// (Re)spawn the state worker. Resets the shutdown latch first: CLAP hosts
    /// may deactivate then re-activate the *same* instance, and `deactivate`
    /// sets `shutdown=true`. Without this reset the freshly spawned worker sees
    /// `while !shutdown` and returns immediately — leaving a dead worker that
    /// drains no commands. Mirrors `engine_start` in the FFI crate.
    fn ensure_worker(&mut self) {
        self.engine
            .shutdown
            .store(false, std::sync::atomic::Ordering::Release);
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
    }

    /// Join the worker, latch shutdown, and reset per-activation state. The body
    /// of `deactivate`; factored out so the lifecycle is unit-testable without a
    /// host `ProcessContext`.
    fn teardown(&mut self) {
        self.engine
            .shutdown
            .store(true, std::sync::atomic::Ordering::Release);
        if let Some(handle) = self.worker_handle.lock().take() {
            let _ = handle.join();
        }
        self.host_render_state = HostRenderState::new();
        self.was_playing = false;
    }
}

impl Plugin for StepForge {
    const NAME: &'static str = "StepForge";
    const VENDOR: &'static str = "StepForge";
    const URL: &'static str = "https://github.com/gustavokch/StepForge";
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

        // Host-driven mode does NOT auto-spawn the state worker — do it here.
        // ensure_worker also clears the shutdown latch so re-activation works.
        self.ensure_worker();

        // State is deserialized into params.session BEFORE initialize; restore it.
        let bytes = self.params.session.read().clone();
        if !bytes.is_empty() {
            let _ = sequencer_engine::midi_out::push_drop_oldest(
                &self.engine.commands,
                sequencer_engine::command::Command::LoadSession { bytes },
            );
        } else {
            // No saved state: seed a demo beat so transport sync is audible even
            // before the editor gains step-editing (Phase 1+). The standalone app
            // has the user tap steps in; this plugin has no step UI yet.
            let env = sequencer_engine::serde_ext::SessionEnvelope {
                version: sequencer_engine::serde_ext::SESSION_FORMAT_VERSION,
                session: demo_session(),
            };
            if let Ok(demo_bytes) = postcard::to_allocvec(&env) {
                let _ = sequencer_engine::midi_out::push_drop_oldest(
                    &self.engine.commands,
                    sequencer_engine::command::Command::LoadSession { bytes: demo_bytes },
                );
            }
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

        // All-notes-off on the play→stop edge. `render_host` emits CC 123 for the
        // same transition, but this structured-note port forwards NoteOn/NoteOff
        // only (CC 123 is dropped by `midi_event_to_note`), so emit explicit
        // NoteOffs across every channel/note as the last line of defense against
        // stuck notes. Fires once per transition; matches by channel+note
        // (voice_id unknown for a forced off). No snapshot read on the RT path.
        if self.was_playing && !transport.is_playing {
            for channel in 0u8..16 {
                for note in 0u8..128 {
                    context.send_event(NoteEvent::NoteOff {
                        timing: 0,
                        voice_id: None,
                        channel,
                        note,
                        velocity: 0.0,
                    });
                }
            }
        }
        self.was_playing = transport.is_playing;

        // RT output: reused buffer (allocated once off-RT), writes [..n] only.
        let n = self.engine.render_host(
            &mut self.host_render_state,
            &transport,
            &[],
            &mut self.midi_buf[..],
        );

        for ev in &self.midi_buf[..n] {
            if let Some(note) = midi::midi_event_to_note(ev) {
                context.send_event(note);
            }
        }

        ProcessStatus::Normal
    }

    fn deactivate(&mut self) {
        // Join the worker, latch shutdown, reset per-activation state. Re-entrant
        // via `ensure_worker` on the next `initialize`.
        self.teardown();
    }

    /// nih_plug calls this on the RT thread after every `initialize()` and on
    /// host resets — with NO intervening `deactivate`. So unlike `deactivate`,
    /// the per-block `host_render_state` is still live (carrying a playhead from
    /// the previous activation) when a preset/project swap re-inits an active
    /// instance. Re-init it here so render resumes from a clean, stopped
    /// baseline instead of a misaligned playhead + stale deferred MIDI.
    ///
    /// Alloc-free (fixed arrays only) — runs on the RT thread. Mirrors the
    /// `host_render_state`/`was_playing` reset in `teardown`; both resetting is
    /// intended and cheap. Does NOT touch the worker (spawned in `initialize`)
    /// or the shutdown latch — RT thread can't join/spawn. (V11)
    fn reset(&mut self) {
        self.host_render_state = HostRenderState::new();
        self.was_playing = false;
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

/// A simple 4-on-the-floor beat so the plugin is audible out of the box (and
/// transport sync can be validated) before the editor gains step-editing (Phase 1+).
fn demo_session() -> sequencer_engine::models::Session {
    use sequencer_engine::models::VelocityZone;
    let mut s = sequencer_engine::models::Session::default();
    // Track→note mapping below is coupled to `Pattern::default()`'s track order
    // (0=Kick(36), 1=Snare(38), 2=Hat(42), 3=Clap(39)). All accesses are
    // bounds-guarded, so a future `Pattern::default()` change only shifts timbre,
    // never panics — but update the indices here to keep the demo beat intact.
    if let Some(pattern) = s.patterns[0].as_mut() {
        let mut activate = |track: usize, steps: &[usize], zone: VelocityZone| {
            if let Some(t) = pattern.tracks.get_mut(track) {
                for &i in steps {
                    if let Some(st) = t.steps.get_mut(i) {
                        st.active = true;
                        st.velocity_zone = zone;
                    }
                }
            }
        };
        activate(0, &[0, 4, 8, 12], VelocityZone::Accent); // four-on-the-floor kick
        activate(1, &[4, 12], VelocityZone::Accent); // backbeat snare
        activate(2, &[0, 2, 4, 6, 8, 10, 12, 14], VelocityZone::Mid); // eighth-note hats
    }
    s
}
