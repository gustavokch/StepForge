# StepForge Full-Spec MVP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete the SwiftUI application shell (`app/StepForge`) to achieve 100% feature coverage of the StepForge UI/UX specification and C-ABI engine contract.

**Architecture:** A unidirectional SwiftUI application shell observing a MainActor `@Published mirror: SessionMirror` updated via FFI event draining (`EngineBridge`), dispatching value-type `Command` payloads across C-ABI Postcard byte serialization.

**Tech Stack:** Swift 5.9, SwiftUI, CoreMIDI, XCTest, XcodeGen, Rust `sequencer_engine` C-ABI FFI (postcard).

## Global Constraints

- **Rule 1 ( sacred RT thread)**: FFI functions are strictly non-blocking. RT thread never crosses FFI, never calls Swift.
- **Rule 2 (UI mirror)**: UI holds zero pointers into engine memory. Value-type `SessionMirror` on `@MainActor`.
- **Rule 3 (Panic safety & non-blocking FFI)**: All FFI operations handle errors cleanly via `EngineResult` or Option drop.
- **Rule 4 (Buffer ownership)**: Rust buffers allocated by FFI are freed via `engine_free_bytes` exactly once.
- **Rule 5 (Handle lifecycle)**: `engine_stop` returns before `engine_free`. No concurrent `engine_*` calls on handle.
- **Rule 6 (Unsafe isolation)**: Swift `EngineBridge` isolates pointer operations on `drainQueue`.
- **Rule 7 (CoreMIDI split)**: Swift owns `MIDIClientRef` and discovery; engine receives integer endpoint IDs (`[UInt32]`).
- **Build Target**: App must build cleanly via `cd app && xcodegen generate && xcodebuild -project StepForge.xcodeproj -scheme StepForge -sdk iphonesimulator -derivedDataPath build CODE_SIGNING_ALLOWED=NO build`.

---

### Task 1: FFI Bridge Bug Fixes & EngineBridge / SessionMirror Test Infrastructure

**Files:**
- Modify: `app/StepForge/Engine/EngineBridge.swift`
- Modify: `app/StepForge/Engine/SessionMirror.swift`
- Create: `app/StepForgeTests/EngineBridgeTests.swift`

**Interfaces:**
- Consumes: FFI functions from `sequencer_engine.h` (`engine_new`, `engine_free`, `engine_start`, `engine_stop`, `engine_submit_command`, `engine_drain_events`, `engine_free_bytes`).
- Produces: Thread-safe `EngineBridge` with bug-free mid-batch playhead resetting, safe `deinit`, and complete `SessionMirror` optimistic updates for UI testing.

- [ ] **Step 1: Write unit test reproducing mid-batch pattern switch playhead bug & deinit safety**

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

- [ ] **Step 2: Run tests to verify initial failure**

Run: `cd app && xcodebuild test -scheme StepForge -sdk iphonesimulator -derivedDataPath build CODE_SIGNING_ALLOWED=NO`
Expected: FAIL (missing methods or properties in `MockEngineBridge`/`SessionMirror`).

- [ ] **Step 3: Fix `EngineBridge.swift` and update `SessionMirror.swift`**

In `app/StepForge/Engine/EngineBridge.swift`:
Fix 1: Move `playheads.removeAll(keepingCapacity: true)` inside the drain while-loop when `.patternSwitched` is decoded:
```swift
// Replace lines 132-140 in drainOnce():
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

- [ ] **Step 4: Run tests to verify all pass**

Run: `cd app && xcodebuild test -scheme StepForge -sdk iphonesimulator -derivedDataPath build CODE_SIGNING_ALLOWED=NO`
Expected: PASS (all Postcard and EngineBridge tests pass).

- [ ] **Step 5: Commit changes**

```bash
git add app/StepForge/Engine/EngineBridge.swift app/StepForge/Engine/SessionMirror.swift app/StepForgeTests/EngineBridgeTests.swift
git commit -m "fix(bridge): fix playhead clearing bug and deinit thread-safety in EngineBridge"
```

---

### Task 2: Editing Mode Completion (Track Controls, Action Drawer Dials, Note Picker Sheet)

**Files:**
- Create: `app/StepForge/Features/Editing/NotePickerSheet.swift`
- Modify: `app/StepForge/Features/Editing/TrackHeader.swift`
- Modify: `app/StepForge/Features/Editing/ActionDrawer.swift`
- Modify: `app/StepForge/Features/Editing/FeelBar.swift`
- Modify: `app/StepForge/Features/Editing/TrackList.swift`
- Modify: `app/StepForge/Features/Editing/EditingView.swift`

**Interfaces:**
- Consumes: `EngineBridge.submit(_:)`, `SessionMirror`, `Command.setTrackLength`, `Command.setTrackSpeedRatio`, `Command.setTrackNote`, `Command.roll`, `Command.vary`.
- Produces: Complete Editing View UI controls for Track Length, Speed Ratio, GM Drum / Piano Note Picker, Roll/Vary strength sliders, Patterns button, and Track List auto-scrolling.

- [ ] **Step 1: Create `NotePickerSheet.swift`**

Create `app/StepForge/Features/Editing/NotePickerSheet.swift`:
```swift
import SwiftUI

