import Foundation

//! Wire models — the postcard-decoded contract between Rust core and Swift
//! shell (architecture-spec §5.1; amendment A4: bytes, not #[repr(C)]). These
//! types are decoded by `EngineEvent.decode` / `engine_serialize` output. The
//! value-type mirror in `SessionMirror` wraps a `Session` plus transient UI
//! state, reusing these enums/structs verbatim (no duplicate representation).

// MARK: - Enums (variant index = Rust declaration order)

/// `VelocityZone { Low=0, Mid=1, Accent=2 }`. Default `Mid` (ui-ux-spec §2.2).
enum VelocityZone: UInt8, PostcardCodable, CaseIterable {
    case low = 0, mid = 1, accent = 2

    func encode(to writer: inout PostcardWriter) { writer.writeTag(UInt(rawValue)); }
    init?(from reader: inout PostcardReader) {
        guard let idx = reader.readTag(), let v = Self(rawValue: UInt8(truncatingIfNeeded: idx)) else { return nil; }
        self = v
    }
}

/// `Ratchet { Off=0, X2=1, X3=2, X4=3 }`.
enum Ratchet: UInt8, PostcardCodable, CaseIterable {
    case off = 0, x2 = 1, x3 = 2, x4 = 3

    func encode(to writer: inout PostcardWriter) { writer.writeTag(UInt(rawValue)); }
    init?(from reader: inout PostcardReader) {
        guard let idx = reader.readTag(), let v = Self(rawValue: UInt8(truncatingIfNeeded: idx)) else { return nil; }
        self = v
    }

    /// Replay multiplier used by dispatch (Off→1).
    var repeats: Int {
        switch self { case .off: 1; case .x2: 2; case .x3: 3; case .x4: 4; }
    }

    /// Short UI label (ratchet popover).
    var label: String {
        switch self { case .off: "Off"; case .x2: "2×"; case .x3: "3×"; case .x4: "4×"; }
    }
}

/// `QuantizeGrain { NextStep=0, NextBeat=1, NextBar=2, EndOfPattern=3 }`.
enum QuantizeGrain: UInt8, PostcardCodable, CaseIterable {
    case nextStep = 0, nextBeat = 1, nextBar = 2, endOfPattern = 3

    func encode(to writer: inout PostcardWriter) { writer.writeTag(UInt(rawValue)); }
    init?(from reader: inout PostcardReader) {
        guard let idx = reader.readTag(), let v = Self(rawValue: UInt8(truncatingIfNeeded: idx)) else { return nil; }
        self = v
    }

    var shortLabel: String {
        switch self { case .nextStep: "Step"; case .nextBeat: "Beat"; case .nextBar: "Bar"; case .endOfPattern: "Pat"; }
    }
}

/// `SyncSource { Free=0, MidiClock=1, Link=2 }`.
enum SyncSource: UInt8, PostcardCodable, CaseIterable {
    case free = 0, midiClock = 1, link = 2

    func encode(to writer: inout PostcardWriter) { writer.writeTag(UInt(rawValue)); }
    init?(from reader: inout PostcardReader) {
        guard let idx = reader.readTag(), let v = Self(rawValue: UInt8(truncatingIfNeeded: idx)) else { return nil; }
        self = v
    }

    var label: String {
        switch self { case .free: "Free"; case .midiClock: "MIDI"; case .link: "Link"; }
    }
}

/// `FollowActionType { None, PlayNext, PlaySpecific(Uuid), PlayPrevious, Stop, PlayRandom }`.
/// `PlaySpecific` carries a `Uuid`, so this is not a simple raw-value enum.
indirect enum FollowActionType: PostcardCodable, Equatable, Hashable {
    case none, playNext, playSpecific(UUID), playPrevious, stop, playRandom

    func encode(to writer: inout PostcardWriter) {
        switch self {
        case .none: writer.writeTag(0)
        case .playNext: writer.writeTag(1)
        case .playSpecific(let u): writer.writeTag(2); writer.writeUUID(u)
        case .playPrevious: writer.writeTag(3)
        case .stop: writer.writeTag(4)
        case .playRandom: writer.writeTag(5)
        }
    }
    init?(from reader: inout PostcardReader) {
        guard let idx = reader.readTag() else { return nil; }
        switch idx {
        case 0: self = .none
        case 1: self = .playNext
        case 2: guard let u = reader.readUUID() else { return nil; }; self = .playSpecific(u)
        case 3: self = .playPrevious
        case 4: self = .stop
        case 5: self = .playRandom
        default: return nil
        }
    }

    var shortLabel: String {
        switch self {
        case .none: "None"; case .playNext: "→Next"; case .playSpecific: "→Pick";
        case .playPrevious: "→Prev"; case .stop: "Stop"; case .playRandom: "→Rand";
        }
    }
}

