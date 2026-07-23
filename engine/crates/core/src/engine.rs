//! Engine: owns the COW snapshot store, bounded queues, transport atomics,
//! shutdown flag, and thread handles. The state worker is sole writer of the
//! Session (publish); the RT thread is a lock-free reader (snapshot_arc / load).

use crate::clipboard::Clipboard;
use crate::clock::{
    advance_speed_ratio, micro_timing_offset_micros, swing_offset_micros, to_q16_16, Clock, Rng,
};
use crate::command::Command;
use crate::event::EngineEvent;
use crate::midi::{build_note_on, humanize_velocity, ratchet_count, velocity_for_zone};
use crate::midi_out::{
    command_queue, hot_event_channel, large_event_channel, midi_out_ring, push_large_event,
    CommandQueue, HotEventChannel, LargeEventChannel, MidiOutRing,
};
use crate::models::{Session, MAX_TRACKS, MIN_TRACKS, PATTERN_SLOTS, STEP_COUNT};
use crate::undo::Undo;
use arc_swap::ArcSwap;
use std::sync::atomic::Ordering;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64};
use std::sync::{Arc, Mutex};
#[cfg(not(target_os = "ios"))]
use ableton_link::link::BasicLink as Link;

#[cfg(target_os = "ios")]
pub struct Link;
#[cfg(target_os = "ios")]
impl Link {
    pub fn new(_bpm: f64) -> Self { Self }
    pub async fn enable(&mut self) {}
    pub async fn disable(&mut self) {}
    pub fn capture_app_session_state(&self) -> DummySessionState { DummySessionState }
    pub fn clock(&self) -> DummyClock { DummyClock }
    pub fn num_peers(&self) -> u32 { 0 }
}
#[cfg(target_os = "ios")]
pub struct DummySessionState;
#[cfg(target_os = "ios")]
impl DummySessionState {
    pub fn beat_at_time(&self, _time: u64, _quantum: f64) -> f64 { 0.0 }
}
#[cfg(target_os = "ios")]
pub struct DummyClock;
#[cfg(target_os = "ios")]
impl DummyClock {
    pub fn micros(&self) -> u64 { 0 }
}

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
/// Algorithm/clipboard/undo helper: bounds-checked `&mut Track` on the active
/// pattern. Out-of-range `idx` returns `None` (no-op) — never panics the worker
/// on a malformed command. `active_pattern_index` is read by direct index
/// (matches `active_pattern_mut`; Task 18 validates it on `LoadSession`).
fn track_mut(s: &mut Session, idx: usize) -> Option<&mut crate::models::Track> {
    s.patterns[s.active_pattern_index]
        .as_mut()?
        .tracks
        .get_mut(idx)
}
/// Deterministic per-track RNG seed: hash of (track_idx, bpm). Stable across
/// the COW snapshot (only `idx` + `bpm` are hashed), so Roll/Vary reproducibly
/// perturb the same active steps for a given BPM until the user changes state.
fn seed_from(s: &Session, idx: usize) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    idx.hash(&mut h);
    s.bpm.to_bits().hash(&mut h);
    h.finish()
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

