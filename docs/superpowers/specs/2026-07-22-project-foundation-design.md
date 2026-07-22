# StepForge — Project Foundation Design

- **Date:** 2026-07-22
- **Status:** Approved in brainstorming; revised after adversarial multi-reviewer pass; pending user spec review
- **Scope:** Repository structure, governing `CLAUDE.md`, and spec-amendment record. This design produces a **compiling skeleton** (contract files seeded, logic stubbed), not finished engine/UI logic.

---

## 1. Purpose

StepForge is an iOS MIDI drum sequencer built from two mature specifications:

- `docs/specs/ui-ux-spec.md` — UI/UX design (modes, grid gestures, patterns, MIDI routing, UI/engine contract).
- `docs/specs/architecture-spec.md` — Technical architecture (Rust core + Swift shell, threading, FFI, data models, logic, MIDI/sync, build, testing).

This document defines the **foundation**: repository layout, the `CLAUDE.md` that governs all future work, and the resolution of contradictions found while reviewing the two specs. The implementation plan produced next (via the `writing-plans` skill) creates a **compiling skeleton**: the cross-layer contract files (`models.rs`, `command.rs`, `event.rs` + serde) and the FFI surface (8 `extern "C"` stubs + codecs) are **seeded** so cbindgen emits a non-empty header, the `.a` links, and the Swift mirror/codecs can be written; everything else (RT loop, algorithms, SwiftUI views) is a **minimal compiling stub** deferred to engine/UI plans. Engine/UI feature logic itself is out of scope.

A spec review surfaced contradictions between the specs' stated invariants and their own code/FFI examples. Foundation-critical resolutions are in §6.1; engine-level items are logged in §6.2. An additional adversarial review (6 expert lenses + synthesis) refined the foundation; its `fix-now` items are folded into §3/§5/§6.1/§7/§8 and the new §9.

---

## 2. Product & Naming

| Artifact | Name | Rationale |
|---|---|---|
| Product / iOS app target | **StepForge** | Matches the repo directory; carries the brand. |
| Rust core crate | **`sequencer_engine`** | Generic, reusable library name retained from the spec. |
| Rust FFI crate | **`sequencer_engine_ffi`** | The `extern "C"` + CoreMIDI shim; depends on the core crate. |
| Generated static lib | `libsequencer_engine_ffi.a` | Built from the ffi crate. |
| Xcframework | `SequencerEngine.xcframework` | Output of `build_engine.sh`, at `engine/dist/`. |
| C header (single source of truth) | `engine/include/sequencer_engine.h` | cbindgen output; committed; referenced directly as the Swift bridging header (no second copy). |

---

## 3. Key Decisions