/// `FollowAction { after_loops: u32, action: FollowActionType }`.
struct FollowAction: PostcardCodable, Equatable {
    var afterLoops: UInt32
    var action: FollowActionType

    func encode(to writer: inout PostcardWriter) {
        writer.writeU32(afterLoops); action.encode(to: &writer)
    }
    init?(from reader: inout PostcardReader) {
        guard let loops = reader.readU32(), let action = FollowActionType(from: &reader) else { return nil; }
        self.init(afterLoops: loops, action: action)
    }
    init(afterLoops: UInt32 = 1, action: FollowActionType = .none) {
        self.afterLoops = afterLoops; self.action = action
    }
}

// MARK: - Aggregate structs

/// `Step { active: bool, velocity_zone, micro_timing_offset: f32, ratchet }`.
/// `micro_timing_offset` is carried (set by Roll) but not surfaced in the UI.
struct Step: PostcardCodable, Equatable, Hashable {
    var active: Bool
    var velocityZone: VelocityZone
    var microTimingOffset: Float
    var ratchet: Ratchet

    func encode(to writer: inout PostcardWriter) {
        writer.writeBool(active)
        velocityZone.encode(to: &writer)
        writer.writeF32(microTimingOffset)
        ratchet.encode(to: &writer)
    }
    init?(from reader: inout PostcardReader) {
        guard let active = reader.readBool(),
              let zone = VelocityZone(from: &reader),
              let offset = reader.readF32(),
              let ratchet = Ratchet(from: &reader) else { return nil; }
        self.init(active: active, velocityZone: zone, microTimingOffset: offset, ratchet: ratchet)
    }
    init(active: Bool = false, velocityZone: VelocityZone = .mid,
         microTimingOffset: Float = 0, ratchet: Ratchet = .off) {
        self.active = active; self.velocityZone = velocityZone
        self.microTimingOffset = microTimingOffset; self.ratchet = ratchet
    }
}

/// `Track { id: Uuid, midi_note: u8, length: usize (1-16 window), speed_ratio: f32,
///           swing_pct: f32, muted: bool, steps: [Step; 16] }`.
/// `length` is a non-destructive window over the fixed 16-step array.
struct Track: PostcardCodable, Identifiable, Equatable {
    let id: UUID
    var midiNote: UInt8
    var length: Int
    var speedRatio: Float
    var swingPct: Float
    var muted: Bool
    var steps: [Step]   // always 16

    func encode(to writer: inout PostcardWriter) {
        writer.writeUUID(id)
        writer.writeU8(midiNote)
        writer.writeUInt(UInt(length))
        writer.writeF32(speedRatio)
        writer.writeF32(swingPct)
        writer.writeBool(muted)
        for step in steps { step.encode(to: &writer); }   // [Step; 16] — no length prefix
    }
    init?(from reader: inout PostcardReader) {
        guard let id = reader.readUUID(),
              let note = reader.readU8(),
              let length = reader.readUInt(),
              let speed = reader.readF32(),
              let swing = reader.readF32(),
              let muted = reader.readBool() else { return nil; }
        var steps: [Step] = []
        steps.reserveCapacity(16)
        for _ in 0..<16 {
            guard let s = Step(from: &reader) else { return nil; }
            steps.append(s)
        }
        self.init(id: id, midiNote: note, length: length, speedRatio: speed,
                  swingPct: swing, muted: muted, steps: steps)
    }
    init(id: UUID = UUID(), midiNote: UInt8 = 36, length: Int = 16, speedRatio: Float = 1.0,
         swingPct: Float = 0, muted: Bool = false, steps: [Step] = Array(repeating: Step(), count: 16)) {
        self.id = id; self.midiNote = midiNote; self.length = length; self.speedRatio = speedRatio
        self.swingPct = swingPct; self.muted = muted; self.steps = steps
    }
}

