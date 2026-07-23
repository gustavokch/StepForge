### Task 4: Performance View & Pattern Jamming Mechanics

**Files:**
- Modify: `app/StepForge/Features/Performance/PerformanceView.swift`
- Create: `app/StepForge/Features/Performance/PatternOptionsSheet.swift`
- Create: `app/StepForgeTests/PerformanceModeTests.swift`

**Interfaces:**
- Consumes: `SessionMirror`, `EngineBridge.submit(_:)`, `Command.queuePattern`, `Command.retriggerPattern`, `Command.setFollowAction`, `QuantizeGrain`.
- Produces: Interactive 3x3 pattern grid, loop progress indicators, pattern option popover, follow action badge & editor, Jam vs. Arrangement mode logic, simplified track rows with activity LED indicators.

**Global Constraints:**
- Rule 2: UI holds zero pointers into engine memory; value-type SessionMirror on @MainActor.
- Rule 3: Panic safety & non-blocking FFI.
- Kinetic Design System: Follow `Theme.swift`, `Color+Kinetic.swift`, and typography rules.
- Build Target: App must build cleanly via `cd app && xcodegen generate && xcodebuild -project StepForge.xcodeproj -scheme StepForge -sdk iphonesimulator -derivedDataPath build CODE_SIGNING_ALLOWED=NO build`.

**Step 1: Write failing unit tests for Performance Mode**
Create `app/StepForgeTests/PerformanceModeTests.swift`:
```swift
import XCTest
@testable import StepForge

final class PerformanceModeTests: XCTestCase {
    func testQueuePatternDispatchesCommand() {
        let bridge = MockEngineBridge()
        
        bridge.submit(.queuePattern(index: 2, quantize: .nextBar))
        XCTAssertEqual(bridge.mirror.queuedPatternIndex, 2)
        
        bridge.submit(.cancelQueuedPattern)
        XCTAssertNil(bridge.mirror.queuedPatternIndex)
    }

    func testFollowActionSettingDispatches() {
        let bridge = MockEngineBridge()
        let action = FollowAction(afterLoops: 4, action: .playNext)
        
        bridge.submit(.setFollowAction(patternIdx: 0, action: action))
        XCTAssertEqual(bridge.mirror.patterns[0]?.followAction.afterLoops, 4)
        XCTAssertEqual(bridge.mirror.patterns[0]?.followAction.action, .playNext)
    }
}
```

**Step 2: Run test to verify it compiles and runs**
Run: `cd app && xcodebuild test -scheme StepForge -sdk iphonesimulator -derivedDataPath build CODE_SIGNING_ALLOWED=NO`

**Step 3: Create `PatternOptionsSheet.swift`**
Create `app/StepForge/Features/Performance/PatternOptionsSheet.swift`:
```swift
import SwiftUI

struct PatternOptionsSheet: View {
    let patternIdx: Int
    let currentFollowAction: FollowAction
    let onSaveFollowAction: (FollowAction) -> Void
    @Environment(\.dismiss) private var dismiss

    @State private var afterLoops: Int
    @State private var actionType: FollowActionType

    init(patternIdx: Int, currentFollowAction: FollowAction, onSaveFollowAction: @escaping (FollowAction) -> Void) {
        self.patternIdx = patternIdx
        self.currentFollowAction = currentFollowAction
        self.onSaveFollowAction = onSaveFollowAction
        _afterLoops = State(initialValue: Int(currentFollowAction.afterLoops))
        _actionType = State(initialValue: currentFollowAction.action)
    }

    var body: some View {
        NavigationStack {
            Form {
                Section("Follow Action") {
                    Stepper("After Loops: \(afterLoops)", value: $afterLoops, in: 1...16)
                    
                    Picker("Action Type", selection: $actionType) {
                        Text("None").tag(FollowActionType.none)
                        Text("Play Next").tag(FollowActionType.playNext)
                        Text("Play Previous").tag(FollowActionType.playPrevious)
                        Text("Stop").tag(FollowActionType.stop)
                        Text("Play Random").tag(FollowActionType.playRandom)
                    }
                }
            }
            .navigationTitle("Pattern \(patternIdx + 1) Options")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") {
                        onSaveFollowAction(FollowAction(afterLoops: UInt32(afterLoops), action: actionType))
                        dismiss()
                    }
                }
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
            }
        }
    }
}
```

**Step 4: Replace placeholder `PerformanceView.swift` with full Performance Mode UI**
Update `app/StepForge/Features/Performance/PerformanceView.swift`:
- Top bar with enlarged Play/Stop, enlarged Patterns button with loop progress ring, and enlarged Quantize Grain selector.
- 3x3 pattern grid rendering 9 pattern slots ($0..8$):
  - Visual states: Empty (dim), Filled (solid), Currently Playing (highlight + loop progress ring), Queued (pulsing).
  - Tap Filled $\rightarrow$ `bridge.submit(.queuePattern(index: idx, quantize: grain))`
  - Tap Active $\rightarrow$ `bridge.submit(.retriggerPattern(quantize: .nextBeat))`
  - Long Press Active $\rightarrow$ `bridge.submit(.retriggerPattern(quantize: .nextStep))` (1/16th note instant retrigger shortcut)
  - Long Press Filled $\rightarrow$ Presents `PatternOptionsSheet`.
- Mode Selector: Jam Mode (default grain `.nextBeat`) vs Arrangement Mode (default grain `.endOfPattern`).
- Simplified track rows showing Track Name, Mute toggle, and Activity LED flash (firing when `playheadStep` hits an active step on that track).

**Step 5: Verify build & tests**
Run: `cd app && xcodegen generate && xcodebuild test -scheme StepForge -sdk iphonesimulator -derivedDataPath build CODE_SIGNING_ALLOWED=NO`
Expected: `** BUILD SUCCEEDED **` and 100% test pass.

**Step 6: Commit changes**
```bash
git add app/StepForge/Features/Performance/ app/StepForgeTests/PerformanceModeTests.swift
git commit -m "feat(performance): implement full 3x3 pattern grid, queueing, retrigger gestures, follow action editor, and activity LEDs"
```