/// Validate an untrusted deserialized `Session` before publishing it
/// (`LoadSession` restore path). Returns `true` only if the invariants that
/// several worker/RT paths assume unchecked hold:
///
/// - `active_pattern_index < PATTERN_SLOTS` — `active_pattern_mut`,
///   `track_mut`, and the RT `process()` index `patterns[active_pattern_index]`;
///   a corrupt index would panic the worker (or worse, the RT thread).
/// - every `Some(pattern)` has `tracks.len()` between `MIN_TRACKS` and
///   `MAX_TRACKS` (inclusive).
/// - every track has `length` in `1..=STEP_COUNT`.
///
/// Command-driven mutations stay safe without this (`SetTrackLength` clamps;
/// `AddTrack`/`RemoveTrack` respect bounds); this guards the restore path,
/// which is the entry point for untrusted sessions (Task 18, amendment A15).
pub fn validate_session(s: &Session) -> bool {
    if s.active_pattern_index >= PATTERN_SLOTS {
        return false;
    }
    for p in s.patterns.iter().flatten() {
        if !(MIN_TRACKS..=MAX_TRACKS).contains(&p.tracks.len()) {
            return false;
        }
        for t in p.tracks.iter() {
            if !(1..=STEP_COUNT).contains(&t.length) {
                return false;
            }
        }
    }
    true
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

/// External-clock shared state (E6). Worker writes via `apply_command`
/// (`LinkPhase` / `MidiClockTick`); the RT loop reads via atomic loads in
/// `run_rt_loop`. All fields are lock-free atomics so the RT thread never
/// blocks (Hard Rule 1). `Arc`-shared so the engine and RT thread see the
/// same counters.
pub struct ExternalClock {
    /// Raw 24-PPQN MIDI Clock tick count — worker increments per `MidiClockTick`.
    pub midi_ticks: AtomicU32,
    /// One pulse per 6 MIDI ticks (= one 16th step). RT consumes via
    /// `swap(0, AcqRel)` each tick; worker `fetch_add`s under it.
    pub midi_step_pulses: AtomicU32,
    /// Absolute Link position as `beats_since_origin * 1_000_000` (integer
    /// micros-of-a-beat). RT computes target 16th-step from this.
    pub link_beats_micros: AtomicU64,
    /// Whether the native Ableton Link session is enabled (Task 1).
    pub link_enabled: AtomicBool,
    pub link: Mutex<Link>,
    #[cfg(not(target_os = "ios"))]
    pub tokio_rt: tokio::runtime::Runtime,
}
impl ExternalClock {
    pub fn new() -> Self {
        #[cfg(not(target_os = "ios"))]
        let tokio_rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        #[cfg(not(target_os = "ios"))]
        let link = tokio_rt.block_on(Link::new(120.0));
        #[cfg(target_os = "ios")]
        let link = Link::new(120.0);

        Self {
            midi_ticks: AtomicU32::new(0),
            midi_step_pulses: AtomicU32::new(0),
            link_beats_micros: AtomicU64::new(0),
            link_enabled: AtomicBool::new(false),
            link: Mutex::new(link),
            #[cfg(not(target_os = "ios"))]
            tokio_rt,
        }
    }
}
impl Default for ExternalClock {
    fn default() -> Self {
        Self::new()
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
    /// 16th-step position consumed by the RT loop's `Link` branch (E6). The RT
    /// thread is the sole writer; `begin_play` resets it to 0 so the loop
    /// catches up to the current Link position on play-start.
    pub link_step_count: u64,
    /// Global 16th-step position within the 16-step bar (0..STEP_COUNT). Drives
    /// scheduler boundary detection (`check_scheduler`). +1 per `process` call
    /// when playing; reset on play-start / retrigger / pattern switch.
    pub global_step: u32,
    pub track_count: usize,
    pub loop_count: u32,
    pub link_peers: usize,
}
impl RtState {
    pub fn new(seed: u64) -> Self {
        Self {
            per_track: std::array::from_fn(|_| TrackRtState {
                step_idx: 0,
                speed_acc: 0,
            }),
            rng: Rng::new(seed),
            link_step_count: 0,
            global_step: 0,
            track_count: 0,
            loop_count: 0,
            link_peers: 0,
        }
    }
}

pub struct Engine {
    pub snapshot: Arc<ArcSwap<Session>>,
    pub commands: CommandQueue,
    pub midi: MidiOutRing,
    pub hot_events: HotEventChannel,
    /// Off-RT large-payload channel (A2/D5) — `Serialized`/`FullSnapshot`/
    /// `Error` ride here, never on the 128-byte `hot_events` slot.
    pub large_events: LargeEventChannel,
    pub transport: Transport,
    pub shutdown: Arc<AtomicBool>,
    pub rt_handle: Mutex<Option<std::thread::JoinHandle<()>>>,
    pub worker_handle: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// CoreMIDI worker handle. NULL before `engine_start` (Task 20a).
    pub coremidi_handle: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// CoreMIDI client reference (owned by engine, Rule 7). Stored as `usize`
    /// (pointer-sized) so the ref survives regardless of how Apple defines
    /// `MIDIClientRef`; 0 means uninitialized.
    pub coremidi_client: Mutex<usize>,
    /// CoreMIDI output port reference (owned by engine). Pointer-sized for the
    /// same reason; 0 means uninitialized.
    pub coremidi_port: Mutex<usize>,
    /// CoreMIDI virtual source reference (owned by engine).
    pub coremidi_source: Mutex<usize>,
    /// Per-track, one-deep undo (Task 12). Mutex-guarded because the worker
    /// applies commands sequentially but the engine is `Send+Sync`; the lock is
    /// never held across an FFI call or on the RT path.
    pub undo: Mutex<Undo>,
    /// Single-track clipboard (Task 13). Same ownership rules as `undo`.
    pub clipboard: Mutex<Clipboard>,
    /// External-clock shared state (E6). Worker writes via `apply_command`
    /// (`LinkPhase` / `MidiClockTick`); the RT loop reads via atomic loads.
    pub external_clock: Arc<ExternalClock>,
    /// Scheduler shared state. Worker writes (`QueuePattern`/`CancelQueuedPattern`/
    /// `RetriggerPattern`); the RT loop reads/fires at quantize boundaries
    /// (`check_scheduler`); the worker observes the resulting `switch_request`.
    pub scheduler: Arc<crate::scheduler::SchedulerClock>,
    /// Monotonic counter bumped each time a `LoadSession` restores the session
    /// (amendment A15). Lets the RT/CoreMIDI side detect a reload and react
    /// (e.g. drop stale per-track state). Read via `Acquire`; the worker bumps
    /// via `AcqRel` after `publish`.
    pub reload_generation: AtomicU32,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            snapshot: Arc::new(ArcSwap::from_pointee(Session::default())),
            commands: command_queue(),
            midi: midi_out_ring(),
            hot_events: hot_event_channel(),
            large_events: large_event_channel(),
            transport: Transport::default(),
            shutdown: Arc::new(AtomicBool::new(false)),
            rt_handle: Mutex::new(None),
            worker_handle: Mutex::new(None),
            coremidi_handle: Mutex::new(None),
            coremidi_client: Mutex::new(0usize),
            coremidi_port: Mutex::new(0usize),
            coremidi_source: Mutex::new(0usize),
            undo: Mutex::new(Undo::default()),
            clipboard: Mutex::new(Clipboard::default()),
            external_clock: Arc::new(ExternalClock::new()),
            scheduler: Arc::new(crate::scheduler::SchedulerClock::default()),
            reload_generation: AtomicU32::new(0),
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

    /// Fresh RT state for the RT thread. The seed is a placeholder —
    /// [`Engine::begin_play`] reseeds from a session hash on play transition.
    pub fn new_rt_state(&self) -> RtState {
        RtState::new(1)
    }

    /// RT thread body (E6 mode-switch). `elevate_priority` runs ONCE at spawn;
    /// per-tick work branches on `snap.sync_source` and is allocation-free
    /// across all three branches:
    ///
    /// - **Free** — internal clock: one [`process`] + `sleep_until` the next
    ///   16th deadline (Task 11 cadence, unchanged).
    /// - **MidiClock** — `swap(0, AcqRel)` the accumulated 16th-step pulses and
    ///   call `process` once per pulse; then a short 500 µs poll-sleep (the
    ///   worker increments pulses under the atomic — RT only consumes).
    /// - **Link** — read `link_beats_micros`, compute the target 16th-step
    ///   count since origin, and catch up by calling `process` until
    ///   `rt.link_step_count` reaches it; then a short 500 µs poll-sleep.
    ///
    /// `process` stays source-agnostic (advances one step per call); the loop
    /// decides cadence. The external branches perform ONLY atomic loads +
    /// `process` calls — no allocation, no lock, no FFI, no CoreMIDI
    /// (Hard Rule 1).
    pub fn run_rt_loop(self: &Arc<Engine>, clock: &dyn Clock) {
        clock.elevate_priority(); // ONCE at spawn
        let mut rt = self.new_rt_state();
        let mut began = false;
        loop {
            if self.shutdown.load(Ordering::Acquire) {
                break;
            }
            let now = clock.now_micros();
            let playing = self.transport.is_playing.load(Ordering::Acquire);
            if playing && !began {
                self.begin_play(&mut rt);
                began = true;
            } else if !playing {
                began = false;
            }
            let snap = self.snapshot.load(); // zero-alloc Guard; immutable for the tick
            match snap.sync_source {
                crate::models::SyncSource::Free => {
                    self.process_one(&mut rt, &snap, playing, now);
                    let period = (60.0 / snap.bpm / 4.0 * 1_000_000.0) as u64;
                    clock.sleep_until(now + period);
                }
                crate::models::SyncSource::MidiClock => {
                    // Acquire-load pairs with the worker's Release store; the
                    // swap clears so pulses never accumulate across ticks.
                    let pulses = self
                        .external_clock
                        .midi_step_pulses
                        .swap(0, Ordering::AcqRel);
                    for _ in 0..pulses {
                        self.process_one(&mut rt, &snap, playing, now);
                    }
                    // Short poll-sleep: external clocks drive cadence, the RT
                    // thread just needs to wake often enough to consume pulses
                    // without burning a core.
                    clock.sleep_until(now + 500);
                }
                crate::models::SyncSource::Link => {
                    if let Ok(link) = self.external_clock.link.try_lock() {
                        let state = link.capture_app_session_state();
                        let time = link.clock().micros();
                        let beats = state.beat_at_time(time, 4.0);
                        let target = (beats * 4.0) as u64;
                        while rt.link_step_count < target {
                            self.process_one(&mut rt, &snap, playing, now);
                            rt.link_step_count += 1;
                        }
                        let peers = link.num_peers();
                        if peers as usize != rt.link_peers {
                            rt.link_peers = peers as usize;
                            let _ = crate::midi_out::push_event(
                                &self.hot_events,
                                &crate::event::EngineEvent::LinkPeersChanged { count: peers as usize },
                            );
                        }
                    }
                    clock.sleep_until(now + 500);
                }
            }
        }
    }

    /// One global 16th step: dispatch via [`process`], advance the bar position,
    /// and check the scheduler at the resulting boundary. RT-safe — `process` is,
    /// and `check_scheduler` does only atomic ops + a bounded ring push + local
    /// counter resets (Hard Rule 1).
    fn process_one(&self, rt: &mut RtState, snap: &Session, playing: bool, now_micros: u64) {
        process(rt, snap, playing, now_micros, &self.midi, &self.hot_events);
        if !playing {
            return;
        }
        rt.global_step = (rt.global_step + 1) % crate::models::STEP_COUNT as u32;
        self.check_scheduler(rt, snap);
    }

    /// RT: fire a queued pattern switch / retrigger at the current boundary.
    /// Atomic loads + a bounded all-notes-off ring burst + local counter resets
    /// only — no alloc/lock/FFI (Hard Rule 1).
    fn check_scheduler(&self, rt: &mut RtState, snap: &Session) {
        let mut reset_occurred = false;
        if self.scheduler.take_retrigger() {
            for s in rt.per_track.iter_mut() {
                s.step_idx = 0;
            }
            rt.global_step = 0;
            rt.loop_count = 0;
            reset_occurred = true;
        }
        if let Some(idx) = self.scheduler.take_if_due(rt.global_step) {
            crate::scheduler::all_notes_off_burst(snap, &self.midi);
            self.scheduler.request_switch(idx);
            for s in rt.per_track.iter_mut() {
                s.step_idx = 0;
            }
            rt.global_step = 0;
            rt.loop_count = 0;
            reset_occurred = true;
        }

        if !reset_occurred && rt.global_step == 0 {
            rt.loop_count += 1;
            let _ = crate::midi_out::push_event(
                &self.hot_events,
                &crate::event::EngineEvent::PatternLoopCountChanged { count: rt.loop_count },
            );
            if let Some(pattern) = snap.patterns.get(snap.active_pattern_index).and_then(|p| p.as_ref()) {
                let fa = &pattern.follow_action;
                if fa.action != crate::models::FollowActionType::None && rt.loop_count >= fa.after_loops {
                    let mut next_idx = snap.active_pattern_index;
                    let mut do_stop = false;

                    match fa.action {
                        crate::models::FollowActionType::PlayNext => {
                            next_idx = (snap.active_pattern_index + 1) % snap.patterns.len();
                        }
                        crate::models::FollowActionType::PlayPrevious => {
                            next_idx = (snap.active_pattern_index + snap.patterns.len() - 1) % snap.patterns.len();
                        }
                        crate::models::FollowActionType::PlaySpecific(id) => {
                            next_idx = snap.patterns.iter().position(|p| p.as_ref().map_or(false, |pat| pat.id == id)).unwrap_or(snap.active_pattern_index);
                        }
                        crate::models::FollowActionType::PlayRandom => {
                            next_idx = rt.rng.range(0, snap.patterns.len() as i32 - 1) as usize;
                        }
                        crate::models::FollowActionType::Stop => {
                            do_stop = true;
                        }
                        crate::models::FollowActionType::None => {}
                    }

                    if do_stop {
                        self.transport.is_playing.store(false, std::sync::atomic::Ordering::Release);
                        self.transport.stop_generation.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                        let _ = crate::midi_out::push_event(
                            &self.hot_events,
                            &crate::event::EngineEvent::PlayStateChanged { playing: false },
                        );
                        rt.loop_count = 0;
                    } else if next_idx != snap.active_pattern_index {
                        crate::scheduler::all_notes_off_burst(snap, &self.midi);
                        self.scheduler.request_switch(next_idx);
                        for s in rt.per_track.iter_mut() {
                            s.step_idx = 0;
                        }
                        rt.global_step = 0;
                        rt.loop_count = 0;
                    } else {
                        rt.loop_count = 0;
                    }
                }
            }
        }
    }

    /// Apply one command by clone-mutate-publish (the worker's per-command body).
    /// Algorithms/scheduler/load-session arms are wired in their own tasks.
    pub fn apply_command(&self, cmd: Command) {
        use crate::command::Command::*;
        match cmd {
            Play => {
                self.transport.is_playing.store(true, Ordering::Release);
                crate::midi_out::push_event(
                    &self.hot_events,
                    &EngineEvent::PlayStateChanged { playing: true },
                );
            }
            Stop => {
                self.transport.is_playing.store(false, Ordering::Release);
                self.transport
                    .stop_generation
                    .fetch_add(1, Ordering::AcqRel);
                crate::midi_out::push_event(
                    &self.hot_events,
                    &EngineEvent::PlayStateChanged { playing: false },
                );
            }
            // E6: external sync. `SetSyncSource` is an outer arm because it
            // publishes a fresh session (the RT loop branches on `sync_source`
            // each tick via the snapshot). `LinkPhase` / `MidiClockTick` live
            // outside the inner `other` block so they don't wastefully
            // clone-mutate-publish an unchanged session — they only store
            // atomics for the RT loop to read.
            SetSyncSource { source } => {
                let mut s = (*self.snapshot.load_full()).clone();
                s.sync_source = source.clone();
                self.publish(s);
                crate::midi_out::push_event(
                    &self.hot_events,
                    &EngineEvent::SyncSourceChanged { source },
                );
            }
            SetLinkEnabled { enabled } => {
                if let Ok(mut link) = self.external_clock.link.lock() {
                    #[cfg(not(target_os = "ios"))]
                    {
                        if enabled {
                            self.external_clock.tokio_rt.block_on(link.enable());
                        } else {
                            self.external_clock.tokio_rt.block_on(link.disable());
                        }
                    }
                    #[cfg(target_os = "ios")]
                    let _ = &mut link;
                }
                self.external_clock
                    .link_enabled
                    .store(enabled, Ordering::Release);
            }
            MidiClockTick => {
                // 24 PPQN → 6 ticks per 16th step. Worker accumulates raw
                // ticks; every 6th tick adds one 16th-step pulse that the RT
                // loop consumes via `swap(0, AcqRel)`.
                let t = self
                    .external_clock
                    .midi_ticks
                    .fetch_add(1, Ordering::AcqRel)
                    + 1;
                if t.is_multiple_of(6) {
                    self.external_clock
                        .midi_step_pulses
                        .fetch_add(1, Ordering::AcqRel);
                }
            }
            RequestFullSnapshot => {
                let snap = self.snapshot.load_full();
                push_large_event(
                    &self.large_events,
                    EngineEvent::FullSnapshot {
                        session: (*snap).clone(),
                    },
                );
            }
            // Off-RT (state worker) → large channel. The state worker is allowed
            // to allocate (CLAUDE.md: RT path is sacred, the worker is not RT);
            // `postcard::to_allocvec` here is fine. Per D5/A2 large payloads
            // (`Serialized`/`FullSnapshot`/`Error`) ride the off-RT
            // `large_events` channel (drop-oldest, E8) — never the 128-byte hot
            // slot, which panics once a real session exceeds MAX_EVENT_BYTES.
            Serialize => {
                let snap = self.snapshot.load_full();
                let env = crate::serde_ext::SessionEnvelope::wrap((*snap).clone());
                if let Ok(bytes) = postcard::to_allocvec(&env) {
                    push_large_event(&self.large_events, EngineEvent::Serialized { bytes });
                }
            }
            // LoadSession restore (amendment A15). The bytes are a
            // `SessionEnvelope` (the output of `engine_serialize`). The worker
            // deserializes, validates the version + structural invariants, and
            // — only if both pass — publishes the new session (COW), bumps the
            // reload generation (so RT/CoreMIDI can react), and emits a
            // `FullSnapshot` on the off-RT LARGE channel. A bad envelope,
            // version, or structurally-invalid session is rejected: no swap, no
            // event, no panic (Task 18 validation correction).
            //
            // `FullSnapshot` rides `large_events` (D5/A2), never the 128-byte
            // hot slot — a real `Session` exceeds `MAX_EVENT_BYTES` and would
            // panic `push_event`. Submit-time decode failure is already caught
            // upstream as `ErrDecode` in the FFI shim.
            LoadSession { bytes } => {
                use crate::serde_ext::{SessionEnvelope, SESSION_FORMAT_VERSION};
                match postcard::from_bytes::<SessionEnvelope>(&bytes) {
                    Ok(env)
                        if env.version == SESSION_FORMAT_VERSION
                            && validate_session(&env.session) =>
                    {
                        self.publish(env.session);
                        self.reload_generation.fetch_add(1, Ordering::AcqRel);
                        let snap = self.snapshot.load_full();
                        push_large_event(
                            &self.large_events,
                            EngineEvent::FullSnapshot {
                                session: (*snap).clone(),
                            },
                        );
                    }
                    _ => { /* bad envelope/version/session: no swap, no event */ }
                }
            }
            // Algorithm/clipboard/undo arm (Task 15). `ref track_idx` keeps
            // `cmd` from being moved so the inner `match cmd` can dispatch by
            // variant (and recover `strength` for Roll/Vary).
            Roll { ref track_idx, .. }
            | Vary { ref track_idx, .. }
            | Cut { ref track_idx }
            | Copy { ref track_idx }
            | Paste { ref track_idx }
            | Trash { ref track_idx }
            | Undo { ref track_idx } => {
                let track_idx = *track_idx;
                let mut s = (*self.snapshot.load_full()).clone();
                // push undo BEFORE mutating for the mutating commands.
                // Copy and Undo are excluded (non-mutating).
                let mutating = matches!(
                    cmd,
                    Roll { .. } | Vary { .. } | Cut { .. } | Paste { .. } | Trash { .. }
                );
                if mutating {
                    if let Some(t) = s.patterns[s.active_pattern_index]
                        .as_ref()
                        .and_then(|p| p.tracks.get(track_idx))
                    {
                        self.undo.lock().unwrap().push(track_idx, t);
                    }
                }
                match cmd {
                    Roll { strength, .. } => {
                        // Seed first to release the immutable borrow of `s`
                        // before `track_mut` takes the mutable borrow.
                        let seed = seed_from(&s, track_idx);
                        if let Some(t) = track_mut(&mut s, track_idx) {
                            crate::algorithms::roll::roll(t, strength, &mut Rng::new(seed));
                        }
                    }
                    Vary { strength, .. } => {
                        let seed = seed_from(&s, track_idx);
                        if let Some(t) = track_mut(&mut s, track_idx) {
                            crate::algorithms::vary::vary(t, strength, &mut Rng::new(seed));
                        }
                    }
                    Cut { .. } => self.clipboard.lock().unwrap().cut(&mut s, track_idx),
                    Copy { .. } => self.clipboard.lock().unwrap().copy(&s, track_idx),
                    Paste { .. } => {
                        self.clipboard.lock().unwrap().paste(&mut s, track_idx);
                    }
                    Trash { .. } => {
                        if let Some(t) = track_mut(&mut s, track_idx) {
                            t.steps = [crate::models::Step::default(); crate::models::STEP_COUNT];
                        }
                    }
                    Undo { .. } => {
                        self.undo.lock().unwrap().undo(&mut s, track_idx);
                    }
                    _ => {}
                }
                let avail = self.undo.lock().unwrap().available(track_idx);
                self.publish(s);
                let snap = self.snapshot.load_full();
                crate::midi_out::push_large_event(
                    &self.large_events,
                    EngineEvent::FullSnapshot { session: (*snap).clone() }
                );
                crate::midi_out::push_event(
                    &self.hot_events,
                    &EngineEvent::UndoAvailable {
                        track_idx,
                        available: avail,
                    },
                );
            }
            // Scheduler (Task 16 module, wired here): the worker records the
            // request in atomics; the RT loop fires it at the quantize boundary
            // (`check_scheduler`); the worker then publishes the switch.
            QueuePattern { index, quantize } => {
                self.scheduler.queue(index, quantize);
            }
            CancelQueuedPattern => {
                self.scheduler.cancel();
            }
            RetriggerPattern { .. } => {
                // Retrigger restarts the current pattern from step 0; the RT loop
                // resets step counters at the next step boundary (NextStep semantics).
                self.scheduler.request_retrigger();
            }
            other => {
                let mut s = (*self.snapshot.load_full()).clone();
                let mut needs_full_snapshot = false;
                match other {
                    SetBpm { bpm } => {
                        s.bpm = bpm;
                        crate::midi_out::push_event(
                            &self.hot_events,
                            &EngineEvent::BpmChanged { bpm },
                        );
                    }
                    SetGlobalSwing { pct } => {
                        s.global_swing_pct = pct;
                        needs_full_snapshot = true;
                    }
                    SetHumanize { timing, velocity } => {
                        s.humanize_timing = timing;
                        s.humanize_velocity = velocity;
                        needs_full_snapshot = true;
                    }
                    // SetSyncSource is handled in the outer match (publishes a
                    // fresh session so the RT loop branches on the new source).
                    SetQuantizeGrain { grain: _ } => { /* stored on the scheduler in Task 16 */ }
                    SetGlobalMidiChannel { channel } => {
                        s.global_midi_channel = channel;
                        needs_full_snapshot = true;
                    }
                    SetMidiDestinations { endpoints } => {
                        s.midi_destinations = endpoints;
                        needs_full_snapshot = true;
                    }
                    SetTrackLength { track_idx, length } => {
                        with_track_mut(&mut s, track_idx, |t| {
                            t.length = length.clamp(1, crate::models::STEP_COUNT);
                            crate::midi_out::push_event(
                                &self.hot_events,
                                &EngineEvent::TrackLengthChanged { track_idx, length: t.length },
                            );
                        });
                    }
                    SetTrackMuted { track_idx, muted } => {
                        with_track_mut(&mut s, track_idx, |t| {
                            t.muted = muted;
                            crate::midi_out::push_event(
                                &self.hot_events,
                                &EngineEvent::TrackMutedChanged { track_idx, muted },
                            );
                        });
                    }
                    SetTrackNote {
                        track_idx,
                        midi_note,
                    } => {
                        with_track_mut(&mut s, track_idx, |t| t.midi_note = midi_note);
                        needs_full_snapshot = true;
                    }
                    SetTrackSpeedRatio { track_idx, ratio } => {
                        with_track_mut(&mut s, track_idx, |t| t.speed_ratio = ratio);
                        needs_full_snapshot = true;
                    }
                    SetTrackSwing {
                        track_idx,
                        swing_pct,
                    } => {
                        with_track_mut(&mut s, track_idx, |t| t.swing_pct = swing_pct);
                        needs_full_snapshot = true;
                    }
                    AddTrack => {
                        let old_len = active_pattern_mut(&mut s).map_or(0, |p| p.tracks.len());
                        add_track(&mut s);
                        if let Some(p) = active_pattern_mut(&mut s) {
                            if p.tracks.len() > old_len {
                                let track_idx = p.tracks.len() - 1;
                                let track = p.tracks[track_idx].clone();
                                crate::midi_out::push_large_event(
                                    &self.large_events,
                                    EngineEvent::TrackAdded { track_idx, track },
                                );
                            }
                        }
                    }
                    RemoveTrack => {
                        let old_len = active_pattern_mut(&mut s).map_or(0, |p| p.tracks.len());
                        remove_track(&mut s);
                        if let Some(p) = active_pattern_mut(&mut s) {
                            if p.tracks.len() < old_len {
                                crate::midi_out::push_event(
                                    &self.hot_events,
                                    &EngineEvent::TrackRemoved { track_idx: p.tracks.len() },
                                );
                            }
                        }
                    }
                    SetStep {
                        track_idx,
                        step_idx,
                        zone,
                    } => {
                        with_track_mut(&mut s, track_idx, |t| {
                            if step_idx < t.steps.len() {
                                t.steps[step_idx].active = true;
                                t.steps[step_idx].velocity_zone = zone;
                                crate::midi_out::push_event(
                                    &self.hot_events,
                                    &EngineEvent::StepChanged {
                                        track_idx,
                                        step_idx,
                                        step: t.steps[step_idx].clone(),
                                    },
                                );
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
                                crate::midi_out::push_event(
                                    &self.hot_events,
                                    &EngineEvent::StepChanged {
                                        track_idx,
                                        step_idx,
                                        step: t.steps[step_idx].clone(),
                                    },
                                );
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
                                crate::midi_out::push_event(
                                    &self.hot_events,
                                    &EngineEvent::StepChanged {
                                        track_idx,
                                        step_idx,
                                        step: t.steps[step_idx].clone(),
                                    },
                                );
                            }
                        });
                    }
                    SetFollowAction { pattern_idx, action } => {
                        if pattern_idx < s.patterns.len() {
                            if let Some(p) = s.patterns[pattern_idx].as_mut() {
                                p.follow_action = action.clone();
                            }
                            let _ = crate::midi_out::push_event(
                                &self.hot_events,
                                &EngineEvent::FollowActionChanged {
                                    pattern_idx,
                                    action,
                                },
                            );
                        }
                    }
                    // unreachable: the match arms above are exhaustive with the outer match
                    _ => {}
                }
                self.publish(s);
                if needs_full_snapshot {
                    let snap = self.snapshot.load_full();
                    crate::midi_out::push_large_event(
                        &self.large_events,
                        EngineEvent::FullSnapshot { session: (*snap).clone() },
                    );
                }
            }
        }
    }

    /// Worker thread body: drain commands until shutdown.
    pub fn run_worker_loop(self: &Arc<Engine>) {
        while !self.shutdown.load(Ordering::Acquire) {
            // Apply any RT-requested pattern switch promptly (the RT loop fired it
            // at a quantize boundary; publish the new active_pattern_index + emit
            // PatternSwitched). Wrapped in catch_unwind like commands.
            if let Some(idx) = self.scheduler.take_switch() {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    self.apply_pattern_switch(idx)
                }));
            }
            if let Some(cmd) = self.commands.dequeue() {
                // Panic-proof the worker: a panicking command is dropped and the
                // worker survives (mirrors the FFI catch_unwind panic-safety).
                // AssertUnwindSafe is the standard pattern — `&self`/`cmd` aren't
                // RefUnwindSafe; a panic here means a malformed command was being
                // applied during clone-mutate, before `publish`, so the live
                // ArcSwap snapshot is untouched.
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    self.apply_command(cmd)
                }));
            } else {
                std::thread::sleep(std::time::Duration::from_micros(200));
            }
        }
    }

    /// Worker: publish an RT-requested pattern switch (COW) + emit `PatternSwitched`.
    /// Validates the index (defensive against a corrupt request).
    fn apply_pattern_switch(&self, idx: usize) {
        if idx >= crate::models::PATTERN_SLOTS {
            return;
        }
        let mut s = (*self.snapshot.load_full()).clone();
        s.active_pattern_index = idx;
        self.publish(s);
        crate::midi_out::push_event(
            &self.hot_events,
            &EngineEvent::PatternSwitched { index: idx },
        );
    }
}