/// `Pattern { id: Uuid, tracks: Vec<Track>, follow_action }`.
struct Pattern: PostcardCodable, Identifiable, Equatable {
    let id: UUID
    var tracks: [Track]
    var followAction: FollowAction

    func encode(to writer: inout PostcardWriter) {
        writer.writeUUID(id)
        writer.writeUInt(UInt(tracks.count))
        for t in tracks { t.encode(to: &writer); }
        followAction.encode(to: &writer)
    }
    init?(from reader: inout PostcardReader) {
        guard let id = reader.readUUID(), let n = reader.readUInt() else { return nil; }
        var tracks: [Track] = []
        tracks.reserveCapacity(n)
        for _ in 0..<n {
            guard let t = Track(from: &reader) else { return nil; }
            tracks.append(t)
        }
        guard let fa = FollowAction(from: &reader) else { return nil; }
        self.init(id: id, tracks: tracks, followAction: fa)
    }
    init(id: UUID = UUID(), tracks: [Track] = [], followAction: FollowAction = FollowAction()) {
        self.id = id; self.tracks = tracks; self.followAction = followAction
    }
}

/// `Session { bpm, sync_source, global_swing_pct, humanize_timing, humanize_velocity,
///             global_midi_channel, active_pattern_index, patterns: [Option<Pattern>; 9],
///             midi_destinations: Vec<u32> }`.
struct Session: PostcardDecodable, Equatable {
    var bpm: Double
    var syncSource: SyncSource
    var globalSwingPct: Float
    var humanizeTiming: Float
    var humanizeVelocity: Float
    var globalMidiChannel: UInt8
    var activePatternIndex: Int
    var patterns: [Pattern?]   // 9
    var midiDestinations: [UInt32]

    init?(from reader: inout PostcardReader) {
        guard let bpm = reader.readF64(),
              let sync = SyncSource(from: &reader),
              let gSwing = reader.readF32(),
              let hTime = reader.readF32(),
              let hVel = reader.readF32(),
              let ch = reader.readU8(),
              let active = reader.readUInt() else { return nil; }
        var patterns: [Pattern?] = []
        patterns.reserveCapacity(9)
        for _ in 0..<9 {
            guard let p = reader.readOption({ Pattern(from: &$0) }) else { return nil; }
            patterns.append(p)
        }
        guard let n = reader.readUInt() else { return nil; }
        var dest: [UInt32] = []
        dest.reserveCapacity(n)
        for _ in 0..<n {
            guard let d = reader.readU32() else { return nil; }
            dest.append(d)
        }
        self.bpm = bpm
        self.syncSource = sync
        self.globalSwingPct = gSwing
        self.humanizeTiming = hTime
        self.humanizeVelocity = hVel
        self.globalMidiChannel = ch
        self.activePatternIndex = active
        self.patterns = patterns
        self.midiDestinations = dest
    }

    /// Default session matching the engine's `Session::default()` (bpm 120, ch 10,
    /// 4 tracks, 9 empty pattern slots, no destinations).
    init() {
        self.bpm = 120
        self.syncSource = .free
        self.globalSwingPct = 0
        self.humanizeTiming = 0
        self.humanizeVelocity = 0
        self.globalMidiChannel = 10
        self.activePatternIndex = 0
        self.patterns = Array(repeating: nil, count: 9)
        // Match Rust Pattern::default(): Kick (36), Snare (38), Closed Hat (42), Clap (39)
        let defaultNotes: [UInt8] = [36, 38, 42, 39]
        self.patterns[0] = Pattern(tracks: defaultNotes.map { Track(midiNote: $0) })
        self.midiDestinations = []
    }
}

/// `SessionEnvelope { version: u8, session: Session }` — the on-disk / serialize
/// wire format (amendment A15; `SESSION_FORMAT_VERSION = 1`).
struct SessionEnvelope: PostcardDecodable, Equatable {
    static let currentVersion: UInt8 = 1

    var version: UInt8
    var session: Session

    init?(from reader: inout PostcardReader) {
        guard let v = reader.readU8(), let s = Session(from: &reader) else { return nil; }
        self.version = v; self.session = s
    }
    init(version: UInt8 = SessionEnvelope.currentVersion, session: Session) {
        self.version = version; self.session = session
    }
}
