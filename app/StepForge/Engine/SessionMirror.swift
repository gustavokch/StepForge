import Foundation

/// An engine error surfaced to the UI (`EngineEvent::Error` / synthesized on
/// `Overflow`). Plain value type so `SessionMirror` stays `Equatable`.
struct EngineErrorMirror: Equatable {
    var code: Int32
    var message: String
}

/// Value-type mirror of engine state, mutated **only** by applying `EngineEvent`s
/// on the MainActor (CLAUDE.md Hard Rule 2: UI reads no pointer into engine
/// memory). Wraps the musical `Session` and adds transient UI-only state (play,
/// queued pattern, per-track playheads, undo availability, last error/overflow)
/// that has no representation on the wire. SwiftUI re-renders from this; leaf
/// views take narrow value slices so a single `StepChanged` rebuilds one cell.
struct SessionMirror: Equatable {
    /// The musical state (patterns, tracks, steps, transport params). Replaced
    /// wholesale on `FullSnapshot`; mutated in place on deltas.
    var session: Session = Session()

    // Transient UI-only state (no wire representation).
    var playing: Bool = false
    var queuedPatternIndex: Int? = nil
    var queuedPatternQuantize: QuantizeGrain? = nil
    var patternLoopCount: UInt32 = 0
    /// Per-track latest step index — the coalesced result of `Playhead` events
    /// (amendment E7: one entry per track per drain batch, not a single global).
    var playheads: [Int: Int] = [:]
    /// Track indices with an undo snapshot available.
    var undoAvailable: Set<Int> = []
    var lastError: EngineErrorMirror? = nil
    var lastOverflow: UInt32? = nil
    var linkPeers: Int = 0
    var linkEnabled: Bool = false

    // MARK: Convenience accessors (views read these)

    var bpm: Double { session.bpm }
    var syncSource: SyncSource { session.syncSource }
    var globalSwingPct: Float { session.globalSwingPct }
    var humanizeTiming: Float { session.humanizeTiming }
    var humanizeVelocity: Float { session.humanizeVelocity }
    var globalMidiChannel: UInt8 { session.globalMidiChannel }
    var activePatternIndex: Int { session.activePatternIndex }
    var activePattern: Pattern? {
        session.patterns.indices.contains(session.activePatternIndex)
            ? session.patterns[session.activePatternIndex] : nil
    }
    /// Tracks of the active pattern (empty if the slot has no pattern).
    var tracks: [Track] { activePattern?.tracks ?? [] }
    /// Patterns in the session.
    var patterns: [Pattern?] { session.patterns }
    /// Playhead step index for track 0.
    var playheadStep: Int? { playheads[0] }

    /// Calculates the next pattern index based on the follow action of the specified pattern index.
    /// Returns nil if there is no next pattern (e.g. action is none or stop).
    func nextPatternIndex(from patternIdx: Int) -> Int? {
        guard patterns.indices.contains(patternIdx),
              let pattern = patterns[patternIdx],
              pattern.followAction.action != .none,
              pattern.followAction.action != .stop else {
            return nil
        }
        
        switch pattern.followAction.action {
        case .playNext:
            return (patternIdx + 1) % patterns.count
        case .playPrevious:
            return (patternIdx + patterns.count - 1) % patterns.count
        case .playSpecific(let id):
            return patterns.firstIndex(where: { $0?.id == id }) ?? patternIdx
        case .playRandom:
            return nil // Random is non-deterministic, no glow
        default:
            return nil
        }
    }

    // MARK: Event application (runs on MainActor, one hop per drain batch)

