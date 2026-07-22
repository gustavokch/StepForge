# iOS MIDI Drum Sequencer — Technical Architecture Spec

## 1. System Architecture

The app is split into two layers with a hard boundary:

### 1.1 Rust Core (`sequencer_engine`)
A static library (`.a` / `.xcframework`) compiled from Rust. Owns all musical-time logic, state, and MIDI dispatch.
- **Real-time thread**: High-priority, lock-free, allocation-free audio callback or custom high-precision timer. Drives step advancement, swing/humanize math, ratchet dispatch, MIDI output, and pattern transitions.
- **Command thread (main thread)**: Receives UI commands via a lock-free SPSC queue (`rtrb` or `crossbeam-queue`). Never blocks.
- **Serialization**: `serde` + `serde_json` (or `bincode`) for Session persistence. The engine produces a `Vec<u8>`; Swift writes it to disk.

### 1.2 Swift Shell (`SequencerApp`)
The iOS application. Owns everything non-musical:
- **SwiftUI views**: Grid, track headers, pattern picker, action drawer, note picker, MIDI device list, transport bar.
- **Gesture recognizers**: Translated to engine commands.
- **`EngineBridge` (`ObservableObject`)**: Wraps the Rust FFI. Holds a `@Published` mirror of engine state. Receives engine events on a dedicated `DispatchQueue`, applies deltas, and pushes to `MainActor`.
- **AbletonLink session**: Swift manages the Link client (peer discovery, session tempo/phase negotiation). Tempo and phase deltas are forwarded to the engine as commands.
- **CoreMIDI device discovery**: Swift enumerates endpoints via `MIDIClientCreate` / `MIDIGetNumberOfDestinations`. Passes endpoint integer IDs to the engine. The engine retains nothing — Swift owns the `MIDIClientRef` lifecycle.
- **Haptics, app lifecycle, file I/O**: All Swift.

### 1.3 Hard Rules
1. The real-time thread never crosses the FFI boundary. It never calls into Swift, never allocates, never locks.
2. The UI never holds a pointer into engine memory. All state flows through the event channel.
3. All FFI functions are non-blocking from the caller's perspective. If the engine needs to do work, it enqueues it for the RT thread or a worker.
4. `#![forbid(unsafe_code)]` is set at the crate level. The only `unsafe` blocks live in the FFI shim module and are reviewed line-by-line.

---

## 2. Crate Structure

```
sequencer_engine/
├── Cargo.toml
├── src/
│   ├── lib.rs                  # Crate root, public API
│   ├── ffi.rs                  # C ABI exports (#[no_mangle] extern "C")
│   ├── engine.rs               # Engine struct: owns RT thread, channels, state
│   ├── clock.rs                # Step scheduler, swing, humanize math
│   ├── scheduler.rs            # Pattern queue, quantize-grain evaluation, follow-actions
│   ├── midi.rs                 # MIDISend dispatch, All-Notes-Off flush
│   ├── algorithms/
│   │   ├── roll.rs             # Roll randomization
│   │   └── vary.rs             # Vary perturbation
│   ├── clipboard.rs            # Track + Session clipboards
│   ├── undo.rs                 # Per-track undo snapshots
│   ├── models.rs               # Session, Pattern, Track, Step, FollowAction
│   ├── command.rs              # Command enum (Swift → Rust)
│   ├── event.rs                # EngineEvent enum (Rust → Swift)
│   └── serde_ext.rs            # Serde serializers for persistence
└── tests/
    ├── clock_tests.rs
    ├── roll_tests.rs
    ├── vary_tests.rs
    ├── scheduler_tests.rs
    └── ffi_tests.rs            # Tests via the C ABI to catch FFI bugs
```

---

## 3. Threading & Communication Model

### 3.2 Command Channel (Swift → Rust)

```rust
// command.rs
#[derive(Debug, Clone)]
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
    Serialize,  // Returns Session as bytes via event
    Deserialize { bytes: Vec<u8> },
}
```

