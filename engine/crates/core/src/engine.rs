//! Engine: owns the COW snapshot store, bounded queues, transport atomics,
//! shutdown flag, and thread handles. The state worker is sole writer of the
//! Session (publish); the RT thread is a lock-free reader (snapshot_arc / load).

use crate::clock::Rng;
use crate::command::Command;
use crate::event::EngineEvent;
use crate::midi_out::{
    command_queue, hot_event_channel, midi_out_ring, CommandQueue, HotEventChannel, MidiOutRing,
};
use crate::models::{Session, MAX_TRACKS};
use arc_swap::ArcSwap;
use std::sync::atomic::Ordering;
use std::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::{Arc, Mutex};

fn active_pattern_mut(s: &mut Session) -> Option<&mut crate::models::Pattern> {
    s.patterns[s.active_pattern_index].as_mut()
}
fn with_track_mut<R>(
    s: &mut Session,
    idx: usize,
    f: impl FnOnce(&mut crate::models::Track) -> R,
) -> Option<R> {
    let p = active_pattern_mut(s)?;
    p.tracks.get_mut(idx).map(f)
}
fn add_track(s: &mut Session) {
    if let Some(p) = active_pattern_mut(s) {
        if p.tracks.len() < MAX_TRACKS {
            p.tracks.push(crate::models::Track::default());
        }
    }
}
fn remove_track(s: &mut Session) {
    if let Some(p) = active_pattern_mut(s) {
        if p.tracks.len() > crate::models::MIN_TRACKS {
            p.tracks.pop();
        }
    }
}

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

    /// Apply one command by clone-mutate-publish (the worker's per-command body).
    /// Algorithms/scheduler/load-session arms are wired in their own tasks.
    pub fn apply_command(&self, cmd: Command) {
        use crate::command::Command::*;
        match cmd {
            Play => {
                self.transport.is_playing.store(true, Ordering::Release);
            }
            Stop => {
                self.transport.is_playing.store(false, Ordering::Release);
                self.transport
                    .stop_generation
                    .fetch_add(1, Ordering::AcqRel);
            }
            RequestFullSnapshot => {
                let snap = self.snapshot.load_full();
                crate::midi_out::push_event(
                    &self.hot_events,
                    &EngineEvent::FullSnapshot {
                        session: (*snap).clone(),
                    },
                );
            }
            // Off-RT large-channel events (Serialize) handled in Task 8.
            Serialize | LoadSession { .. } => { /* Task 8 / Task 18 */ }
            // Algorithms/scheduler arms added in Tasks 14-16.
            Roll { .. }
            | Vary { .. }
            | Cut { .. }
            | Copy { .. }
            | Paste { .. }
            | Trash { .. }
            | Undo { .. }
            | QueuePattern { .. }
            | CancelQueuedPattern
            | RetriggerPattern { .. } => { /* later tasks */ }
            other => {
                let mut s = (*self.snapshot.load_full()).clone();
                match other {
                    SetBpm { bpm } => s.bpm = bpm,
                    SetGlobalSwing { pct } => s.global_swing_pct = pct,
                    SetHumanize { timing, velocity } => {
                        s.humanize_timing = timing;
                        s.humanize_velocity = velocity;
                    }
                    SetSyncSource { source } => s.sync_source = source,
                    SetQuantizeGrain { grain: _ } => { /* stored on the scheduler in Task 16 */ }
                    SetGlobalMidiChannel { channel } => s.global_midi_channel = channel,
                    SetMidiDestinations { endpoints } => s.midi_destinations = endpoints,
                    SetTrackLength { track_idx, length } => {
                        with_track_mut(&mut s, track_idx, |t| {
                            t.length = length.clamp(1, crate::models::STEP_COUNT);
                        });
                    }
                    SetTrackMuted { track_idx, muted } => {
                        with_track_mut(&mut s, track_idx, |t| t.muted = muted);
                    }
                    SetTrackNote {
                        track_idx,
                        midi_note,
                    } => {
                        with_track_mut(&mut s, track_idx, |t| t.midi_note = midi_note);
                    }
                    SetTrackSpeedRatio { track_idx, ratio } => {
                        with_track_mut(&mut s, track_idx, |t| t.speed_ratio = ratio);
                    }
                    SetTrackSwing {
                        track_idx,
                        swing_pct,
                    } => {
                        with_track_mut(&mut s, track_idx, |t| t.swing_pct = swing_pct);
                    }
                    AddTrack => add_track(&mut s),
                    RemoveTrack => remove_track(&mut s),
                    SetStep {
                        track_idx,
                        step_idx,
                        zone,
                    } => {
                        with_track_mut(&mut s, track_idx, |t| {
                            if step_idx < t.steps.len() {
                                t.steps[step_idx].active = true;
                                t.steps[step_idx].velocity_zone = zone;
                            }
                        });
                    }
                    DeleteStep {
                        track_idx,
                        step_idx,
                    } => {
                        with_track_mut(&mut s, track_idx, |t| {
                            if step_idx < t.steps.len() {
                                t.steps[step_idx].active = false;
                            }
                        });
                    }
                    SetRatchet {
                        track_idx,
                        step_idx,
                        ratchet,
                    } => {
                        with_track_mut(&mut s, track_idx, |t| {
                            if step_idx < t.steps.len() {
                                t.steps[step_idx].ratchet = ratchet;
                            }
                        });
                    }
                    SetFollowAction { .. } => { /* Task 16 */ }
                    // unreachable: the match arms above are exhaustive with the outer match
                    _ => {}
                }
                self.publish(s);
            }
        }
    }

    /// Worker thread body: drain commands until shutdown.
    pub fn run_worker_loop(self: &Arc<Engine>) {
        while !self.shutdown.load(Ordering::Acquire) {
            if let Some(cmd) = self.commands.dequeue() {
                self.apply_command(cmd);
            } else {
                std::thread::sleep(std::time::Duration::from_micros(200));
            }
        }
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
    #[test]
    fn worker_applies_set_bpm_via_cow() {
        let e = Engine::new();
        e.apply_command(Command::SetBpm { bpm: 174.0 });
        assert_eq!(e.snapshot_arc().bpm, 174.0);
    }
    #[test]
    fn play_stop_toggle_transport_atomic() {
        let e = Engine::new();
        e.apply_command(Command::Play);
        assert!(e
            .transport
            .is_playing
            .load(std::sync::atomic::Ordering::Acquire));
        e.apply_command(Command::Stop);
        assert!(!e
            .transport
            .is_playing
            .load(std::sync::atomic::Ordering::Acquire));
        assert_eq!(
            e.transport
                .stop_generation
                .load(std::sync::atomic::Ordering::Acquire),
            1
        );
    }
}
