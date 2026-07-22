//! Engine: owns the COW snapshot store, bounded queues, transport atomics,
//! shutdown flag, and thread handles. The state worker is sole writer of the
//! Session (publish); the RT thread is a lock-free reader (snapshot_arc / load).

use crate::clock::Rng;
use crate::midi_out::{
    command_queue, hot_event_channel, midi_out_ring, CommandQueue, HotEventChannel, MidiOutRing,
};
use crate::models::{Session, MAX_TRACKS};
use arc_swap::ArcSwap;
use std::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::{Arc, Mutex};

pub struct Transport {
    pub is_playing: AtomicBool,
    pub stop_generation: AtomicU32,
}
impl Default for Transport {
    fn default() -> Self {
        Self {
            is_playing: AtomicBool::new(false),
            stop_generation: AtomicU32::new(0),
        }
    }
}

#[derive(Clone)]
pub struct TrackRtState {
    pub step_idx: usize,
    pub speed_acc: u32,
}

pub struct RtState {
    pub per_track: [TrackRtState; MAX_TRACKS],
    pub rng: Rng,
}
impl RtState {
    pub fn new(seed: u64) -> Self {
        Self {
            per_track: std::array::from_fn(|_| TrackRtState {
                step_idx: 0,
                speed_acc: 0,
            }),
            rng: Rng::new(seed),
        }
    }
}

pub struct Engine {
    pub snapshot: Arc<ArcSwap<Session>>,
    pub commands: CommandQueue,
    pub midi: MidiOutRing,
    pub hot_events: HotEventChannel,
    pub transport: Transport,
    pub shutdown: Arc<AtomicBool>,
    pub rt_handle: Mutex<Option<std::thread::JoinHandle<()>>>,
    pub worker_handle: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            snapshot: Arc::new(ArcSwap::from_pointee(Session::default())),
            commands: command_queue(),
            midi: midi_out_ring(),
            hot_events: hot_event_channel(),
            transport: Transport::default(),
            shutdown: Arc::new(AtomicBool::new(false)),
            rt_handle: Mutex::new(None),
            worker_handle: Mutex::new(None),
        }
    }
    /// Worker: publish a new authoritative session (COW).
    pub fn publish(&self, session: Session) {
        self.snapshot.store(Arc::new(session));
    }
    /// Serialize path: owned snapshot (lock-free load_full).
    pub fn snapshot_arc(&self) -> Arc<Session> {
        self.snapshot.load_full()
    }
    /// Play-start: seed RNG from a snapshot hash + reset counters.
    pub fn begin_play(&self, rt: &mut RtState) {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let snap = self.snapshot.load_full();
        let mut h = DefaultHasher::new();
        // Hash a few scalar invariants of the session for a stable seed.
        snap.bpm.to_bits().hash(&mut h);
        snap.active_pattern_index.hash(&mut h);
        snap.global_swing_pct.to_bits().hash(&mut h);
        *rt = RtState::new(h.finish());
    }
    #[cfg(test)]
    pub fn load_session_for_test(&self, session: Session) {
        self.publish(session);
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn publish_then_snapshot_arc_round_trips() {
        let e = Engine::new();
        let s = Session {
            bpm: 140.0,
            ..Default::default()
        };
        e.publish(s.clone());
        assert_eq!(e.snapshot_arc().bpm, 140.0);
    }
    #[test]
    fn begin_play_seeds_rng_deterministically() {
        let e = Engine::new();
        let mut a = RtState::new(1);
        let mut b = RtState::new(2);
        e.begin_play(&mut a);
        e.begin_play(&mut b);
        // same snapshot -> same seed -> same RNG stream
        assert_eq!(a.rng.next_u32(), b.rng.next_u32());
    }
}