| # | Decision | Rationale |
|---|---|---|
| D1 | **Cargo workspace split** — `sequencer_engine` (core, `#![forbid(unsafe_code)]`) + `sequencer_engine_ffi` (allow unsafe). | Makes the spec's "only ffi is unsafe" rule a compiler-enforced fact; core stays host-testable. |
| D2 | **Monorepo**, `engine/` + `app/` siblings at root. | Two first-class components, one source of truth, atomic cross-layer changes. |
| D3 | **Pin toolchain via `rust-toolchain.toml`**, with the three iOS targets declared in `[toolchain].targets` (the single source of truth; `setup.sh` no longer runs `rustup target add`). | Homebrew `rustc` cannot add iOS std targets; rustup-managed toolchains can. Putting targets in the TOML auto-installs them on first cargo invocation in CI and the Xcode run-script. |
| D4 | **Pull-drain events only** — `engine_drain_events`; no `engine_set_event_callback` push path. | A push callback would make the RT thread call into Swift, violating Hard Rule 1. |
| D5 | **Byte-serialized commands AND events across the FFI.** The hot RT→Swift channel yields **one event per `engine_drain_events` call** (Swift loops until an empty/zero-length result — no framing needed); large payloads (`Serialized`/`Error`) travel on a **separate off-RT channel** that may heap-allocate and is freed via `engine_free_bytes`. | Avoids exposing data-carrying `#[repr(C)]` Rust enum layouts across the ABI; keeps the RT channel bounded and allocation-free while giving kilobyte-sized snapshots their own path. |
| D6 | **Spec amendments split** — foundation resolutions folded into CLAUDE.md + this doc; engine-level issues logged to `amendments.md`. | Keeps the foundation moving without re-opening algorithm design. |
| D7 | **xcframework never committed; generated header committed; xcframework is *linked*, not embedded.** Built to `engine/dist/SequencerEngine.xcframework`. | Static `.a` slices are linked via "Link Binary With Libraries" and dead-stripped — "Embed & Sign" is for dynamic slices and is wrong here. The header is small and lets Swift builds succeed without cargo present. |
| D8 | **CoreMIDI framework link via `#[link(name = "CoreMIDI", kind = "framework")]` on the `extern` blocks in `ffi/src/coremidi.rs`**, **and** the app target adds `CoreMIDI.framework` to "Link Binary With Libraries". | A staticlib crate-type does not embed framework link directives into the `.a`; without this the app link fails with undefined `_MIDISend` symbols. Belt-and-suspenders (annotation + Xcode step). |
| D9 | **Command queue is MPSC** (lock-free multi-producer, e.g. `crossbeam-queue::ArrayQueue` / `heapless::mpsc`). | Multiple Swift producers submit off-MainActor: UI (MainActor), Sync (Link/MIDI-Clock background threads), Persistence. `engine_submit_command` is internally synchronized; no Swift-side single-queue discipline required. |
| D10 | **Single header source of truth** — `engine/include/sequencer_engine.h`, pointed at directly by `SWIFT_OBJC_BRIDGING_HEADER` via `USER_HEADER_SEARCH_PATHS`. No second committed copy. | Prevents the app-side copy from drifting out of sync on ABI changes. |
| D11 | **Session load path = `Command::LoadSession(Vec<u8>)`** submitted via `engine_submit_command`; byte format identical to `engine_serialize` output and version-tagged. | No new FFI function; consistent with the command channel; round-trips with `engine_serialize`. |

---

## 4. Repository Structure

Files are annotated **[SEEDED]** (foundation creates real content — the cross-layer contract), **[STUB]** (foundation creates a minimal compiling stub; logic deferred), or **[DEFERRED]** (engine/UI plan creates the file/feature).

