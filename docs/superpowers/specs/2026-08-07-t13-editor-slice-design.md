# T13 Editor Slice — CLAP SettingsSheet + Theme/Typography

**Date:** 2026-08-07
**Status:** Design approved; pending implementation plan
**Phase:** T13 (Phase 4) — UI sub-slice. Branches off `origin/main` (`ef9e0e0`), parallel to PR #47 (pattern-level undo, not yet merged).

## Context

T13 is the SPEC.md §T Phase-4 umbrella: *"SettingsSheet + theme/typography polish +
VST3 (clap-wrapper) + codesign/notarization + CI + Live dummy 2×2 audio bus."* The
post-bug-sweep roadmap splits it into *"T13 editor slice → distribution → mono Plan A
→ AUv3."* This spec covers the **editor slice** only — the two UI halves:

- **T13a — `SettingsSheet`.** The CLAP egui editor has no settings surface at all today
  (`transport.rs:41` mentions `SettingsSheet` only in a doc comment). iOS has a dedicated
  `SettingsSheet.swift` (3 sections); the CLAP editor ports the subset that applies to a
  hosted plugin.
- **T13b — theme/typography polish.** The editor's palette is a Phase-0 stub: 9 `pub(crate)`
  `Color32` consts in `grid.rs:39-50` and `apply_theme` (`lib.rs:82-87`) sets only
  `Visuals::dark()` + white text. Typography has zero abstraction. Both carry the comment
  *"Full palette/typography module lands in Phase 4."* T13 lands it.

The distribution half of T13 (VST3 / codesign / CI / Live dummy bus) is a separate slice
and is **out of scope** here.

## Goal

Deliver a CLAP editor `SettingsSheet` with parity to the iOS Session section (one writable
musical control + read-only status), and a real theme/typography token system that ports
the iOS `Theme.swift` + `Typography.swift` design language into egui. Both are confined to
`stepforge_editor_egui`: no `core`, no `ffi`, no C header, no new `Command` / `EngineEvent`,
no serde bump, no RT path.

## Decisions (resolved forks)

1. **Sync source is read-only in the sheet.** In a hosted CLAP plugin the host owns transport
   (`clap_plugin/src/transport.rs:5-22` maps host `tempo_bpm` + `is_playing`). The implemented
   invariant at `editor_egui/src/transport.rs:40` and the passing test
   `transport_sync_badge_emits_no_setsyncsource` already forbid the editor from emitting
   `SetSyncSource`. The design doc (`2026-07-27-clap-egui-editor-design.md` §Widgets) also says
   sync source is read-only. The handoff doc's T13a note ("emit `SetSyncSource`") is rejected —
   it contradicts the code, the test, and the design doc. The sheet shows sync source (and BPM,
   and Link status) as read-only display only.
