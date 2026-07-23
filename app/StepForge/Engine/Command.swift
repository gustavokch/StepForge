import Foundation

/// Swift mirror of the Rust `Command` enum + postcard encoder. Variants are in
/// **exact Rust declaration order** so the postcard variant index matches
/// `engine/crates/core/src/command.rs` (verified via golden fixtures). The UI
/// encodes a `Command` to `[UInt8]` and the bridge submits it via
/// `engine_submit_command(ptr, len)` (amendment A4).
enum Command {
    // -- step / track editing --
    case setStep(trackIdx: Int, stepIdx: Int, zone: VelocityZone)          // 0
    case deleteStep(trackIdx: Int, stepIdx: Int)                           // 1
    case setRatchet(trackIdx: Int, stepIdx: Int, ratchet: Ratchet)         // 2
    case setTrackLength(trackIdx: Int, length: Int)                        // 3
    case setTrackMuted(trackIdx: Int, muted: Bool)                         // 4
    case setTrackNote(trackIdx: Int, midiNote: UInt8)                      // 5
    case setTrackSpeedRatio(trackIdx: Int, ratio: Float)                   // 6
    case setTrackSwing(trackIdx: Int, swingPct: Float)                     // 7
    case addTrack                                                          // 8
    case removeTrack                                                       // 9
    // -- algorithms / clipboard / undo --
    case roll(trackIdx: Int, strength: Float)                              // 10
    case vary(trackIdx: Int, strength: Float)                              // 11
    case cut(trackIdx: Int)                                                // 12
    case copy(trackIdx: Int)                                               // 13
    case paste(trackIdx: Int)                                              // 14
    case trash(trackIdx: Int)                                              // 15
    case undo(trackIdx: Int)                                               // 16
    // -- patterns / scheduling --
    case queuePattern(index: Int, quantize: QuantizeGrain)                 // 17
    case cancelQueuedPattern                                               // 18
    case retriggerPattern(quantize: QuantizeGrain)                         // 19
    // -- global feel --
    case setGlobalSwing(pct: Float)                                        // 20
    case setHumanize(timing: Float, velocity: Float)                       // 21
    case setBpm(bpm: Double)                                               // 22
    case setSyncSource(source: SyncSource)                                 // 23
    case setQuantizeGrain(grain: QuantizeGrain)                            // 24
    case setFollowAction(patternIdx: Int, action: FollowAction)            // 25
    case setMidiDestinations(endpoints: [UInt32])                          // 26
    case setGlobalMidiChannel(channel: UInt8)                              // 27
    // -- transport / snapshot / persistence --
    case play                                                              // 28
    case stop                                                              // 29
    case requestFullSnapshot                                               // 30
    case serialize                                                         // 31
    case loadSession(bytes: [UInt8])                                       // 32
    // -- sync (amendments A15 / E6) --
    case setLinkEnabled(enabled: Bool)                                     // 33
    case midiClockTick                                                     // 34

    /// Postcard variant index (Rust declaration order). Asserted in `PostcardTests`.
    var tag: UInt {
        switch self {
        case .setStep: 0; case .deleteStep: 1; case .setRatchet: 2; case .setTrackLength: 3
        case .setTrackMuted: 4; case .setTrackNote: 5; case .setTrackSpeedRatio: 6; case .setTrackSwing: 7
        case .addTrack: 8; case .removeTrack: 9; case .roll: 10; case .vary: 11
        case .cut: 12; case .copy: 13; case .paste: 14; case .trash: 15; case .undo: 16
        case .queuePattern: 17; case .cancelQueuedPattern: 18; case .retriggerPattern: 19
        case .setGlobalSwing: 20; case .setHumanize: 21; case .setBpm: 22; case .setSyncSource: 23
        case .setQuantizeGrain: 24; case .setFollowAction: 25; case .setMidiDestinations: 26
        case .setGlobalMidiChannel: 27; case .play: 28; case .stop: 29; case .requestFullSnapshot: 30
        case .serialize: 31; case .loadSession: 32; case .setLinkEnabled: 33; case .midiClockTick: 34
        }
    }

    /// Encode to postcard bytes (tag + fields in declared order). Matches
    /// `command_codec::encode_command` byte-for-byte.
    func encode() -> [UInt8] {
        var w = PostcardWriter()
        w.writeTag(tag)
        switch self {
        case .setStep(let t, let s, let zone):
            w.writeUInt(UInt(t)); w.writeUInt(UInt(s)); zone.encode(to: &w)
        case .deleteStep(let t, let s):
            w.writeUInt(UInt(t)); w.writeUInt(UInt(s))
        case .setRatchet(let t, let s, let ratchet):
            w.writeUInt(UInt(t)); w.writeUInt(UInt(s)); ratchet.encode(to: &w)
        case .setTrackLength(let t, let length):
            w.writeUInt(UInt(t)); w.writeUInt(UInt(length))
        case .setTrackMuted(let t, let muted):
            w.writeUInt(UInt(t)); w.writeBool(muted)
        case .setTrackNote(let t, let note):
            w.writeUInt(UInt(t)); w.writeU8(note)
        case .setTrackSpeedRatio(let t, let ratio):
            w.writeUInt(UInt(t)); w.writeF32(ratio)
        case .setTrackSwing(let t, let swing):
            w.writeUInt(UInt(t)); w.writeF32(swing)
        case .addTrack, .removeTrack, .cancelQueuedPattern:
            break
        case .roll(let t, let strength), .vary(let t, let strength):
            w.writeUInt(UInt(t)); w.writeF32(strength)
        case .cut(let t), .copy(let t), .paste(let t), .trash(let t), .undo(let t):
            w.writeUInt(UInt(t))
        case .queuePattern(let i, let q):
            w.writeUInt(UInt(i)); q.encode(to: &w)
        case .retriggerPattern(let q):
            q.encode(to: &w)
        case .setGlobalSwing(let pct):
            w.writeF32(pct)
        case .setHumanize(let timing, let velocity):
            w.writeF32(timing); w.writeF32(velocity)
        case .setBpm(let bpm):
            w.writeF64(bpm)
        case .setSyncSource(let source):
            source.encode(to: &w)
        case .setQuantizeGrain(let grain):
            grain.encode(to: &w)
        case .setFollowAction(let p, let action):
            w.writeUInt(UInt(p)); action.encode(to: &w)
        case .setMidiDestinations(let endpoints):
            w.writeUInt(UInt(endpoints.count))
            for e in endpoints { w.writeU32(e); }
        case .setGlobalMidiChannel(let channel):
            w.writeU8(channel)
        case .play, .stop, .requestFullSnapshot, .serialize, .midiClockTick:
            break
        case .loadSession(let bytes):
            w.writeBytes(bytes)
        case .setLinkEnabled(let enabled):
            w.writeBool(enabled)
        }
        return w.bytes
    }
}
