# Issue #1 — CoreMIDI → `mirror` Data Race (Swift-only Fix)

- **Date:** 2026-07-26
- **Branch:** `feat/sync-implementation` (PR #6, landed as review follow-ups)
- **Scope:** Swift shell only (`app/StepForge/Engine/`). No Rust, no wire, no fixture change.
- **Prior art:** commit `80e757e` already lands a lock-protected snapshot. This spec
  endorses that fix as correct, adds one small DRY/test-parity refinement, and specifies
  the verification (including a ThreadSanitizer A/B that proves the race is real and the
  fix closes it).

## 1. Problem

`MidiManager.handleMidiInput(_:)` runs on CoreMIDI's callback thread (caulk/CoreMIDI —
thread ≠ main). Before `80e757e`, it read `bridge.mirror.syncSource` at three sites
(`MidiManager.swift`, handling `0xF8` / `0xFA` / `0xFC`) and `estimateBPM` read
`bridge?.mirror.bpm`.

`EngineBridge.mirror` is `@Published fileprivate(set) var mirror`, mutated **only** inside
the `DispatchQueue.main.async { … }` drain hop in `drainOnce()` — i.e. on the MainActor.

So an off-main thread reads a `@Published` value that the MainActor mutates, with no
synchronization. That is a data race. PR #6 wired `MidiManager.bind(to:)`, which made the
read path live. The race is **latent on iOS / Simulator** (no real MIDI Clock source) and
**real on macOS** against an external clock — see project memory `stepforge-sync-macos-surface`.

### Confirmed by inspection

- Every other `.mirror.` reference in the app lives in MainActor SwiftUI views; `MidiManager`
  was the **only** off-main reader. Closing its read closes the race.
- The race cannot be reproduced through TSan on the `@Published` path directly: TSan's Swift
  access-race detector does **not** flag `@Published`-wrapped property accesses at any density
  (memory `stepforge-tsan-macos`). A clean TSan run on `mirror` is meaningless; the race is
  real by inspection and is proven via a plain-property probe (§4).

## 2. The fix

### Core (commit `80e757e` — endorsed unchanged)

Add a lock-protected snapshot of the two sync-critical mirror fields to `EngineBridge`,
refreshed as the **last** statement of each drain batch (so it reflects that batch), and
expose it through unpublished accessors for off-main readers:

```swift
import os   // NOT `import OS` — the iOS Simulator SDK has no module named `OS`

private struct SyncSnapshot {
    var syncSource: SyncSource = .free
    var bpm: Double = 120.0
}
private let syncState = OSAllocatedUnfairLock(initialState: SyncSnapshot())

var currentSyncSource: SyncSource { syncState.withLock { $0.syncSource } }
var currentBpm: Double { syncState.withLock { $0.bpm } }
```

`MidiManager` reads through those accessors instead of `mirror`: `handleMidiInput` reads
`currentSyncSource` **once** into a local `syncIsClock` (one read serves the whole packet),
and `estimateBPM` reads `currentBpm`. `mirror` stays the `@Published` SwiftUI source of
truth; the snapshot is **unpublished**, so it never triggers a redraw and adds no
main-thread churn.

### Refinement (the only code change in this plan)

De-duplicate the refresh and give the mock parity:

- `EngineBridge.swift` — add a `fileprivate` helper (fileprivate — not `private` — because the
  same-file subclass `MockEngineBridge` must call it; `private` does not compile for a
  subclass, verified with `swiftc -typecheck`):

  ```swift
  /// Copy the mirror's sync fields into the lock-protected snapshot. Called on the
  /// MainActor as the tail of each drain batch, and by the mock's optimistic echo, so
  /// off-main readers (`currentSyncSource` / `currentBpm`) see the latest applied state.
  /// (Issue #1)
  fileprivate func refreshSyncSnapshot() {
      syncState.withLock { snap in
          snap.syncSource = mirror.syncSource
          snap.bpm = mirror.bpm
      }
  }
  ```

- Replace the inline `syncState.withLock { … }` block at the tail of the drain hop
  (`EngineBridge.swift`, currently lines 172-175) with `self.refreshSyncSnapshot()`.
- In `MockEngineBridge.submit` (currently lines 200-202), call `refreshSyncSnapshot()`
  after `mirror.applyOptimistic(command)` so the mock's snapshot tracks its optimistic
  mirror. (No test reads it today, but it removes a latent foot-gun and keeps the mock
  faithful to the production invariant.)

Net production behavior is identical to `80e757e`. `MidiManager.swift` is **not** touched
by this plan — the committed reads (`:61` `syncIsClock`, `:107` `currentBpm`) are the fix.

## 3. Why this synchronization strategy

