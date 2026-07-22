# Spec Amendments — StepForge

Authoritative list of changes to the two source specs (`ui-ux-spec.md`, `architecture-spec.md`).
Foundation resolutions (A1–A16) are applied in the foundation design
(`docs/superpowers/specs/2026-07-22-project-foundation-design.md`) and encoded in `CLAUDE.md`.
Engine-level items (E1–E11) are open; resolve them in the engine implementation plan.

## Foundation resolutions (applied)

- **A1** Crate-level `#![forbid(unsafe_code)]` cannot coexist with unsafe in `ffi` → workspace split (core forbids, ffi allows).
- **A2** RT no-alloc/no-lock/no-FFI vs. emitting `Serialized`/`Error` and calling `MIDISend` on RT → produce Serialized/Error on an off-RT worker; MIDI via fixed-slot ring to a CoreMIDI worker; worker reads non-blocking; RT self-scheduled from a Rust clock.
- **A3** Push `engine_set_event_callback` makes RT call into Swift → dropped; pull `engine_drain_events` only.
- **A4** `#[repr(C)]` data-carrying enums across ABI → commands and events cross as postcard bytes; no repr(C).
- **A5** Naming → crate `sequencer_engine`(+`_ffi`) generic; app `StepForge`.
- **A6** No FFI panic contract → every extern wraps in `catch_unwind` + returns `EngineResult`; codecs total.
- **A7** `engine_free_bytes` ownership unstated → buffer-ownership invariant (Rust allocates, Swift frees only via `engine_free_bytes`).
- **A8** Handle teardown race → `engine_stop` before `engine_free`; no concurrent calls.
- **A9** "SPSC" command queue is multi-producer → MPSC.
- **A10** CoreMIDI.framework link unaccounted → `build.rs` + app "Link Binary".
- **A11** Two header copies drift → single committed header at `engine/include/`.
- **A12** "Embed" is wrong for static `.a` → linked, not embedded; output `engine/dist/`.
- **A13** Drain framing unspecified → one event per call on the hot channel; empty/zero = drained.
- **A14** Foundation scope ambiguous → compiling skeleton (contract seeded, logic stubbed).
- **A15** No load path → `Command::LoadSession(bytes)`, version-tagged format.
- **A16** Rebuild guard path wrong → guard on `engine/crates/**` + manifests + `Cargo.lock`, excluding `target/`.

## Engine-level (open — resolve in the engine plan)

- **E1** `speed_ratio` defined but unused by clock/dispatch; needs per-track step counters.
- **E2** Global-vs-per-track swing combination unspecified.
- **E3** `Step.micro_timing_offset` set by Roll but never read by dispatch.
- **E4** MIDI 0–127 velocity mapping for Low/Mid/Accent undefined; humanize-velocity on discrete zones unspecified.
- **E5** Note-Off scheduling after a Note-On (drum gate length) unspecified.
- **E6** Missing commands: `LinkPhase` (§9.2) and a clock-tick/step-advance for inbound MIDI Clock (§9.3). (`Command::LoadSession` added by A15; `LinkPhase` + `MidiClockTick` placeholders seeded.)
- **E7** Per-event `Task { @MainActor }` hop → drain→coalesce→single MainActor hop.
- **E8** Per-channel drop policy for full bounded queues (RT must not block).
- **E9** `~120 Hz` drain rate — validate against worst-case production at max BPM / ratchet X4.
- **E10** Structural home for RT→CoreMIDI ring + worker (e.g. `core/src/midi_out.rs`).
- **E11** `EngineBridge` exact actor isolation (Sendable handle wrapper) — honor "one MainActor hop per batch".