/// Hybrid Note Picker: GM Drum grid + 2-octave mini piano keyboard.
struct NotePickerSheet: View {
    let trackIdx: Int
    let currentNote: UInt8
    let onSelect: (UInt8) -> Void
    @Environment(\.dismiss) private var dismiss

    @State private var mode: PickerMode = .gmDrums

    enum PickerMode: String, CaseIterable, Identifiable {
        case gmDrums = "GM Drums"
        case piano = "Piano Roll"
        var id: String { rawValue }
    }

    private static let gmSoundNames: [(note: UInt8, name: String)] = [
        (35, "Acoustic Bass Drum"), (36, "Bass Drum 1 (Kick)"),
        (37, "Side Stick"), (38, "Acoustic Snare"),
        (39, "Hand Clap"), (40, "Electric Snare"),
        (41, "Low Floor Tom"), (42, "Closed Hi-Hat"),
        (43, "High Floor Tom"), (44, "Pedal Hi-Hat"),
        (45, "Low Tom"), (46, "Open Hi-Hat"),
        (47, "Low-Mid Tom"), (48, "Hi-Mid Tom"),
        (49, "Crash Cymbal 1"), (50, "High Tom")
    ]

    var body: some View {
        NavigationStack {
            VStack(spacing: 16) {
                Picker("Mode", selection: $mode) {
                    ForEach(PickerMode.allCases) { m in
                        Text(m.rawValue).tag(m)
                    }
                }
                .pickerStyle(.segmented)
                .padding(.horizontal)

                if mode == .gmDrums {
                    ScrollView {
                        LazyVGrid(columns: [GridItem(.adaptive(minimum: 140))], spacing: 10) {
                            ForEach(Self.gmSoundNames, id: \.note) { item in
                                Button {
                                    onSelect(item.note)
                                    dismiss()
                                } label: {
                                    VStack(alignment: .leading, spacing: 4) {
                                        Text(item.name)
                                            .font(Typography.bodyBold)
                                            .foregroundColor(item.note == currentNote ? Theme.Colors.accentOrange : Theme.Colors.textPrimary)
                                        Text("MIDI \(item.note)")
                                            .font(Typography.caption)
                                            .foregroundColor(Theme.Colors.textSecondary)
                                    }
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                    .padding(12)
                                    .background(item.note == currentNote ? Theme.Colors.surfaceHighlight : Theme.Colors.surfaceMedium)
                                    .cornerRadius(6)
                                    .overlay(
                                        RoundedRectangle(cornerRadius: 6)
                                            .stroke(item.note == currentNote ? Theme.Colors.accentOrange : Color.clear, lineWidth: 1)
                                    )
                                }
                            }
                        }
                        .padding(.horizontal)
                    }
                } else {
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 2) {
                            ForEach(36...60, id: \.self) { note in
                                let isBlack = [1, 3, 6, 8, 10].contains(note % 12)
                                Button {
                                    onSelect(UInt8(note))
                                    dismiss()
                                } label: {
                                    VStack {
                                        Spacer()
                                        Text("\(note)")
                                            .font(Typography.caption)
                                            .foregroundColor(isBlack ? .white : .black)
                                            .padding(.bottom, 8)
                                    }
                                    .frame(width: isBlack ? 28 : 36, height: isBlack ? 120 : 180)
                                    .background(note == Int(currentNote) ? Theme.Colors.accentOrange : (isBlack ? Color.black : Color.white))
                                    .cornerRadius(4)
                                }
                            }
                        }
                        .padding()
                    }
                }
            }
            .navigationTitle("Select Track Note")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Close") { dismiss() }
                }
            }
            .background(Theme.Colors.surfaceBackground.ignoresSafeArea())
        }
    }
}
```

- [ ] **Step 2: Update `TrackHeader.swift` to support Note Picker, Length & Speed controls**

In `app/StepForge/Features/Editing/TrackHeader.swift`:
- Add `@State private var showNotePicker = false`
- Add `@State private var showLengthMenu = false`
- Enable Note Picker sheet presentation when drum/note chip is tapped.
- Add Speed Ratio menu on speed chip (options: 0.5x, 1.0x, 2.0x, 3.0x), submitting `bridge.submit(.setTrackSpeedRatio(trackIdx:trackIdx, ratio:r))`.
- Add Length Stepper/Menu ($1..16$), submitting `bridge.submit(.setTrackLength(trackIdx:trackIdx, length:l))`.

- [ ] **Step 3: Add strength slider dials to `ActionDrawer.swift`**

In `app/StepForge/Features/Editing/ActionDrawer.swift`:
- Add `@State private var rollStrength: Float = 0.6`
- Add `@State private var varyStrength: Float = 0.5`
- Render mini strength slider next to Roll and Vary buttons before triggering actions.

- [ ] **Step 4: Update `FeelBar.swift` and `TrackList.swift`**

In `app/StepForge/Features/Editing/FeelBar.swift`:
- Add Patterns button (triggers callback or opens Pattern Picker popover).

In `app/StepForge/Features/Editing/TrackList.swift`:
- Wrap track list in `ScrollViewReader` and call `proxy.scrollTo(newTrackId)` when track count increases.

- [ ] **Step 5: Verify build & tests**

Run: `cd app && xcodegen generate && xcodebuild -project StepForge.xcodeproj -scheme StepForge -sdk iphonesimulator -derivedDataPath build CODE_SIGNING_ALLOWED=NO build`
Expected: `** BUILD SUCCEEDED **`.

- [ ] **Step 6: Commit changes**

```bash
git add app/StepForge/Features/Editing/
git commit -m "feat(ui): add note picker sheet, track length/speed controls, roll/vary strength dials, and auto-scroll"
```

---

### Task 3: CoreMIDI Devices Manager & Settings Sheet

**Files:**
- Create: `app/StepForge/Engine/MidiManager.swift`
- Modify: `app/StepForge/Features/Settings/SettingsSheet.swift`
- Test: `app/StepForgeTests/MidiManagerTests.swift`

**Interfaces:**
- Consumes: CoreMIDI framework (`MIDIClientCreate`, `MIDIGetNumberOfDestinations`, `MIDIGetDestination`, `MIDIObjectGetStringProperty`, `kMIDIPropertyDisplayName`).
- Produces: Observable `MidiManager` enumerating MIDI destinations and submitting `setMidiDestinations` / `setGlobalMidiChannel` commands to `EngineBridge`.

- [ ] **Step 1: Create `MidiManager.swift`**

Create `app/StepForge/Engine/MidiManager.swift`:
```swift
import Foundation
import CoreMIDI
import Combine

