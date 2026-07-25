import XCTest
@testable import StepForge

final class EngineBridgeTests: XCTestCase {
    func testMockBridgeOptimisticCommands() {
        let bridge = MockEngineBridge()
        XCTAssertEqual(bridge.mirror.bpm, 120.0)
        
        bridge.submit(.setBpm(bpm: 144.0))
        XCTAssertEqual(bridge.mirror.bpm, 144.0)
        
        bridge.submit(.setTrackLength(trackIdx: 0, length: 8))
        XCTAssertEqual(bridge.mirror.patterns[0]?.tracks[0].length, 8)
        
        bridge.submit(.setTrackSpeedRatio(trackIdx: 0, ratio: 2.0))
        XCTAssertEqual(bridge.mirror.patterns[0]?.tracks[0].speedRatio, 2.0)
        
        bridge.submit(.setTrackNote(trackIdx: 0, midiNote: 38))
        XCTAssertEqual(bridge.mirror.patterns[0]?.tracks[0].midiNote, 38)
    }

    func testSessionMirrorPlayheadApplication() {
        var mirror = SessionMirror.demoSeed
        mirror.applyPlayhead(trackIdx: 0, stepIdx: 4)
        XCTAssertEqual(mirror.playheadStep, 4)
    }

    /// Defect 4 fix: the engine now echoes Link state as `LinkEnabledChanged`,
    /// and the mirror applies it. Before, `linkEnabled` was never updated by the
    /// real engine (only by the mock), so the Settings toggle had no feedback.
    func testSessionMirrorAppliesLinkEnabledChanged() {
        var mirror = SessionMirror()
        XCTAssertEqual(mirror.linkEnabled, false)
        mirror.apply(.linkEnabledChanged(enabled: true))
        XCTAssertTrue(mirror.linkEnabled)
        mirror.apply(.linkEnabledChanged(enabled: false))
        XCTAssertFalse(mirror.linkEnabled)
    }
}