```
StepForge/
├── CLAUDE.md                                        # = §5 fenced block
├── .gitignore                                       # [SEEDED] target/, *.xcuserdata, engine/dist/, derived data
├── .gitattributes                                   # [SEEDED] (no binaries by default; LFS hook ready)
├── docs/
│   ├── specs/
│   │   ├── ui-ux-spec.md                            # provided spec
│   │   ├── architecture-spec.md                     # provided spec
│   │   └── amendments.md                            # [SEEDED] §6.2 engine-level list + §6.1 resolutions note
│   ├── superpowers/specs/
│   │   └── 2026-07-22-project-foundation-design.md  # this document
│   └── plans/                                       # implementation plans land here
├── engine/                                          # ── Cargo workspace (Rust) ──
│   ├── Cargo.toml                                   # [SEEDED] workspace: members = crates/core, crates/ffi
│   ├── Cargo.lock                                   # [SEEDED]
│   ├── rust-toolchain.toml                          # [SEEDED] channel + [toolchain].targets (3 iOS targets)
│   ├── cbindgen.toml                                # [SEEDED]
│   ├── scripts/
│   │   ├── setup.sh                                 # [SEEDED] cargo install cbindgen + verify echo (targets via TOML)
│   │   └── build_engine.sh                          # [SEEDED] 3 slices -> engine/dist/SequencerEngine.xcframework + header
│   ├── include/
│   │   └── sequencer_engine.h                       # [SEEDED, generated+committed] single source of truth
│   ├── dist/                                        # gitignored — xcframework output
│   └── crates/
│       ├── core/                                    # crate `sequencer_engine`  — #![forbid(unsafe_code)]
│       │   ├── Cargo.toml                           # [SEEDED]
│       │   ├── src/
│       │   │   ├── lib.rs                           # [SEEDED] root + #![forbid(unsafe_code)]
│       │   │   ├── models.rs                        # [SEEDED] Session/Pattern/Track/Step + enums (contract)
│       │   │   ├── command.rs                       # [SEEDED] Command enum (contract)
│       │   │   ├── event.rs                         # [SEEDED] EngineEvent enum (contract)
│       │   │   ├── serde_ext.rs                     # [SEEDED] serde derives + version tag
│       │   │   ├── engine.rs                        # [STUB] Engine struct shell
│       │   │   ├── clock.rs                         # [STUB]
│       │   │   ├── scheduler.rs                     # [STUB]
│       │   │   ├── midi.rs                          # [STUB] pure dispatch math — NO CoreMIDI
│       │   │   ├── algorithms/{mod,roll,vary}.rs    # [STUB]
│       │   │   ├── clipboard.rs                     # [STUB]
│       │   │   └── undo.rs                          # [STUB]
│       │   └── tests/                               # [DEFERRED] clock/roll/vary/scheduler/undo/clipboard
│       └── ffi/                                     # crate `sequencer_engine_ffi` — #![allow(unsafe_code)]
│           ├── Cargo.toml                           # [SEEDED] depends on sequencer_engine; crate-type=["staticlib","rlib"]
│           ├── build.rs                             # [SEEDED] cargo:rustc-link-lib=framework=CoreMIDI
│           ├── src/
│           │   ├── lib.rs                           # [SEEDED] 8 extern "C" no-op stubs (see §9) so cbindgen links
│           │   ├── handle.rs                        # [STUB] Box<Engine> into_raw/from_raw
│           │   ├── command_codec.rs                 # [SEEDED-skeleton] total bytes<->Command (Result, never panic)
│           │   ├── event_codec.rs                   # [SEEDED-skeleton] encode_into fixed buffer; total decode
│           │   └── coremidi.rs                      # [STUB] unsafe extern blocks + #[link(CoreMIDI, framework)]
│           └── tests/                               # [DEFERRED] ffi_tests.rs — C-ABI round-trips incl. garbage bytes
├── app/                                             # ── Xcode project: StepForge ──
│   ├── StepForge.xcodeproj/                         # [SEEDED] xcuserdata gitignored
│   ├── StepForge/
│   │   ├── StepForgeApp.swift                       # [SEEDED] @main (empty UI OK for foundation)
│   │   ├── Engine/                                  # [STUB] bridge skeleton
│   │   │   ├── EngineBridge.swift                   # [STUB] drain queue + scene-phase stop()
│   │   │   ├── SessionMirror.swift                  # [STUB]
│   │   │   ├── Command.swift                        # [STUB] enum + encode()
│   │   │   ├── EngineEvent.swift                    # [STUB] decode()
│   │   │   └── (bridging header = engine/include/sequencer_engine.h via USER_HEADER_SEARCH_PATHS; no local copy)
│   │   ├── Features/{Transport,Grid,Tracks,Patterns,NotePicker,MIDI}/  # [DEFERRED]
│   │   ├── Sync/                                    # [DEFERRED] AbletonLink + MIDIClock clients (Swift-owned)
│   │   ├── Haptics/                                 # [DEFERRED]
│   │   ├── Persistence/                             # [STUB] save/load wiring (engine_serialize <-> Command::LoadSession)
│   │   └── Resources/                               # [SEEDED] Assets/AppIcon placeholders
│   ├── StepForgeTests/                              # [DEFERRED]
│   └── StepForgeUITests/                            # [DEFERRED]
└── (engine/dist/SequencerEngine.xcframework)        # gitignored; linked (not embedded) into the app
```