2. **`SettingsSheet` content = minimal parity.** One writable control — Global MIDI Channel
   (picker 1-16, emits `SetGlobalMidiChannel`) — plus a read-only Session status block. MIDI
   Routing is excluded (host owns MIDI output; `SetMidiDestinations` has no CLAP meaning).
   Swing is excluded (already editable in `FeelBar`; avoids iOS's redundant duplication).
3. **Theme/typography = full token port on egui default fonts.** Port the iOS token set into a
   new `theme.rs` (5 surface tiers, border/brand/text tokens, `Spacing`, `Radius`) + a
   `typography.rs` type-scale registered as egui `TextStyle`s. Stay on egui's default fonts
   (Ubuntu/Hack); do not bundle a real typeface. egui default fonts expose only normal/bold, so
   `medium → normal` and `semibold → bold` in the type-scale mapping.
4. **Construction = `egui::Area` + `Frame::popup`, not `egui::Window`.** The design-doc sketch
   says `egui::Window` (line 362), but the three implemented overlays (`action_drawer`,
   `note_picker`, `pattern_options`) all use `Area` + `Frame::popup` with a load-bearing
   rationale: a `Window` title-bar absorbs the first click, and a settings sheet's ComboBox needs
   every click to register. The implemented pattern is authoritative. The sheet reuses the shared
   `overlay::should_dismiss` open-frame guard verbatim.
5. **Slice structure = T13a → T13b, one branch, one PR.** T13a ships first (user-visible feature
   on an isolated new file, low blast radius); T13b second (mechanical token refactor that then
   benefits the whole crate, including the new sheet).

## Non-goals (explicitly deferred)

- **MIDI Routing section** — host owns CLAP MIDI output; `SetMidiDestinations` + the CoreMIDI
  endpoint toggles are iOS-only (Hard Rule 7: Swift owns the CoreMIDI lifecycle). AUv3 already
  excludes the settings sheet for the same reason (`PluginEditorView.swift:10`).
- **Writable sync source / BPM** — host-owned transport. Decided read-only.
- **Theme toggle / light variant** — the design language is dark-only (iOS uses
  `preferredColorScheme(.dark)`). Tokens are bare consts, not mode-functions. A theme switch is a
  follow-up that would require tokens to become mode-aware.
- **Real font install** — egui default fonts. Bundling Inter/Geist via `FontDefinitions` is a
  follow-up (adds a font asset + bundle-size cost).
- **Track-level trio** `SetTrackLength` / `SetTrackSpeedRatio` / `SetTrackSwing` — per-track
  settings. A global sheet is the wrong shape; these belong in a future per-track header overlay
  mirroring iOS `TrackHeader.swift`. `SetTrackSwing` is dead on both surfaces today.
- **Accessibility / font-size scale, reset-to-defaults, default-velocity** — out of Phase-4 scope.
- **Distribution parts** — VST3 (clap-wrapper), codesign/notarization, CI, Live dummy 2×2 audio
  bus. Separate slice(s).
- **Forced migration of all 20 `.strong()` / `.small()` sites** — T13b installs the type-scale
  and uses it in the new `SettingsSheet` + the BPM display. Wholesale migration of existing call
  sites is a follow-up.

## Scope

### T13a — `SettingsSheet`

A new overlay `settings.rs`, cloned from the `pattern_options.rs:28-213` template, plus a gear
trigger in `TransportBar` and one `UiState` accessor.

**State** (in `ctx.data` temp, keyed `stepforge.settings`; presence = open):

```rust
pub struct SettingsState {
    opened_at: u64, // frame_nr at open; feeds overlay::should_dismiss open-frame guard
}
```

No draft field. The MIDI channel commits on change (pick → emit), matching iOS
`SettingsSheet.swift` Picker semantics and avoiding stale-draft risk.

**Helpers** mirror `pattern_options.rs`: `id()`, `read(ctx) -> Option<SettingsState>`,
`write(ctx, st)`, `open(ctx, &UiState)` (writes `Some({ opened_at: frame_nr(ctx) })`),
`close(ctx)`.

**Mutual exclusion** (symmetric, small edits):
- `settings::open` calls `note_picker::close` + `action_drawer::close` + `pattern_options::close`.
- Each of those three `open()` fns calls `settings::close`.

**Mode scope** — settings is **mode-agnostic**. The `transport.rs` AppMode-toggle currently closes
mode-bound overlays on switch (`transport.rs:177-196`); it must NOT close settings. The toggle's
close list is edited to exclude settings, so the sheet persists across Editing ↔ Performance.

**Render** — slotted into the `lib.rs` tick loop after the other overlays (~line 146):

```rust
settings::render(ctx, &ui_state, &sink);
```

`render`: if `read(ctx)` is `Some`, draw
`egui::Area::new(id()).order(Foreground).current_pos(Pos2::new(40.0, 60.0)).show(ctx, |ui| Frame::popup(ui.style()).show(ui, |ui| { ... }))`.
Body:

1. **Section "Session" (read-only):** `BPM` (`ui_state.bpm()`), `Sync` (the existing
   `sync_label`), and when `link_enabled`: `Link peers` / `Link session`
   (`ui_state.link_peers`, `link_enabled`).
2. **Writable — Global MIDI Channel:** a `ComboBox` 1-16. Reads
   `ui_state.global_midi_channel()`; on change emits `Command::SetGlobalMidiChannel(ch)`.
3. **"Done" button** → `close(ctx)`.

Post-`Area` tail (identical 6-line block to the other overlays): capture `area.response.rect`,
compute `is_open_frame = frame_nr(ctx) == st.opened_at`, call `overlay::should_dismiss`, close on
true.

**Gear button** — added at the end of the `transport.rs` row. On click →
`settings::open(ctx, &ui_state)`.

**`UiState` change** — one accessor in `ui_state.rs` beside `bpm()` / `sync_source()`
(`ui_state.rs:254-310`):

```rust
pub fn global_midi_channel(&self) -> u8 {
    self.session.as_ref().map(|s| s.global_midi_channel).unwrap_or(10)
}
```

The field already exists on `Session` (`models.rs:24`, default 10); only the read-fn is missing.

**Files (T13a):**

| Action | Path |
|---|---|
| CREATE | `engine/crates/editor_egui/src/settings.rs` |
| MODIFY | `engine/crates/editor_egui/src/lib.rs` (`pub mod settings;` + render call ~L146) |
| MODIFY | `engine/crates/editor_egui/src/transport.rs` (gear button) |
| MODIFY | `engine/crates/editor_egui/src/ui_state.rs` (`global_midi_channel()` accessor) |
| MODIFY | `engine/crates/editor_egui/src/{action_drawer,note_picker,pattern_options}.rs` (`settings::close` on open) |
| TESTS | headless, in `engine/crates/editor_egui/` via the existing `test_support` harness |

**T13a tests:**

- `settings_open_renders_area` — gear click → area present; `close` → absent.
- `settings_midi_channel_emits_setglobalmidichannel` — pick ch 12 → captured
  `Command::SetGlobalMidiChannel(12)`.
- `settings_emits_no_setsyncsource` — guards the T10c invariant; interacting with the sheet
  emits no `SetSyncSource`.
- `settings_mutual_exclusion` — opening settings closes `pattern_options`; opening
  `note_picker` closes settings.
- `settings_survives_mode_toggle` — AppMode switch does not close settings.

### T13b — theme/typography

**`theme.rs`** — single source of truth for visuals. Port the iOS `Theme.swift` token set:

| Group | Tokens |
|---|---|
| Surface (5 tiers) | `LOWEST 0x0E0E0E`, `LOW 0x1B1B1B`, `DEFAULT 0x202020`, `HIGH 0x2A2A2A`, `HIGHEST 0x353535` |
| Border | `BORDER_WEAK`, `BORDER_STRONG`, `BORDER_ACCENT` |
| Brand | `PRIMARY 0xFF7F00`, `PRIMARY_DIM 0xFFB688`, `ON_PRIMARY 0x231300` |
| Text | `TEXT_PRIMARY`, `TEXT_SECONDARY 0xA0A0A0`, `TEXT_MUTED` |
| Velocity zones | `ZONE_ACCENT 0xFF7F00`, `ZONE_MID 0xFFB688`, `ZONE_LOW 0x98CBFF` |
| Spacing | `xs 4 / sm 8 / md 16 / lg 24 / xl 48 / gutter 12` |
| Radius | `sm 4 / md 6 / lg 8` |

Spacing and Radius as named `pub const` under a `spacing::` / `radius::` path. `Radius::sm = 4`
**fixes the existing `CORNER = 3` drift** (`grid.rs:59-71`) off iOS `Theme.Radius.sm = 4`.

**`typography.rs`** — 7 named roles mapped to egui (fixed px, default fonts):

| Role | size px | family | bold | iOS source |
|---|---|---|---|---|
| `BpmLarge` | 28 | Monospace | yes | `bpmLarge` (title2 mono bold) |
| `MonoValue` | 13 | Monospace | yes | `monoValue` (13 semibold mono) |
| `StepIndex` | 10 | Monospace | no | `stepIndex` (10 medium mono) |
| `TrackName` | 14 | Proportional | yes | `trackName` (subheadline semibold) |
| `ControlLabel` | 12 | Proportional | no | `controlLabel` (caption medium) |
| `SectionTag` | 11 | Monospace | yes | `sectionTag` (caption2 mono semibold) |
| `Badge` | 10 | Monospace | yes | `badge` (10 bold mono) |

egui default fonts expose only normal/bold, so `medium → normal`, `semibold → bold`. Delivered as
a `TextStyle` registration in `apply_theme` (`ctx.style_mut().text_styles`) plus helper fns
returning `RichText` per role.

**`apply_theme` wiring** (`lib.rs:82-87`, currently a stub) — keep `Visuals::dark()` + white text;
add: set `ctx.style_mut().spacing.item_spacing` and button/window rounding from `Radius`, and
install the 7 `text_styles`. Tokenize the stray `LIGHT_RED` literal (`lib.rs:112`) to a semantic
`DANGER` color in `theme.rs`.

**`grid.rs`** — remove the 9 palette consts (lines 39-50); keep the layout sizing consts
(`HEADER_WIDTH`, `CELL_*`, `STEP_GAP`, `ROW_SPACING`, etc.) for now (sizing migration is a
follow-up). The comment at `grid.rs:36-38` already intended this move.

**Import migration** — 9 widget files change `use crate::grid::{TOKEN...}` →
`use crate::theme::{TOKEN...}`: the 8 existing importers — `action_drawer.rs:27`, `feel.rs:26`,
`note_picker.rs:23`, `pattern_options.rs:25`, `performance.rs:35`, `track_management.rs:23`,
`transport.rs:19`, and `grid.rs` itself — plus `settings.rs` (created in T13a, which initially
imports the old `crate::grid::` palette because `theme.rs` does not yet exist). Tokenize the
`Color32::BLACK` / `WHITE` literals (`note_picker.rs:268-275`, `grid.rs:569`) →
`ON_PRIMARY` / `TEXT_PRIMARY`.

**Files (T13b):**

| Action | Path |
|---|---|
| CREATE | `engine/crates/editor_egui/src/theme.rs` |
| CREATE | `engine/crates/editor_egui/src/typography.rs` |
| MODIFY | `engine/crates/editor_egui/src/lib.rs` (`apply_theme` body + `pub mod theme;` `pub mod typography;`) |
| MODIFY | `engine/crates/editor_egui/src/grid.rs` (remove palette consts; tokenize `BLACK` literal) |
| MODIFY | 9 widget files (8 existing + `settings.rs` from T13a) — import migration + `BLACK`/`WHITE` tokenization |
| TESTS | headless, in `engine/crates/editor_egui/` |

**T13b tests:**

- `palette_matches_ios_hex` — assert each `theme::` const equals the iOS `Theme.swift` hex.
- `apply_theme_installs_spacing_radius_textstyles` — after `apply_theme(&ctx)`, assert
  `ctx.style().spacing` and `text_styles` carry the new values.
- Existing 117 editor tests stay green — the import migration is mechanical; this is the real
  regression guard.

## Cross-cutting

**SPEC §T split** (current line 76 → three rows):

- `T13a` — CLAP `SettingsSheet` (gear in `TransportBar`; read-only BPM/Sync/Link status + writable
  Global MIDI Channel; `Area` + `Frame::popup`; no new Command — `SetGlobalMidiChannel` /
  `SetSyncSource` exist; sync read-only honors T10c).
- `T13b` — CLAP theme/typography polish (`theme.rs` + `typography.rs`; port iOS tokens; wire
  `apply_theme`; egui default fonts).
- `T13` (narrowed) — distribution remainder: VST3 (clap-wrapper) + codesign/notarization + CI +
  Live dummy 2×2 audio bus.

When T13a + T13b land, both flip `.` → `x`; the parent T13 stays `.` (distribution pending).

**Invariant reaffirmation:**

- `#![forbid(unsafe_code)]` preserved — all work in `editor_egui`, which already allows no `unsafe`.
- RT thread untouched — every change is editor UI, off-RT.
- C header byte-identical — no `engine_*` entry point changes.
- No `SESSION_FORMAT_VERSION` bump — no serialized model changes.
- No new `Command` / `EngineEvent` — `SetGlobalMidiChannel` (`command.rs:100`) and `SetSyncSource`
  (`command.rs:87`) both exist; the sheet reads/emits the former and only displays the latter.

## Build order

T13a first, then T13b. T13a is isolated to a new file (`settings.rs`) + small symmetric edits, so
it lands with the smallest blast radius. T13b is a mechanical refactor (move consts, add tokens,
migrate 8 import lines, wire `apply_theme`); doing it second means the new `SettingsSheet` can
consume the final token system, and the existing 117 tests guard the migration.

Within T13b, the order is: create `theme.rs` + `typography.rs` → migrate imports → wire
`apply_theme` → tokenize stray literals. Each step keeps the crate compiling.

## Verification bar

Headless tests + iOS guard + header check + DAW smoke.

- `cargo test -p stepforge_editor_egui` — new tests + existing 117 green.
- `cargo clippy -p stepforge_editor_egui --all-targets -- -D warnings`.
- `cargo check -p sequencer_engine --target aarch64-apple-ios` (iOS guard; prefix
  `PATH="$HOME/.cargo/bin:$PATH"` — Homebrew rustc cannot cross).
- `git diff --exit-code -- engine/include/sequencer_engine.h` (header byte-identical).
- `cargo xtask bundle -p stepforge_clap --release` + DAW smoke (Bitwig): open `SettingsSheet`
  from the gear, change MIDI channel, confirm the host sees MIDI on the new channel; confirm sync
  source displays read-only and emits nothing; confirm theme renders consistently across
  Editing/Performance.

## Open follow-ups (not in this slice)

- Theme toggle (dark/light) — requires mode-aware tokens.
- Real font install (Inter/Geist) via `FontDefinitions`.
- Wholesale migration of the 20 `.strong()` / `.small()` call sites to the new type-scale.
- Track-level settings overlay (`SetTrackLength` / `SetTrackSpeedRatio` / `SetTrackSwing`).
- Distribution half of T13 (VST3 / codesign / CI / Live bus).