/// Per-tick counters returned by [`process`]. Testability seam so callers
/// (and the RT loop) can assert dispatch behavior without draining queues.
pub struct TickOutcome {
    pub playheads_emitted: u32,
    pub notes_pushed: u32,
}

/// One global tick. Pure except for queue pushes (MIDI ring + hot event slot).
///
/// `playing` is read by the caller ([`Engine::run_rt_loop`]) from the transport
/// atomics, so `process` stays free of atomics and is directly unit-testable.
/// RT-safety: no allocations, no locks, no FFI, no panics — the active pattern
/// is fetched via `patterns.get(...)` (bounds-checked) and array indexes are
/// either modulo `STEP_COUNT` or guarded by `idx < per_track.len()`.
#[allow(clippy::too_many_arguments)]
pub fn process(
    rt: &mut RtState,
    session: &Session,
    playing: bool,
    now_micros: u64,
    midi: &MidiOutRing,
    events: &HotEventChannel,
) -> TickOutcome {
    let mut outcome = TickOutcome {
        playheads_emitted: 0,
        notes_pushed: 0,
    };
    let _ = now_micros; // reserved: absolute tick time (LinkPhase positioning — Task 17)
    if !playing {
        return outcome;
    }
    // Bounds-checked pattern fetch — NEVER index `[Option<Pattern>; 9]` directly
    // on the RT path (out-of-range `active_pattern_index` would panic on RT,
    // violating Hard Rule 1). Task 18 validates the index on `LoadSession`.
    let Some(pattern) = session
        .patterns
        .get(session.active_pattern_index)
        .and_then(|p| p.as_ref())
    else {
        return outcome;
    };
    let step_period_micros = (60.0 / session.bpm / 4.0 * 1_000_000.0) as u64;
    let endpoint = session.midi_destinations.first().copied().unwrap_or(0);
    let channel = session.global_midi_channel;

    for (idx, track) in pattern.tracks.iter().enumerate() {
        if idx >= rt.per_track.len() || track.muted {
            continue;
        }
        let (steps, new_acc) =
            advance_speed_ratio(rt.per_track[idx].speed_acc, to_q16_16(track.speed_ratio));
        rt.per_track[idx].speed_acc = new_acc;
        for _ in 0..steps {
            let si = rt.per_track[idx].step_idx;
            rt.per_track[idx].step_idx = (si + 1) % track.length.max(1);
            // `si` was set under a bounded idx (0..length≤16) so it is < STEP_COUNT;
            // the modulo is belt-and-suspenders against a corrupted length.
            let step = track.steps[si % STEP_COUNT];
            if step.active {
                let base = velocity_for_zone(step.velocity_zone);
                let vel = humanize_velocity(
                    base,
                    session.humanize_velocity,
                    zone_weight(step.velocity_zone),
                    &mut rt.rng,
                );
                // E2/E3: swing + micro_timing become a per-note send_at_offset
                // the CoreMIDI worker applies when the deadline arrives.
                let swings = swing_offset_micros(
                    crate::clock::effective_swing(session.global_swing_pct, track.swing_pct),
                    si,
                    step_period_micros,
                );
                let mt = micro_timing_offset_micros(step.micro_timing_offset, step_period_micros);
                let offset = (swings + mt).max(0) as u32;
                for _ in 0..ratchet_count(step.ratchet) {
                    let _ = crate::midi_out::push_drop_oldest(
                        midi,
                        build_note_on(endpoint, channel, track.midi_note, vel, offset),
                    );
                    outcome.notes_pushed += 1;
                }
            }
            let _ = crate::midi_out::push_event(
                events,
                &EngineEvent::Playhead {
                    track_idx: idx,
                    step_idx: rt.per_track[idx].step_idx,
                },
            );
            outcome.playheads_emitted += 1;
        }
    }
    outcome
}