**Shape notes**

- `core/src/midi.rs` is **pure dispatch math** (velocity mapping, ratchet counts, gate/note-off timing). All CoreMIDI `unsafe` (`MIDISend`, `MIDIPacketList`, all-notes-off) lives in `ffi/src/coremidi.rs`, so core can stay `#![forbid(unsafe_code)]`.
- The bridging header has **one** committed copy (`engine/include/sequencer_engine.h`); Xcode points `SWIFT_OBJC_BRIDGING_HEADER` at it via `USER_HEADER_SEARCH_PATHS` (D10).
- `engine/dist/` is gitignored; the xcframework it holds is **linked** into the app target, not embedded (D7).

---

## 5. Proposed `CLAUDE.md`

The fenced block below is the exact proposed content of the repo-root `CLAUDE.md`.

```markdown
# StepForge

iOS MIDI drum sequencer. Two layers with a hard boundary:
- **Rust core** (`sequencer_engine`) — all musical-time logic, state, MIDI dispatch. Compiled to a static lib / xcframework.
- **Swift shell** (app `StepForge`) — SwiftUI, gestures, Ableton Link, CoreMIDI discovery, haptics, persistence.

Full specs live in `docs/specs/`. Read `ui-ux-spec.md` + `architecture-spec.md` before touching either layer; see `amendments.md` for resolved contradictions and open issues.

## Repository layout
`engine/` Cargo workspace (`crates/core` + `crates/ffi`); `app/` Xcode project; `docs/specs`, `docs/superpowers/specs`, `docs/plans`. Xcframework is built to `engine/dist/` (gitignored) and **linked** (not embedded) into the app. The bridging header is the single committed copy at `engine/include/sequencer_engine.h`.

## Hard rules (do not violate)
1. **RT thread is sacred.** The engine's real-time thread never crosses FFI, never calls into Swift, never locks, and never allocates on the hot path. Specifically:
   - It is **self-scheduled from a Rust clock source** (`std::time::Instant` / high-res timer); it is **never** driven by a Swift, CoreMIDI, CoreAudio, or Ableton-Link callback. External timing (Link phase, inbound MIDI Clock) is received in Swift and forwarded as **Commands**.
   - The RT→Swift event channel and the RT→CoreMIDI ring use **fixed-size slots** (`[u8; MAX_EVENT_BYTES]` for events, a small fixed MIDI-message struct for the ring) — never heap types. The RT-side event encoder writes into a caller-provided fixed buffer (`encode_into(&EngineEvent, &mut [u8; MAX_EVENT_BYTES]) -> usize`); it never returns `Vec<u8>`.
   - Bounded lock-free queues **never block or spin**; on overflow the RT thread drops (the engine defines a per-channel drop policy — see `amendments.md`).
   - Workers that read state the RT thread mutates (e.g. the Serialized snapshot worker) use a **non-blocking** read (seqlock / COW publish / staged snapshot), never a mutex/rwlock the RT thread could block on.
2. **UI holds no long-lived or shared pointer into engine state.** State otherwise flows only through the command/event channel; SwiftUI reads the value-type `SessionMirror` on the MainActor. Transient byte buffers returned by `engine_drain_events` / `engine_serialize` are borrowed by Swift and **must** be released via `engine_free_bytes` immediately after decode — the one sanctioned, short-lived exception.
3. **All FFI functions are non-blocking and panic-safe.** `engine_submit_command` enqueues to a lock-free **MPSC** queue and returns (UI, Sync, and Persistence threads may all submit). `engine_drain_events` pulls. Every `extern "C"` entry wraps its body in `catch_unwind` and returns a `#[repr(C)]` status (`EngineResult`); a Rust panic never crosses the FFI. The byte codecs are **total** (decode returns `Result`, never panics). No push callback from the engine into Swift — `engine_set_event_callback` is intentionally absent.
4. **Buffer ownership.** Buffers returned by `engine_drain_events` / `engine_serialize` are Rust-allocated and freed **only** by `engine_free_bytes`, exactly once, never by Swift `free`/`deallocate()`; `engine_free_bytes(NULL, 0)` is a no-op. Command bytes passed into `engine_submit_command` are Swift-allocated and borrowed by Rust for the call only.
5. **Handle lifecycle.** `engine_stop` must be called and return before `engine_free`; `engine_free` must not run concurrently with any other `engine_*` call on the same handle. `EngineBridge` owns an explicit `stop()` invoked from scene-phase teardown (drain source cancelled, in-flight drain completed) before `engine_free` runs in `deinit`. `engine_free(NULL)` is a tolerated no-op.
6. **Unsafe isolation.** `sequencer_engine` (core) is `#![forbid(unsafe_code)]` — keep it. All `unsafe` lives in `sequencer_engine_ffi` (CoreMIDI `extern "C"`, handle pointer mgmt), reviewed line-by-line.
7. **No Rust enum layouts across the ABI.** Commands and events cross as serialized bytes; encode/decode in `ffi` (Rust) and `Engine/` (Swift). `engine_drain_events` drains **both** the hot RT→Swift channel (fixed-slot, one event per call) and the off-RT large-payload channel (`Serialized`/`Error`, may heap-allocate); Swift loops until an empty/zero-length result; large payloads are freed via `engine_free_bytes`.

