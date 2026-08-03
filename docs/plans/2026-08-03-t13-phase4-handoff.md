# T13 handoff — Phase 4 (SettingsSheet + theme/typography + VST3 + codesign/notarize + CI + Live audio bus)

Handoff prompt to pick up T13 in a fresh session. Self-contained.

## Prerequisite

**PR #32 (T12) must be reviewed + merged first.** T12 = PerformanceView + PatternOptionsSheet + whole-pattern clipboard (`feat/clap-phase3-performanceview`, `c9a71b6`+`dce6acd`, SPEC T12 `x`, DAW smoke GO Bitwig). After merge, `origin/main` carries T12. Branch off `main`.

Two follow-up issues are OPEN but NOT blocking T13:
- **#33** — iOS `Command.swift` parity for the 4 pattern-clipboard commands + iOS `PatternOptionsSheet` buttons.
- **#34** — pattern-level undo (undo is currently track-scoped).

In `SPEC.md §T`, `T13` is the LAST `.` row. Phase 4 = the final phase.

## You are picking up

StepForge's **CLAP plugin egui editor, Phase 4 — the LAST phase.** SPEC: `T13|.|Phase 4 SettingsSheet + theme/typography polish + VST3 (clap-wrapper) + codesign/notarization + CI + Live dummy 2×2 audio bus|V7`.

**T13 is an UMBRELLA, not a single task** (like T10 was). It spans 6 distinct concerns — recommend splitting into sub-tasks (T13a–T13f), each its own spec→plan→execute + PR. Suggested split + order:

- **T13f — Live dummy 2×2 audio bus** (do first; tiny, unblocks Live testing). `AUDIO_IO_LAYOUTS = &[]` (MIDI-only, `clap_plugin/src/lib.rs:230`) works in Bitwig but **Ableton Live requires audio I/O**. Add a dummy 2×2 `AudioIOLayout`. Small `clap_plugin` change + a Live smoke.
- **T13a — SettingsSheet** (UI, no infra). The missing settings UI: sync source picker (Free/MIDI Clock/Link → `SetSyncSource`), global MIDI channel (`SetGlobalMidiChannel`). Transport bar shows sync read-only today with comment "full sync UI lands in Phase 4 SettingsSheet" (`transport.rs:40`). Pure-egui overlay — reuse the T11/T12 `Area`+`Frame::popup`+`overlay::should_dismiss`+`opened_at` open-frame guard idiom. Trigger: a "Settings" button in the TransportBar (next to the T12 AppMode toggle). **No new Command** — both already exist in core.
- **T13b — theme/typography module** (refactor, no infra). Extract the inline palette consts (`grid.rs:39-50`: `SURFACE_LOW/HIGH`, `PRIMARY`, `ZONE_*`, `TEXT_*`, `BORDER_WEAK`) into a `theme.rs` + add typography. Editor comment: "full palette/typography module lands in Phase 4" (`grid.rs:38`). Design ref: `docs/superpowers/specs/2026-07-27-clap-egui-editor-design.md` §Widgets/palette.
- **T13e — CI** (no secrets). GitHub Actions. **None exists** (`.github/workflows/` absent). Likely: workspace `cargo test` + `cargo clippy --all-targets -- -D warnings` + `cargo check -p sequencer_engine --target aarch64-apple-ios` (V5). macOS runner.
- **T13c — VST3** (design-heavy). Wrap the CLAP as VST3. **KEY OPEN QUESTION** — nih-plug native VST3 backend vs external `clap-wrapper` (see below). `engine/crates/xtask` bundles CLAP only today. SPEC says "clap-wrapper." Needs a design pass first.
- **T13d — codesign/notarize** (do last; needs USER INFRA). macOS signing for distribution. **Currently unbuilt** — `build_install_macos.sh` does NOT sign (no codesign/notarize/xcrun). Needs an Apple Developer ID + notarization credentials — **cannot be done without the user's signing identity.**

