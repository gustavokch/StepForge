### Task 1: FFI Bridge Bug Fixes & EngineBridge / SessionMirror Test Infrastructure

**Files:**
- Modify: `app/StepForge/Engine/EngineBridge.swift`
- Modify: `app/StepForge/Engine/SessionMirror.swift`
- Create: `app/StepForgeTests/EngineBridgeTests.swift`

**Interfaces:**
- Consumes: FFI functions from `sequencer_engine.h` (`engine_new`, `engine_free`, `engine_start`, `engine_stop`, `engine_submit_command`, `engine_drain_events`, `engine_free_bytes`).
- Produces: Thread-safe `EngineBridge` with bug-free mid-batch playhead resetting, safe `deinit`, and complete `SessionMirror` optimistic updates for UI testing.

**Global Constraints:**
- Rule 1: RT thread is sacred. FFI functions non-blocking.
- Rule 2: UI holds zero pointers into engine memory; value-type SessionMirror on @MainActor.
- Rule 3: Panic safety & non-blocking FFI.
- Rule 4: Buffer ownership: engine_free_bytes called exactly once.
- Rule 5: Handle lifecycle: engine_stop returns before engine_free. No concurrent engine_* calls on handle.
- Rule 6: Unsafe isolation: Swift EngineBridge isolates pointer operations on drainQueue.
- Rule 7: CoreMIDI split: Swift owns MIDIClientRef; engine receives integer endpoint IDs.
- Build Target: App must build cleanly via `cd app && xcodegen generate && xcodebuild -project StepForge.xcodeproj -scheme StepForge -sdk iphonesimulator -derivedDataPath build CODE_SIGNING_ALLOWED=NO build`.

**Step 1: Write unit test reproducing mid-batch pattern switch playhead bug & deinit safety**
Create `app/StepForgeTests/EngineBridgeTests.swift`:
```swift
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
}
```

**Step 2: Run tests to verify initial failure**
Run: `cd app && xcodebuild test -scheme StepForge -sdk iphonesimulator -derivedDataPath build CODE_SIGNING_ALLOWED=NO`
Expected: FAIL (missing methods or properties in `MockEngineBridge`/`SessionMirror`).

**Step 3: Fix `EngineBridge.swift` and update `SessionMirror.swift`**
In `app/StepForge/Engine/EngineBridge.swift`:
Fix 1: Move `playheads.removeAll(keepingCapacity: true)` inside the drain while-loop when `.patternSwitched` is decoded:
```swift
if case .playhead(let t, let s) = event {
    playheads[t] = s
} else {
    if case .patternSwitched = event {
        playheads.removeAll(keepingCapacity: true)
    }
    events.append(event)
}
```

Fix 2: Wrap `deinit` handle teardown in `drainQueue.sync`:
```swift
deinit {
    drainTimer?.cancel()
    let h = handle
    let stopped = didStop
    drainQueue.sync {
        if let h, !stopped { _ = engine_stop(h) }
        if let h { engine_free(h) }
    }
}
```

In `app/StepForge/Engine/EngineBridge.swift` (`MockEngineBridge.applyOptimistic`):
Add handlers for optimistic updates: `.setTrackLength`, `.setTrackSpeedRatio`, `.setTrackNote`, `.setMidiDestinations`, `.setGlobalMidiChannel`, `.setFollowAction`, `.queuePattern`.

**Step 4: Run tests to verify all pass**
Run: `cd app && xcodebuild test -scheme StepForge -sdk iphonesimulator -derivedDataPath build CODE_SIGNING_ALLOWED=NO`
Expected: PASS.

**Step 5: Commit changes**
```bash
git add app/StepForge/Engine/EngineBridge.swift app/StepForge/Engine/SessionMirror.swift app/StepForgeTests/EngineBridgeTests.swift
git commit -m "fix(bridge): fix playhead clearing bug and deinit thread-safety in EngineBridge"
```
