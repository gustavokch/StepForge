//! Commands: Swift → Rust. Cross the FFI as postcard bytes (amendments A4, A15).
//! NO #[repr(C)]. Faithful to architecture-spec §3.2, plus the seeded sync/load
//! placeholders (amendments A15, E6).

use crate::models::{FollowAction, QuantizeGrain, Ratchet, SyncSource, VelocityZone};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Command {
    SetStep { track_idx: usize, step_idx: usize, zone: VelocityZone },
    DeleteStep { track_idx: usize, step_idx: usize },
    SetRatchet { track_idx: usize, step_idx: usize, ratchet: Ratchet },
    SetTrackLength { track_idx: usize, length: usize },
    SetTrackMuted { track_idx: usize, muted: bool },
    SetTrackNote { track_idx: usize, midi_note: u8 },
    SetTrackSpeedRatio { track_idx: usize, ratio: f32 },
    SetTrackSwing { track_idx: usize, swing_pct: f32 },
    AddTrack,
    RemoveTrack,
    Roll { track_idx: usize, strength: f32 },
    Vary { track_idx: usize, strength: f32 },
    Cut { track_idx: usize },
    Copy { track_idx: usize },
    Paste { track_idx: usize },
    Trash { track_idx: usize },
    Undo { track_idx: usize },
    QueuePattern { index: usize, quantize: QuantizeGrain },
    CancelQueuedPattern,
    RetriggerPattern { quantize: QuantizeGrain },
    SetGlobalSwing { pct: f32 },
    SetHumanize { timing: f32, velocity: f32 },
    SetBpm { bpm: f64 },
    SetSyncSource { source: SyncSource },
    SetQuantizeGrain { grain: QuantizeGrain },
    SetFollowAction { pattern_idx: usize, action: FollowAction },
    SetMidiDestinations { endpoints: Vec<u32> },
    SetGlobalMidiChannel { channel: u8 },
    Play,
    Stop,
    RequestFullSnapshot,
    Serialize,
    /// Restore a previously serialized session (amendment A15). Bytes are the
    /// output of `engine_serialize` (a `SessionEnvelope`).
    LoadSession { bytes: Vec<u8> },
    /// Ableton Link phase alignment (architecture-spec §9.2; amendment E6).
    LinkPhase { beats_since_origin: f64, phase: f64 },
    /// Inbound MIDI Clock tick — drives step advance when sync = MidiClock (§9.3; E6).
    MidiClockTick,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_roundtrip() {
        let cmds = [
            Command::SetBpm { bpm: 140.0 },
            Command::AddTrack,
            Command::SetStep { track_idx: 0, step_idx: 3, zone: VelocityZone::Accent },
            Command::LoadSession { bytes: vec![9, 9] },
            Command::LinkPhase { beats_since_origin: 4.0, phase: 0.5 },
            Command::MidiClockTick,
        ];
        for c in cmds {
            let bytes = postcard::to_allocvec(&c).expect("serialize");
            let back: Command = postcard::from_bytes(&bytes).expect("deserialize");
            assert_eq!(back, c);
        }
    }
}
