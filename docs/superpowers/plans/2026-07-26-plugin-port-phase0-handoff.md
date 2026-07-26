# Phase 0 Host-Adapter — Execution Handoff

A self-contained prompt to bootstrap execution of the reviewed plan in a fresh
session. Paste everything below the rule into the new session.

---

You are picking up execution of an approved, peer-reviewed implementation plan
for StepForge (Rust musical-time core `sequencer_engine` + SwiftUI shell, strict
two-layer C-ABI boundary — read CLAUDE.md for the hard rules).

TASK
Execute the plan at
  docs/superpowers/plans/2026-07-26-plugin-port-phase0-host-adapter.md
task-by-task. This is Phase 0 of the plugin port: a host-driven render mode for
`sequencer_engine` — pure Rust, host-testable, no plugin wrappers yet (those are
Phases 1–4, explicitly out of scope).

METHOD (required)
Invoke the `superpowers:subagent-driven-development` skill and follow it. The
plan header mandates it. Concretely:
  - Dispatch one fresh subagent per task. Hand each subagent ONLY its task
    section plus the plan's "Global Constraints" and the task's
    "Interfaces (Consumes/Produces)" block — subagents have no prior context.
  - Two-stage review between tasks: review each task's diff before dispatching
    the next.
  - The plan is TDD with exact code, exact commands, and expected output. The
    snippets were verified against the current codebase during review —
    transcribe them verbatim; do not "improve" them mid-flight. Preserve the
    write-failing-test → run-to-fail → implement → run-to-pass → commit order.
  - Commit per task (the plan gives exact `git add`/`commit -m` text). No batching.

BEFORE STARTING
1. Read the plan end-to-end first (9 tasks). Note the "Deviations from the
   design spec" and "Scope boundary" sections — they explain deliberate choices.
2. Isolate the work: the repo is currently on `feat/sync-implementation`, but
   this is unrelated plugin-port work. Create a dedicated branch
   (`feat/plugin-port-phase0`) — or use `superpowers:using-git-worktrees` —
   before the first commit.

ENVIRONMENT (critical, easy to miss)
- Run all engine commands from `engine/`.
- The build needs the rustup toolchain, NOT Homebrew `rustc` (which can't
  cross-compile to iOS). Prefix engine commands with:
    export PATH="$HOME/.cargo/bin:$PATH"
  Host `cargo clippy`/`cargo test` are also blind to `#[cfg(target_os="ios")]`,
  so the iOS gate (`cargo check --target aarch64-apple-ios`) is a real check,
  not a formality.

GUARDRAILS
- RT path is sacred (Hard Rule 1): `engine_render`/`render_host`/`emit_midi_msg`
  run on the host RT thread — no allocation, no locks, no `Vec`/`String`/`format!`,
  no FFI out, no CoreMIDI, no Link. If a task seems to need a `Vec` there, use a
  fixed-size array (see `PendingMidiQueue`).
- Additive only: standalone `engine_new`/`engine_start` must stay bit-for-bit
  unchanged; iOS must still build.
- No orphans: if reality diverges from a snippet (signature changed, line moved),
  STOP and reconcile — fix the plan and the code together. Don't improvise.

DONE = Task 9 passes: `cargo test`, `cargo clippy --all-targets -- -D warnings`,
`cargo check --target aarch64-apple-ios`, `engine/scripts/build_engine.sh`, and
`/audit-rt` on `engine.rs` + `host.rs` all green. Then report the final task
status, the commit range, and any deviations you made with reasons.