The FFI function `engine_submit_command` pushes onto the SPSC queue and returns immediately. The RT thread drains the queue at the top of each tick (every 1/16th note or finer).

### 3.3 Event Channel (Rust → Swift)

```rust
// event.rs
#[derive(Debug, Clone)]
pub enum EngineEvent {
    // Deltas
    StepChanged { track_idx: usize, step_idx: usize, step: Step },
    TrackLengthChanged { track_idx: usize, length: usize },
    TrackMutedChanged { track_idx: usize, muted: bool },
    TrackAdded { track_idx: usize, track: Track },
    TrackRemoved { track_idx: usize },
    PatternQueued { index: usize, quantize: QuantizeGrain },
    PatternSwitched { index: usize },
    PatternCleared { index: usize },
    FollowActionChanged { pattern_idx: usize, action: FollowAction },
    Playhead { track_idx: usize, step_idx: usize },
    PlayStateChanged { playing: bool },
    BpmChanged { bpm: f64 },
    SyncSourceChanged { source: SyncSource },
    UndoAvailable { track_idx: usize, available: bool },

    // Full state
    FullSnapshot { session: Session },

    // Persistence
    Serialized { bytes: Vec<u8> },

    // Error
    Error { code: i32, message: String },
}
```

Events are pushed from the RT thread onto a lock-free SPSC queue. A Swift `DispatchQueue` drains it on a timer (or via a `DispatchSourceUserDataOr` signal) at ~120 Hz, applies deltas to the mirror, and dispatches to `MainActor` in batches.

### 3.4 Playhead Coalescing
The RT thread emits `Playhead` events for every active step on every track. At 200 BPM with 8 tracks, that's ~400 events/sec. The Swift drain loop coalesces these into a single `Set<TrackIdx>` per drain cycle, so SwiftUI only re-renders the affected rows once per ~8 ms window.

---

## 4. FFI Boundary

### 4.1 C ABI Exports

```rust
// ffi.rs
use std::os::raw::{c_char, c_void};

#[repr(C)]
pub struct EngineHandle {
    _opaque: [u8; 0], // Never dereferenced in Swift
}

#[no_mangle]
pub extern "C" fn engine_new() -> *mut EngineHandle;

#[no_mangle]
pub extern "C" fn engine_free(engine: *mut EngineHandle);

#[no_mangle]
pub extern "C" fn engine_start(engine: *mut EngineHandle);

#[no_mangle]
pub extern "C" fn engine_stop(engine: *mut EngineHandle);

#[no_mangle]
pub extern "C" fn engine_submit_command(
    engine: *mut EngineHandle,
    cmd_ptr: *const u8,
    cmd_len: usize,
) -> bool; // true if successfully enqueued

#[no_mangle]
pub extern "C" fn engine_set_event_callback(
    engine: *mut EngineHandle,
    callback: extern "C" fn(*const EngineEventC, *mut c_void),
    context: *mut c_void,
);

#[no_mangle]
pub extern "C" fn engine_drain_events(
    engine: *mut EngineHandle,
    callback: extern "C" fn(*const EngineEventC, *mut c_void),
    context: *mut c_void,
) -> u32; // returns number of events drained

#[no_mangle]
pub extern "C" fn engine_serialize(
    engine: *mut EngineHandle,
    out_ptr: *const *mut u8,
    out_len: *const usize,
) -> bool;

#[no_mangle]
pub extern "C" fn engine_free_bytes(ptr: *mut u8, len: usize);
```

### 4.2 Command Serialization Across FFI
Commands are serialized to a compact byte buffer on the Swift side (using a generated encoder from `swift-bridge` or a hand-written binary codec) and passed as a pointer + length. The engine deserializes on the main-thread side of the FFI call, then pushes the typed `Command` enum onto the SPSC queue. This avoids exposing Rust enum layouts across the ABI.

