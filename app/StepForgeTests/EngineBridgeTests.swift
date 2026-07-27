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

    /// Issue #3: the mirror must stay self-consistent if the separate
    /// `LinkEnabledChanged` event is dropped under hot-channel overflow. Applying
    /// `SyncSourceChanged` alone must derive `linkEnabled` (Link → true, else false).
    func testSessionMirrorDerivesLinkEnabledFromSyncSourceChanged() {
        var mirror = SessionMirror()
        mirror.apply(.syncSourceChanged(source: .link))
        XCTAssertTrue(mirror.linkEnabled, "selecting Link must derive linkEnabled=true")
        mirror.apply(.syncSourceChanged(source: .free))
        XCTAssertFalse(mirror.linkEnabled, "selecting Free must derive linkEnabled=false")
    }

    /// Issue #1 refinement: `MockEngineBridge` must keep its off-main-readable
    /// sync snapshot consistent with its optimistic mirror, so the lock-protected
    /// `currentBpm` / `currentSyncSource` reflect submitted commands (parity with
    /// the production bridge, which refreshes the snapshot at the tail of each
    /// drain batch).
    func testMockBridgeRefreshesSyncSnapshotOnOptimisticEcho() {
        let bridge = MockEngineBridge()
        XCTAssertEqual(bridge.currentBpm, 120.0)
        XCTAssertEqual(bridge.currentSyncSource, .free)

        bridge.submit(.setBpm(bpm: 144.0))
        XCTAssertEqual(bridge.currentBpm, 144.0,
                       "mock snapshot must track optimistic setBpm")

        bridge.submit(.setSyncSource(source: .midiClock))
        XCTAssertEqual(bridge.currentSyncSource, .midiClock,
                       "mock snapshot must track optimistic setSyncSource")
    }

    /// Phase 1: a borrowed-handle bridge (AU mode) must NOT own the handle
    /// lifecycle. start()/stop() arm/cancel the drain timer but skip
    /// engine_start/engine_stop; deinit must NOT engine_free (the AU owns it).
    /// The borrowed handle stays valid after the bridge deinits.
    func testBorrowedBridgeDoesNotOwnLifecycle() {
        let raw = engine_new_host_driven()
        XCTAssertNotNil(raw, "engine_new_host_driven must return a handle")
        defer { engine_free(raw) }   // TEST owns it; the borrowed bridge must not free it

        var stolenByDeinit = false
        do {
            let bridge = EngineBridge(handle: raw!)
            XCTAssertTrue(bridge.hasHandle)
            bridge.start()                       // borrowed: arms timer, NO engine_start
            XCTAssertNotNil(bridge.serialize(),  // borrowed handle is usable for serialize
                            "borrowed bridge must serialize against the AU's handle")
            bridge.stop()                        // borrowed: cancels timer, NO engine_stop
            stolenByDeinit = false
            _ = stolenByDeinit                   // deinit runs here — must NOT engine_free(raw)
        }
        // raw must still be valid after the borrowed bridge deinit'd:
        let bridge2 = EngineBridge(handle: raw!)
        XCTAssertNotNil(bridge2.serialize(),
                        "borrowed deinit must not free the AU's handle")
        bridge2.stop()
    }

    /// Regression: the standalone init() path is unchanged — makeHandle() still
    /// returns engine_new() and the bridge owns lifecycle. (Existing
    /// MockEngineBridge tests cover the FFI-free path; this pins the production
    /// standalone constructor's ownership.)
    func testStandaloneInitStillOwnsLifecycle() {
        // The production init() must still construct a standalone engine. We
        // can't easily assert ownsLifecycle (private), but we assert the
        // observable contract: a standalone bridge has a handle and serializes.
        let bridge = EngineBridge()
        XCTAssertTrue(bridge.hasHandle, "standalone init still constructs engine_new()")
        XCTAssertNotNil(bridge.serialize())
        bridge.start(); bridge.stop()
    }
}
