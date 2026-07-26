//! Integration tests for `Engine::render_host` (Phase 0 host-driven mode).
//! Drives the reused `process()` core from a synthetic host transport — no
//! threads, no plugin wrapper.

use proptest::prelude::*;
use sequencer_engine::engine::Engine;
use sequencer_engine::host::{HostRenderState, HostTransport};
use sequencer_engine::models::{Session, Step, VelocityZone, STEP_COUNT};

fn session_with_step0_hit() -> Session {
    let mut s = Session::default(); // bpm 120, 4 default tracks, patterns all Some
    let p = s.patterns[0].as_mut().unwrap();
    p.tracks[0].steps[0] = Step {
        active: true,
        velocity_zone: VelocityZone::Accent,
        ..Default::default()
    };
    p.tracks[0].midi_note = 36;
    s
}

fn transport(tempo: f64, sr: f64, block: u32, beat: f64, bar: f64, playing: bool) -> HostTransport {
    HostTransport {
        tempo_bpm: tempo,
        sample_rate: sr,
        block_samples: block,
        block_start_beat: beat,
        bar_start_beat: bar,
        is_playing: playing,
        beats_per_bar: 4.0,
    }
}

#[test]
fn stopped_engine_emits_nothing() {
    let eng = Engine::new_host_driven();
    eng.publish(session_with_step0_hit());
    let mut rs = HostRenderState::new();
    let mut out = [sequencer_engine::host::MidiEvent::zero(); 64];
    let n = eng.render_host(
        &mut rs,
        &transport(120.0, 48_000.0, 256, 0.0, 0.0, false),
        &[],
        &mut out,
    );
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
    let mut first_note_block: Option<usize> = None;
    for i in 0..16 {
        let mut out = [sequencer_engine::host::MidiEvent::zero(); 64];
        let n = eng.render_host(
            &mut rs,
            &transport(120.0, sr, block, beat, 0.0, true),
            &[],
            &mut out,
        );
        for ev in &out[..n] {
            // Status FAMILY, not a literal nibble: the default
            // global_midi_channel is 10, so the drum channel rides nibble 0xA
            // (0x9A note-on / 0x8A note-off). Asserting the family keeps this
            // correct if the default channel ever changes.
            let fam = ev.status & 0xF0;
            assert!(
                fam == 0x90 || fam == 0x80,
                "note-on/off status family, got {:#04x}",
                ev.status
            );
            if fam == 0x90 && ev.data2 > 0 {
                total_notes += 1;
                if first_note_block.is_none() {
                    first_note_block = Some(i);
                }
            }
        }
        beat += 0.25; // one 16th per block
    }
    // Session defaults have no active steps, so only track 0 step 0 fires —
    // exactly once, in block 0 (immediate downbeat on play-start — I1 fix);
    // the next step-0 downbeat would be at beat 4.0, past this 16-block run.
    assert_eq!(
        total_notes, 1,
        "track 0 step 0 fires exactly once (at the downbeat)"
    );
    assert_eq!(first_note_block, Some(0), "downbeat note-on fires in block 0");
    // 16 blocks × exactly one 16th boundary each → 16 advances → global_step
    // wraps a full bar (STEP_COUNT == 16) back to 0.
    assert_eq!(
        rs.rt.global_step, 0,
        "16 one-16th blocks advance global_step a full bar (wraps to 0)"
    );
}

#[test]
fn play_start_mid_bar_aligns_per_track_step() {
    // Host resumes at beat 1.0 — four 16ths into the bar. Each track's playhead
    // must align to step 4 (not step 0), so a mid-bar resume doesn't replay the
    // downbeat. With immediate-fire (I1 fix), step 4 also FIRES in this block,
    // advancing `global_step` and each `per_track[..].step_idx` by 1 (the
    // default `speed_ratio == 1.0` advances exactly +1 per `process_one`).
    // If alignment were WRONG (step 0), `global_step` would be 1, not 5 — so
    // the assertions below still distinguish correct alignment.
    let eng = Engine::new_host_driven();
    eng.publish(session_with_step0_hit());
    let mut rs = HostRenderState::new();
    let mut out = [sequencer_engine::host::MidiEvent::zero(); 64];
    eng.render_host(
        &mut rs,
        &transport(120.0, 48_000.0, 256, 1.0, 0.0, true),
        &[],
        &mut out,
    );
    let length = eng.snapshot_arc().patterns[0].as_ref().unwrap().tracks[0].length;
    assert_eq!(
        rs.rt.per_track[0].step_idx,
        5 % length,
        "step 4 aligned + fired (advancing to 5)"
    );
    assert_eq!(
        rs.rt.global_step, 5,
        "global_step 4 aligned + fired (advancing to 5)"
    );
}