### 4.3 Event Deserialization on Swift Side
The engine produces `EngineEventC` structs (C-compatible). Swift reads the event tag, switches on it, and constructs a Swift `EngineEvent` enum. For events carrying `Vec<u8>` or `String` (e.g., `Serialized`, `Error`), the engine allocates the data and passes ownership; Swift calls `engine_free_bytes` after consuming.

### 4.4 EngineBridge (Swift)

```swift
// EngineBridge.swift
@MainActor
final class EngineBridge: ObservableObject {
    @Published private(set) var mirror = SessionMirror()

    private var handle: OpaquePointer?
    private let eventQueue = DispatchQueue(label: "engine.events", qos: .userInitiated)

    init() {
        handle = engine_new()
        engine_set_event_callback(handle, { eventC, ctx in
            guard let ctx = ctx else { return }
            let bridge = Unmanaged<EngineBridge>.fromOpaque(ctx).takeUnretainedValue()
            bridge.handleEvent(eventC!)
        }, Unmanaged.passUnretained(self).toOpaque())
    }

    func submit(_ command: Command) {
        let bytes = command.encode()
        bytes.withUnsafeBufferPointer { buf in
            _ = engine_submit_command(handle, buf.baseAddress, buf.count)
        }
    }

    private nonisolated func handleEvent(_ eventC: UnsafePointer<EngineEventC>) {
        let event = EngineEvent(from: eventC)
        Task { @MainActor in
            self.mirror.apply(event)
        }
    }

    func saveState() -> Data {
        var ptr: UnsafeMutablePointer<UInt8>?
        var len: Int = 0
        guard engine_serialize(handle, &ptr, &len), let ptr else { return Data() }
        defer { engine_free_bytes(ptr, len) }
        return Data(bytes: ptr, count: len)
    }

    deinit {
        if let handle { engine_free(handle) }
    }
}
```

---

## 5. Data Models

### 5.1 Rust Models

```rust
// models.rs
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Session {
    pub bpm: f64,
    pub sync_source: SyncSource,
    pub global_swing_pct: f32,
    pub humanize_timing: f32,
    pub humanize_velocity: f32,
    pub global_midi_channel: u8, // Default 10
    pub active_pattern_index: usize,
    pub patterns: [Option<Pattern>; 9],
    pub midi_destinations: Vec<u32>, // MIDIEndpointRef as UInt32
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum SyncSource {
    Free,
    MidiClock,
    Link,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Pattern {
    pub id: Uuid,
    pub tracks: Vec<Track>, // 4-8 tracks
    pub follow_action: FollowAction,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FollowAction {
    pub after_loops: u32, // Default 1
    pub action: FollowActionType,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum FollowActionType {
    None,
    PlayNext,
    PlaySpecific(Uuid),
    PlayPrevious,
    Stop,
    PlayRandom,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Track {
    pub id: Uuid,
    pub midi_note: u8,
    pub length: usize, // 1-16 (playback window)
    pub speed_ratio: f32, // 0.5, 1.0, 2.0, 3.0
    pub swing_pct: f32, // Relative to global
    pub muted: bool,
    pub steps: [Step; 16], // Fixed array — length is a window, not a resize
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Step {
    pub active: bool,
    pub velocity_zone: VelocityZone,
    pub micro_timing_offset: f32, // Applied by Roll
    pub ratchet: Ratchet,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum VelocityZone { Low, Mid, Accent }

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum Ratchet { Off, X2, X3, X4 }

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum QuantizeGrain { NextStep, NextBeat, NextBar, EndOfPattern }
```

### 5.2 Swift Mirror Types

The Swift mirror is a simplified, immutable snapshot of engine state. It is never edited directly — only replaced or patched via `EngineEvent` application.

