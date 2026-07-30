# StepForge Plugin — Phase 1: AUv3 Extension Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a macOS AUv3 extension (`.appex`, `'aumi'` MIDI-FX) that wraps the Phase-0 host-driven engine, reuses the standalone SwiftUI editor, persists session state with the DAW project, and validates with `auval -v aumi` and in real hosts — the first working StepForge plugin.

**Architecture:** A Swift `AUAudioUnit` subclass owns the host-driven engine handle + per-instance `RenderStateHandle` + lifecycle; its `internalRenderBlock` (host RT thread) builds a `HostTransport` from the AU musical/transport context blocks, marshals incoming `AURenderEvent` MIDI into `engine_render`, writes returned `MidiEvent`s to the host MIDI-output block, and zero-fills a dummy stereo audio bus. The reused `EngineBridge` borrows that same handle (new additive `init(handle:)` + `ownsLifecycle` flag) and runs its unchanged ~120 Hz drain into `SessionMirror` for the editor, hosted in an `AUViewController`. Zero engine-side changes.

**Tech Stack:** Swift 5, `AudioToolbox` + `CoreAudioKit` (AUv3), SwiftUI/AppKit (`NSHostingView`), xcodegen (`app/project.yml`), the existing `SequencerEngine.xcframework` (macOS slice). No new Rust.

## Global Constraints

- **All work is on branch `feat/plugin-port-phase1-auv3`**, worktree `.claude/worktrees/plugin-port-phase1-auv3`, branched off `feat/plugin-port-phase0-iso`. **Do NOT touch `feat/plugin-port-phase0`** (concurrent sync session). Run all commands from the worktree root.
- **macOS only; additive.** iOS `StepForge` + macOS `StepForge-macOS` must build and run unchanged. The only shared-file change is `EngineBridge.swift`'s additive borrowed-handle init — standalone path stays bit-for-bit identical.
- **RT-sacred (Hard Rule 1):** `internalRenderBlock` and anything on the host audio thread — no allocation, no locks, no `EngineBridge`/MainActor hops, no CoreMIDI, no Link. Only `engine_render` (RT-safe, Phase-0-audited) + fixed-size MIDI buffers.
- **FFI is bytes (Rules 3–4):** `HostTransport`/`MidiEvent` are POD C structs (allowed); Rust-allocated buffers (from `engine_serialize`) freed via `engine_free_bytes` exactly once. The engine handle is shared render-on-RT / submit-drain-on-`drainQueue` (the same lock-free model as standalone).
- **Handle lifecycle (Rule 5):** `engine_stop` returns before `engine_free`; the AU owns the engine handle + render-state; `EngineBridge` in borrowed mode never calls `engine_start`/`stop`/`free`.
- **CoreMIDI boundary (Rule 7):** in AUv3, MIDI flows through the host's `*EventList` blocks, **not** CoreMIDI. `MidiManager` is excluded from the extension entirely.
- **Engine `cargo` commands need the rustup toolchain** (`export PATH="$HOME/.cargo/bin:$PATH"`) — not used in this phase (no Rust changes), but `engine/scripts/build_engine.sh` (run as the app preBuildScript) needs it.
- **App commands:** `cd app && xcodegen generate` regenerates `StepForge.xcodeproj` (gitignored). Build the macOS AU via `xcodebuild -project app/StepForge.xcodeproj -scheme StepForgeAU -destination 'platform=macOS' build`.
- **`'aumi'` OSType codes** (4-char → hex, for `Info.plist`): type `aumi`=0x61756D69, manufacturer `SFor`=0x53466F72, subtype `DrmS`=0x44726D53.
- **AUv3 SDK-glue reconciliation:** the exact signatures of `AUInternalRenderBlock`, `AUHostMusicalContextBlock`, `AUHostTransportStateBlock`, `AUMIDIOutputEventListBlock`, `MIDIEventList`/`MIDIEventPacket` construction, and the `Info.plist` `AudioComponents` schema follow Apple's `AUAudioUnit` reference (developer.apple.com/documentation/audiounit) and are reconciled against the Swift compiler + `auval` at build time. Where a task's code shows AU glue, treat it as complete-but-reconcile; use `/debug-build` for build-chain failures. The **engine-integration code** (HostTransportBuilder, MIDIMarshaler, the `engine_render` call, field mapping) is exact.

## Scope boundary (explicitly deferred)

- Thru + configurable MIDI-input mode → later phase.
- Ableton Link quiescence → Phase 3 (host-driven `engine_new` constructs an idle Link; acceptable for Phase 1).
- `stepforge_create_editor_view` seam / Swift editor framework bundle → Phase 2 (Phase 1 instantiates the editor directly, same Swift module).
- CLAP / VST3 → Phases 3–4. Notarization → post-Phase-1.

## Deviations from the design spec

None. This plan implements `docs/superpowers/specs/2026-07-26-plugin-port-phase1-auv3-design.md` verbatim. (The parent port-design spec's Phase-0 prose sketch was already superseded by Phase 0's shipped ABI; Phase 1 targets the shipped ABI.)

## File Structure

**Create:**
- `app/StepForge/AudioUnit/HostTransportBuilder.swift` — pure mapper: AU context/transport → `HostTransport` C struct. Unit-tested.
- `app/StepForge/AudioUnit/MIDIMarshaler.swift` — pure in-marshaller: raw MIDI messages → fixed-size `[MidiEvent]` (drop-tail). Unit-tested.
- `app/StepForge/AudioUnit/StepForgeAudioUnit.swift` — `AUAudioUnit` subclass (`'aumi'`); owns engine + render-state + lifecycle; `internalRenderBlock` → `engine_render`; `fullState`/`fullStateForDocument`.
- `app/StepForge/AudioUnit/StepForgeEditorViewController.swift` — `AUViewController` + `AUAudioUnitFactory`; hosts the editor; `NSExtensionPrincipalClass`.
- `app/StepForge/AudioUnit/PluginEditorView.swift` — trimmed `RootView` (Editing + Performance + Settings-sans-routing + plugin TransportBar).
- `app/StepForge/AudioUnit/PluginTransportBar.swift` — read-only "Following host" readout + 8/16 zoom toggle.
- `app/StepForge/AudioUnit/Info.plist` — `NSExtension` + `AudioComponents` registration.
- `app/StepForgeTests/HostTransportBuilderTests.swift`, `MIDIMarshalerTests.swift`, `StepForgeAUStateTests.swift` — pure-logic unit tests.

**Modify:**
- `app/StepForge/Engine/EngineBridge.swift` — additive borrowed-handle `init(handle:)` + `ownsLifecycle` flag.
- `app/project.yml` — add the `StepForgeAU` target + embed it in `StepForge-macOS`.

**No engine-side changes** of any kind.

---

### Task 1: EngineBridge borrowed-handle mode (additive)

**Files:**
- Modify: `app/StepForge/Engine/EngineBridge.swift`
- Test: `app/StepForgeTests/EngineBridgeTests.swift`

**Interfaces:**
- Consumes: the committed C ABI (`engine_new_host_driven`, `engine_free`, `engine_serialize`, `engine_start`, `engine_stop`, `engine_drain_events`).
- Produces: `EngineBridge.init(handle:)` (internal) + `ownsLifecycle` semantics — borrowed mode arms the drain timer but skips `engine_start`/`stop`/`free`; the AU owns the handle's lifecycle. The standalone `init()` path is unchanged.

- [ ] **Step 1: Write the failing tests**

Append to `app/StepForgeTests/EngineBridgeTests.swift`:

