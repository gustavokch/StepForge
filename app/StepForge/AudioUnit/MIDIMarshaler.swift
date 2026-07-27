import Foundation

/// Pure, allocation-free marshalling between host MIDI messages and the engine's
/// `MidiEvent` C struct. The AU glue (StepForgeAudioUnit) extracts `RawMIDI`
/// values from incoming `AURenderEvent`s; this type does only the fixed-buffer
/// translation + drop-tail overflow handling (RT-safe, Hard Rule 1).
enum MIDIMarshaler {
    /// Max incoming MIDI messages marshalled per block. Bounded → RT-safe.
    static let inCapacity = 64

    /// One 3-byte channel-voice message at a sample offset within the block.
    struct RawMIDI {
        let sampleOffset: UInt32
        let status: UInt8
        let data1: UInt8
        let data2: UInt8
    }

    /// Marshal raw messages into a fixed-size `MidiEvent` buffer, drop-tail on
    /// overflow (bounded → never blocks the RT thread). Returns the count written.
    /// `out` must have at least `inCapacity` slots.
    static func marshalIn(_ events: [RawMIDI], into out: inout [MidiEvent]) -> Int {
        var n = 0
        let cap = Swift.min(inCapacity, out.count)
        for e in events {
            if n >= cap { break }   // drop-tail
            out[n] = MidiEvent(
                sample_offset: e.sampleOffset,
                status: e.status,
                data1: e.data1,
                data2: e.data2)
            n += 1
        }
        return n
    }
}