```swift
// SessionMirror.swift
struct SessionMirror {
    var bpm: Double = 120
    var syncSource: SyncSource = .free
    var globalSwingPct: Float = 0
    var humanizeTiming: Float = 0
    var humanizeVelocity: Float = 0
    var globalMidiChannel: UInt8 = 10
    var activePatternIndex: Int = 0
    var patterns: [PatternMirror?] = Array(repeating: nil, count: 9)
    var playheadStep: Int = 0
    var playing: Bool = false
    var queuedPatternIndex: Int? = nil
    var undoAvailable: Set<Int> = [] // track indices with undo
}

struct PatternMirror {
    var tracks: [TrackMirror]
    var followAction: FollowActionMirror
}

struct TrackMirror: Identifiable {
    let id: UUID
    var midiNote: UInt8
    var length: Int
    var speedRatio: Float
    var swingPct: Float
    var muted: Bool
    var steps: [StepMirror] // Always 16
}

struct StepMirror: Hashable {
    var active: Bool
    var velocityZone: VelocityZone
    var ratchet: Ratchet
}

extension SessionMirror {
    mutating func apply(_ event: EngineEvent) {
        switch event {
        case .stepChanged(let track, let step, let s):
            patterns[activePatternIndex]???.tracks[track].steps[step] = StepMirror(from: s)
        case .playhead(let _, let step):
            playheadStep = step
        case .fullSnapshot(let session):
            self = SessionMirror(from: session)
        // ... all cases
        }
    }
}
```

### 5.3 Why Mirror Instead of Direct Binding
Rust structs live in engine memory. The RT thread mutates them continuously. If SwiftUI held a reference, it would race. The mirror is a value-type snapshot on the main actor — SwiftUI can diff it safely. The cost is a small allocation per event batch, which is negligible compared to SwiftUI's own diffing.

---

## 6. Sequencer Logic & Invariants

### 6.1 Non-Destructive Length
`Track.length` is strictly a playback loop window. `steps` is always a fixed `[Step; 16]`. Shrinking `length` to 8 does not touch `steps[8..16]`. Expanding back to 16 restores original data. `length` changes do not push to the undo stack. The RT thread's step counter wraps at `length`, not at 16.

```rust
// clock.rs
fn current_step(&self, track: &Track) -> usize {
    (self.global_step_counter as usize) % track.length
}
```

### 6.2 Roll Algorithm
```rust
// algorithms/roll.rs
pub fn roll(track: &mut Track, strength: f32, rng: &mut impl Rng) {
    track.push_undo_snapshot();
    for step in track.steps[..track.length].iter_mut() {
        if rng.gen::<f32>() < strength {
            step.active = true;
            step.velocity_zone = match rng.gen_range(0..3) {
                0 => VelocityZone::Low,
                1 => VelocityZone::Mid,
                _ => VelocityZone::Accent,
            };
            step.micro_timing_offset = rng.gen_range(-0.5..0.5) * strength;
        } else {
            step.active = false;
        }
    }
    // length, midi_note, speed_ratio untouched
}
```
- **Invariants**: Does not alter `length`. Pushes prior state to track's undo snapshot.

### 6.3 Vary Algorithm
```rust
// algorithms/vary.rs
pub fn vary(track: &mut Track, strength: f32, rng: &mut impl Rng) {
    track.push_undo_snapshot();

    let accent_indices: Vec<usize> = track.steps[..track.length]
        .iter()
        .enumerate()
        .filter(|(_, s)| s.velocity_zone == VelocityZone::Accent)
        .map(|(i, _)| i)
        .collect();

    if accent_indices.is_empty() {
        // Fallback: behave like Roll
        roll::roll(track, strength, rng);
        return;
    }

    for (i, step) in track.steps[..track.length].iter_mut().enumerate() {
        if accent_indices.contains(&i) {
            continue; // Lock accents
        }
        if rng.gen::<f32>() < strength {
            step.active = !step.active;
        }
        if step.active && step.velocity_zone != VelocityZone::Accent {
            step.velocity_zone = if rng.gen::<f32>() < 0.5 {
                VelocityZone::Low
            } else {
                VelocityZone::Mid
            };
        }
    }
    // length untouched
}
```
- **Invariants**: Does not alter `length`. Pushes prior state to track's undo snapshot.