## Threading & data flow
SwiftUI / Sync / Persistence →(Command bytes via FFI)→ **MPSC** queue → RT thread. RT advances time from a Rust clock and dispatches MIDI via a fixed-slot ring to a **CoreMIDI worker thread** (`MIDISend` never runs on RT). RT emits small events on a fixed-slot channel; a Serialized/Error worker emits large events off-RT. Swift drains both on a dedicated `DispatchQueue` (~120 Hz), coalesces playhead events, and makes **one MainActor hop per batch** → `SessionMirror` → SwiftUI.

## Build & setup
- One-time: `engine/scripts/setup.sh` (`cargo install cbindgen`; iOS targets come from `rust-toolchain.toml`). Requires a `rustup`-managed toolchain + Xcode + Command Line Tools.
- Build engine: `engine/scripts/build_engine.sh` → `engine/dist/SequencerEngine.xcframework`. The Xcode run-script phase re-runs only when `engine/crates/**`, `engine/Cargo.toml`, `engine/Cargo.lock`, or any `engine/crates/*/Cargo.toml` changed (with `engine/target/` excluded), checked via a timestamp.
- Run app: open `app/StepForge.xcodeproj`, pick target, Run.
- Rust tests: `cargo test` in `engine/` (core tests in `crates/core/tests`; C-ABI round-trip tests in `crates/ffi/tests`, including a garbage-bytes decode that must return a non-fatal error status).

## Working agreement (per change)
- Adding a `Command`: Rust variant + `command_codec` encode/decode + Swift `Command.encode()` + an FFI round-trip test. Mirror symmetry for `EngineEvent` (and it must fit `MAX_EVENT_BYTES` on the hot path, else route via the large-event channel).
- Keep the RT path allocation-free (fixed buffers/slots; never `Vec`/`String`/`format!` on RT).
- Model change → update `serde_ext` (+version tag), the Swift mirror, and the snapshot test.
- Core logic changes (clock/scheduler/algorithms) need unit tests; ABI changes need C-ABI round-trip tests green, including malformed-byte handling.
- Non-destructive invariants: `Track.length` is a window over a fixed `[Step; 16]`; Roll/Vary/Cut/Trash never touch `length`/`midi_note`/`speed_ratio`; Paste carries `length`+`speed_ratio` from the clipboard but never `midi_note`.