## Context — read first

- **Surface:** pure-Rust `nih_plug` + `egui`. `engine/crates/editor_egui` (`stepforge_editor_egui`), wrapped by `engine/crates/clap_plugin` (`stepforge_clap`), bundled by `engine/crates/xtask`. Consumes `engine/crates/core` in-process. **No Swift, no FFI, no `engine_*` entry points.**
- **Spec of record:** `SPEC.md §T` (T13 row) + `§V` invariants. T13's V-col is **V7** — read what V7 asserts + whether it broadens per sub-task.
- **Design doc:** `docs/superpowers/specs/2026-07-27-clap-egui-editor-design.md` (CLAP editor design — §Widgets/palette + Phase 4 sketch). **No dedicated Phase-4 / VST3 / signing / CI design doc exists yet** — T13c/T13d/T13e likely each need a `ck:grill`/`ck:spec` design pass first.
- **Memory:** `stepforge-clap-egui-port` (project memory) — T10–T12 done; nih-plug pinned `f36931f`; DAW-smoke-is-the-only-catch; egui 0.31 gotchas; the durable mirror-glue lesson (replay real event ORDER, don't seed fields).

### CLAP plugin wiring points (T13f/T13c)

- **`engine/crates/clap_plugin/src/lib.rs:230`** — `AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[]` (MIDI-only). T13f adds the dummy 2×2. Read nih-plug `AudioIOLayout` docs (Context7).
- **`engine/crates/clap_plugin/src/params.rs`** — `StepForgeParams` (two `#[persist]` fields: editor_state + session bytes; **zero automation params**). Settings emit `Command`s, not param changes.
- **`engine/crates/xtask`** — runs `nih_plug_xtask` to bundle `.clap`. T13c adds VST3 here (nih-plug native) OR a separate clap-wrapper step.

### Editor wiring points (T13a/T13b)

- **SettingsSheet**: floating `Area`+`Frame::popup` overlay (reuse `pattern_options.rs`/`action_drawer.rs`); a "Settings" button in TransportBar.
- **Settings commands already exist**: `SetSyncSource { source }`, `SetGlobalMidiChannel { channel }`. (`SetMidiDestinations` is CoreMIDI endpoint discovery — Swift-owned per Hard Rule 7; CLAP outputs MIDI to host, no endpoint picker.)
- **`UiState` read accessors**: `sync_source()` (`ui_state.rs:247`), `link_peers`/`link_enabled` fields (exist). MIDI channel — on `Session.global_midi_channel`; may need an accessor like `bpm()`.
- **Theme (T13b)**: palette consts `pub(crate)` in `grid.rs:39-50`; all widgets import from `grid` today → re-export from `theme.rs` or update imports.

## Conventions (CLAUDE.md hard rules + working agreement)

- **No orphans.** T13a/T13b editor-only (no new Command/Event — verify). T13f a `clap_plugin` change (no core change). T13c/T13d/T13e infra (no core/editor change). Any new `Command`/`EngineEvent` → cross-layer `/add-feature`.
- **V4 / Hard Rule 2.** SettingsSheet reads `UiState`, emits `Command`s, never mutates the engine.
- **RT path untouched** by T13a/T13b/T13f.
- **iOS guard (V5)** stays green.

## Known traps

- **DAW smoke is the ONLY catch** for host-facing behavior. T13f (Live audio bus) + T13c (VST3) NEED real-DAW smokes (Live for T13f; a VST3 host for T13c). `bash engine/scripts/install_clap.sh` from the worktree (not main); `Cmd+Q` the host (plugins cached).
- **Mirror-glue lesson** (T11/T12): if T13a adds a new mirror field driven by an event, replay the real event ORDER through `apply`, don't seed.
- **clap-validator crashes upstream** (nih-plug `f36931f` state-invalid-random OOM; zero-param divide-by-zero). Headless `cargo test` + DAW smoke only.
- **egui 0.31 gotchas**: `Window` absorbs first click (use `Area`+`Frame::popup`); floating `Area` needs ~4 settle frames + open-frame guard; `ctx.time()` doesn't exist (use `ctx.input(|i| i.time)`); `RichText` has no `.truncate()` (on `Label`); `Stroke::new` width needs `_f32`.
- **T13d (signing)** needs the user's Apple Developer ID + notarization creds — flag early, do last.
- **T13c (VST3)** — verify nih-plug native vs clap-wrapper against the pinned `f36931f`; don't assume.

## Out of scope (separate issues, already filed)

- **#33** iOS pattern-clipboard parity.
- **#34** pattern-level undo.
- Track speed/length/swing pickers (deferred from T11 — `SetTrackSpeedRatio`/`SetTrackLength`/`SetTrackSwing` have no editor UI; latent `TODO(T11)` ratchet-spacing bug for non-1.0 `speed_ratio`).

## Task — T13

T13 is the umbrella; **first step = decide the split + order** (suggest T13f→T13a→T13b→T13e→T13c→T13d, or your own). Then per sub-task: design (infra ones → `ck:grill`/`ck:spec`), implement (TDD where there's code), headless verify, DAW smoke (host-facing ones), flip SPEC sub-row. Each sub-task → its own PR. The SPEC T13 row flips `.`→`x` only when ALL sub-areas land — or split T13 into T13a–f rows first (via the spec skill) so each can flip independently.

## Verification — must pass before each PR

```bash
export PATH="$HOME/.cargo/bin:$PATH"   # rustup, not Homebrew
cd engine
cargo test                                 # workspace green
cargo clippy --all-targets -- -D warnings  # clean
cargo fmt
cargo check -p sequencer_engine --target aarch64-apple-ios   # V5
cargo xtask bundle -p stepforge_clap --release              # bundle builds
bash engine/scripts/install_clap.sh                          # from the worktree
# + DAW smoke for host-facing sub-tasks (T13f Live, T13c VST3)
```

## Done-criteria (SPEC T13)

- [ ] SettingsSheet (sync source, MIDI channel) → existing `Command`s.
- [ ] theme/typography module (palette extracted from `grid.rs`).
- [ ] VST3 build (clap-wrapper or nih-plug native) + VST3-host smoke.
- [ ] codesign + notarize (needs user signing identity).
- [ ] CI (GitHub Actions: test + clippy + iOS guard, macOS runner).
- [ ] Live dummy 2×2 audio bus (`AUDIO_IO_LAYOUTS`) + Live smoke.
- [ ] `SPEC.md §T`: `T13` → `x` (or split T13a–f, all `x`).

## Branch / PR

Branch off **`main`** (after #32 merges). Suggested `feat/clap-phase4-<subtask>` per sub-task. Work in a worktree (user runs concurrent sessions in the shared tree). Follow `/add-feature` for any cross-layer piece.

## Open questions to resolve or ask

- **VST3 approach** — nih-plug `f36931f` has its own VST3 backend (can `nih_plug_xtask` bundle VST3?) OR external `clap-wrapper`? SPEC says "clap-wrapper" — verify against the pinned nih-plug (check `nih_plug_xtask` capabilities); this drives all of T13c.
- **Split** — T13a–f as above, or a different decomposition? (`ck:grill` the umbrella first?)
- **Signing identity** — does the user have an Apple Developer ID + notarization creds for T13d? (Blocks T13d; do last.)
- **CI scope** — workspace test + clippy + iOS guard only, or also xcframework/xcodebuild (macOS-runner cost)?
- **Live access** — does the user have Ableton Live for the T13f smoke? (Bitwig can't validate the audio-bus requirement.)
- **SettingsSheet contents** — sync + MIDI channel only, or more (Link toggle? theme switch?)?
- **theme/typography** — palette extraction only, or a full design-system pass?
