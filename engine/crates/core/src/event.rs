//! Engine events: Rust → Swift. Cross the FFI as postcard bytes (amendment A4).
//! NO #[repr(C)] — never passed as a raw enum pointer. Faithful to §3.3.

use crate::models::{FollowAction, QuantizeGrain, Session, Step, SyncSource, Track};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum EngineEvent {
    StepChanged {
        track_idx: usize,
        step_idx: usize,
        step: Step,
    },
    TrackLengthChanged {
        track_idx: usize,
        length: usize,
    },
    TrackMutedChanged {
        track_idx: usize,
        muted: bool,
    },
    TrackAdded {
        track_idx: usize,
        track: Track,
    },
    TrackRemoved {
        track_idx: usize,
    },
    PatternQueued {
        index: usize,
        quantize: QuantizeGrain,
    },
    PatternSwitched {
        index: usize,
    },
    PatternCleared {
        index: usize,
    },
    FollowActionChanged {
        pattern_idx: usize,
        action: FollowAction,
    },
    Playhead {
        track_idx: usize,
        step_idx: usize,
    },
    PlayStateChanged {
        playing: bool,
    },
    BpmChanged {
        bpm: f64,
    },
    SyncSourceChanged {
        source: SyncSource,
    },
    UndoAvailable {
        track_idx: usize,
        available: bool,
    },
    FullSnapshot {
        session: Session,
    },
    Serialized {
        bytes: Vec<u8>,
    },
    Error {
        code: i32,
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_events_roundtrip() {
        let events = [
            EngineEvent::PlayStateChanged { playing: true },
            EngineEvent::BpmChanged { bpm: 123.0 },
            EngineEvent::Playhead {
                track_idx: 2,
                step_idx: 7,
            },
        ];
        for e in events {
            let bytes = postcard::to_allocvec(&e).expect("serialize");
            let back: EngineEvent = postcard::from_bytes(&bytes).expect("deserialize");
            assert_eq!(back, e);
        }
    }
}
