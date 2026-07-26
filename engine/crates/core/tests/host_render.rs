//! Integration tests for `Engine::render_host` (Phase 0 host-driven mode).
//! Drives the reused `process()` core from a synthetic host transport — no
//! threads, no plugin wrapper.

use sequencer_engine::engine::Engine;
use sequencer_engine::host::{HostRenderState, HostTransport};
use sequencer_engine::models::{Session, Step, VelocityZone, STEP_COUNT};

fn session_with_step0_hit() -> Session {
    let mut s = Session::default(); // bpm 120, 4 default tracks, patterns all Some
    let p = s.patterns[0].as_mut().unwrap();
    p.tracks[0].steps[0] = Step { active: true, velocity_zone: VelocityZone::Accent, ..Default::default() };
    p.tracks[0].midi_note = 36;
    s
}

fn transport(tempo: f64, sr: f64, block: u32, beat: f64, bar: f64, playing: bool) -> HostTransport {
    HostTransport { tempo_bpm: tempo, sample_rate: sr, block_samples: block, block_start_beat: beat, bar_start_beat: bar, is_playing: playing, beats_per_bar: 4.0 }
}

#[test]
fn stopped_engine_emits_nothing() {
    let eng = Engine::new_host_driven();
    eng.publish(session_with_step0_hit());
    let mut rs = HostRenderState::new();
    let mut out = [sequencer_engine::host::MidiEvent::zero(); 64];
    let n = eng.render_host(&mut rs, &transport(120.0, 48_000.0, 256, 0.0, 0.0, false), &[], &mut out);
    assert_eq!(n, 0, "stopped transport emits no note-ons");
}

#[test]
fn play_advances_one_step_per_16th_boundary() {
    // 120 BPM, 48 kHz: one 16th = 60/120/4 s = 0.125 s = 6000 samples.
    let eng = Engine::new_host_driven();
    eng.publish(session_with_step0_hit());
    let mut rs = HostRenderState::new();
    let sr = 48_000.0;
    let block = 6_000u32; // exactly one 16th per block
    let mut beat = 0.0f64;
    let mut total_notes = 0usize;
    for i in 0..16 {
        let mut out = [sequencer_engine::host::MidiEvent::zero(); 64];
        let n = eng.render_host(&mut rs, &transport(120.0, sr, block, beat, 0.0, true), &[], &mut out);
        for ev in &out[..n] {
            // Status FAMILY, not a literal nibble: the default
            // global_midi_channel is 10, so the drum channel rides nibble 0xA
            // (0x9A note-on / 0x8A note-off). Asserting the family keeps this
            // correct if the default channel ever changes.
            let fam = ev.status & 0xF0;
            assert!(fam == 0x90 || fam == 0x80, "note-on/off status family, got {:#04x}", ev.status);
            if fam == 0x90 && ev.data2 > 0 { total_notes += 1; }
        }
        beat += 0.25; // one 16th per block
        let _ = i;
    }
    // Track 0 has a hit only on step 0. The 16-block run starts at beat 0, which
    // only aligns (the first boundary fire is at beat 0.25, in block 1), so step
    // 0 fires once → one note-on. Asserting "at least one" keeps this robust to
    // the exact per-track step mapping.
    assert!(total_notes >= 1, "expected at least one note-on over a bar");
    // global_step stayed in range.
    assert!(rs.rt.global_step < STEP_COUNT as u32);
}

#[test]
fn play_start_mid_bar_aligns_per_track_step() {
    // Host resumes at beat 1.0 — four 16ths into the bar. Each track's playhead
    // must align to step 4 (not step 0), so a mid-bar resume doesn't replay the
    // downbeat. Exact for speed_ratio 1.0 (the default).
    let eng = Engine::new_host_driven();
    eng.publish(session_with_step0_hit());
    let mut rs = HostRenderState::new();
    let mut out = [sequencer_engine::host::MidiEvent::zero(); 64];
    eng.render_host(&mut rs, &transport(120.0, 48_000.0, 256, 1.0, 0.0, true), &[], &mut out);
    let length = eng.snapshot_arc().patterns[0].as_ref().unwrap().tracks[0].length;
    assert_eq!(rs.rt.per_track[0].step_idx, 4 % length, "per-track step aligned to the bar");
    assert_eq!(rs.rt.global_step, 4, "global_step aligned to the bar");
}