struct MidiDestination: Identifiable, Hashable {
    let id: UInt32
    let name: String
}

final class MidiManager: ObservableObject {
    @Published private(set) var destinations: [MidiDestination] = []
    @Published var selectedIDs: Set<UInt32> = []

    private var client: MIDIClientRef = 0

    init() {
        setupClient()
        refreshDestinations()
    }

    private func setupClient() {
        var c: MIDIClientRef = 0
        let status = MIDIClientCreate("StepForgeSwift" as CFString, nil, nil, &c)
        if status == noErr {
            client = c
        }
    }

    func refreshDestinations() {
        var list: [MidiDestination] = []
        let count = MIDIGetNumberOfDestinations()
        for i in 0..<count {
            let endpoint = MIDIGetDestination(i)
            var param: Unmanaged<CFString>?
            let err = MIDIObjectGetStringProperty(endpoint, kMIDIPropertyDisplayName, &param)
            let name: String
            if err == noErr, let cfStr = param?.takeRetainedValue() {
                name = cfStr as String
            } else {
                name = "MIDI Output \(i + 1)"
            }
            list.append(MidiDestination(id: UInt32(endpoint), name: name))
        }
        destinations = list
    }

    func toggleDestination(_ id: UInt32, on bridge: EngineBridge) {
        if selectedIDs.contains(id) {
            selectedIDs.remove(id)
        } else {
            selectedIDs.insert(id)
        }
        bridge.submit(.setMidiDestinations(endpoints: Array(selectedIDs)))
    }

