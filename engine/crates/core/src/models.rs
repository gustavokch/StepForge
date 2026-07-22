//! Cross-layer data models — the source-of-truth contract between the Rust core
//! and the Swift shell. These cross the FFI as postcard bytes, so they carry NO
//! `#[repr(C)]` (amendment A4). Faithful to architecture-spec §5.1.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MAX_TRACKS: usize = 8;
pub const MIN_TRACKS: usize = 4;
pub const STEP_COUNT: usize = 16;
pub const PATTERN_SLOTS: usize = 9;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Session {
    pub bpm: f64,
    pub sync_source: SyncSource,
    pub global_swing_pct: f32,
    pub humanize_timing: f32,
    pub humanize_velocity: f32,
    pub global_midi_channel: u8, // default 10
    pub active_pattern_index: usize,
    pub patterns: [Option<Pattern>; PATTERN_SLOTS],
    pub midi_destinations: Vec<u32>, // MIDIEndpointRef as UInt32
}

impl Default for Session {
    fn default() -> Self {
        Self {
            bpm: 120.0,
            sync_source: SyncSource::Free,
            global_swing_pct: 0.0,
            humanize_timing: 0.0,
            humanize_velocity: 0.0,
            global_midi_channel: 10,
            active_pattern_index: 0,
            patterns: Default::default(), // [None; 9]
            midi_destinations: Vec::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SyncSource {
    #[default]
    Free,
    MidiClock,
    Link,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Pattern {
    pub id: Uuid,
    pub tracks: Vec<Track>, // 4-8 tracks
    pub follow_action: FollowAction,
}

impl Default for Pattern {
    fn default() -> Self {
        Self {
            id: Uuid::nil(),
            tracks: (0..MIN_TRACKS).map(|_| Track::default()).collect(),
            follow_action: FollowAction::default(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FollowAction {
    pub after_loops: u32, // default 1
    pub action: FollowActionType,
}

impl Default for FollowAction {
    fn default() -> Self {
        Self {
            after_loops: 1,
            action: FollowActionType::default(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum FollowActionType {
    None,
    PlayNext,
    PlaySpecific(Uuid),
    PlayPrevious,
    Stop,
    PlayRandom,
}

impl Default for FollowActionType {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Track {
    pub id: Uuid,
    pub midi_note: u8,
    pub length: usize,    // 1-16 (playback window over a fixed [Step; 16])
    pub speed_ratio: f32, // 0.5, 1.0, 2.0, 3.0
    pub swing_pct: f32,   // relative to global
    pub muted: bool,
    pub steps: [Step; STEP_COUNT],
}

impl Default for Track {
    fn default() -> Self {
        Self {
            id: Uuid::nil(),
            midi_note: 36, // default Kick (C2 region)
            length: STEP_COUNT,
            speed_ratio: 1.0,
            swing_pct: 0.0,
            muted: false,
            steps: [Step::default(); STEP_COUNT],
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
pub struct Step {
    pub active: bool,
    pub velocity_zone: VelocityZone,
    pub micro_timing_offset: f32, // set by Roll (amendment E3: not yet read by dispatch)
    pub ratchet: Ratchet,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum VelocityZone {
    Low,
    #[default]
    Mid, // a tap places a hit at Mid velocity (ui-ux-spec §2.2)
    Accent,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Ratchet {
    #[default]
    Off,
    X2,
    X3,
    X4,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum QuantizeGrain {
    #[default]
    NextStep,
    NextBeat,
    NextBar,
    EndOfPattern,
}