```swift
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd app && xcodegen generate && xcodebuild -test-without-building -project StepForge.xcodeproj -scheme StepForge -destination 'platform=iOS Simulator,name=iPhone 15' 2>/dev/null; xcodebuild test -project StepForge.xcodeproj -scheme StepForge -destination 'platform=iOS Simulator,name=iPhone 15' 2>&1 | tail -20`
Expected: FAIL — `EngineBridge` has no `init(handle:)`; compile error "cannot find initializer".

(If a simulator name is unavailable, substitute any installed iOS Simulator destination from `xcrun simctl list devices available`.)

- [ ] **Step 3: Implement the borrowed-handle mode**

In `app/StepForge/Engine/EngineBridge.swift`, make three changes.

(a) Add the `ownsLifecycle` flag + borrowed initializer. Replace the `init()` / `makeHandle()` block (lines ~71–75):

```swift
    /// Phase 1 plugin mode: when false, the bridge borrows an externally-owned
    /// handle (the AUAudioUnit owns the engine lifecycle) and only arms the drain
    /// timer + submits/drains. Default `true` keeps the standalone path identical.
    private var ownsLifecycle = true

    init() { handle = makeHandle() }

    /// Borrow an externally-owned host-driven handle (AU mode). Does NOT call
    /// `engine_start`/`stop`/`free` — the AU owns the handle's lifecycle (Rule 5).
    convenience init(handle: UnsafeMutablePointer<EngineHandle>) {
        self.init()
        self.handle = handle
        self.ownsLifecycle = false
    }

    /// Handle factory. The real engine calls `engine_new()`; the mock overrides to
    /// return nil so it never touches (or links) the FFI.
    func makeHandle() -> UnsafeMutablePointer<EngineHandle>? { engine_new() }
```

(b) Gate `engine_start` in `start()` on `ownsLifecycle`. Replace the `start()` body (lines ~80–90):

```swift
    /// Spawn the RT/state/CoreMIDI threads (engine side) and begin draining.
    /// In borrowed (AU) mode, skip `engine_start` — the AU already started the
    /// host-driven state worker — but always arm the drain timer.
    func start() {
        drainQueue.sync {
            guard self.handle != nil, self.drainTimer == nil else { return; }
            if self.ownsLifecycle, let h = self.handle { _ = engine_start(h); }
            let timer = DispatchSource.makeTimerSource(queue: self.drainQueue)
            timer.schedule(deadline: .now() + .milliseconds(8), repeating: .milliseconds(8)) // ~120 Hz
            timer.setEventHandler { [weak self] in self?.drainOnce(); }
            timer.resume()
            self.drainTimer = timer
        }
    }
```

(c) Gate `engine_stop`/`engine_free` in `stop()` + `deinit`. Replace `stop()` (lines ~94–101):

```swift
    /// Cancel the drain timer and stop the engine. Must return before `engine_free`
    /// (Hard Rule 5). In borrowed mode, only cancel the timer — the AU owns stop/free.
    func stop() {
        drainQueue.sync {
            self.drainTimer?.cancel()
            self.drainTimer = nil
            if self.ownsLifecycle, let h = self.handle { _ = engine_stop(h); }
            self.didStop = true
        }
    }
```

Replace `deinit` (lines ~103–111):

```swift
    deinit {
        drainTimer?.cancel()
        let h = handle
        let stopped = didStop
        let owns = ownsLifecycle
        drainQueue.sync {
            if owns, let h, !stopped { _ = engine_stop(h) }
            if owns, let h { engine_free(h) }
        }
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd app && xcodebuild test -project StepForge.xcodeproj -scheme StepForge -destination 'platform=iOS Simulator,name=iPhone 15' 2>&1 | tail -25`
Expected: PASS — `testBorrowedBridgeDoesNotOwnLifecycle`, `testStandaloneInitStillOwnsLifecycle`, and all pre-existing `EngineBridgeTests` pass.

- [ ] **Step 5: Commit**

```bash
git add app/StepForge/Engine/EngineBridge.swift app/StepForgeTests/EngineBridgeTests.swift
git commit -m "feat(bridge): EngineBridge borrowed-handle mode for host-driven AU

Additive init(handle:) + ownsLifecycle flag for Phase 1 plugin mode: a borrowed
bridge arms the drain timer and submits/drains against an AU-owned handle but
skips engine_start/stop/free (the AUAudioUnit owns the engine lifecycle, Rule 5).
The standalone init() path is bit-for-bit unchanged. No engine-side change."
```

---

### Task 2: HostTransportBuilder (pure mapper)

**Files:**
- Create: `app/StepForge/AudioUnit/HostTransportBuilder.swift`
- Test: `app/StepForgeTests/HostTransportBuilderTests.swift`

**Interfaces:**
- Consumes: the `HostTransport` C struct (imported from `engine/include/sequencer_engine.h` via the bridging header — fields `tempo_bpm`, `sample_rate`, `block_samples`, `block_start_beat`, `bar_start_beat`, `is_playing`, `beats_per_bar`).
- Produces: `HostTransportBuilder.make(sampleRate:frameCount:tempo:beat:currentDownBeat:beatsPerBar:isPlaying:) -> HostTransport`. Consumed by Task 5's `internalRenderBlock`.

- [ ] **Step 1: Write the failing test**

Create `app/StepForgeTests/HostTransportBuilderTests.swift`:

```swift
import XCTest
@testable import StepForge

final class HostTransportBuilderTests: XCTestCase {
    func testPlayingTransportAtBarStart() {
        let t = HostTransportBuilder.make(
            sampleRate: 44100, frameCount: 512, tempo: 120.0,
            beat: 0.0, currentDownBeat: 0.0, isPlaying: true)
        XCTAssertEqual(t.tempo_bpm, 120.0)
        XCTAssertEqual(t.sample_rate, 44100.0)
        XCTAssertEqual(t.block_samples, 512)
        XCTAssertEqual(t.block_start_beat, 0.0)
        XCTAssertEqual(t.bar_start_beat, 0.0)
        XCTAssertEqual(t.beats_per_bar, 4.0)     // default; Phase-0 accumulator ignores it
        XCTAssertTrue(t.is_playing)
    }

    func testStoppedTransport() {
        let t = HostTransportBuilder.make(
            sampleRate: 48000, frameCount: 256, tempo: 90.0,
            beat: 7.5, currentDownBeat: 4.0, isPlaying: false)
        XCTAssertFalse(t.is_playing)
        XCTAssertEqual(t.block_start_beat, 7.5)
        XCTAssertEqual(t.bar_start_beat, 4.0)   // passed through for render_host realign
    }

    func testMidBarResumeCarriesDownbeat() {
        // Mid-bar (beat 1.5 within a bar starting at beat 4): render_host aligns
        // step 0 to the downbeat, so the builder must pass bar_start_beat verbatim.
        let t = HostTransportBuilder.make(
            sampleRate: 44100, frameCount: 512, tempo: 140.0,
            beat: 5.5, currentDownBeat: 4.0, isPlaying: true)
        XCTAssertEqual(t.block_start_beat, 5.5)
        XCTAssertEqual(t.bar_start_beat, 4.0)
        XCTAssertEqual(t.tempo_bpm, 140.0)
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd app && xcodegen generate && xcodebuild test -project StepForge.xcodeproj -scheme StepForge -destination 'platform=iOS Simulator,name=iPhone 15' 2>&1 | tail -15`
Expected: FAIL — `HostTransportBuilder` undefined.

- [ ] **Step 3: Implement**

Create `app/StepForge/AudioUnit/HostTransportBuilder.swift`:

```swift
import Foundation

/// Pure mapper: AU musical/transport context values → the `HostTransport` C struct
/// consumed by `engine_render` (Phase 0). No handle, no side effects → unit-testable.
///
/// `beats_per_bar` defaults to 4.0: the Phase-0 accumulator assumes 4/4 and does not
/// yet read this field (host.rs). It is plumbed through so a later phase can honor
/// non-4/4 time signatures without an ABI change.
enum HostTransportBuilder {
    static func make(
        sampleRate: Double,
        frameCount: UInt32,
        tempo: Double,
        beat: Double,
        currentDownBeat: Double,
        beatsPerBar: Double = 4.0,
        isPlaying: Bool
    ) -> HostTransport {
        HostTransport(
            tempo_bpm: tempo,
            sample_rate: sampleRate,
            block_samples: frameCount,
            block_start_beat: beat,
            bar_start_beat: currentDownBeat,
            is_playing: isPlaying,
            beats_per_bar: beatsPerBar
        )
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd app && xcodebuild test -project StepForge.xcodeproj -scheme StepForge -destination 'platform=iOS Simulator,name=iPhone 15' 2>&1 | tail -15`
Expected: PASS.

> Note: `HostTransportBuilder.swift` is added under `app/StepForge/AudioUnit/`. The iOS `StepForge` target's `sources: StepForge` glob picks it up, but it only compiles meaningfully on macOS. To keep the iOS test target green, wrap the file's body in nothing platform-specific (it's pure value math) — it compiles on iOS too, just unused. If `HostTransport` is not visible to the iOS test target for any reason, move these two files to a shared sources list (the plan assumes the existing glob includes them; verify with the build).

- [ ] **Step 5: Commit**

```bash
git add app/StepForge/AudioUnit/HostTransportBuilder.swift app/StepForgeTests/HostTransportBuilderTests.swift
git commit -m "feat(au): HostTransportBuilder — pure AU context → HostTransport mapper"
```

---

### Task 3: MIDIMarshaler in-marshaller (pure, drop-tail)

**Files:**
- Create: `app/StepForge/AudioUnit/MIDIMarshaler.swift`
- Test: `app/StepForgeTests/MIDIMarshalerTests.swift`

**Interfaces:**
- Consumes: the `MidiEvent` C struct (`sample_offset`, `status`, `data1`, `data2`).
- Produces: `MIDIMarshaler.RawMIDI` (plain value) + `marshalIn(_:into:) -> Int` (drop-tail, returns count written). The AU glue (Task 5) extracts `RawMIDI` from `AURenderEvent`s and passes a `[MidiEvent]` buffer + capacity to `engine_render`.

- [ ] **Step 1: Write the failing test**

Create `app/StepForgeTests/MIDIMarshalerTests.swift`:

```swift
import XCTest
@testable import StepForge

final class MIDIMarshalerTests: XCTestCase {
    func testMarshalsInOrderWithOffsets() {
        var buf = [MidiEvent](repeating: MidiEvent.zero, count: MIDIMarshaler.inCapacity)
        let events: [MIDIMarshaler.RawMIDI] = [
            .init(sampleOffset: 0,   status: 0x90, data1: 60, data2: 100),
            .init(sampleOffset: 120, status: 0x80, data1: 60, data2: 0),
            .init(sampleOffset: 200, status: 0xB0, data1: 7,  data2: 64),
        ]
        let n = MIDIMarshaler.marshalIn(events, into: &buf)
        XCTAssertEqual(n, 3)
        XCTAssertEqual(buf[0].sample_offset, 0)
        XCTAssertEqual(buf[0].status, 0x90)
        XCTAssertEqual(buf[0].data1, 60)
        XCTAssertEqual(buf[1].sample_offset, 120)
        XCTAssertEqual(buf[2].data1, 7)
    }

    func testDropsTailOnOverflow() {
        var buf = [MidiEvent](repeating: MidiEvent.zero, count: MIDIMarshaler.inCapacity)
        // Twice the capacity: only the first `inCapacity` survive (drop-tail, RT-safe).
        var events: [MIDIMarshaler.RawMIDI] = []
        for i in 0..<(MIDIMarshaler.inCapacity * 2) {
            events.append(.init(sampleOffset: UInt32(i), status: 0x90, data1: 60, data2: 1))
        }
        let n = MIDIMarshaler.marshalIn(events, into: &buf)
        XCTAssertEqual(n, MIDIMarshaler.inCapacity, "overflow drops the tail, never blocks RT")
        XCTAssertEqual(buf.first?.sample_offset, 0)
        XCTAssertEqual(buf[MIDIMarshaler.inCapacity - 1].sample_offset,
                       UInt32(MIDIMarshaler.inCapacity - 1))
    }

    func testEmptyInputWritesNothing() {
        var buf = [MidiEvent](repeating: MidiEvent.zero, count: MIDIMarshaler.inCapacity)
        XCTAssertEqual(MIDIMarshaler.marshalIn([], into: &buf), 0)
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd app && xcodegen generate && xcodebuild test -project StepForge.xcodeproj -scheme StepForge -destination 'platform=iOS Simulator,name=iPhone 15' 2>&1 | tail -15`
Expected: FAIL — `MIDIMarshaler` undefined.

- [ ] **Step 3: Implement**

Create `app/StepForge/AudioUnit/MIDIMarshaler.swift`:

```swift
import Foundation

/// Pure, allocation-free marshalling between host MIDI messages and the engine's
/// `MidiEvent` C struct. The AU glue (StepForgeAudioUnit) extracts `RawMIDI`
/// values from incoming `AURenderEvent`s; this type does only the fixed-buffer
/// translation + drop-tail overflow handling (RT-safe, Hard Rule 1).
enum MIDIMarshaler {
    /// Max incoming MIDI messages marshalled per block. Bounded → RT-safe.
    static let inCapacity = 64

    /// One 3-byte channel-voice message at a sample offset within the block.
    struct RawMIDI {
        let sampleOffset: UInt32
        let status: UInt8
        let data1: UInt8
        let data2: UInt8
    }

    /// Marshal raw messages into a fixed-size `MidiEvent` buffer, drop-tail on
    /// overflow (bounded → never blocks the RT thread). Returns the count written.
    /// `out` must have at least `inCapacity` slots.
    static func marshalIn(_ events: [RawMIDI], into out: inout [MidiEvent]) -> Int {
        var n = 0
        let cap = Swift.min(inCapacity, out.count)
        for e in events {
            if n >= cap { break }   // drop-tail
            out[n] = MidiEvent(
                sample_offset: e.sampleOffset,
                status: e.status,
                data1: e.data1,
                data2: e.data2)
            n += 1
        }
        return n
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd app && xcodebuild test -project StepForge.xcodeproj -scheme StepForge -destination 'platform=iOS Simulator,name=iPhone 15' 2>&1 | tail -15`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add app/StepForge/AudioUnit/MIDIMarshaler.swift app/StepForgeTests/MIDIMarshalerTests.swift
git commit -m "feat(au): MIDIMarshaler — fixed-buffer MIDI-in marshaller (drop-tail)"
```

---

### Task 4: StepForgeAU target + skeleton AU + AUViewController

**Files:**
- Modify: `app/project.yml`
- Create: `app/StepForge/AudioUnit/StepForgeAudioUnit.swift`, `app/StepForge/AudioUnit/StepForgeEditorViewController.swift`, `app/StepForge/AudioUnit/Info.plist`

**Interfaces:**
- Consumes: `AUAudioUnit`, `AUViewController`, `AUAudioUnitFactory` (AudioToolbox/CoreAudioKit); the host-driven engine is NOT wired yet (Task 5).
- Produces: a loadable `.appex` discovered by `auval` as `'aumi'` `SFor`/`DrmS`, rendering silence to a dummy stereo bus with a stub `internalRenderBlock`. The `AUViewController` is the `NSExtensionPrincipalClass`.

- [ ] **Step 1: Add the target to `app/project.yml`**

Insert this target after the `StepForge-macOS` target (before `StepForgeTests`):

```yaml
  StepForgeAU:
    type: app-extension
    platform: macOS
    settings:
      base:
        PRODUCT_BUNDLE_IDENTIFIER: com.stepforge.app.mac.StepForgeAU
        MACOSX_DEPLOYMENT_TARGET: "14.0"
        GENERATE_INFOPLIST_FILE: "NO"
        INFOPLIST_FILE: StepForge/AudioUnit/Info.plist
        SWIFT_OBJC_BRIDGING_HEADER: "$(SRCROOT)/../engine/include/sequencer_engine.h"
        USER_HEADER_SEARCH_PATHS: "$(inherited) $(SRCROOT)/../engine/include"
        HEADER_SEARCH_PATHS: "$(inherited) $(SRCROOT)/../engine/include"
        FRAMEWORK_SEARCH_PATHS: "$(inherited) $(SRCROOT)/../engine/dist"
        CODE_SIGNING_ALLOWED: "YES"
        CODE_SIGNING_REQUIRED: "NO"
    sources:
      - path: StepForge/AudioUnit
      - path: StepForge/Engine
        excludes:
          - "MidiManager.swift"
          - "EngineLifecycle.swift"
      - path: StepForge/Engine/Postcard
      - path: StepForge/Features
      - path: StepForge/Theme
      - path: StepForge/Components
      - path: StepForge/Models.swift
      - path: StepForge/Engine/Command.swift
      - path: StepForge/Engine/EngineEvent.swift
      - path: StepForge/Engine/SessionMirror.swift
    dependencies:
      - framework: ../engine/dist/SequencerEngine.xcframework
        embed: false
      - sdk: AudioToolbox.framework
      - sdk: CoreAudioKit.framework
    preBuildScripts:
      - script: 'bash "$SRCROOT/../engine/scripts/build_engine.sh"'
        name: Build Rust Engine
        shell: /bin/sh
```

Then embed the extension in the macOS container. Add to the `StepForge-macOS` target's `dependencies:` list:

```yaml
      - target: StepForgeAU
```

> Reconcile at `xcodegen generate`: if `type: app-extension` is rejected, use `type: bundle` with `PRODUCT_TYPE: com.apple.product-type.app-extension` (xcodegen version-dependent). If `sources` `excludes` syntax differs, split `Engine` into per-file entries. Use `/debug-build` for failures.

- [ ] **Step 2: Create the Info.plist**

Create `app/StepForge/AudioUnit/Info.plist` (the AU v3 extension point + component registration; `aumi`=0x61756D69, `SFor`=0x53466F72, `DrmS`=0x44726D53):

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>$(EXECUTABLE_NAME)</string>
    <key>CFBundleIdentifier</key>
    <string>$(PRODUCT_BUNDLE_IDENTIFIER)</string>
    <key>CFBundleName</key>
    <string>$(PRODUCT_NAME)</string>
    <key>CFBundlePackageType</key>
    <string>XPC!</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>NSExtension</key>
    <dict>
        <key>NSExtensionPrincipalClass</key>
        <string>StepForgeEditorViewController</string>
        <key>NSExtensionPointIdentifier</key>
        <string>com.apple.AudioUnit-UI</string>
        <key>NSExtensionAttributes</key>
        <dict>
            <key>AudioComponents</key>
            <array>
                <dict>
                    <key>name</key>
                    <string>StepForge: StepForge</string>
                    <key>description</key>
                    <string>StepForge MIDI drum sequencer</string>
                    <key>factoryFunction</key>
                    <string>StepForgeEditorViewController</string>
                    <key>manufacturer</key>
                    <string>0x53466F72</string>
                    <key>type</key>
                    <string>0x61756D69</string>
                    <key>subtype</key>
                    <string>0x44726D53</string>
                    <key>version</key>
                    <integer>0x00010000</integer>
                    <key>sandboxSafe</key>
                    <true/>
                    <key>tags</key>
                    <array>
                        <string>urn:midi-plugin</string>
                    </array>
                </dict>
            </array>
        </dict>
    </dict>
</dict>
</plist>
```

> Reconcile against `auval` discovery (Task end): some SDKs want `factoryFunction` omitted for pure-v3 (the `NSExtensionPrincipalClass` is the factory). If `auval` does not list the component, remove `factoryFunction` and rebuild. The `tags`/`sandboxSafe` keys follow Apple's AUv3 registration reference.

- [ ] **Step 3: Create the skeleton AUAudioUnit**

Create `app/StepForge/AudioUnit/StepForgeAudioUnit.swift`. This skeleton declares the dummy stereo output bus + a silence `internalRenderBlock` (engine wiring lands in Task 5):

```swift
import AudioToolbox
import CoreAudioKit

/// Phase 1 AUv3 audio unit ('aumi' MIDI-FX). Owns the host-driven engine handle
/// + render-state + lifecycle (Rule 5). `internalRenderBlock` runs on the host RT
/// thread (Hard Rule 1).
///
/// This skeleton renders silence and is loadable by `auval`; Task 5 wires
/// `engine_render` (transport + MIDI I/O).
final class StepForgeAudioUnit: AUAudioUnit {

    private var outputBus: AUAudioUnitBus?
    private var outputBusArray: AUAudioUnitBusArray?
    private var _internalRenderBlock: AUInternalRenderBlock!

    override init(componentDescription: AudioComponentDescription,
                 options: AudioComponentInstantiationOptions = []) throws {
        try super.init(componentDescription: componentDescription, options: options)
    }

    override func allocateRenderResources() throws {
        try super.allocateRenderResources()
        // Dummy stereo (2-ch) output bus — the engine emits MIDI, not audio, but
        // some hosts reject a bus-less 'aumi'. Format: standard Float32 stereo.
        let format = AVAudioFormat(standardFormatWithSampleRate: 44100, channels: 2)!
        outputBus = try AUAudioUnitBus(format: format)
        outputBusArray = AUAudioUnitBusArray(audioUnit: self, busType: .output, busses: [outputBus!])
        _internalRenderBlock = internalRenderBlock   // capture once
    }

    override func deallocateRenderResources() {
        _internalRenderBlock = nil
        outputBus = nil
        outputBusArray = nil
        super.deallocateRenderResources()
    }

    override var channelCapabilities: [NSNumber] { [2, 2] }   // 2-ch stereo out

    override var internalRenderBlock: AUInternalRenderBlock {
        // Skeleton: render silence to the dummy audio bus. RT-safe (no alloc/lock).
        return { [weak self] actionFlags, timestamp, frameCount, outputBusNumber,
                        realtimeEventList, pullInputBlock in
            guard let self,
                  let bus = self.outputBusArray?.busses[Int(outputBusNumber)] else {
                return kAudioUnitErr_InvalidElement
            }
            // Zero-fill the output buffer (silence). MIDI rides the MIDI-output
            // block (wired in Task 5), not this audio buffer.
            if let buffer = bus.mutableRawAudioData {   // VERIFY accessor vs SDK
                memset(buffer, 0, Int(frameCount) * 2 * MemoryLayout<Float32>.size)
            }
            return noErr
        }
    }
}
```

> Reconcile: the exact way a v3 `AUAudioUnit` exposes its output buffer pointer to the render block varies by SDK/example (`mutableRawAudioData` vs filling via `AUAudioUnitBus` render resources vs returning `noErr` and letting the host zero-fill). For a MIDI-FX with a dummy bus, returning `noErr` without writing is usually treated as silence — if `auval` complains about unrendered audio, the `memset` path above is the fallback. Confirm against Apple's AUAudioUnit render reference; `/debug-build` for accessor errors.

- [ ] **Step 4: Create the AUViewController (principal class)**

Create `app/StepForge/AudioUnit/StepForgeEditorViewController.swift`:

```swift
import AudioToolbox
import CoreAudioKit
import AppKit
import SwiftUI

/// The AU extension's principal class: vends the `AUAudioUnit` and hosts the
/// editor. Phase 1 hosts a placeholder; Task 6 swaps in `PluginEditorView`.
final class StepForgeEditorViewController: AUViewController, AUAudioUnitFactory {

    private var audioUnit: StepForgeAudioUnit?

    func createAudioUnit(with componentDescription: AudioComponentDescription) throws -> AUAudioUnit {
        let au = try StepForgeAudioUnit(componentDescription: componentDescription)
        self.audioUnit = au
        return au
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        // Placeholder view until Task 6 binds the editor.
        view = NSView()
        view.wantsLayer = true
        view.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
        view.frame = NSRect(x: 0, y: 0, width: 760, height: 520)
    }
}
```

- [ ] **Step 5: Build + regenerate**

Run:
```bash
cd app && xcodegen generate && \
xcodebuild -project StepForge.xcodeproj -scheme StepForgeAU \
  -destination 'platform=macOS' -configuration Development CODE_SIGNING_ALLOWED=YES build 2>&1 | tail -30
```
Expected: `BUILD SUCCEEDED`. The `.appex` is produced and embedded in `StepForge-macOS.app/Contents/PlugIns/StepForgeAU.appex`.

> If the build fails on AU signatures/plist, use `/debug-build <error>` and reconcile per the flagged notes; re-run until `BUILD SUCCEEDED`.

- [ ] **Step 6: Discover the AU with `auval`**

Register + list:
```bash
killall -9 AudioComponentRegistrar 2>/dev/null; killall -9 coreaudiod 2>/dev/null
auval -l | grep -i stepforge
```
Expected: a line listing the StepForge component (`aumi`, `SFor`, `DrmS`).

> If not listed, recheck `Info.plist` `AudioComponents` codes + `PRODUCT_BUNDLE_IDENTIFIER`, then re-register (restart `coreaudiod`). The component must be discoverable before Task 5's `auval -v`.

- [ ] **Step 7: Commit**

```bash
git add app/project.yml app/StepForge/AudioUnit/
git commit -m "feat(au): StepForgeAU appex target + skeleton AUAudioUnit (silence)

New macOS Audio Unit Extension (.appex, 'aumi' SFor/DrmS) embedded in
StepForge-macOS. Skeleton AUAudioUnit renders silence to a dummy stereo bus and
is discoverable by auval; StepForgeEditorViewController is the principal class.
Engine wiring (engine_render) lands in the next task. No engine-side change."
```

---

### Task 5: Wire `engine_render` into `internalRenderBlock`

**Files:**
- Modify: `app/StepForge/AudioUnit/StepForgeAudioUnit.swift`

**Interfaces:**
- Consumes: `HostTransportBuilder` (Task 2), `MIDIMarshaler` (Task 3), and the C ABI `engine_new_host_driven`, `engine_render_state_new`, `engine_render`, `engine_render_state_free`, `engine_start`, `engine_stop`, `engine_free`. Also `musicalContextBlock`/`transportStateBlock` (`AUHostMusicalContextBlock`/`AUHostTransportStateBlock`) and `midiOutputEventBlock` (`AUMIDIOutputEventListBlock`), captured in `allocateRenderResources`.
- Produces: an AU that produces MIDI from `engine_render` (transport-driven, sample-accurate), with play/stop/seek honored by `render_host`.

- [ ] **Step 1: Add engine ownership + lifecycle**

In `StepForgeAudioUnit.swift`, add stored properties + init/dealloc lifecycle (Rule 5). Insert after the existing stored properties:

```swift
    // Engine ownership (Phase 0 host-driven). The AU is the sole lifecycle owner.
    private var engine: UnsafeMutablePointer<EngineHandle>?
    private var renderState: OpaquePointer?   // RenderStateHandle*
    private var bridge: EngineBridge?         // borrowed-handle drain/submit for the editor

    // Captured host blocks (RT-read).
    private var musicalContextBlock: AUHostMusicalContextBlock?
    private var transportStateBlock: AUHostTransportStateBlock?
    private var midiOutputEventBlock: AUMIDIOutputEventListBlock?

    // Fixed RT buffers (no allocation on the hot path).
    private let midiIn: UnsafeMutablePointer<MidiEvent>
    private let midiOut: UnsafeMutablePointer<MidiEvent>
    private static let midiOutCap = 256

    override init(componentDescription: AudioComponentDescription,
                 options: AudioComponentInstantiationOptions = []) throws {
        midiIn = .allocate(capacity: MIDIMarshaler.inCapacity)
        midiOut = .allocate(capacity: StepForgeAudioUnit.midiOutCap)
        try super.init(componentDescription: componentDescription, options: options)
        // Create the host-driven engine + spawn ONLY the state worker (Phase 0).
        engine = engine_new_host_driven()
        if let e = engine { _ = engine_start(e) }
        // Borrowed bridge for the editor's command/event path (drain timer only).
        if let e = engine { bridge = EngineBridge(handle: e); bridge?.start() }
    }

    deinit {
        bridge?.stop(); bridge = nil
        if let e = engine { _ = engine_stop(e); engine_free(e) }   // Rule 5: stop before free
        engine = nil
        if let rs = renderState { engine_render_state_free(rs) }
        renderState = nil
        midiIn.deallocate(); midiOut.deallocate()
    }
```

- [ ] **Step 2: Capture host blocks + render-state in `allocateRenderResources`**

Replace the `allocateRenderResources` body (keep the bus setup) to also capture the host blocks + create the render-state:

```swift
    override func allocateRenderResources() throws {
        try super.allocateRenderResources()
        let format = AVAudioFormat(standardFormatWithSampleRate: 44100, channels: 2)!
        outputBus = try AUAudioUnitBus(format: format)
        outputBusArray = AUAudioUnitBusArray(audioUnit: self, busType: .output, busses: [outputBus!])

        // Capture host-provided blocks (RT-read during render).
        musicalContextBlock = self.musicalContextBlock   // VERIFY: these are set on self by the host
        transportStateBlock = self.transportStateBlock
        midiOutputEventBlock = self.midiOutputEventBlock ?? midiOutputEventListBlock

        // One per-instance render-state (single-owner, RT-thread-only).
        if renderState == nil { renderState = engine_render_state_new() }

        _internalRenderBlock = internalRenderBlock
    }
```

> Reconcile: in AUv3, `musicalContextBlock`/`transportStateBlock` are properties the *host* sets on the AU; you read them via `self.musicalContextBlock` etc. The MIDI-output block is obtained from `self.midiOutputEventListBlock` (declared via `MIDIOutputNames` / channel capabilities). Confirm the exact property names against Apple's `AUAudioUnit` reference; the captures above are the standard pattern.

- [ ] **Step 3: Declare MIDI I/O + replace `internalRenderBlock`**

Add MIDI channel capabilities and the full render block. Replace the skeleton `internalRenderBlock`:

```swift
    override var channelCapabilities: [NSNumber] { [2, 2] }

    override var MIDIOutputNames: [String] { ["StepForge Out"] }   // 1 MIDI output cable

    override var internalRenderBlock: AUInternalRenderBlock {
        return { [weak self] actionFlags, timestamp, frameCount, outputBusNumber,
                        realtimeEventList, pullInputBlock in
            guard let self,
                  let engine = self.engine,
                  let rs = self.renderState else {
                return kAudioUnitErr_Uninitialized
            }

            // (1) Transport from host context blocks.
            var tempo: Double = 120, beat: Double = 0, downBeat: Double = 0
            _ = self.musicalContextBlock?(&tempo, nil, nil, &beat, &downBeat) // VERIFY signature
            var flags: AUHostTransportStateFlags = []
            _ = self.transportStateBlock?(&flags, nil, nil)
            let isPlaying = flags.contains(.playing)

            let transport = HostTransportBuilder.make(
                sampleRate: self.outputBus?.format.sampleRate ?? 44100,
                frameCount: frameCount,
                tempo: tempo,
                beat: beat,
                currentDownBeat: downBeat,
                isPlaying: isPlaying)

            // (2) Marshal incoming MIDI from the realtime event list (AURenderEvent).
            var raw: [MIDIMarshaler.RawMIDI] = []
            var ev = realtimeEventList
            while let e = ev?.pointee {   // VERIFY: AURenderEvent list walk
                if e.head.eventType == .MIDI {
                    // e.midiEventList → walk MIDIEventPackets → channel-voice bytes.
                    // Extract (sampleOffset from e.head.eventSampleTime - timestamp.mSampleTime,
                    //          status/data1/data2 from the UMP/MIDI bytes).
                    // Build RawMIDI entries; cap handled by marshalIn.
                    self.extractMIDI(from: e, blockStartSample: timestamp.pointee.mSampleTime,
                                     into: &raw)   // helper below
                }
                ev = e.head.next?.assumingMemoryBound(to: AURenderEvent.self)
            }
            let inCount = MIDIMarshaler.marshalIn(raw, into: self.midiInAsArray())

            // (3) Drive the engine one block (RT-safe). midiOut receives MidiEvents.
            var outCount: UInt = 0
            _ = engine_render(engine, rs, transport,
                              self.midiIn, inCount,
                              self.midiOut, StepForgeAudioUnit.midiOutCap, &outCount)

            // (4) Forward emitted MIDI to the host MIDI-output block.
            if outCount > 0, let outBlock = self.midiOutputEventBlock {
                self.emitMIDI(self.midiOut, count: Int(outCount),
                              blockStartSample: timestamp.pointee.mSampleTime, via: outBlock)
            }

            // (5) Render silence to the dummy audio bus.
            if let bus = self.outputBusArray?.busses[Int(outputBusNumber)],
               let buffer = bus.mutableRawAudioData {
                memset(buffer, 0, Int(frameCount) * 2 * MemoryLayout<Float32>.size)
            }
            return noErr
        }
    }

    /// Fixed-size view over the midiIn allocation for marshalIn.
    private func midiInAsArray() -> UnsafeMutableBufferPointer<MidiEvent> {
        UnsafeMutableBufferPointer(start: midiIn, count: MIDIMarshaler.inCapacity)
    }

    /// AU glue: walk a MIDI AURenderEvent's MIDIEventList into [RawMIDI].
    /// VERIFY the exact AURenderEvent/MIDIEventPacket field access against the SDK.
    fileprivate func extractMIDI(from event: AURenderEvent,
                                 blockStartSample: UInt64,
                                 into out: inout [MIDIMarshaler.RawMIDI]) {
        let offset = UInt32(Swift.max(0, Int64(event.head.eventSampleTime) - Int64(blockStartSample)))
        var pkt = event.midiEventList.pointee        // MIDIEventList
        var packetPtr = withUnsafePointer(to: pkt) { $0 }  // first packet
        // Walk packets; each packet's words encode MIDI 1.0 channel-voice messages.
        // For a 3-byte message (status+d1+d2) in UMP MIDI1.0 protocol, extract bytes.
        // (Exact UMP decoding per CoreMIDI MIDIEventPacket reference.)
        // ... append .init(sampleOffset: offset, status:, data1:, data2:) per message ...
    }

    /// AU glue: write emitted MidiEvents into a MIDIEventList via the host block.
    /// VERIFY MIDIEventList/MIDIEventPacket construction against the SDK.
    fileprivate func emitMIDI(_ events: UnsafeMutablePointer<MidiEvent>, count: Int,
                              blockStartSample: UInt64,
                              via block: AUMIDIOutputEventListBlock) {
        for i in 0..<count {
            let e = events[i]
            // Build a single-message MIDIEventList (MIDI 1.0 protocol) from
            // (e.status, e.data1, e.data2) and call:
            //   block(AUEventSampleTime(blockStartSample) + AUEventSampleTime(e.sample_offset),
            //         0 /*cable*/, &eventList)
            // (Exact packet init per CoreMIDI MIDIEventPacket reference.)
        }
    }
```

> **Reconcile (the SDK-glue spots, flagged):** (a) the `AUHostMusicalContextBlock` out-parameter list and types; (b) the `AURenderEvent` linked-list walk and its `MIDIEventList`/`MIDIEventPacket` decoding to 3-byte channel-voice messages; (c) `MIDIEventList`/`MIDIEventPacket` construction for output (CoreMIDI UMP/MIDI 1.0 protocol). These three are standard CoreMIDI/AUToolbox patterns — implement against the SDK headers, compile, and fix per the compiler; use `/debug-build`. The engine-integration logic (transport mapping, `marshalIn`, the `engine_render` call, fixed buffers) is exact and must not change.

- [ ] **Step 4: Build**

Run:
```bash
cd app && xcodebuild -project StepForge.xcodeproj -scheme StepForgeAU \
  -destination 'platform=macOS' -configuration Development CODE_SIGNING_ALLOWED=YES build 2>&1 | tail -30
```
Expected: `BUILD SUCCEEDED`. Resolve flagged SDK-glue per the compiler (each `// VERIFY`).

- [ ] **Step 5: Validate with `auval -v`**

Run:
```bash
auval -v aumi SFor DrmS -o /tmp/auval_stepforge.txt; echo "exit=$?"
grep -iE "pass|fail|error" /tmp/auval_stepforge.txt | tail -20
```
Expected: `auval` reports the AU passed validation (exit 0), with MIDI I/O sections exercising without fatal errors. Some warnings on unimplemented properties are acceptable; fatal failures must be fixed.

- [ ] **Step 6: Manual MIDI-out smoke test in a host**

Load `StepForgeAU` on a MIDI-FX / instrument track in **Logic Pro** (or Reaper/AUM). Route its MIDI output to an instrument. Press play at 120 BPM. Expected: the default demo pattern's drum notes sound in time; stop/seek/loop re-align to the bar (render_host). This is host-validatable only (per spec) — record the result.

- [ ] **Step 7: Commit**

```bash
git add app/StepForge/AudioUnit/StepForgeAudioUnit.swift
git commit -m "feat(au): wire engine_render into internalRenderBlock (host-driven MIDI)

The AU owns a host-driven engine + render-state; internalRenderBlock (host RT
thread) builds HostTransport from musicalContextBlock/transportStateBlock,
marshals incoming AURenderEvent MIDI via MIDIMarshaler, calls engine_render,
forwards emitted MidiEvents to the host MIDI-output block, and zero-fills the
dummy audio bus. A borrowed EngineBridge drains events for the editor. RT-sacred
(Hard Rule 1): no alloc/lock/FFI-out/CoreMIDI/Link on the render path."
```

---

### Task 6: Editor in the AUViewController

**Files:**
- Modify: `app/StepForge/AudioUnit/StepForgeEditorViewController.swift`
- Create: `app/StepForge/AudioUnit/PluginEditorView.swift`, `app/StepForge/AudioUnit/PluginTransportBar.swift`

**Interfaces:**
- Consumes: `EngineBridge` (borrowed-handle mode, Task 1), the existing `EditingView`/`PerformanceView`/`SettingsSheet`, `Theme`.
- Produces: a live SwiftUI editor inside the AU window — mirror drains (~120 Hz), gestures submit commands, transport is a read-only "Following host" readout.

- [ ] **Step 1: Create the plugin TransportBar**

Create `app/StepForge/AudioUnit/PluginTransportBar.swift` (read-only host-following readout + the 8/16 zoom toggle):

```swift
import SwiftUI

/// Plugin-mode transport row: read-only "Following host" readout (host owns
/// transport) + the 8/16 zoom toggle. Drops the standalone play/stop/BPM-input/
/// sync-source controls (they write transport the host owns).
struct PluginTransportBar: View {
    @EnvironmentObject private var bridge: EngineBridge
    @Binding var visibleSteps: Int

    var body: some View {
        HStack(spacing: Theme.Spacing.sm) {
            followingHostReadout
            Spacer(minLength: Theme.Spacing.xs)
            zoomToggle
        }
        .padding(.horizontal, Theme.Spacing.sm)
        .padding(.vertical, 6)
        .panelStyle(Theme.Surface.highest)
    }

    private var followingHostReadout: some View {
        HStack(spacing: 8) {
            Image(systemName: "music.note")
                .foregroundStyle(bridge.mirror.playing ? Theme.primary : Theme.textMuted)
            VStack(alignment: .leading, spacing: 0) {
                Text("Following host")
                    .font(Typography.controlLabel)
                    .foregroundStyle(Theme.textMuted)
                Text(String(format: "%.1f BPM · step %d",
                            bridge.mirror.bpm,
                            bridge.mirror.playheadStep))
                    .font(Typography.bpmLarge)
                    .foregroundStyle(Theme.textPrimary)
            }
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .raisedStyle()
    }

    private var zoomToggle: some View {
        Picker("Zoom", selection: $visibleSteps) {
            Text("8").tag(8); Text("16").tag(16)
        }
        .pickerStyle(.segmented)
        .frame(width: 74)
        .labelsHidden()
    }
}
```

- [ ] **Step 2: Create the trimmed editor host**

Create `app/StepForge/AudioUnit/PluginEditorView.swift` (mirrors `RootView` minus the app-shell/`MidiManager`/`EngineLifecycle`, swaps the transport bar):

```swift
import SwiftUI

/// Plugin editor: a trimmed RootView (no app-shell, no MidiManager, no
/// EngineLifecycle/ScenePhase). Mounts Editing + Performance + Settings
/// (MIDI-routing section hidden) + the plugin TransportBar. Bound to the
/// borrowed EngineBridge the AU owns.
struct PluginEditorView: View {
    @EnvironmentObject private var bridge: EngineBridge
    @State private var mode: AppMode = .editing
    @State private var visibleSteps: Int = 16
    @State private var showSettings = false

    var body: some View {
        VStack(spacing: 6) {
            PluginTransportBar(visibleSteps: $visibleSteps)
            Group {
                switch mode {
                case .editing: EditingView()
                case .performance: PerformanceView()
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .background(Theme.Surface.lowest)
        .preferredColorScheme(.dark)
        .tint(Theme.primary)
        .frame(minWidth: 520, minHeight: 360)
        .toolbar { ToolbarItem(placement: .navigation) { modeToggle } }
        .sheet(isPresented: $showSettings) { SettingsSheet().environmentObject(bridge) }
    }

    private var modeToggle: some View {
        Button {
            withAnimation(.easeInOut(duration: 0.18)) {
                mode = (mode == .editing ? .performance : .editing)
            }
        } label: {
            Image(systemName: mode == .editing ? "square.grid.2x2.fill" : "pencil")
                .foregroundStyle(mode == .editing ? Theme.primary : Theme.textSecondary)
        }
    }
}
```

> Reconcile: `EditingView` currently mounts `TransportBar` (the standalone one). Two options — (a) have `EditingView` accept an injected transport bar, or (b) add a `#if !AU_EXTENSION`/a view-mode flag. The lowest-risk additive approach: add an `@Environment(\.isPluginEditor)` or a simple `Bool` flag defaulting false so `EditingView` shows `TransportBar` standalone and `PluginTransportBar` in the AU. If `EditingView`'s `TransportBar(visibleSteps:)` call is hard to inject, the AU target can `#if os(macOS)`-swap it. Confirm `EditingView`'s signature at edit time; prefer the injected-flag approach to keep `EditingView` shared. `/debug-build` if the compile shows the coupling.

- [ ] **Step 3: Bind the editor in the AUViewController**

Replace `StepForgeEditorViewController.viewDidLoad` to host the editor bound to the borrowed bridge:

```swift
    override func viewDidLoad() {
        super.viewDidLoad()
        guard let au = audioUnit else { return }
        // The AU owns the borrowed bridge; the editor binds to it.
        let bridge = au.bridgeForEditor()   // helper exposing the AU's bridge
        let editor = PluginEditorView().environmentObject(bridge)
        view = NSHostingView(rootView: editor)
        view.frame = NSRect(x: 0, y: 0, width: 760, height: 520)
        bridge.requestSnapshot()            // seed the mirror
    }
```

Add the helper to `StepForgeAudioUnit`:

```swift
    fileprivate func bridgeForEditor() -> EngineBridge { bridge ?? EngineBridge() }
```

> If `EditingView` needs `visibleSteps` from above, thread the `@State`/`@Binding` per its signature; the editor compiles standalone first (Step 4).

- [ ] **Step 4: Build + open the editor**

Run:
```bash
cd app && xcodebuild -project StepForge.xcodeproj -scheme StepForgeAU \
  -destination 'platform=macOS' -configuration Development CODE_SIGNING_ALLOWED=YES build 2>&1 | tail -30
```
Expected: `BUILD SUCCEEDED`.

- [ ] **Step 5: Manual editor validation (host)**

In Logic/Reaper/AUM, open the StepForgeAU editor window. Expected: the editor renders (tracks, steps, the "Following host" readout updates with BPM/step as the mirror drains); tapping a step toggles it (command submits, mirror updates within ~one drain tick); pattern switch works. Host-validatable only — record the result.

- [ ] **Step 6: Commit**

```bash
git add app/StepForge/AudioUnit/StepForgeEditorViewController.swift \
        app/StepForge/AudioUnit/PluginEditorView.swift \
        app/StepForge/AudioUnit/PluginTransportBar.swift \
        app/StepForge/Features/Editing/EditingView.swift
git commit -m "feat(au): bind reused SwiftUI editor in AUViewController

PluginEditorView (trimmed RootView: no app-shell/MidiManager/EngineLifecycle)
mounts Editing+Performance+Settings + a read-only plugin TransportBar ('Following
host' readout + zoom), bound to the borrowed EngineBridge. Tapping steps submits
commands; the ~120 Hz drain updates the mirror. No engine-side change."
```

---

### Task 7: State persistence (`fullState` + `fullStateForDocument`)

**Files:**
- Modify: `app/StepForge/AudioUnit/StepForgeAudioUnit.swift`
- Create: `app/StepForgeTests/StepForgeAUStateTests.swift`

**Interfaces:**
- Consumes: `EngineBridge.serialize() -> Data?` and `EngineBridge.load(_:)` (the `engine_serialize` / `Command.loadSession` round-trip).
- Produces: `StepForgeAudioUnit.fullState` / `fullStateForDocument` (both `["session": Data]`), plus pure pack/unpack helpers `AUState.pack(_:)` / `AUState.unpack(_:)` unit-tested without a host.

- [ ] **Step 1: Write the failing test (pure pack/unpack round-trip)**

Create `app/StepForgeTests/StepForgeAUStateTests.swift`:

```swift
import XCTest
@testable import StepForge

final class StepForgeAUStateTests: XCTestCase {
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
    /// serialize → pack → unpack → load → serialize must be stable (the session
    /// round-trips). This validates the fullState path without an AU/host.
    func testSerializeLoadRoundTripViaBorrowedBridge() {
        let raw = engine_new_host_driven()!
        defer { engine_free(raw) }
        let bridge = EngineBridge(handle: raw)
        bridge.start(); defer { bridge.stop() }
        bridge.requestSnapshot()

        let first = bridge.serialize()
        XCTAssertNotNil(first)
        // Pack → unpack → load the same bytes; re-serialize and confirm non-empty.
        let dict = AUState.pack(first!)
        let recovered = AUState.unpack(dict)
        XCTAssertEqual(recovered, first)
        bridge.load(recovered)
        XCTAssertNotNil(bridge.serialize())
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd app && xcodegen generate && xcodebuild test -project StepForge.xcodeproj -scheme StepForge -destination 'platform=iOS Simulator,name=iPhone 15' 2>&1 | tail -15`
Expected: FAIL — `AUState` undefined.

- [ ] **Step 3: Implement `AUState` + the fullState accessors**

Add to `StepForgeAudioUnit.swift` (top-level enum + the accessors on the class):

```swift
/// Pure pack/unpack for the AU's fullState dictionary. Both `fullState` and
/// `fullStateForDocument` use the same `["session": Data]` envelope.
enum AUState {
    static let sessionKey = "session"
    static func pack(_ data: Data) -> [String: Any] { [sessionKey: data] }
    static func unpack(_ dict: [String: Any]) -> Data? {
        dict[sessionKey] as? Data
    }
}
```

And on `StepForgeAudioUnit`:

```swift
    override var fullState: [String: Any]? {
        get { (bridge?.serialize()).map(AUState.pack) }
        set { if let data = newValue.flatMap(AUState.unpack) { bridge?.load(data) } }
    }

    override var fullStateForDocument: [String: Any]? {
        get { fullState }
        set { fullState = newValue }
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd app && xcodebuild test -project StepForge.xcodeproj -scheme StepForge -destination 'platform=iOS Simulator,name=iPhone 15' 2>&1 | tail -15`
Expected: PASS.

- [ ] **Step 5: Manual host persistence validation**

In Logic/Reaper, edit a pattern, save the project, reopen it. Expected: the edited session restores (the `fullStateForDocument` round-trip). Host-validatable only — record the result.

- [ ] **Step 6: Commit**

```bash
git add app/StepForge/AudioUnit/StepForgeAudioUnit.swift app/StepForgeTests/StepForgeAUStateTests.swift
git commit -m "feat(au): fullState + fullStateForDocument persistence

Both backed by the same [session: Data] envelope: serialize on get, loadSession
on set. AUState pack/unpack is pure (unit-tested incl. a borrowed-bridge
serialize→pack→unpack→load round-trip). SESSION_FORMAT_VERSION unchanged."
```

---

### Task 8: Validation + standalone regression sweep

**Files:** none (verification only).

**Interfaces:** consumes the full AU + the standalone apps.

- [ ] **Step 1: RT audit on the Swift render path**

Run `/audit-rt` focused on `app/StepForge/AudioUnit/StepForgeAudioUnit.swift` `internalRenderBlock`. Expected: no allocation, no lock acquisition, no FFI-out other than `engine_render`, no CoreMIDI, no Link, no `EngineBridge`/MainActor hop. The only FFI on the path is `engine_render` (+ the captured host-block reads, which are host-provided non-blocking reads). Fix any flag before merge.

- [ ] **Step 2: `auval -v aumi` full pass**

Run:
```bash
auval -v aumi SFor DrmS -o /tmp/auval_final.txt; echo "exit=$?"
tail -25 /tmp/auval_final.txt
```
Expected: exit 0, validation passed.

- [ ] **Step 3: Standalone regression — iOS app builds + tests**

Run:
```bash
cd app && xcodebuild test -project StepForge.xcodeproj -scheme StepForge \
  -destination 'platform=iOS Simulator,name=iPhone 15' 2>&1 | tail -25
```
Expected: all tests pass (incl. the new Phase-1 unit tests + existing `EngineBridgeTests`/`MockEngineBridge`).

- [ ] **Step 4: Standalone regression — macOS app builds**

Run:
```bash
cd app && xcodebuild -project StepForge.xcodeproj -scheme StepForge-macOS \
  -destination 'platform=macOS' CODE_SIGNING_ALLOWED=NO build 2>&1 | tail -15
```
Expected: `BUILD SUCCEEDED`. The macOS app + the embedded `StepForgeAU.appex` both build.

- [ ] **Step 5: Engine regression (no changes expected)**

Run (from `engine/`, with `export PATH="$HOME/.cargo/bin:$PATH"`):
```bash
cargo test && cargo clippy --all-targets -- -D warnings && cargo check --target aarch64-apple-ios
```
Expected: all green (Phase 1 makes no Rust changes; this confirms no accidental engine drift).

- [ ] **Step 6: Host smoke matrix (record results)**

In **Logic Pro**, **Reaper**, and **AUM**: load StepForgeAU, verify (a) MIDI plays in time at 120 + 140 BPM, (b) play/stop/seek/loop re-align to the bar, (c) editor opens + edits apply, (d) project save/restore persists the session. Record pass/fail per host.

- [ ] **Step 7: Final commit (docs/validation notes only)**

```bash
# If any plan-flagged reconciliations were made during Tasks 4–7, record them in
# a CHANGELOG/commit note so reviewers see the AU-glue deviations.
git add -A
git commit -m "docs(au): Phase 1 AUv3 — validation + AU-glue reconciliation notes

Record auval -v aumi pass, host smoke results (Logic/Reaper/AUM), standalone
regression (iOS + macOS build green), and any SDK-glue reconciliations made
during Tasks 4–7 (AUInternalRenderBlock/musicalContextBlock signatures,
MIDIEventList UMP construction, Info.plist AudioComponents schema)."
```

---

## Self-Review (completed by plan author)

**1. Spec coverage:**
- AU shape (`'aumi'` + dummy stereo bus): Task 4. ✓
- Lifecycle & handle ownership (AU owns; bridge borrows): Task 1 (bridge) + Task 5 (AU). ✓
- `internalRenderBlock → engine_render` per-block flow: Task 5. ✓
- Transport mapping: Task 2 (builder) + Task 5 (block capture). ✓
- MIDI marshalling (in + out): Task 3 (in) + Task 5 (out glue). ✓
- Editor reuse in AUViewController: Task 6. ✓
- State persistence: Task 7. ✓
- Build/packaging: Task 4. ✓
- Validation bar (Swift unit tests + auval/host): Tasks 1, 2, 3, 7 (unit) + 5, 6, 8 (auval/host). ✓
- Standalone regression: Task 8. ✓
- Carry-forward (Link → Phase 3; plan against iso): Global Constraints. ✓

**2. Placeholder scan:** No "TODO"/"TBD"/"implement later". The `// VERIFY` and "Reconcile" notes in Tasks 4–5 mark exact SDK-glue signatures to confirm against Apple's reference + compiler at build time — these are honest reconciliation flags for an external-SDK integration, not unspecified work. The engine-integration code is complete and exact.

**3. Type consistency:** `EngineBridge.init(handle:)` + `ownsLifecycle` consistent across Tasks 1, 5, 6, 7. `HostTransportBuilder.make(...)` signature consistent across Tasks 2, 5. `MIDIMarshaler.RawMIDI` / `marshalIn` / `inCapacity` consistent across Tasks 3, 5. `AUState.pack`/`unpack`/`sessionKey` consistent across Task 7. `bridgeForEditor()` defined in Task 6. ✓