### 6.4 Swing & Humanize
Swing delays odd-numbered 16th steps by a percentage of the step duration. Humanize adds per-step jitter to timing (±ms) and velocity (±value). Both are applied at dispatch time in the RT thread, not baked into the `Step` struct. This keeps the model clean and allows real-time adjustment.

```rust
// clock.rs
fn dispatch_step(&self, track: &Track, step_idx: usize, base_time: AudioTime) {
    let mut time = base_time;

    // Swing: delay odd steps
    if step_idx % 2 == 1 {
        time += self.step_duration() * track.swing_pct;
    }

    // Humanize timing
    let jitter = if self.humanize_timing > 0.0 {
        self.rng.gen_range(-1.0..1.0) * self.humanize_timing * self.step_duration() * 0.25
    } else { 0.0 };
    time += jitter;

    let step = &track.steps[step_idx];
    if !step.active || track.muted { return; }

    // Humanize velocity
    let velocity = self.apply_velocity_humanize(step.velocity_zone);

    // Ratchet
    let repeats = match step.ratchet {
        Ratchet::Off => 1,
        Ratchet::X2 => 2,
        Ratchet::X3 => 3,
        Ratchet::X4 => 4,
    };
    let ratchet_interval = self.step_duration() / repeats as f32;

    for r in 0..repeats {
        let ratchet_time = time + (r as f32 * ratchet_interval);
        self.schedule_midi(track.midi_note, velocity, ratchet_time);
    }
}
```

---

## 7. Clipboard & Undo Logic

### 7.1 Clipboards
Two independent global clipboard buffers, owned by the engine:

```rust
// clipboard.rs
pub struct Clipboards {
    pub track: Option<TrackClipboard>,
    pub session: Option<Pattern>,
}

#[derive(Clone)]
pub struct TrackClipboard {
    pub steps: [Step; 16],
    pub length: usize,
    pub speed_ratio: f32,
    // NOTE: midi_note is intentionally NOT stored.
    // Pasting a track's steps does not retune the destination.
}
```

- **Cut**: Copies to `track` clipboard, clears source steps, pushes undo snapshot.
- **Copy**: Copies to `track` clipboard, source unchanged.
- **Paste**: Pushes undo snapshot, overwrites destination steps/length/speed_ratio from clipboard. `midi_note` is preserved.
- **Trash**: Pushes undo snapshot, clears all steps to inactive. `length`, `midi_note`, `speed_ratio` preserved.

### 7.2 Undo System
Per-track, one-deep:

```rust
// undo.rs
pub struct UndoStack {
    snapshots: Vec<Option<Track>>, // One slot per track, max 8
}

impl UndoStack {
    pub fn push(&mut self, track_idx: usize, track: &Track) {
        if track_idx < self.snapshots.len() {
            self.snapshots[track_idx] = Some(track.clone());
        }
    }

    pub fn pop(&mut self, track_idx: usize) -> Option<Track> {
        self.snapshots.get_mut(track_idx)?.take()
    }

    pub fn has_undo(&self, track_idx: usize) -> bool {
        self.snapshots.get(track_idx).map_or(false, |s| s.is_some())
    }
}
```

- **Snapshot triggers**: Roll, Vary, Cut, Paste, Trash.
- **Consumption**: Either via the inline "✕ revert" button (post-Roll/Vary) or a global undo gesture/icon. Each consumes the snapshot for that track.
- **Session Paste Undo**: Overwriting a whole pattern is high blast-radius. Session Paste requires a Swift-side confirmation dialog ("Replace Pattern X with copied contents?"). If confirmed, the command is sent to the engine. The engine does not offer pattern-level undo in v1.

---

## 8. Performance & Live Jamming Mechanics

### 8.1 Mode Toggles
Modes are a UI concept. The engine is mode-agnostic — it simply honors the `QuantizeGrain` attached to each `QueuePattern` command and the `FollowAction` stored on each pattern.

