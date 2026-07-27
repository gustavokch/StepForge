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

    /// End-to-end through a borrowed bridge against a real host-driven engine,
    /// exercising the AU's `fullState` set path. `engine_submit_command` only
    /// enqueues to the MPSC queue — the self-scheduled state worker applies the
    /// command asynchronously — so the test mutates to a *distinct* state first,
    /// then restores the original via `load`, polling `serialize()` to a deadline
    /// rather than asserting immediately. (An immediate assert would pass whether
    /// or not the worker ever ran: loading the *current* session back into itself
    /// is a no-op either way — the flaw in the previous version of this test.)
    /// `engine_start(raw)` arms the worker; the borrowed bridge skips
    /// `engine_start` in AU mode, so arming is the test's responsibility.
    func testSerializeLoadRoundTripViaBorrowedBridge() {
        let raw = engine_new_host_driven()!
        XCTAssertNotNil(raw, "engine_new_host_driven must return a handle")
        XCTAssertEqual(engine_start(raw).rawValue, 0, "state worker must arm (EngineResult.Ok = 0)")
        defer {
            engine_stop(raw)   // must return before free (Hard Rule 5)
            engine_free(raw)
        }

        let bridge = EngineBridge(handle: raw)   // borrowed: won't start/stop/free
        bridge.start(); defer { bridge.stop() }   // arms the drain timer only

        // S0 = the initial session bytes (default tempo).
        guard let s0 = bridge.serialize() else {
            return XCTFail("serialize must return bytes against a live handle")
        }

        // Mutate to a DISTINCT state (tempo 144 != default), then wait for the
        // worker to apply it. If the worker never ran, this times out -> fail.
        bridge.submit(.setBpm(bpm: 144.0))
        guard pollSerialize(bridge, until: { $0 != s0 }, timeout: 2.0) != nil else {
            return XCTFail("worker never applied setBpm (serialize stayed == S0)")
        }

        // Restore S0 through the AU's fullState envelope (pack -> unpack -> load).
        bridge.load(AUState.unpack(AUState.pack(s0))!)

        // The worker MUST apply LoadSession -> serialize() must return S0 again.
        // If LoadSession were dropped/ignored, serialize stays at the mutated
        // state and this times out -> fail.
        let restored = pollSerialize(bridge, until: { $0 == s0 }, timeout: 2.0)
        XCTAssertNotNil(restored,
                        "LoadSession must be applied by the worker (serialize must return S0)")
    }

    /// Poll `bridge.serialize()` every ~5ms until `predicate` matches or `timeout`
    /// elapses; returns the matching bytes or nil on timeout. Required because
    /// `engine_submit_command` only enqueues to the MPSC queue — the self-scheduled
    /// state worker applies the command asynchronously, so it is not reflected in
    /// `serialize()` until the worker drains the queue.
    private func pollSerialize(_ bridge: EngineBridge,
                               until predicate: (Data) -> Bool,
                               timeout: TimeInterval) -> Data? {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if let bytes = bridge.serialize(), predicate(bytes) { return bytes }
            Thread.sleep(forTimeInterval: 0.005)
        }
        return nil
    }
}