/// Humanize weight per zone — accents stay tighter than non-accented hits (E4).
fn zone_weight(z: crate::models::VelocityZone) -> f32 {
    match z {
        crate::models::VelocityZone::Accent => 1.0,
        _ => 0.6,
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

    #[test]
    fn serialize_command_emits_serialized_event_with_default_session() {
        // Drives the Serialize arm directly (no worker thread — Task 20 spawns it).
        // The arm must produce an EngineEvent::Serialized on the OFF-RT large
        // channel (D5/A2 — large payloads never ride the 128-byte hot slot)
        // whose bytes decode to a SessionEnvelope at the default bpm (120).
        let e = Engine::new();
        e.apply_command(Command::Serialize);
        // Hot channel must be empty — Serialized rides the large channel only.
        assert!(
            e.hot_events.dequeue().is_none(),
            "Serialize must not ride the hot channel"
        );
        let ev = e
            .large_events
            .dequeue()
            .expect("Serialized event pushed to large channel");
        let bytes = match ev {
            EngineEvent::Serialized { bytes } => bytes,
            other => panic!("expected Serialized, got {other:?}"),
        };
        let env: crate::serde_ext::SessionEnvelope =
            postcard::from_bytes(&bytes).expect("decode SessionEnvelope");
        assert_eq!(
            env.version,
            crate::serde_ext::SESSION_FORMAT_VERSION,
            "version tag must round-trip"
        );
        assert_eq!(env.session.bpm, 120.0, "default bpm must be 120");
        // The large channel must be drained (one event per Serialize).
        assert!(
            e.large_events.dequeue().is_none(),
            "Serialize produced more than one event"
        );
    }

    /// Task 18: `LoadSession` of a valid envelope publishes the session, bumps
    /// `reload_generation`, and emits exactly one `FullSnapshot` on the off-RT
    /// LARGE channel (never the 128-byte hot slot — a real Session exceeds
    /// MAX_EVENT_BYTES). Mirrors the Serialize arm's channel discipline.
    #[test]
    fn load_session_emits_full_snapshot_on_large_channel() {
        use crate::serde_ext::SessionEnvelope;
        let e = Engine::new();
        // Build a valid envelope (default session, correct version).
        let env = SessionEnvelope::wrap(Session {
            bpm: 77.0,
            ..Default::default()
        });
        let bytes = postcard::to_allocvec(&env).unwrap();
        let gen_before = e.reload_generation.load(Ordering::Acquire);
        e.apply_command(Command::LoadSession { bytes });
        // reload_generation bumped exactly once.
        assert_eq!(
            e.reload_generation.load(Ordering::Acquire),
            gen_before + 1,
            "reload_generation must bump on a successful LoadSession"
        );
        // Session was published.
        assert_eq!(e.snapshot_arc().bpm, 77.0);
        // Hot channel must be empty — FullSnapshot rides the large channel only.
        assert!(
            e.hot_events.dequeue().is_none(),
            "FullSnapshot must not ride the hot channel"
        );
        let ev = e
            .large_events
            .dequeue()
            .expect("FullSnapshot event pushed to large channel");
        match ev {
            EngineEvent::FullSnapshot { session } => {
                assert_eq!(
                    session.bpm, 77.0,
                    "snapshot must reflect the loaded session"
                );
            }
            other => panic!("expected FullSnapshot, got {other:?}"),
        }
        assert!(
            e.large_events.dequeue().is_none(),
            "LoadSession produced more than one event"
        );
    }

    /// Task 18 validation correction: a structurally-valid envelope with a bad
    /// `active_pattern_index` (>= PATTERN_SLOTS) is rejected at the engine layer
    /// — no publish, no reload_generation bump, no event. The C-ABI test covers
    /// the end-to-end no-swap; this pins the engine-layer guards directly.
    #[test]
    fn load_session_invalid_session_is_dropped_no_event() {
        use crate::serde_ext::{SessionEnvelope, SESSION_FORMAT_VERSION};
        let e = Engine::new();
        let bad = SessionEnvelope {
            version: SESSION_FORMAT_VERSION,
            session: Session {
                active_pattern_index: crate::models::PATTERN_SLOTS, // == 9 is out of range
                ..Default::default()
            },
        };
        let bytes = postcard::to_allocvec(&bad).unwrap();
        let gen_before = e.reload_generation.load(Ordering::Acquire);
        e.apply_command(Command::LoadSession { bytes });
        assert_eq!(
            e.reload_generation.load(Ordering::Acquire),
            gen_before,
            "reload_generation must NOT bump on a rejected LoadSession"
        );
        assert_eq!(e.snapshot_arc().bpm, 120.0, "session must be unchanged");
        assert!(
            e.large_events.dequeue().is_none(),
            "rejected LoadSession must not emit an event"
        );
        assert!(
            e.hot_events.dequeue().is_none(),
            "rejected LoadSession must not emit a hot event"
        );
    }

    /// `validate_session` directly: the invariants the restore path relies on.
    #[test]
    fn validate_session_accepts_default_rejects_corrupt() {
        use crate::models::{Pattern, PATTERN_SLOTS, STEP_COUNT};
        // Helper: a Session with `patterns[0]` set to the given pattern.
        fn with_pattern(p: Pattern) -> Session {
            let mut patterns = <[Option<Pattern>; PATTERN_SLOTS]>::default();
            patterns[0] = Some(p);
            Session {
                patterns,
                ..Default::default()
            }
        }
        assert!(validate_session(&Session::default()), "default is valid");
        // active_pattern_index out of range.
        let s = Session {
            active_pattern_index: PATTERN_SLOTS,
            ..Default::default()
        };
        assert!(
            !validate_session(&s),
            "active_pattern_index >= PATTERN_SLOTS"
        );
        // track length 0 (below the 1..=STEP_COUNT bound).
        let mut p = Pattern::default();
        p.tracks[0].length = 0;
        assert!(
            !validate_session(&with_pattern(p)),
            "track length 0 is invalid"
        );
        // length > STEP_COUNT.
        let mut p = Pattern::default();
        p.tracks[0].length = STEP_COUNT + 1;
        assert!(
            !validate_session(&with_pattern(p)),
            "track length > STEP_COUNT is invalid"
        );
        // length == STEP_COUNT boundary is valid.
        let mut p = Pattern::default();
        p.tracks[0].length = STEP_COUNT;
        assert!(
            validate_session(&with_pattern(p)),
            "length == STEP_COUNT is valid"
        );
    }

    #[test]
    fn scheduler_wiring_queue_fires_and_switches() {
        // Full no-threads wiring: worker queues → RT fires at the boundary →
        // worker publishes the switch + emits PatternSwitched.
        use crate::command::Command;
        use crate::models::{Pattern, QuantizeGrain, Step, VelocityZone};
        let e = Engine::new();
        // two populated patterns, active = 0
        let mut s = Session::default();
        s.patterns[0] = Some(Pattern::default());
        let mut p1 = Pattern::default();
        p1.tracks[0].steps[0] = Step {
            active: true,
            velocity_zone: VelocityZone::Accent,
            ..Default::default()
        };
        s.patterns[1] = Some(p1);
        s.active_pattern_index = 0;
        e.publish(s);

        // worker: queue pattern 1 at NextBar
        e.apply_command(Command::QueuePattern {
            index: 1,
            quantize: QuantizeGrain::NextBar,
        });

        // RT: at a bar boundary (global_step 0 → EndOfPattern), check_scheduler fires.
        // global_step 0 satisfies NextBar (via EndOfPattern).
        let mut rt = e.new_rt_state(); // global_step == 0
        let snap = e.snapshot_arc();
        e.check_scheduler(&mut rt, &snap);
        // RT requested the switch; all-notes-off burst pushed (MAX_TRACKS msgs)
        assert_eq!(e.scheduler.take_switch(), Some(1));
        for _ in 0..crate::models::MAX_TRACKS {
            assert!(e.midi.dequeue().is_some(), "all-notes-off burst pushed");
        }

        // worker: apply the switch → active_pattern_index == 1 + PatternSwitched
        e.apply_pattern_switch(1);
        assert_eq!(e.snapshot_arc().active_pattern_index, 1);
        let slot = e.hot_events.dequeue().expect("a PatternSwitched event");
        let ev: crate::event::EngineEvent =
            postcard::from_bytes(&slot.bytes[..slot.len as usize]).unwrap();
        assert_eq!(ev, crate::event::EngineEvent::PatternSwitched { index: 1 });
    }

    #[test]
    fn scheduler_retrigger_resets_step_counters() {
        use crate::command::Command;
        use crate::models::Pattern;
        let e = Engine::new();
        let mut s = Session::default();
        s.patterns[0] = Some(Pattern::default());
        e.publish(s);
        let mut rt = e.new_rt_state();
        rt.per_track[0].step_idx = 7; // mid-pattern
        rt.global_step = 5;
        e.apply_command(Command::RetriggerPattern {
            quantize: crate::models::QuantizeGrain::NextStep,
        });
        let snap = e.snapshot_arc();
        e.check_scheduler(&mut rt, &snap);
        assert_eq!(
            rt.per_track[0].step_idx, 0,
            "retrigger resets step counters"
        );
        assert_eq!(rt.global_step, 0);
    }

    #[test]
    fn scheduler_evaluates_follow_action_play_next() {
        use crate::models::{Pattern, FollowAction, FollowActionType};
        let e = Engine::new();
        let mut s = Session::default();
        let mut p0 = Pattern::default();
        p0.follow_action = FollowAction {
            after_loops: 2,
            action: FollowActionType::PlayNext,
        };
        s.patterns[0] = Some(p0);
        s.patterns[1] = Some(Pattern::default());
        s.active_pattern_index = 0;
        e.publish(s);

        let mut rt = e.new_rt_state();
        let snap = e.snapshot_arc();
        
        // Loop 1
        rt.global_step = 0;
        e.check_scheduler(&mut rt, &snap);
        assert_eq!(e.scheduler.take_switch(), None, "Should not switch on loop 1");

        // Loop 2
        rt.global_step = 0;
        e.check_scheduler(&mut rt, &snap);
        assert_eq!(e.scheduler.take_switch(), Some(1), "Should switch to pattern 1 on loop 2");
    }
}
