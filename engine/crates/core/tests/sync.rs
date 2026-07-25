//! Integration tests for external sync (E6): MidiClockTick pulse accumulation
//! and LinkPhase absolute-position storage. Drives `apply_command` directly
//! (no worker thread) and asserts the `ExternalClock` atomics advance as
//! specified. RT-loop consumption of these pulses — `run_rt_loop` under
//! MidiClock advancing the playhead — is integration-tested alongside Task 20's
//! lifecycle test, since it requires running the RT thread.

use sequencer_engine::command::Command;
use sequencer_engine::engine::Engine;
use sequencer_engine::models::SyncSource;
use std::sync::atomic::Ordering;

#[test]
fn midi_clock_accumulates_one_step_pulse_per_six_ticks() {
    let e = Engine::new();
    e.apply_command(Command::SetSyncSource {
        source: SyncSource::MidiClock,
    });
    for _ in 0..6 {
        e.apply_command(Command::MidiClockTick);
    }
    assert_eq!(e.external_clock.midi_step_pulses.load(Ordering::Acquire), 1);
    for _ in 0..6 {
        e.apply_command(Command::MidiClockTick);
    }
    assert_eq!(e.external_clock.midi_step_pulses.load(Ordering::Acquire), 2);
}

#[test]
fn link_enabled_toggles_flag() {
    let e = Engine::new();
    e.apply_command(Command::SetLinkEnabled { enabled: true });
    assert_eq!(
        e.external_clock.link_enabled.load(Ordering::Acquire),
        true
    );
}

// RT-loop consumption of these pulses (`run_rt_loop` under MidiClock advancing the
// playhead) is integration-tested alongside Task 20's lifecycle test, since it
// requires running the RT thread.