| Approach | Verdict |
|---|---|
| **`OSAllocatedUnfairLock` snapshot (chosen)** | The hot MIDI path pays one `os_unfair_lock` acquire (~ns, no hop, no allocation). `mirror` stays `@Published` for SwiftUI; the snapshot is unpublished → zero redraw-frequency change. `SyncSource` is a raw-`UInt8` enum → implicitly `Sendable`; `SyncSnapshot` (SyncSource + Double) is `Sendable` → the lock is `Sendable`, so `EngineBridge`'s `@unchecked Sendable` stays justified (no new unsynchronized shared mutable state). `let` + `withLock` = interior mutability, safe to capture from any thread. Available iOS 16+ / macOS 14+ (deployment target is iOS 17 / macOS 14). Hard Rules 1 & 2 preserved. |
| Full `@MainActor` isolation of the read path | Rejected. The MIDI callback is hot and latency-sensitive; hopping to main per packet just to read a `Bool` adds latency and main-thread load. It also forces restructuring the `@unchecked Sendable` + `drainQueue.sync` model and breaks the MIDI → `submit` path (`submit` does `drainQueue.sync`, so it cannot be `@MainActor`). Large change, worse hot path. |
| Hop the read onto main (`DispatchQueue.main.sync`) | Rejected. Blocks the CoreMIDI callback on the main run loop — priority inversion / latency risk — and `main.sync` from an unknown thread is a deadlock hazard. |
| `Mutex` (Foundation) / per-field `Atomics` | `Mutex` requires iOS 18 / macOS 15 — not guaranteed by the iOS 17 / macOS 14 target. Per-field `Atomic` does not fit a raw-value `SyncSource` cleanly and offers no benefit over the unfair lock. |

### Hard-rule compliance

- **Hard Rule 1 (RT thread sacred):** untouched. The lock is Swift-side only; the Rust RT
  path never crosses FFI here, never locks, never allocates.
- **Hard Rule 2 (UI holds no long-lived pointer into engine state):** preserved. The UI still
  reads only the value-type `SessionMirror` on MainActor. The snapshot is a copy of two scalar
  fields held inside `EngineBridge` for off-main readers; it is not engine memory.
- **No cross-layer symmetry obligation:** no `Command` / `EngineEvent` variant is added, so the
  "symmetric, no orphans" working agreement needs no Rust counterpart. The lock adds no wire
  surface.

## 4. Verification

### 4.1 Build + unit tests (iOS Simulator)

```bash
cd app && xcodegen generate
xcodebuild -project app/StepForge.xcodeproj -scheme StepForge \
  -destination 'platform=iOS Simulator' CODE_SIGNING_ALLOWED=NO build
xcodebuild -project app/StepForge.xcodeproj -scheme StepForge \
  -destination 'platform=iOS Simulator' CODE_SIGNING_ALLOWED=NO test
```

Existing `EngineBridgeTests` / `MidiManagerTests` read `mirror` directly (not the snapshot),
so they must stay green. This step is also the **import-safety gate** (memory
`stepforge-swift-os-module`): a real iOS-sim `build` is conclusive that `import os` resolves;
never trust a macOS `-parse` exit-0.

### 4.2 ThreadSanitizer A/B on `StepForge-macOS` (the only place the race is real)

TSan cannot see the `@Published` race, so the evidence is a **plain-property probe** that
reproduces the exact access pattern (main writes, off-main reads) on an unprotected stored
property. The probe is temporary instrumentation, reverted before merge.

**Setup (temporary):**

1. Add `private var probeBpm: Double = 120.0` (plain, **not** `@Published`) to `EngineBridge`,
   and write it in the drain hop: `self.probeBpm = self.mirror.bpm`.
2. Add a temp `bridge.submit(.setSyncSource(source: .midiClock))` in `RootView.onAppear`
   (after `midiManager.bind(to:)`) to enter MIDI-Clock mode without UI.

**Build & drive:**

```bash
xcodebuild -project app/StepForge.xcodeproj -scheme StepForge-macOS \
  -configuration Debug -destination 'platform=macOS' \
  -enableThreadSanitizer YES CODE_SIGNING_ALLOWED=NO build
# launch the built binary with TSAN_OPTIONS="halt_on_error=0:history_size=9", capture stderr
# drive with a CoreMIDI virtual-source CLI: MIDISourceCreate + MIDIReceived,
#   setbuf(stdout,nil); create the source BEFORE launching the app (MidiManager connects
#   once at init, no dynamic notification); pump dense, drifting 0xF8. TCC does not block.
```

**PRE-FIX leg:** make `estimateBPM` read `bridge?.probeBpm` (the unprotected probe). **Expect
exactly one** `data race` report: `EngineBridge.probeBpm.setter` on the main thread
(`drainOnce` main-hop) vs `probeBpm.getter` on the CoreMIDI thread
(`MidiManager.estimateBPM ← handleMidiInput`).

**POST-FIX leg:** restore `estimateBPM` to `bridge?.currentBpm` (the lock). **Expect zero**
races.

**Why this validates the production fix:** the probe access is structurally identical to the
`mirror.bpm` access (main writes via the drain hop, off-main reads via `estimateBPM`); the
only difference is `@Published`, which is precisely what TSan cannot see through. PRE=1 /
POST=0 on the probe therefore demonstrates (a) the access pattern is racy, and (b) the lock
resolves it — transferring the conclusion to the `mirror` path that TSan is blind to.

**Teardown:** revert the probe (`probeBpm` + its drain-hop write + the `estimateBPM` read
change) and the `RootView.onAppear` temp before merge. The final diff is the §2 refinement
only (production code already matches `80e757e`).

## 5. Out of scope

- Any change to `MidiManager.swift` beyond the committed `:61` / `:107` reads.
- Any Rust / `Command` / `EngineEvent` / serialization-format change.
- Reworking `EngineBridge` onto `@MainActor` or actor isolation generally (see §3).
- Hardening the mock's general threading model (the mock is test scaffolding; only its
  snapshot parity is addressed here).
