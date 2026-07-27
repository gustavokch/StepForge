import XCTest
@testable import StepForge

final class MIDIMarshalerTests: XCTestCase {
    func testMarshalsInOrderWithOffsets() {
        var buf = [MidiEvent](repeating: MidiEvent(), count: MIDIMarshaler.inCapacity)
        let events: [MIDIMarshaler.RawMIDI] = [
            .init(sampleOffset: 0,   status: 0x90, data1: 60, data2: 100),
            .init(sampleOffset: 120, status: 0x80, data1: 60, data2: 0),
            .init(sampleOffset: 200, status: 0xB0, data1: 7,  data2: 64),
        ]
        let n = MIDIMarshaler.marshalIn(events, into: &buf)
        XCTAssertEqual(n, 3)
        XCTAssertEqual(buf[0].sample_offset, 0)
        XCTAssertEqual(buf[0].status, 0x90)
        XCTAssertEqual(buf[0].data1, 60)
        XCTAssertEqual(buf[1].sample_offset, 120)
        XCTAssertEqual(buf[2].data1, 7)
    }

    func testDropsTailOnOverflow() {
        var buf = [MidiEvent](repeating: MidiEvent(), count: MIDIMarshaler.inCapacity)
        // Twice the capacity: only the first `inCapacity` survive (drop-tail, RT-safe).
        var events: [MIDIMarshaler.RawMIDI] = []
        for i in 0..<(MIDIMarshaler.inCapacity * 2) {
            events.append(.init(sampleOffset: UInt32(i), status: 0x90, data1: 60, data2: 1))
        }
        let n = MIDIMarshaler.marshalIn(events, into: &buf)
        XCTAssertEqual(n, MIDIMarshaler.inCapacity, "overflow drops the tail, never blocks RT")
        XCTAssertEqual(buf.first?.sample_offset, 0)
        XCTAssertEqual(buf[MIDIMarshaler.inCapacity - 1].sample_offset,
                       UInt32(MIDIMarshaler.inCapacity - 1))
    }

    func testEmptyInputWritesNothing() {
        var buf = [MidiEvent](repeating: MidiEvent(), count: MIDIMarshaler.inCapacity)
        XCTAssertEqual(MIDIMarshaler.marshalIn([], into: &buf), 0)
    }
}