## Where things live
Specs `docs/specs/` · amendments `docs/specs/amendments.md` · design docs `docs/superpowers/specs/` · plans `docs/plans/`.
```

---

## 6. Spec Amendments

### 6.1 Foundation resolutions (applied now — encoded in CLAUDE.md + this doc)

| ID | Spec issue | Resolution |
|---|---|---|
| **A1** | Crate-level `#![forbid(unsafe_code)]` **and** "unsafe only in `ffi.rs`" are mutually exclusive (`forbid` can't be relaxed by an inner module). | Workspace split: core `#![forbid(unsafe_code)]`, ffi `#![allow(unsafe_code)]` (D1). |
| **A2** | RT "never allocates/locks/crosses FFI," yet emits `Serialized{Vec}`/`Error{String}` from RT and calls `MIDISend` on RT (which can block). | Serialized/Error produced on an off-RT **worker**; MIDI handed off via a fixed-slot ring to a **CoreMIDI worker thread**; worker reads via a **non-blocking** mechanism; RT self-scheduled from a Rust clock (CLAUDE.md Hard Rule 1). |
| **A3** | `engine_set_event_callback` push path makes RT call into Swift. | Dropped; `engine_drain_events` (pull) only (D4). |
| **A4** | `#[repr(C)]` data-carrying enums (`Session`/`Vec`/`String`/`Uuid`) across the ABI are fragile. | Commands **and** events cross as bytes; hot channel one-event-per-call, large payloads on an off-RT channel (D5). |
| **A5** | Naming: repo `StepForge` vs spec's `sequencer_engine`/`SequencerApp`. | Crate `sequencer_engine`(+`_ffi`) generic; app `StepForge` (§2). |
| **A6** | No FFI panic/error contract; malformed bytes abort across `extern "C"`. | Every `extern "C"` wraps in `catch_unwind` + returns `#[repr(C)] EngineResult`; codecs total (CLAUDE.md Hard Rule 3). |
| **A7** | `engine_free_bytes` ownership + command/event ownership asymmetry unstated. | Buffer-ownership invariant (CLAUDE.md Hard Rule 4). |
| **A8** | Handle teardown race (free during in-flight drain). | Handle-lifecycle invariant + `EngineBridge.stop()` from scene-phase teardown (CLAUDE.md Hard Rule 5). |
| **A9** | "SPSC" command queue is actually multi-producer (UI + Sync + Persistence). | MPSC queue; `engine_submit_command` internally synchronized (D9, Hard Rule 3). |
| **A10** | CoreMIDI.framework link unaccounted in staticlib build. | `#[link(name="CoreMIDI", kind="framework")]` + `build.rs` + app "Link Binary" step (D8). |
| **A11** | Two committed header copies drift. | Single source of truth via `USER_HEADER_SEARCH_PATHS` (D10). |
| **A12** | "Embeds the xcframework" is wrong for a static `.a`. | Linked (not embedded); output at `engine/dist/` (D7). |
| **A13** | Drain framing model unspecified. | One event per call on the hot channel; empty/zero-length = drained (D5, Hard Rule 7). |
| **A14** | Foundation scope ambiguous (lists ~20 logic files but "no logic"). | Foundation = compiling skeleton: contract files seeded, logic stubbed (§9). |
| **A15** | No load/deserialize path for persistence. | `Command::LoadSession(bytes)`, identical+version-tagged format (D11). |
| **A16** | `engine/src/**` rebuild guard nonexistent; `engine/**` includes `target/`. | Guard on `engine/crates/**` + manifests + `Cargo.lock`, excluding `target/` (§7, Hard Rule build). |

### 6.2 Engine-level (logged to `docs/specs/amendments.md`, resolved during engine implementation — non-blocking)