- **Arrangement Mode**: Swift sets default quantize grain to `.endOfPattern` when entering. Follow-actions drive the sequence.
- **Jam Mode**: Swift sets default quantize grain to `.nextBeat` when entering. Any `QueuePattern` command resets the active pattern's `follow_action.after_loops` counter to its configured value, pausing follow-action evaluation for one loop cycle.

### 8.2 Pattern Switching & Quantization
All pattern transitions are evaluated by the RT thread at quantize boundaries:

```rust
// scheduler.rs
fn on_quantize_boundary(&mut self, grain: QuantizeGrain) {
    // Check if a pattern is queued and the grain matches
    if let Some(queued) = &self.queued_pattern {
        if queued.quantize == grain || grain == QuantizeGrain::NextStep {
            self.transition_to_pattern(queued.index);
            self.queued_pattern = None;
        }
    }

    // Evaluate follow-action if no manual queue is pending
    if self.queued_pattern.is_none() && self.active_pattern.follow_action.action != FollowActionType::None {
        self.loops_completed += 1;
        if self.loops_completed >= self.active_pattern.follow_action.after_loops {
            self.execute_follow_action();
            self.loops_completed = 0;
        }
    }
}

fn transition_to_pattern(&mut self, new_index: usize) {
    // CRITICAL: Flush any active notes from the outgoing pattern
    self.midi.flush_all_notes_off(self.global_midi_channel);

    self.active_pattern_index = new_index;
    self.global_step_counter = 0;
    self.loops_completed = 0;

    self.emit_event(EngineEvent::PatternSwitched { index: new_index });
}
```

### 8.3 Retrigger Shortcuts
- **Tap Active**: Queues a retrigger at the next global quantize grain.
- **Long Press Active**: Queues a retrigger quantized strictly to 1/16th note, bypassing the global grain setting. Implemented by sending `RetriggerPattern { quantize: QuantizeGrain::NextStep }`.
- **Cancel Queue**: Tapping a queued cell sends `CancelQueuedPattern`, which clears `queued_pattern` on the engine.

### 8.4 All-Notes-Off Flush
At every pattern transition, the engine sends MIDI Note Off for all 128 notes on the global MIDI channel to all configured destinations. This prevents hung notes when switching patterns mid-sequence. The flush is synchronous on the RT thread and completes before the new pattern's first step dispatches.

```rust
// midi.rs
pub fn flush_all_notes_off(&mut self, channel: u8) {
    for note in 0..128u8 {
        for &endpoint in &self.destinations {
            self.send_note_off(endpoint, channel, note, 0);
        }
    }
}
```

---

## 9. MIDI & Synchronization

### 9.1 CoreMIDI Integration
- **Swift side**: Creates a `MIDIClientRef` with a notification callback. Enumerates destinations via `MIDIGetNumberOfDestinations()` / `MIDIGetDestination()`. Maintains a `Set<MIDIEndpointRef>` of selected outputs. When the set changes, sends a `SetMidiDestinations { endpoints: Vec<u32> }` command to the engine.
- **Rust side**: Stores `Vec<u32>` endpoint IDs. On the RT thread, constructs `MIDIPacketList` and calls `MIDISend` directly via CoreMIDI's C API (linked through `extern "C"`). No `MIDIClientRef` is retained in Rust — it borrows the client ref passed from Swift for each `MIDISend` call, or Swift passes a `MIDIPortRef` at init.

### 9.2 AbletonLink Integration
- **Swift side**: Owns the `ABLLink` session instance. Receives tempo/peers callbacks. On tempo change, sends `SetBpm { bpm }`. On phase/beginning-of-beat notifications, sends a `LinkPhase` command so the engine can align its step counter.
- **Rust side**: The engine treats Link like any other sync source. When `sync_source == Link`, the RT thread does not advance its own clock — it waits for phase-sync commands from Swift.