    mutating func apply(_ event: EngineEvent) {
        switch event {
        case .stepChanged(let t, let s, let step):
            mutateStep(at: t, stepIdx: s) { $0 = step }
        case .trackLengthChanged(let t, let length):
            mutateTrack(at: t) { $0.length = length }
        case .trackMutedChanged(let t, let muted):
            mutateTrack(at: t) { $0.muted = muted }
        case .trackAdded(let t, let track):
            mutateActivePattern { $0.tracks.insert(track, at: min(t, $0.tracks.count)) }
        case .trackRemoved(let t):
            mutateActivePattern { if $0.tracks.indices.contains(t) { $0.tracks.remove(at: t) } }
        case .patternQueued(let i, let q):
            queuedPatternIndex = i; queuedPatternQuantize = q
        case .patternSwitched(let i):
            session.activePatternIndex = i
            queuedPatternIndex = nil; queuedPatternQuantize = nil
            patternLoopCount = 0
            playheads.removeAll(keepingCapacity: true)   // RT resets its counters on switch
            undoAvailable.removeAll(keepingCapacity: true) // undo is pattern-scoped
        case .patternCleared(let i):
            if session.patterns.indices.contains(i) { session.patterns[i] = nil }
            if i == session.activePatternIndex { queuedPatternIndex = nil }
        case .patternLoopCountChanged(let count):
            patternLoopCount = count
        case .followActionChanged(let p, let action):
            if session.patterns.indices.contains(p) {
                session.patterns[p]?.followAction = action
            }
        case .playhead:
            break   // never reaches apply(); handled via applyPlayhead (coalesced)
        case .playStateChanged(let isPlaying):
            playing = isPlaying
            if !isPlaying {
                patternLoopCount = 0
            }
        case .bpmChanged(let bpm):
            session.bpm = bpm
        case .syncSourceChanged(let s):
            session.syncSource = s
            // Mirror the engine's auto-enable rule (Defect 3 — same rationale as
            // `applyOptimistic(.setSyncSource)` below): selecting Link enables the
            // session, otherwise disables it. Keeps the mirror self-consistent
            // even if the separate `LinkEnabledChanged` event is dropped under
            // hot-channel overflow (drop-oldest, 32 slots).
            linkEnabled = (s == .link)
        case .undoAvailable(let t, let available):
            if available { undoAvailable.insert(t) } else { undoAvailable.remove(t) }
        case .fullSnapshot(let snapshot):
            session = snapshot
            queuedPatternIndex = nil; queuedPatternQuantize = nil
            patternLoopCount = 0
            playheads.removeAll(keepingCapacity: true)
            // undoAvailable is NOT cleared here. Two sources drive it:
            // `.undoAvailable` (set/clear per track) and `.patternSwitched`
            // (clear on switch — undo is pattern-scoped). Only the algorithm/
            // clipboard/undo command arm co-emits `.fullSnapshot` +
            // `.undoAvailable`; clearing here would wipe the just-emitted undo
            // flag for that command and Undo could never enable. Other mutating
            // commands (setGlobalSwing, setTrackNote, …) emit `.fullSnapshot`
            // alone; they don't push undo, so there is nothing to wipe either way.
            lastError = nil
            lastOverflow = nil
        case .serialized:
            break   // the bridge routes Serialized bytes to SessionStore, not the mirror
        case .error(let code, let message):
            lastError = EngineErrorMirror(code: code, message: message)
        case .overflow(let dropped):
            lastOverflow = dropped
        case .linkPeersChanged(let count):
            linkPeers = count
        case .linkEnabledChanged(let enabled):
            linkEnabled = enabled
        }
    }

    /// Apply one coalesced per-track playhead (called per entry of the batch map).
    mutating func applyPlayhead(trackIdx: Int, stepIdx: Int) {
        playheads[trackIdx] = stepIdx
    }

    // MARK: Optimistic echo (MockEngineBridge only — never the real bridge)

    /// Mirrors a command's musical effect locally so the mock bridge behaves as if
    /// the engine had already echoed the event. Used for previews/UI tests against
    /// the (currently stubbed) engine. The real `EngineBridge` never calls this —
    /// it forwards the command and lets the engine's event update the mirror.
    mutating func applyOptimistic(_ command: Command) {
        switch command {
        case .setStep(let t, let s, let zone):
            mutateStep(at: t, stepIdx: s) { $0.active = true; $0.velocityZone = zone } // in place: preserve ratchet/timing
        case .deleteStep(let t, let s):
            mutateStep(at: t, stepIdx: s) { $0.active = false }
        case .setRatchet(let t, let s, let ratchet):
            mutateStep(at: t, stepIdx: s) { $0.ratchet = ratchet }
        case .setTrackLength(let t, let length):
            mutateTrack(at: t) { $0.length = max(1, min(16, length)) }
        case .setTrackMuted(let t, let muted):
            mutateTrack(at: t) { $0.muted = muted }
        case .setTrackNote(let t, let note):
            mutateTrack(at: t) { $0.midiNote = note }
        case .setTrackSpeedRatio(let t, let ratio):
            mutateTrack(at: t) { $0.speedRatio = ratio }
        case .setTrackSwing(let t, let swing):
            mutateTrack(at: t) { $0.swingPct = swing }
        case .addTrack:
            mutateActivePattern { if $0.tracks.count < 8 { $0.tracks.append(Track()) } }
        case .removeTrack:
            mutateActivePattern { if $0.tracks.count > 4 { $0.tracks.removeLast() } }
        case .trash(let t):
            mutateTrack(at: t) { $0.steps = Array(repeating: Step(), count: 16) }
        case .setGlobalSwing(let pct):
            session.globalSwingPct = pct
        case .setHumanize(let timing, let velocity):
            session.humanizeTiming = timing; session.humanizeVelocity = velocity
        case .setBpm(let bpm):
            session.bpm = bpm
        case .setSyncSource(let source):
            session.syncSource = source
            // Mirror the engine's auto-enable rule (Defect 3): selecting Link
            // enables the session, otherwise disables it.
            linkEnabled = (source == .link)
        case .setGlobalMidiChannel(let ch):
            session.globalMidiChannel = ch
        case .setQuantizeGrain:
            break   // engine-only state; no optimistic echo
        case .play:
            playing = true
        case .stop:
            playing = false
            patternLoopCount = 0
        case .queuePattern(let index, let quantize):
            queuedPatternIndex = index
            queuedPatternQuantize = quantize
        case .cancelQueuedPattern:
            queuedPatternIndex = nil
            queuedPatternQuantize = nil
        case .setFollowAction(let patternIdx, let action):
            if session.patterns.indices.contains(patternIdx) {
                session.patterns[patternIdx]?.followAction = action
            }
        case .setMidiDestinations(let endpoints):
            session.midiDestinations = endpoints
        case .setLinkEnabled(let enabled):
            linkEnabled = enabled
        // Roll/Vary/Cut/Copy/Paste/Undo/sync/load — no deterministic
        // optimistic echo (engine logic / RNG / external state); the real engine
        // echoes the result. The mock leaves the mirror untouched for these.
        case .roll, .vary, .cut, .copy, .paste, .undo,
             .retriggerPattern, .requestFullSnapshot,
             .serialize, .loadSession, .midiClockTick,
             .copyPattern, .cutPattern, .pastePattern, .clearPattern:
            break
        }
    }

