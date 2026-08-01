# Plan A Execution — Handoff Prompt (fresh session)

Paste the block below into a fresh Claude Code session to execute Plan A
(`midi_kernel` extraction) with zero prior context. Guardrails encode the
verification findings so the session does not re-stumble on the byte-trap,
cbindgen, or genericization gotchas.

Companion artifacts (on `main`):
- Plan: `docs/superpowers/plans/2026-08-01-midi-kernel-extraction.md`
- Spec: `docs/superpowers/specs/2026-08-01-mono-sequencer-engine-design.md`

---

```
# Task: Execute Plan A — midi_kernel extraction (StepForge repo)

You have no prior context. Read the plan + guardrails below, then execute task-by-task.
Repo: /Users/gus/Git/StepForge (currently on `main` at 936738d).

## What this is
StepForge = iOS/macOS MIDI drum sequencer: Rust musical-time core
(`engine/crates/core`, crate `sequencer_engine`) + SwiftUI shell, split by a
byte-serialized C ABI. Plan A extracts the drum-agnostic LEAF infrastructure
out of `core` into a new shared crate `midi_kernel`, migrates drum core onto
it, and PROVES the shipping drum app is byte-compatible. It is the prerequisite
gate for a future mono sequencer (Plan B) — do NOT start Plan B.

## Read first, in order
1. `docs/superpowers/plans/2026-08-01-midi-kernel-extraction.md` — THE plan.
   12 tasks, TDD, exact file paths + code + commands. Follow it literally.
2. `docs/superpowers/specs/2026-08-01-mono-sequencer-engine-design.md`
   §1, §2, §9, §10 (phases 0–1) — the design + why.
3. `CLAUDE.md` — the Hard Rules + Working Agreement are binding (RT-safety,
   forbid-unsafe isolation, symmetric cross-layer, byte-FFI seam).

## Execution mode
Use the `superpowers:subagent-driven-development` skill (the plan's REQUIRED
SUB-SKILL): fresh subagent per task, review between tasks. Announce it at start.

## Setup
- Branch off `main`: `feat/midi-kernel-extraction`. Commit per-task (the plan
  gives commit messages). Do NOT push or open a PR until the Phase-1 gate
  (Task 12) is fully green AND I approve.
- Worktree optional (via `superpowers:using-git-worktrees`) if you want
  isolation; otherwise a feature branch matches StepForge convention.
- Toolchain: rustup-managed (Homebrew `rustc` can't cross-compile to iOS).
  Prefix engine commands with `export PATH="$HOME/.cargo/bin:$PATH"`. Host
  `cargo clippy`/`test` is blind to `#[cfg(target_os="ios")]` — verify iOS
  with `cargo check --target aarch64-apple-ios`.
- Run engine cargo commands from `engine/`; run `./build_install_macos.sh`
  and xcodebuild from repo root.

## Critical guardrails (already verified — do not re-derive, do not violate)
- BYTE-COMPAT IS THE WHOLE POINT. Drum serialized structs
  (Session/Pattern/Track/Step/Command/EngineEvent/SessionEnvelope) stay in
  `core`, field-for-field unchanged. models.rs embeds zero kernel types
  (proven) → postcard bytes are invariant IF you don't break it.
- TASK 1 FIRST AND LITERALLY. The byte-equality golden test + header snapshot
  are captured BEFORE any code moves. This is the ONLY test that catches a
  postcard field-order flip — all existing envelope tests are self-consistent
  round-trips that pass silently after a flip. Capture the real bytes (Step 2);
  do not leave the 0x00 placeholder.
- VersionedEnvelope<T> FIELD ORDER IS LOAD-BEARING. Postcard is tagless /
  field-ORDER sensitive. Pin field 0 = `version: u8`, payload field NAMED
  `session` (no #[serde(rename)]). A `{ data: T, version: u8 }` would silently
  flip the wire format and break every saved session. Golden test guards it.
- CBINDGEN INCLUDE WHITELIST (Task 10). Moving HostTransport/MidiEvent (the
  #[repr(C)] structs) to midi_kernel makes cbindgen emit them opaque UNLESS
  you add `"stepforge_midi_kernel"` to `engine/cbindgen.toml [parse] include`.
  Gate: `git diff --exit-code -- engine/include/sequencer_engine.h` empty
  after `engine/scripts/build_engine.sh`.
- NO DRUM SYMBOL CROSSES INTO midi_kernel. Genericize LargeEventChannel<E> +
  push_event<E> + push_large_event<E> (Task 5) — drum instantiates
  E = EngineEvent. Co-extract QuantizeGrain WITH SchedulerClock (Task 7) —
  pure transport grain, not drum-typed, but SchedulerClock depends on it.
- ffi CRATE SOURCE STAYS LITERALLY UNCHANGED. Achieved via
  `pub use midi_kernel::{clock, host, midi_out, scheduler, serde_ext}`
  re-exports in core/src/lib.rs (Task 9) + `HostRenderState::<RtState>`
  turbofish in ffi/src/handle.rs (Task 6). If an ffi file fails to resolve a
  symbol, add the re-export — do NOT edit ffi source.
- midi_kernel is #![forbid(unsafe_code)] (Hard Rule 6). All unsafe stays in
  sequencer_engine_ffi.
- NAME COLLISION (Task 7): core cannot have both `pub mod scheduler` and
  `pub use midi_kernel::scheduler`. Plan's resolution: move drum residue
  `all_notes_off_burst` into core/src/midi.rs and drop core/src/scheduler.rs;
  then `pub use midi_kernel::scheduler` is unambiguous.

## Phase-1 gate (Task 12 — must be FULLY green before stop)
- `cargo test` (all Rust tests)
- `cargo test -p sequencer_engine_ffi --test ffi_api`
- `cargo test -p sequencer_engine_ffi --test coremidi_host`
- `cargo test -p sequencer_engine --test envelope_bytes_baseline`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo fmt --check`
- `cargo check --target aarch64-apple-ios` (rustup toolchain, not Homebrew)
- `git diff --exit-code -- engine/include/sequencer_engine.h` (header clean)
- `./build_install_macos.sh` (Swift drum app builds, untouched behavior)
  — if it needs the xcframework bootstrapped, run
  `engine/scripts/build_engine.sh` first (preBuildScript can't self-bootstrap).

## Stop conditions
- STOP after Task 12 green. Paste the gate command outputs.
- Do NOT start Plan B (mono sequencer) — depends on this merging first.
- Do NOT push or open a PR until I review the gate output.
- If any task's verification fails: STOP, paste the failure, propose the fix.
  Do not blindly force forward. Consider whether a step was misread before
  retrying.

## Where things live
Plan: docs/superpowers/plans/2026-08-01-midi-kernel-extraction.md
Spec:  docs/superpowers/specs/2026-08-01-mono-sequencer-engine-design.md
Engine: engine/crates/{core, ffi, midi_kernel (new)}
Header: engine/include/sequencer_engine.h (regenerated; single source of truth for Swift)
```