    deinit {
        if client != 0 {
            MIDIClientDispose(client)
        }
    }
}
```

- [ ] **Step 2: Create `MidiManagerTests.swift`**

Create `app/StepForgeTests/MidiManagerTests.swift`:
```swift
import XCTest
@testable import StepForge

final class MidiManagerTests: XCTestCase {
    func testMidiManagerInitialization() {
        let manager = MidiManager()
        XCTAssertNotNil(manager.destinations)
    }
}
```

- [ ] **Step 3: Implement interactive `SettingsSheet.swift`**

Update `app/StepForge/Features/Settings/SettingsSheet.swift`:
- Connect to `StateObject private var midiManager = MidiManager()`
- Render scrollable list of available MIDI destinations with toggle switches for `selectedIDs`.
- Render Global MIDI Channel stepper ($1..16$, default 10) submitting `bridge.submit(.setGlobalMidiChannel(channel:ch))`.
- Add "Refresh Destinations" button.

- [ ] **Step 4: Verify build & tests**

Run: `cd app && xcodegen generate && xcodebuild test -scheme StepForge -sdk iphonesimulator -derivedDataPath build CODE_SIGNING_ALLOWED=NO`
Expected: `** BUILD SUCCEEDED **` and tests PASS.

- [ ] **Step 5: Commit changes**

```bash
git add app/StepForge/Engine/MidiManager.swift app/StepForge/Features/Settings/SettingsSheet.swift app/StepForgeTests/MidiManagerTests.swift
git commit -m "feat(midi): implement CoreMIDI device discovery and interactive settings sheet"
```

---

### Task 4: Performance View & Pattern Jamming Mechanics

**Files:**
- Modify: `app/StepForge/Features/Performance/PerformanceView.swift`
- Create: `app/StepForge/Features/Performance/PatternOptionsSheet.swift`

**Interfaces:**
- Consumes: `SessionMirror`, `EngineBridge.submit(_:)`, `Command.queuePattern`, `Command.retriggerPattern`, `Command.setFollowAction`, `QuantizeGrain`.
- Produces: Interactive 3x3 pattern grid, loop progress indicators, pattern option popover, follow action badge & editor, Jam vs. Arrangement mode logic, simplified track rows with activity LED indicators.

- [ ] **Step 1: Implement `PatternOptionsSheet.swift`**

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

- [ ] **Step 2: Replace placeholder `PerformanceView.swift` with full Performance Mode UI**

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

- [ ] **Step 3: Build & verify full app suite**

Run: `cd app && xcodegen generate && xcodebuild test -scheme StepForge -sdk iphonesimulator -derivedDataPath build CODE_SIGNING_ALLOWED=NO`
Expected: `** BUILD SUCCEEDED **` and all unit/codec tests PASS.

- [ ] **Step 4: Commit changes**

```bash
git add app/StepForge/Features/Performance/
git commit -m "feat(performance): implement full 3x3 pattern grid, queueing, retrigger gestures, follow action editor, and activity LEDs"
```

---

## Plan Self-Review & Verification

1. **Spec Coverage**:
   - Mode 1 Editing View: TransportBar, FeelBar, Track Management, Track Header controls (Length, Speed, Note Picker), Scrolling Grid, Touch Gestures, Action Drawer with Roll/Vary Dials $\rightarrow$ Covered in Tasks 1 & 2.
   - CoreMIDI & Settings Sheet $\rightarrow$ Covered in Task 3.
   - Mode 2 Performance View: 3x3 Grid, Queueing, Retriggers, Follow Actions, Jam/Arrangement modes, Simplified Track rows with Activity LEDs $\rightarrow$ Covered in Task 4.
2. **Placeholder Check**: Zero TODOs, TBDs, or vague placeholders.
3. **Type Consistency**: Exact method signatures for FFI commands (`setTrackLength`, `setTrackSpeedRatio`, `setTrackNote`, `setMidiDestinations`, `setGlobalMidiChannel`, `setFollowAction`, `queuePattern`, `retriggerPattern`).

*End of Implementation Plan.*