#[test]
fn play_start_at_bar_boundary_fires_downbeat_immediately() {
    // I1 regression guard: at a bar boundary (sixteenths == 0), the downbeat
    // must fire IMMEDIATELY in block 0 at sample_offset == 0 — not at
    // beat 0.25 (~125 ms silence at 120 BPM). Pre-fix, block 0 emitted
    // nothing because `next_step_beat` was set to the NEXT boundary; this
    // test pins the immediate-downbeat behavior after the `+ 1.0` removal.
    let eng = Engine::new_host_driven();
    eng.publish(session_with_step0_hit());
    let mut rs = HostRenderState::new();
    let mut out = [sequencer_engine::host::MidiEvent::zero(); 64];
    let n = eng.render_host(
        &mut rs,
        &transport(120.0, 48_000.0, 256, 0.0, 0.0, true),
        &[],
        &mut out,
    );
    assert!(
        out[..n].iter().any(|ev| {
            (ev.status & 0xF0) == 0x90 && ev.data1 == 36 && ev.data2 > 0 && ev.sample_offset == 0
        }),
        "downbeat (note 36) must fire at sample_offset 0 in block 0 (immediate on play-start)"
    );
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
        let n = eng.render_host(
            &mut rs,
            &transport(120.0, sr, block, beat, 0.0, true),
            &[],
            &mut out,
        );
        for ev in &out[..n] {
            // Status family, not literal nibble (default channel 10 → 0xA).
            if (ev.status & 0xF0) == 0x90 && ev.data1 == 36 && ev.data2 > 0 {
                saw_note_on = true;
            }
            if (ev.status & 0xF0) == 0x80 && ev.data1 == 36 {
                saw_note_off = true;
            }
        }
        beat += (block as f64) / sr * 2.0; // advance ~one 16th (6000 samples) every few blocks
        if saw_note_on && saw_note_off {
            break;
        }
    }
    assert!(saw_note_on, "note-on for note 36 must fire");
    assert!(
        saw_note_off,
        "matching note-off must fire (possibly a later block via the pending queue)"
    );
}