    // MARK: Nested mutation helpers (copy-out pattern for the optional array)

    @inline(__always) private mutating func mutateTrack(at trackIdx: Int, _ body: (inout Track) -> Void) {
        guard session.patterns.indices.contains(session.activePatternIndex),
              var pattern = session.patterns[session.activePatternIndex],
              pattern.tracks.indices.contains(trackIdx) else { return; }
        body(&pattern.tracks[trackIdx])
        session.patterns[session.activePatternIndex] = pattern
    }

    @inline(__always) private mutating func mutateActivePattern(_ body: (inout Pattern) -> Void) {
        guard session.patterns.indices.contains(session.activePatternIndex),
              var pattern = session.patterns[session.activePatternIndex] else { return; }
        body(&pattern)
        session.patterns[session.activePatternIndex] = pattern
    }

    /// Mutate a single step, validating both track and step indices (an
    /// out-of-range index from a malformed/racy event is dropped, never traps —
    /// Hard Rule 3 at the value-type layer).
    @inline(__always) private mutating func mutateStep(at trackIdx: Int, stepIdx: Int, _ body: (inout Step) -> Void) {
        guard session.patterns.indices.contains(session.activePatternIndex),
              var pattern = session.patterns[session.activePatternIndex],
              pattern.tracks.indices.contains(trackIdx),
              pattern.tracks[trackIdx].steps.indices.contains(stepIdx) else { return; }
        body(&pattern.tracks[trackIdx].steps[stepIdx])
        session.patterns[session.activePatternIndex] = pattern
    }
}

// MARK: - Demo seed (previews / UI tests)

extension SessionMirror {
    /// A rich, deterministic mirror for SwiftUI previews and UI tests: 6 tracks
    /// (Kick / Snare / Closed Hat / Open Hat / Perc / Bass) with varied steps,
    /// bpm 120, one pattern. Never touches the engine.
    static var demoSeed: SessionMirror {
        var m = SessionMirror()
        let names: [(UInt8, String)] = [
            (36, "Kick"), (38, "Snare"), (42, "Closed Hat"), (46, "Open Hat"), (50, "Perc"), (35, "Bass")
        ]
        let tracks: [Track] = names.enumerated().map { idx, pair in
            var t = Track(id: SeedUUID.track(idx), midiNote: pair.0)
            var steps = Array(repeating: Step(), count: 16)
            func hit(_ i: Int, _ zone: VelocityZone) { steps[i] = Step(active: true, velocityZone: zone); }
            switch idx {
            case 0: for i in [0, 4, 8, 12] { hit(i, .accent) }            // four-on-floor
            case 1: hit(4, .accent); hit(12, .accent)                     // backbeat
            case 2: for i in stride(from: 0, to: 16, by: 2) { hit(i, .mid) }
            case 3: hit(14, .low)
            case 4: hit(7, .low); hit(15, .low)
            default: hit(0, .accent); hit(6, .mid); hit(10, .low)
            }
            t.steps = steps
            return t
        }
        var pattern = Pattern(id: SeedUUID.pattern, tracks: tracks)
        m.session.patterns[0] = pattern
        m.session.bpm = 120
        m.playheads = [0: 0, 1: 0, 2: 0, 3: 0, 4: 0, 5: 0]
        return m
    }
}

/// Deterministic UUIDs for the demo seed (avoids `UUID()` randomness in previews).
private enum SeedUUID {
    static func track(_ i: Int) -> UUID {
        UUID(uuid: (0xA0, 0x0A, UInt8(i), 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, UInt8(i + 1)))
    }
    static let pattern = UUID(uuid: (0xB0, 0x0B, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01))
}