### 9.3 MIDI Clock (Inbound)
- **Swift side**: Registers a MIDI input port for MIDI Clock messages. Parses `0xF8` (Clock) and `0xFA` (Start). forwards tempo derived from clock interval to the engine via `SetBpm`.
- **Rust side**: Same as Link — when `sync_source == MidiClock`, step advancement is driven by incoming clock commands rather than the internal scheduler.

### 9.4 No BLE MIDI
v1 explicitly drops Bluetooth LE MIDI. All MIDI routing is USB/Lightning class-compliant or Network MIDI. This is an intentional latency trade-off.

---

## 10. Build Pipeline & Distribution

### 10.1 Build Script

```bash
#!/bin/bash
# build_engine.sh — produces SequencerEngine.xcframework

set -e

# Build for device
cargo build --release --target aarch64-apple-ios

# Build for simulator (Apple Silicon)
cargo build --release --target aarch64-apple-ios-sim

# Build for simulator (Intel)
cargo build --release --target x86_64-apple-ios

# Create xcframework
xcodebuild -create-xcframework \
  -lib target/aarch64-apple-ios/release/libsequencer_engine.a \
  -headers include/ \
  -lib target/aarch64-apple-ios-sim/release/libsequencer_engine.a \
  -headers include/ \
  -lib target/x86_64-apple-ios/release/libsequencer_engine.a \
  -headers include/ \
  -output SequencerEngine.xcframework
```

### 10.2 Xcode Integration
- `SequencerEngine.xcframework` is embedded in the app bundle.
- A Run Script Phase invokes `build_engine.sh` only when `Cargo.toml` or `src/` files have changed (checked via a timestamp file). This avoids recompiling Rust on every Swift build.
- The C header (`include/sequencer_engine.h`) is generated by `cbindgen` and added to the Xcode project as a bridging header.

### 10.3 `cbindgen` Configuration

```toml
# cbindgen.toml
language = "C"
include_guard = "SEQUENCER_ENGINE_H"
autogen_warning = "/* Auto-generated by cbindgen. Do not edit. */"
tab_width = 4
style = "both"
```

### 10.4 Binary Size & Compile Times
- Stripped release build of the engine crate: ~1.5–3 MB.
- Cold cargo build: ~30–60s. Incremental: ~3–8s.
- The xcframework is checked into git LFS or fetched from CI to avoid local cargo dependency for designers.

---

## 11. Testing & Validation

### 11.1 Rust Unit Tests
- **Clock**: Swing offset math, step wrapping at `length`, speed-ratio multiplication.
- **Roll**: Determinism with seeded RNG, density distribution, invariant: `length` unchanged.
- **Vary**: Accent locking, fallback to Roll when no accents, invariant: `length` unchanged.
- **Scheduler**: Follow-action evaluation, loop counter reset, pattern queue at quantize boundaries.
- **Undo**: Snapshot push/pop, one-deep limit, per-track isolation.
- **Clipboard**: Track clipboard preserves `steps`/`length`/`speed_ratio` but not `midi_note`.

### 11.2 FFI Integration Tests
Tests that call the engine through the C ABI (not the Rust API directly) to catch serialization and ABI mismatches:
- Submit a `SetStep` command, drain events, assert `StepChanged` received.
- Serialize a session, deserialize into a new engine, assert state matches.
- Rapid-fire 1000 commands, assert all applied in order.

### 11.3 Swift UI Tests
- Tap a step → assert visual state matches within 1 display frame.
- Drag velocity → assert hue changes at zone boundaries.
- Long-press ratchet → assert popover appears at ~450ms.
- Pattern switch → assert All-Notes-Off sent (verify via virtual MIDI port capture).

### 11.4 Real-Time Safety Audit
- Run the engine under `Instruments → Time Profiler` with the RT thread flagged.
- Assert no system calls on the RT thread (no `malloc`, no `pthread_mutex_lock`, no I/O).
- The engine crate sets `#![forbid(unsafe_code)]` except in `ffi.rs`, which is reviewed line-by-line. `unsafe` in `ffi.rs` is limited to pointer dereference of the engine handle and `extern "C"` declarations.
