import Foundation

/// Swift mirror of the Rust `EngineEvent` enum + postcard decoder. Variant tags
/// match `engine/crates/core/src/event.rs` declaration order (verified via golden
/// fixtures). Decoded from the bytes returned by `engine_drain_events`. Any
/// truncation → nil (the event is dropped, never crashes — Hard Rule 3).
enum EngineEvent: Equatable {
    case stepChanged(trackIdx: Int, stepIdx: Int, step: Step)         // 0
    case trackLengthChanged(trackIdx: Int, length: Int)               // 1
    case trackMutedChanged(trackIdx: Int, muted: Bool)                // 2
    case trackAdded(trackIdx: Int, track: Track)                      // 3
    case trackRemoved(trackIdx: Int)                                  // 4
    case patternQueued(index: Int, quantize: QuantizeGrain)           // 5
    case patternSwitched(index: Int)                                  // 6
    case patternCleared(index: Int)                                   // 7
    case patternLoopCountChanged(count: UInt32)                       // 8
    case followActionChanged(patternIdx: Int, action: FollowAction)   // 9
    case playhead(trackIdx: Int, stepIdx: Int)                        // 10
    case playStateChanged(playing: Bool)                              // 11
    case bpmChanged(bpm: Double)                                      // 12
    case syncSourceChanged(source: SyncSource)                        // 13
    case undoAvailable(trackIdx: Int, available: Bool)                // 14
    case fullSnapshot(session: Session)                               // 15
    case serialized(bytes: [UInt8])                                   // 16
    case error(code: Int32, message: String)                          // 17
    case overflow(dropped: UInt32)                                    // 18
    case linkPeersChanged(count: Int)                                 // 19
    case linkEnabledChanged(enabled: Bool)                            // 20

    /// Decode postcard bytes → event. nil on truncation or unknown variant.
    static func decode(_ bytes: [UInt8]) -> EngineEvent? {
        var r = PostcardReader(bytes)
        guard let tag = r.readTag() else { return nil; }
        switch tag {
        case 0:
            guard let t = r.readUInt(), let s = r.readUInt(), let step = Step(from: &r) else { return nil; }
            return .stepChanged(trackIdx: t, stepIdx: s, step: step)
        case 1:
            guard let t = r.readUInt(), let length = r.readUInt() else { return nil; }
            return .trackLengthChanged(trackIdx: t, length: length)
        case 2:
            guard let t = r.readUInt(), let muted = r.readBool() else { return nil; }
            return .trackMutedChanged(trackIdx: t, muted: muted)
        case 3:
            guard let t = r.readUInt(), let track = Track(from: &r) else { return nil; }
            return .trackAdded(trackIdx: t, track: track)
        case 4:
            guard let t = r.readUInt() else { return nil; }
            return .trackRemoved(trackIdx: t)
        case 5:
            guard let i = r.readUInt(), let q = QuantizeGrain(from: &r) else { return nil; }
            return .patternQueued(index: i, quantize: q)
        case 6:
            guard let i = r.readUInt() else { return nil; }
            return .patternSwitched(index: i)
        case 7:
            guard let i = r.readUInt() else { return nil; }
            return .patternCleared(index: i)
        case 8:
            guard let count = r.readU32() else { return nil; }
            return .patternLoopCountChanged(count: count)
        case 9:
            guard let p = r.readUInt(), let action = FollowAction(from: &r) else { return nil; }
            return .followActionChanged(patternIdx: p, action: action)
        case 10:
            guard let t = r.readUInt(), let s = r.readUInt() else { return nil; }
            return .playhead(trackIdx: t, stepIdx: s)
        case 11:
            guard let playing = r.readBool() else { return nil; }
            return .playStateChanged(playing: playing)
        case 12:
            guard let bpm = r.readF64() else { return nil; }
            return .bpmChanged(bpm: bpm)
        case 13:
            guard let source = SyncSource(from: &r) else { return nil; }
            return .syncSourceChanged(source: source)
        case 14:
            guard let t = r.readUInt(), let available = r.readBool() else { return nil; }
            return .undoAvailable(trackIdx: t, available: available)
        case 15:
            guard let session = Session(from: &r) else { return nil; }
            return .fullSnapshot(session: session)
        case 16:
            guard let bytes = r.readBytes() else { return nil; }
            return .serialized(bytes: bytes)
        case 17:
            guard let code = r.readI32(), let message = r.readString() else { return nil; }
            return .error(code: code, message: message)
        case 18:
            guard let dropped = r.readU32() else { return nil; }
            return .overflow(dropped: dropped)
        case 19:
            guard let count = r.readUInt() else { return nil; }
            return .linkPeersChanged(count: count)
        case 20:
            guard let enabled = r.readBool() else { return nil; }
            return .linkEnabledChanged(enabled: enabled)
        default:
            print("[EngineEvent] decode error: unknown variant \(tag)")
            return nil   // unknown variant — future-proof skip
        }
    }
}
