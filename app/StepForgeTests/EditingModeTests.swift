import XCTest
@testable import StepForge

final class EditingModeTests: XCTestCase {
    func testTrackHeaderCommandDispatches() {
        let bridge = MockEngineBridge()
        
        bridge.submit(.setTrackLength(trackIdx: 0, length: 12))
        XCTAssertEqual(bridge.mirror.patterns[0]?.tracks[0].length, 12)
        
        bridge.submit(.setTrackSpeedRatio(trackIdx: 0, ratio: 0.5))
        XCTAssertEqual(bridge.mirror.patterns[0]?.tracks[0].speedRatio, 0.5)
        
        bridge.submit(.setTrackNote(trackIdx: 0, midiNote: 42))
        XCTAssertEqual(bridge.mirror.patterns[0]?.tracks[0].midiNote, 42)
    }

    func testRollAndVaryCommandDispatches() {
        let bridge = MockEngineBridge()
        
        bridge.submit(.roll(trackIdx: 0, strength: 0.8))
        bridge.submit(.vary(trackIdx: 0, strength: 0.4))
        // Verify mock bridge processes commands without runtime panic (Hard Rule 3)
        XCTAssertNotNil(bridge.mirror.patterns[0]?.tracks[0])
    }

    func testTrackCountAutoScrollSupport() {
        let bridge = MockEngineBridge()
        XCTAssertEqual(bridge.mirror.tracks.count, 6)
        
        bridge.submit(.addTrack)
        XCTAssertEqual(bridge.mirror.tracks.count, 7)
    }
}
