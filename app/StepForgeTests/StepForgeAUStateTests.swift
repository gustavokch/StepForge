import XCTest
@testable import StepForge

/// Task 7: validates the AU's `fullState` / `fullStateForDocument` envelope.
/// `AUState` is pure-Swift and lives in the iOS app target (no `#if os(macOS)`,
/// not excluded in `project.yml`), so the iOS test target can reach it via its
/// dependency on `StepForge` — even though the AU accessors themselves live
/// behind the macOS guard on `StepForgeAudioUnit`.
final class StepForgeAUStateTests: XCTestCase {

    // MARK: - Pure pack/unpack (no engine, no host)

    func testPackUnpackRoundTripsSessionBytes() {
        let bytes = Data([0x01, 0x02, 0x03, 0xFF, 0x10])
        let dict = AUState.pack(bytes)
        XCTAssertEqual(dict[AUState.sessionKey] as? Data, bytes)

        let recovered = AUState.unpack(dict)
        XCTAssertEqual(recovered, bytes, "pack→unpack must round-trip the session bytes")
    }

    func testUnpackReturnsNilForMissingKey() {
        XCTAssertNil(AUState.unpack([:]))
        XCTAssertNil(AUState.unpack(["other": Data()]))
    }

    func testUnpackReturnsNilForWrongType() {
        XCTAssertNil(AUState.unpack([AUState.sessionKey: "not data"]))
    }

    /// End-to-end through a borrowed bridge against a real host-driven engine:
    /// serialize → pack → unpack → load → serialize must round-trip the SAME
    /// bytes. This is the exact path the AU's `fullState` accessor takes.
    ///
    /// `engine_start(raw)` is called on the raw handle BEFORE `bridge.start()`:
    /// the bridge is borrowed (skips `engine_start`), so without this call the
    /// state worker would never run, the `LoadSession` command would never be
    /// processed, and the final `serialize()` would return the INITIAL session
    /// (not the loaded bytes). Starting the worker makes this a real round-trip
    /// — `bridge.serialize()` after `load` must equal the bytes we packed.
    func testSerializeLoadRoundTripViaBorrowedBridge() {
        let raw = engine_new_host_driven()!
        XCTAssertNotNil(raw, "engine_new_host_driven must return a handle")
        // Arm the state worker on the raw handle so it processes LoadSession.
        // The borrowed bridge will NOT call engine_start (it skips lifecycle in
        // AU mode), so this is the test's responsibility.
        XCTAssertEqual(engine_start(raw).rawValue, 0, "state worker must arm (EngineResult.Ok = 0)")
        defer {
            engine_stop(raw)   // must return before free (Hard Rule 5)
            engine_free(raw)
        }

        let bridge = EngineBridge(handle: raw)   // borrowed: won't start/stop/free
        bridge.start(); defer { bridge.stop() }   // arms the drain timer only
        bridge.requestSnapshot()

        // Serialize the current session → pack → unpack → reload it.
        let first = bridge.serialize()
        XCTAssertNotNil(first, "serialize is worker-free; must return bytes against a live handle")

        let dict = AUState.pack(first!)
        let recovered = AUState.unpack(dict)
        XCTAssertEqual(recovered, first, "AUState pack/unpack must round-trip serialize's bytes")

        bridge.load(recovered!)

        // State worker is running → LoadSession is applied → re-serialize must
        // equal the bytes we just loaded (a genuine round-trip, not a no-op).
        let after = bridge.serialize()
        XCTAssertEqual(after, recovered,
                       "serialize after load must equal the loaded bytes (worker processed LoadSession)")
    }
}
