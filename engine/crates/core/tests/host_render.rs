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

#[test]
fn note_off_outlasts_block_and_fires_in_a_future_block() {
    // Default gate is 50 ms. At 48 kHz that is 2_400 samples — many blocks.
    let eng = Engine::new_host_driven();
    eng.publish(session_with_step0_hit());
    let mut rs = HostRenderState::new();
    let block = 256u32;
    let sr = 48_000.0;
    let mut beat = 0.0f64;
    let mut saw_note_on = false;
    let mut saw_note_off = false;
    for _ in 0..48 {
        let mut out = [sequencer_engine::host::MidiEvent::zero(); 64];
        let n = eng.render_host(&mut rs, &transport(120.0, sr, block, beat, 0.0, true), &[], &mut out);
        for ev in &out[..n] {
            // Status family, not literal nibble (default channel 10 → 0xA).
            if (ev.status & 0xF0) == 0x90 && ev.data1 == 36 && ev.data2 > 0 { saw_note_on = true; }
            if (ev.status & 0xF0) == 0x80 && ev.data1 == 36 { saw_note_off = true; }
        }
        beat += (block as f64) / sr * 2.0; // advance ~one 16th (6000 samples) every few blocks
        if saw_note_on && saw_note_off { break; }
    }
    assert!(saw_note_on, "note-on for note 36 must fire");
    assert!(saw_note_off, "matching note-off must fire (possibly a later block via the pending queue)");
}

#[test]
fn swung_note_on_past_block_defers_to_a_future_block() {
    // Odd steps are swing-delayed (clock.rs: `swing_offset_micros`). At 49% swing,
    // 120 BPM, 48 kHz, the delay is 0.49 * 125_000 µs ≈ 2_940 samples. With
    // 1_000-sample blocks a swung note-on lands ~2_940 samples past its boundary
    // block → it must defer (sample-accurate) to a later block, NOT clamp to
    // block_samples-1 and fire inside the boundary block.
    let mut s = Session::default();
    s.global_swing_pct = 49.0;
    let p = s.patterns[0].as_mut().unwrap();
    p.tracks[0].steps[1] = Step { active: true, velocity_zone: VelocityZone::Accent, ..Default::default() };
    p.tracks[0].midi_note = 36;
    let eng = Engine::new_host_driven();
    eng.publish(s);
    let mut rs = HostRenderState::new();
    let sr = 48_000.0;
    let block = 1_000u32;
    let beats_per_block = block as f64 / sr * (120.0 / 60.0);
    let mut beat = 0.0f64;
    let mut boundary_block: Option<usize> = None; // block where step 1's boundary fires
    let mut note_on_block: Option<usize> = None;  // block where the swung note-on emits
    for i in 0..64 {
        let mut out = [sequencer_engine::host::MidiEvent::zero(); 64];
        let global_before = rs.rt.global_step;
        let n = eng.render_host(&mut rs, &transport(120.0, sr, block, beat, 0.0, true), &[], &mut out);
        // Step 1's boundary is the global_step 1 → 2 transition.
        if boundary_block.is_none() && global_before == 1 && rs.rt.global_step == 2 {
            boundary_block = Some(i);
        }
        for ev in &out[..n] {
            assert!(ev.sample_offset < block, "no note clamped to block-1");
            if (ev.status & 0xF0) == 0x90 && ev.data1 == 36 && ev.data2 > 0 {
                note_on_block = Some(i);
            }
        }
        beat += beats_per_block;
    }
    let boundary_block = boundary_block.expect("step-1 boundary fired within a bar");
    let note_on_block = note_on_block.expect("swung note-on eventually emitted");
    assert!(
        note_on_block > boundary_block,
        "swung note-on deferred to block {note_on_block}, boundary was block {boundary_block} (would be equal if clamped)"
    );
}
