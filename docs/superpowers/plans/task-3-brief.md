### Task 3: CoreMIDI Devices Manager & Settings Sheet

**Files:**
- Create: `app/StepForge/Engine/MidiManager.swift`
- Create: `app/StepForgeTests/MidiManagerTests.swift`
- Modify: `app/StepForge/Features/Settings/SettingsSheet.swift`

**Interfaces:**
- Consumes: CoreMIDI framework (`MIDIClientCreate`, `MIDIGetNumberOfDestinations`, `MIDIGetDestination`, `MIDIObjectGetStringProperty`, `kMIDIPropertyDisplayName`).
- Produces: Observable `MidiManager` enumerating MIDI destinations and submitting `setMidiDestinations` / `setGlobalMidiChannel` commands to `EngineBridge`.

**Global Constraints:**
- Rule 7: Swift owns the `MIDIClientRef` lifecycle; integer endpoint IDs (`[UInt32]`) passed to engine via `SetMidiDestinations`.
- Kinetic Design System: Follow `Theme.swift`, `Color+Kinetic.swift`, and typography rules.
- Build Target: App must build cleanly via `cd app && xcodegen generate && xcodebuild -project StepForge.xcodeproj -scheme StepForge -sdk iphonesimulator -derivedDataPath build CODE_SIGNING_ALLOWED=NO build`.

**Step 1: Create `MidiManager.swift`**
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

**Step 2: Create `MidiManagerTests.swift`**
Create `app/StepForgeTests/MidiManagerTests.swift`:
```swift
import XCTest
@testable import StepForge

final class MidiManagerTests: XCTestCase {
    func testMidiManagerInitialization() {
        let manager = MidiManager()
        XCTAssertNotNil(manager.destinations)
    }

    func testMidiManagerToggleSubmitsDestinations() {
        let bridge = MockEngineBridge()
        let manager = MidiManager()
        
        manager.toggleDestination(1001, on: bridge)
        XCTAssertTrue(manager.selectedIDs.contains(1001))
        
        manager.toggleDestination(1001, on: bridge)
        XCTAssertFalse(manager.selectedIDs.contains(1001))
    }
}
```

**Step 3: Update `SettingsSheet.swift`**
Update `app/StepForge/Features/Settings/SettingsSheet.swift`:
- Connect to `@StateObject private var midiManager = MidiManager()`
- Render scrollable list of available MIDI destinations with toggle switches for `selectedIDs`.
- Render Global MIDI Channel picker ($1..16$, default 10 for GM Drums), submitting `bridge.submit(.setGlobalMidiChannel(channel:ch))`.
- Add "Refresh Destinations" button.

**Step 4: Verify build & tests**
Run: `cd app && xcodegen generate && xcodebuild test -scheme StepForge -sdk iphonesimulator -derivedDataPath build CODE_SIGNING_ALLOWED=NO`
Expected: `** BUILD SUCCEEDED **` and 100% test pass.

**Step 5: Commit changes**
```bash
git add app/StepForge/Engine/MidiManager.swift app/StepForge/Features/Settings/SettingsSheet.swift app/StepForgeTests/MidiManagerTests.swift
git commit -m "feat(midi): implement CoreMIDI device discovery and interactive settings sheet"
```