| ID | Issue |
|---|---|
| **E1** | `speed_ratio` defined but unused by clock/dispatch; needs per-track step counters. |
| **E2** | Global-vs-per-track swing combination unspecified. |
| **E3** | `Step.micro_timing_offset` set by Roll but never read by dispatch. |
| **E4** | MIDI 0–127 velocity mapping for Low/Mid/Accent undefined; humanize-velocity on discrete zones unspecified. |
| **E5** | Note-Off scheduling after a Note-On (drum gate length) unspecified. |
| **E6** | Missing commands: `LinkPhase` (§9.2) and a clock-tick/step-advance command for inbound MIDI Clock (§9.3). (Now joined by `Command::LoadSession` from A15.) |
| **E7** | Per-event `Task { @MainActor }` hop — superseded by the batched "one hop per batch" design; engine plan must implement drain→coalesce→single-hop. |
| **E8** | Per-channel drop policy for full bounded queues (RT must not block). Default: drop-newest for hot events with playhead coalescing; drop-oldest + an `Overflow` diagnostic for the MIDI ring. |
| **E9** | `~120 Hz` drain rate is asserted; validate against worst-case non-coalesceable production at max BPM with ratchet X4. |
| **E10** | Structural home for the RT→CoreMIDI ring + worker (e.g. a `core/src/midi_out.rs`) is engine module granularity; the forbid-unsafe boundary already rests on `midi.rs` pure / `coremidi.rs` unsafe. |
| **E11** | `EngineBridge` exact actor isolation (plain `ObservableObject` vs `@MainActor` + nonisolated/Sendable handle wrapper so the background drain doesn't hop) is app-implementation; must honor "one MainActor hop per batch." |

---

## 7. Build & Toolchain Setup

- **`engine/rust-toolchain.toml`** — pins the toolchain **and** declares `[toolchain].targets = ["aarch64-apple-ios", "aarch64-apple-ios-sim", "x86_64-apple-ios"]` (single source of truth; auto-installed on first cargo run in CI/Xcode).
- **`engine/scripts/setup.sh`** — one-time: `cargo install cbindgen`; verify the three targets are installed (echo). No `rustup target add` (handled by the TOML). Requires `rustup`-managed toolchain + Xcode + CLT.
- **`engine/scripts/build_engine.sh`** — builds the three iOS slices of `sequencer_engine_ffi` and assembles **`engine/dist/SequencerEngine.xcframework`**, regenerating `engine/include/sequencer_engine.h` via cbindgen.
- **`engine/crates/ffi/build.rs`** — emits `cargo:rustc-link-lib=framework=CoreMIDI`; `coremidi.rs` `extern` blocks also carry `#[link(name = "CoreMIDI", kind = "framework")]` (D8).
- **Xcode integration** — `SequencerEngine.xcframework` added to **Link Binary With Libraries** (static `.a`, **not** embedded); `CoreMIDI.framework` also added to Link Binary; `SWIFT_OBJC_BRIDGING_HEADER` → `engine/include/sequencer_engine.h` via `USER_HEADER_SEARCH_PATHS`. A Run Script Phase invokes `build_engine.sh` only when `engine/crates/**`, `engine/Cargo.toml`, `engine/Cargo.lock`, or `engine/crates/*/Cargo.toml` changed (timestamp file), with `engine/target/` never counted as a trigger.

---

## 8. Acceptance Criteria (for the implementation plan)

The foundation is complete when:

- [ ] **Repo skeleton** matches §4 with the §9 file-state partition: `models.rs`/`command.rs`/`event.rs`/`serde_ext.rs` **seeded** (real enums + derives); the 8 `extern "C"` functions in `ffi/src/lib.rs` exist as panic-free stubs returning safe defaults; all other listed files exist as minimal compiling stubs (`[STUB]`).
- [ ] **CLAUDE.md** at repo root whose content equals the fenced block in §5.
- [ ] The two provided specs are at `docs/specs/ui-ux-spec.md` and `docs/specs/architecture-spec.md`; `docs/specs/amendments.md` exists with §6.2's list and a note that §6.1 resolutions are applied.
- [ ] **`.gitignore`** ignores `engine/target/`, `engine/dist/`, `*.xcuserdata`, derived data; **`.gitattributes`** present.
- [ ] **`engine/` workspace compiles**: `cargo check` passes for both crates; core remains `#![forbid(unsafe_code)]`; ffi `extern` blocks link CoreMIDI via `#[link]` + `build.rs`.
- [ ] **Header**: `build_engine.sh` generates `engine/include/sequencer_engine.h`, which contains the 8 `extern "C"` declarations; it is the only committed copy.
- [ ] **Toolchain**: `rust-toolchain.toml` pins the toolchain + 3 iOS targets; `setup.sh` installs cbindgen and verifies targets.
- [ ] **Clean-env reproducibility**: on a fresh rustup toolchain + Xcode CLT, `setup.sh` then `build_engine.sh` produces `engine/dist/SequencerEngine.xcframework` (verified, not just on the author's machine).
- [ ] **`app/StepForge.xcodeproj`** exists, **links** (not embeds) the xcframework + `CoreMIDI.framework`, uses `engine/include/sequencer_engine.h` as bridging header, and builds/launches with an empty UI.
- [ ] **FFI safety**: a `crates/ffi/tests` round-trip feeds truncated/garbage command bytes and asserts a non-fatal `EngineResult` error status, not a crash; a serialize→`LoadSession` round-trip restores equivalent state.
- [ ] Initial commit(s) made on `main`.

---

## 9. Foundation-created file partition

To resolve the "no logic vs ~20 logic files" ambiguity (A14), the plan creates a **compiling skeleton**:

- **Seeded (real content — the cross-layer contract):**
  - `core/src/models.rs` — `Session`, `Pattern`, `Track`, `Step`, and the enums (`SyncSource`, `VelocityZone`, `Ratchet`, `QuantizeGrain`, `FollowActionType`) with `Serialize/Deserialize/Clone/Debug` and **no `#[repr(C)]`** (per A4: these cross the FFI as bytes, so the C repr is unnecessary).
  - `core/src/command.rs` — the `Command` enum (spec §3.2) **plus** `Command::LoadSession(Vec<u8>)` (A15) and placeholders for `LinkPhase`/clock-tick (E6).
  - `core/src/event.rs` — the `EngineEvent` enum (spec §3.3), **without** `#[repr(C)]` (A4).
  - `core/src/serde_ext.rs` — serde derives + a version tag for session bytes.
  - `ffi/src/lib.rs` — the 8 `extern "C"` entry points (`engine_new/free/start/stop/submit_command/drain_events/serialize/free_bytes`) as **panic-free stubs** wrapped in `catch_unwind`, each returning a safe-default `EngineResult`; bodies `todo!()`-free and non-panicking so the `.a` links and cbindgen emits declarations.
  - `ffi/src/command_codec.rs` / `event_codec.rs` — codec **skeletons** with total `Result`-returning decode and `encode_into(&mut [u8; MAX_EVENT_BYTES])` (round-trip behavior filled by engine plan).
  - The 8 FFI declarations define the cbindgen-emitted `EngineResult` status enum.
- **Stub (minimal, compiles, logic deferred):** `core/src/{engine,clock,scheduler,midi}.rs`, `core/src/algorithms/*`, `core/src/{clipboard,undo}.rs`, `ffi/src/{handle,coremidi}.rs`, `app/StepForge/Engine/*`, `app/StepForge/Persistence/*`.
- **Deferred (created by engine/UI plans):** `core/tests/*`, `ffi/tests/*`, `app/.../Features/*`, `Sync/`, `Haptics/`, `*Tests/`.

The foundation deliverable is a repo that `cargo check`s, builds an xcframework whose header declares the 8 entry points, and links into an empty-but-launching StepForge app.

---

## 10. References

- Provided specs: `docs/specs/ui-ux-spec.md`, `docs/specs/architecture-spec.md`.
- Tracked amendments: `docs/specs/amendments.md`.
- Next step: implementation plan (produced via the `writing-plans` skill) under `docs/plans/`.