#[test]
fn swung_note_on_past_block_defers_to_a_future_block() {
    // Odd steps are swing-delayed (clock.rs: `swing_offset_micros`). At 49% swing,
    // 120 BPM, 48 kHz, the delay is 0.49 * 125_000 µs ≈ 2_940 samples. With
    // 1_000-sample blocks a swung note-on lands ~2_940 samples past its boundary
    // block → it must defer (sample-accurate) to a later block, NOT clamp to
    // block_samples-1 and fire inside the boundary block.
    let mut s = Session {
        global_swing_pct: 49.0,
        ..Default::default()
    };
    let p = s.patterns[0].as_mut().unwrap();
    p.tracks[0].steps[1] = Step {
        active: true,
        velocity_zone: VelocityZone::Accent,
        ..Default::default()
    };
    p.tracks[0].midi_note = 36;
    let eng = Engine::new_host_driven();
    eng.publish(s);
    let mut rs = HostRenderState::new();
    let sr = 48_000.0;
    let block = 1_000u32;
    let beats_per_block = block as f64 / sr * (120.0 / 60.0);
    let mut beat = 0.0f64;
    let mut boundary_block: Option<usize> = None; // block where step 1's boundary fires
    let mut note_on_block: Option<usize> = None; // block where the swung note-on emits
    for i in 0..64 {
        let mut out = [sequencer_engine::host::MidiEvent::zero(); 64];
        let global_before = rs.rt.global_step;
        let n = eng.render_host(
            &mut rs,
            &transport(120.0, sr, block, beat, 0.0, true),
            &[],
            &mut out,
        );
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

#[test]
fn stop_transition_emits_all_notes_off_and_freezes() {
    let eng = Engine::new_host_driven();
    eng.publish(session_with_step0_hit());
    let mut rs = HostRenderState::new();
    let mut out = [sequencer_engine::host::MidiEvent::zero(); 64];
    // Play a block, then stop.
    eng.render_host(
        &mut rs,
        &transport(120.0, 48_000.0, 256, 0.0, 0.0, true),
        &[],
        &mut out,
    );
    out.iter_mut()
        .for_each(|e| *e = sequencer_engine::host::MidiEvent::zero());
    let before_next = rs.next_step_beat;
    let n = eng.render_host(
        &mut rs,
        &transport(120.0, 48_000.0, 256, 0.25, 0.0, false),
        &[],
        &mut out,
    );
    // Stop emits CC 123 all-notes-off on channel 10.
    // CC 123 all-notes-off on the drum channel (default channel 10 → 0xBA).
    assert!(
        out[..n]
            .iter()
            .any(|e| (e.status & 0xF0) == 0xB0 && e.data1 == 123),
        "all-notes-off on stop"
    );
    // No new note-ons while stopped.
    assert!(
        !out[..n]
            .iter()
            .any(|e| (e.status & 0xF0) == 0x90 && e.data2 > 0),
        "no note-ons while stopped"
    );
    assert!(!rs.was_playing);
    // next_step_beat unchanged across the stopped block.
    assert_eq!(rs.next_step_beat, before_next);
}

#[test]
fn play_start_reseeds_rng_deterministically() {
    // Two engines + render states with identical sessions must produce the same
    // first note velocity after begin_play reseeds from the snapshot hash.
    // humanize_velocity > 0 forces the RNG to actually shape velocity — with the
    // default 0.0 this test would pass trivially without exercising the reseed.
    // The block is 12_000 samples (two 16ths at 120 BPM/48 kHz) so the step-0
    // boundary at beat 0.25 falls strictly inside the block and actually fires;
    // a 6_000-sample block ends exactly on beat 0.25 and (strict `<`) fires
    // nothing, leaving va == vb == None — a false pass.
    let mut s = session_with_step0_hit();
    s.humanize_velocity = 0.5;
    let mut out_a = [sequencer_engine::host::MidiEvent::zero(); 64];
    let mut out_b = [sequencer_engine::host::MidiEvent::zero(); 64];
    let eng_a = Engine::new_host_driven();
    eng_a.publish(s.clone());
    let mut rs_a = HostRenderState::new();
    let na = eng_a.render_host(
        &mut rs_a,
        &transport(120.0, 48_000.0, 12_000, 0.0, 0.0, true),
        &[],
        &mut out_a,
    );
    let eng_b = Engine::new_host_driven();
    eng_b.publish(s);
    let mut rs_b = HostRenderState::new();
    let nb = eng_b.render_host(
        &mut rs_b,
        &transport(120.0, 48_000.0, 12_000, 0.0, 0.0, true),
        &[],
        &mut out_b,
    );
    // Status family, not literal 0x99: default channel 10 → 0x9A. A literal
    // 0x99 would match nothing, leaving va == vb == None (a false pass).
    let va = out_a[..na]
        .iter()
        .find(|e| (e.status & 0xF0) == 0x90 && e.data2 > 0)
        .map(|e| e.data2);
    let vb = out_b[..nb]
        .iter()
        .find(|e| (e.status & 0xF0) == 0x90 && e.data2 > 0)
        .map(|e| e.data2);
    assert_eq!(va, vb, "identical sessions reseed identically");
}

proptest! {
    #[test]
    fn rendered_offsets_stay_in_block_and_step_stays_bounded(
        tempo in 60.0f64..200.0,
        sr in 44_100.0f64..96_000.0,
        block in 16u32..1024,
        n_blocks in 1usize..64,
    ) {
        let eng = Engine::new_host_driven();
        eng.publish(session_with_step0_hit());
        let mut rs = HostRenderState::new();
        let beats_per_block = (block as f64) / sr * (tempo / 60.0);
        let mut beat = 0.0f64;
        for _ in 0..n_blocks {
            let bar = (beat / 4.0).floor() * 4.0;
            let mut out = [sequencer_engine::host::MidiEvent::zero(); 256];
            let n = eng.render_host(
                &mut rs,
                &transport(tempo, sr, block, beat, bar, true),
                &[],
                &mut out,
            );
            for ev in &out[..n] {
                prop_assert!(ev.sample_offset < block, "offset {} >= block {}", ev.sample_offset, block);
            }
            prop_assert!(rs.rt.global_step < STEP_COUNT as u32, "global_step out of range");
            beat += beats_per_block;
        }
        // Over a steady run next_step_beat tracks the playhead within one 16th
        // (it is the first unconsumed boundary, in [beat, beat + 0.25)).
        prop_assert!(
            rs.next_step_beat >= beat - 1e-6 && rs.next_step_beat <= beat + 0.25 + 1e-6,
            "next_step_beat {} drifted from beat {}", rs.next_step_beat, beat
        );
    }
}

#[test]
fn incoming_command_octave_note_queues_pattern_select() {
    let eng = Engine::new_host_driven();
    let s = session_with_step0_hit();
    eng.publish(s);
    let mut rs = HostRenderState::new();
    // Note 61 in the command octave (60..) → pattern index 1.
    let midi_in = [sequencer_engine::host::MidiEvent {
        sample_offset: 0,
        status: 0x90,
        data1: 61,
        data2: 100,
    }];
    let mut out = [sequencer_engine::host::MidiEvent::zero(); 64];
    eng.render_host(
        &mut rs,
        &transport(120.0, 48_000.0, 256, 0.0, 0.0, true),
        &midi_in,
        &mut out,
    );
    // The render pushed a QueuePattern{1} command; apply it directly (no worker thread here).
    let cmd = eng.commands.dequeue().expect("a queued command");
    assert!(matches!(
        cmd,
        sequencer_engine::command::Command::QueuePattern { index: 1, .. }
    ));
}

#[test]
fn stop_transition_does_not_emit_deferred_note_on_after_all_notes_off() {
    // Regression (Medium, PR #8 review): on the play→stop transition block, a
    // deferred note-on due this block must NOT fire. It would land at
    // sample_offset > 0, after the offset-0 CC 123 all-notes-off, re-arming a
    // note nothing later turns off → stuck note. Pre-fix, `render_host` drained
    // `pending` BEFORE the stop branch, so the deferred note-on leaked out.
    //
    // The seed mimics state a prior swung block leaves behind: a deferred note-on
    // scheduled mid-block + `was_playing == true` (so this block is a play→stop
    // transition that emits CC 123).
    let eng = Engine::new_host_driven();
    eng.publish(session_with_step0_hit());
    let mut rs = HostRenderState::new();
    rs.pending.schedule(100, 0x9A, 36, 100); // deferred note-on, due mid-block
    rs.was_playing = true; // makes this a play→stop transition (CC 123 path)
    rs.sample_time = 0; // block_start_abs = 0 → abs 100 ∈ [0, 256)
    let mut out = [sequencer_engine::host::MidiEvent::zero(); 64];
    let n = eng.render_host(
        &mut rs,
        &transport(120.0, 48_000.0, 256, 0.0, 0.0, false),
        &[],
        &mut out,
    );
    // CC 123 all-notes-off must be emitted at offset 0.
    assert!(
        out[..n]
            .iter()
            .any(|e| (e.status & 0xF0) == 0xB0 && e.data1 == 123 && e.sample_offset == 0),
        "all-notes-off at offset 0 (got {:?})",
        &out[..n]
    );
    // No deferred note-on may fire on the stop block.
    assert!(
        out[..n]
            .iter()
            .all(|e| !((e.status & 0xF0) == 0x90 && e.data2 > 0)),
        "no note-on on the stop block (pre-fix the deferred note-on fired here, sticking the note): {:?}",
        &out[..n]
    );
    assert!(!rs.was_playing);
}
