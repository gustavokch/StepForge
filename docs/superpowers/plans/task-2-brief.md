### Task 2: Editing Mode Completion (Track Controls, Action Drawer Dials, Note Picker Sheet)

**Files:**
- Create: `app/StepForge/Features/Editing/NotePickerSheet.swift`
- Create: `app/StepForgeTests/EditingModeTests.swift`
- Modify: `app/StepForge/Features/Editing/TrackHeader.swift`
- Modify: `app/StepForge/Features/Editing/ActionDrawer.swift`
- Modify: `app/StepForge/Features/Editing/FeelBar.swift`
- Modify: `app/StepForge/Features/Editing/TrackList.swift`
- Modify: `app/StepForge/Features/Editing/EditingView.swift`

**Interfaces:**
- Consumes: `EngineBridge.submit(_:)`, `SessionMirror`, `Command.setTrackLength`, `Command.setTrackSpeedRatio`, `Command.setTrackNote`, `Command.roll`, `Command.vary`.
- Produces: Complete Editing View UI controls for Track Length, Speed Ratio, GM Drum / Piano Note Picker, Roll/Vary strength sliders, Patterns button, and Track List auto-scrolling.

**Global Constraints:**
- Rule 2: UI holds zero pointers into engine memory; value-type SessionMirror on @MainActor.
- Rule 3: Panic safety & non-blocking FFI.
- Kinetic Design System: Follow `Theme.swift`, `Color+Kinetic.swift`, and typography rules.
- Build Target: App must build cleanly via `cd app && xcodegen generate && xcodebuild -project StepForge.xcodeproj -scheme StepForge -sdk iphonesimulator -derivedDataPath build CODE_SIGNING_ALLOWED=NO build`.

**Step 1: Write failing unit test for Editing Mode components**
Create `app/StepForgeTests/EditingModeTests.swift`:
```swift
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
}
```

**Step 2: Run test to verify it compiles and runs**
Run: `cd app && xcodebuild test -scheme StepForge -sdk iphonesimulator -derivedDataPath build CODE_SIGNING_ALLOWED=NO`

**Step 3: Create `NotePickerSheet.swift`**
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

**Step 4: Update `TrackHeader.swift`**
In `app/StepForge/Features/Editing/TrackHeader.swift`:
- Add `@State private var showNotePicker = false`
- Present `NotePickerSheet` when note name button is tapped, submitting `bridge.submit(.setTrackNote(trackIdx:trackIdx, midiNote:note))`.
- Add Speed Ratio menu on speed chip (options: `0.5x`, `1.0x`, `2.0x`, `3.0x`), submitting `bridge.submit(.setTrackSpeedRatio(trackIdx:trackIdx, ratio:r))`.
- Add Length Stepper/Menu ($1..16$), submitting `bridge.submit(.setTrackLength(trackIdx:trackIdx, length:l))`.

**Step 5: Update `ActionDrawer.swift`**
In `app/StepForge/Features/Editing/ActionDrawer.swift`:
- Add `@State private var rollStrength: Float = 0.6`
- Add `@State private var varyStrength: Float = 0.5`
- Render mini strength sliders before triggering Roll and Vary actions.

**Step 6: Update `FeelBar.swift`, `TrackList.swift`, and `EditingView.swift`**
- In `FeelBar.swift`: Add Patterns button triggering pattern popover callback.
- In `TrackList.swift`: Wrap track rows in `ScrollViewReader` and scroll to newly added track when track count increases.

**Step 7: Verify build & tests**
Run: `cd app && xcodegen generate && xcodebuild test -scheme StepForge -sdk iphonesimulator -derivedDataPath build CODE_SIGNING_ALLOWED=NO`
Expected: `** BUILD SUCCEEDED **` and 100% test pass.

**Step 8: Commit changes**
```bash
git add app/StepForge/Features/Editing/ app/StepForgeTests/EditingModeTests.swift
git commit -m "feat(ui): add note picker sheet, track length/speed controls, roll/vary strength dials, and auto-scroll"
```
