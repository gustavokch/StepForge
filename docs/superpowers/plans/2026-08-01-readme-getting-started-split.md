# README Getting-Started Split — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split `README.md`'s `## Getting started` into two self-contained build/install tracks (Swift app + AUv3 / CLAP plugin) and add `engine/scripts/install_clap.sh`, which the CLAP track references.

**Architecture:** Two artifacts. (1) A new ~45-line bash script — symmetric with `build_install_macos.sh` — that bundles the CLAP plugin via `cargo xtask bundle` and ditto-copies the `.clap` into `~/Library/Audio/Plug-Ins/CLAP/`. (2) A rewrite of the README getting-started section into a one-H2 + two-H3 structure with a chooser, each H3 self-contained. No Rust or Swift source changes.

**Tech Stack:** bash (`set -euo pipefail`), markdown, `cargo xtask` (nih_plug_xtask bundler).

**Spec:** [`docs/superpowers/specs/2026-08-01-readme-getting-started-design.md`](../specs/2026-08-01-readme-getting-started-design.md)

## Global Constraints

- Every script under `engine/scripts/` opens with `set -euo pipefail` and `export PATH="$HOME/.cargo/bin:$PATH"` — Homebrew rust shadows rustup and must not win. (Mirrors `setup.sh` / `build_engine.sh` / `build_install_macos.sh`.)
- Command forms in the README are lifted **verbatim** from `CLAUDE.md` / the current README wherever they already exist. The only genuinely new command line is `engine/scripts/install_clap.sh`.
- No changes to Rust crates, Swift sources, `CLAUDE.md`, specs, or the license. README dual-license (`MIT OR Apache-2.0`) unchanged.
- `.clap` bundles are directories on macOS — use `ditto` (not `cp`) and `[ -d ]` checks, matching `build_install_macos.sh`'s bundle handling.
- No shellcheck in this environment — hard-gate script syntax on `bash -n`; run `shellcheck` only if `command -v shellcheck` succeeds.

---

## Task 1: `engine/scripts/install_clap.sh`

**Files:**
- Create: `engine/scripts/install_clap.sh`

**Interfaces:**
- Consumes: the `cargo xtask` bundler (`crates/xtask`, bin `xtask`, invoked as `cargo xtask`) and the `stepforge_clap` package (`crates/clap_plugin/Cargo.toml` → `name = "stepforge_clap"`). Bundle output path is fixed by nih_plug_xtask: `engine/target/bundled/stepforge_clap.clap`.
- Produces: an executable `engine/scripts/install_clap.sh` that, on success, leaves `~/Library/Audio/Plug-Ins/CLAP/stepforge_clap.clap` installed and exits `0`; on any failure (cargo missing, bundle not produced), exits non-zero with a clear message. Task 2's README references this script by exact path.

- [ ] **Step 1: Write the script**

Create `engine/scripts/install_clap.sh` with this exact content:

```bash
#!/usr/bin/env bash
# Bundle the StepForge CLAP plugin (Release) and install it into
# ~/Library/Audio/Plug-Ins/CLAP/ so a host can load it.
#
# Runs `cargo xtask bundle -p stepforge_clap --release` (cargo-incremental,
# fast on repeat runs), then ditto-copies the produced .clap over any previous
# install and strips the quarantine attribute (the bundle is unsigned locally).
#
# Usage:
#   engine/scripts/install_clap.sh
set -euo pipefail

# Homebrew rust shadows rustup; cargo must resolve to the rustup toolchain.
# (Mirrors setup.sh / build_engine.sh / build_install_macos.sh.)
export PATH="$HOME/.cargo/bin:$PATH"

ENGINE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLAP_PKG="stepforge_clap"
BUNDLE_NAME="${CLAP_PKG}.clap"
BUILT_BUNDLE="$ENGINE_DIR/target/bundled/$BUNDLE_NAME"

CLAP_DIR="$HOME/Library/Audio/Plug-Ins/CLAP"
INSTALLED_BUNDLE="$CLAP_DIR/$BUNDLE_NAME"

log() { echo "[$1] $2"; }
start=$SECONDS

# -----------------------------------------------------------------------------
# 0. Preflight
# -----------------------------------------------------------------------------
command -v cargo >/dev/null 2>&1 || {
  echo "[error] cargo not found (install rustup from https://rustup.rs and put ~/.cargo/bin on PATH)" >&2
  exit 1
}

mkdir -p "$CLAP_DIR"

# -----------------------------------------------------------------------------
# 1. Bundle the plugin (Release)
# -----------------------------------------------------------------------------
log bundle "cargo xtask bundle -p $CLAP_PKG --release ..."
( cd "$ENGINE_DIR" && cargo xtask bundle -p "$CLAP_PKG" --release )

[ -d "$BUILT_BUNDLE" ] || {
  echo "[error] bundle produced no .clap at $BUILT_BUNDLE" >&2
  exit 1
}
log bundle "done -> $BUILT_BUNDLE"

# -----------------------------------------------------------------------------
# 2. Install into ~/Library/Audio/Plug-Ins/CLAP/
# -----------------------------------------------------------------------------
log install "copying to $INSTALLED_BUNDLE ..."
rm -rf "$INSTALLED_BUNDLE"
ditto "$BUILT_BUNDLE" "$INSTALLED_BUNDLE"

# Locally built and unsigned; strip any quarantine attribute so the host loads it.
log install "stripping quarantine xattr ..."
xattr -cr "$INSTALLED_BUNDLE" 2>/dev/null || true

elapsed=$(( SECONDS - start ))
echo
log done "installed -> $INSTALLED_BUNDLE  (${elapsed}s)"
echo "Restart or rescan your host to load it."
```

Then make it executable:

```bash
chmod +x engine/scripts/install_clap.sh
```

- [ ] **Step 2: Syntax check (hard gate)**

Run: `bash -n engine/scripts/install_clap.sh`
Expected: no output, exit `0`.

- [ ] **Step 3: Lint (only if shellcheck is installed)**

Run: `command -v shellcheck >/dev/null 2>&1 && shellcheck engine/scripts/install_clap.sh || echo "shellcheck not installed — skipped"`
Expected: if shellcheck is present, no warnings; if absent, prints `shellcheck not installed — skipped`. (This environment has no shellcheck, so the skip path is the expected one.)

- [ ] **Step 4: Acceptance test — run the script end-to-end**

This step is **slow** (first Release bundle = a full Rust release build of `stepforge_clap`) and **mutates `~/Library/Audio/Plug-Ins/CLAP/`** — that is the script's purpose; it is reversible (Step 5 removes the installed bundle).

Run: `engine/scripts/install_clap.sh`
Expected: script prints `[bundle] ...`, `[install] copying to /Users/<you>/Library/Audio/Plug-Ins/CLAP/stepforge_clap.clap ...`, `[done] installed -> ...`, exit `0`.

- [ ] **Step 5: Assert the install landed, then clean up**

Assert the bundle is installed as a directory:

```bash
test -d "$HOME/Library/Audio/Plug-Ins/CLAP/stepforge_clap.clap" && echo "OK installed" || echo "MISS"
```
Expected: `OK installed`.

Assert quarantine was stripped (no `com.apple.quarantine` on the bundle):

```bash
xattr "$HOME/Library/Audio/Plug-Ins/CLAP/stepforge_clap.clap" 2>/dev/null | grep -q com.apple.quarantine && echo "FAIL: quarantine present" || echo "OK no quarantine"
```
Expected: `OK no quarantine`.

Remove the installed bundle to leave the system clean (the script will recreate it next run):

```bash
rm -rf "$HOME/Library/Audio/Plug-Ins/CLAP/stepforge_clap.clap"
```

- [ ] **Step 6: Commit**

```bash
git add engine/scripts/install_clap.sh
git commit -m "feat(scripts): add install_clap.sh CLAP plugin installer"
```

---

## Task 2: Rewrite `README.md` `## Getting started`

**Files:**
- Modify: `README.md` — replace the current `## Getting started` block (begins at the `## Getting started` heading, ends immediately before the `## Tests` heading).

**Interfaces:**
- Consumes: Task 1's `engine/scripts/install_clap.sh` (referenced by exact path in the CLAP track). All other command lines already exist in `CLAUDE.md` / the current README.
- Produces: a README whose getting-started section has two H3 tracks and a chooser. Downstream: none (this is the terminal deliverable).

- [ ] **Step 1: Failing structure assertion — confirm the new tracks are not yet present**

Run: `grep -c '^### Swift app + AUv3 (iOS + macOS)$' README.md`
Expected: `0` (heading not present yet).

Run: `grep -c '^### CLAP plugin (macOS)$' README.md`
Expected: `0`.

- [ ] **Step 2: Replace the `## Getting started` block**

In `README.md`, replace everything from the line `## Getting started` up to (but **not** including) the line `## Tests` with this exact content:

````markdown
## Getting started

StepForge ships as two independent surfaces that build separately. Pick one:

- **Swift app + AUv3 (iOS & macOS)** — SwiftUI/Rust standalone apps plus the
  AUv3 host-driven MIDI effect. Needs Xcode + XcodeGen + a rustup toolchain with
  iOS targets. See [Swift app + AUv3](#swift-app--auv3-ios--macos) below.
- **CLAP plugin (macOS)** — pure-Rust `nih-plug` + `egui`, no Swift, no C ABI.
  Needs only a rustup toolchain. See [CLAP plugin](#clap-plugin-macos) below.

### Swift app + AUv3 (iOS + macOS)

One-time setup (rustup-managed toolchain required — Homebrew rust can't
cross-compile to iOS — plus Xcode + Command Line Tools):

```bash
brew install xcodegen                 # generates the Xcode project from app/project.yml
engine/scripts/setup.sh               # rustup stable + iOS/macOS targets + cbindgen
```

Build the engine (the app's prebuild script runs this too):

```bash
engine/scripts/build_engine.sh        # -> engine/dist/SequencerEngine.xcframework
                                      #    engine/include/sequencer_engine.h
```

Generate the (gitignored) Xcode project and build a target:

```bash
cd app && xcodegen generate           # -> app/StepForge.xcodeproj

# iOS (simulator)
xcodebuild -project app/StepForge.xcodeproj -scheme StepForge \
  -destination 'generic/platform=iOS Simulator' CODE_SIGNING_ALLOWED=NO build

# macOS
xcodebuild -project app/StepForge.xcodeproj -scheme StepForge-macOS \
  -destination 'platform=macOS' CODE_SIGNING_ALLOWED=NO build
```

Install the standalone macOS app into `~/Applications` (clean build + ad-hoc sign
+ quarantine strip):

```bash
./build_install_macos.sh              # SKIP_RUST_CLEAN=1 keeps engine/target for fast Swift-only cycles
```

> **AUv3:** `StepForgeAU` is an app-extension built automatically inside the
> macOS target — no separate build or install command. The host registers it
> when the macOS app launches. AUv3 is macOS-only (the iOS target excludes
> `AudioUnit/`).

### CLAP plugin (macOS)

Pure Rust — no Xcode, xcodegen, cbindgen, or iOS targets. One-time setup is just
the rustup toolchain + macOS targets:

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
```

Build and test the editor UI, then bundle the plugin:

```bash
cd engine
cargo test -p stepforge_editor_egui                 # editor UI tests (pure-egui, host-free)
cargo clippy -p stepforge_editor_egui --all-targets -- -D warnings
cargo xtask bundle -p stepforge_clap --release      # -> engine/target/bundled/stepforge_clap.clap
```

Install it into `~/Library/Audio/Plug-Ins/CLAP/` (bundles, copies, strips
quarantine):

```bash
engine/scripts/install_clap.sh        # -> ~/Library/Audio/Plug-Ins/CLAP/stepforge_clap.clap
```

Restart or rescan your host to load it.

````

(The outer `````markdown fence is only to display the block here — the README file content is the inner text, from `## Getting started` through the blank line before `## Tests`.)

- [ ] **Step 3: Passing structure assertion — both tracks now present**

Run: `grep -c '^### Swift app + AUv3 (iOS + macOS)$' README.md`
Expected: `1`.

Run: `grep -c '^### CLAP plugin (macOS)$' README.md`
Expected: `1`.

- [ ] **Step 4: Assert the chooser anchor links and the install_clap.sh reference exist**

The chooser links point at the GitHub-generated heading anchors. For these exact headings the slugs are deterministic:
- `### Swift app + AUv3 (iOS + macOS)` → `#swift-app--auv3-ios--macos` (the `+` symbols are stripped, leaving double hyphens — correct).
- `### CLAP plugin (macOS)` → `#clap-plugin-macos`.

Assert both link targets and the script reference are present:

```bash
grep -F '(#swift-app--auv3-ios--macos)' README.md && grep -F '(#clap-plugin-macos)' README.md && grep -F 'engine/scripts/install_clap.sh' README.md
```
Expected: prints all three matched lines, exit `0`.

- [ ] **Step 5: Assert the old flat structure is gone**

The old section had a standalone paragraph starting `One-time setup` immediately under `## Getting started` with no intervening chooser. Confirm the chooser now separates them, and the old lone `Build and test the CLAP plugin` line is gone (it moved into the CLAP track as a comment-led block):

```bash
grep -c 'Build and test the CLAP plugin' README.md
```
Expected: `0` (that exact old lead-in phrase is no longer present).

- [ ] **Step 6: Sanity-render check**

Run: `grep -n '^## \|^### ' README.md`
Expected: the heading list shows `## Getting started`, then `### Swift app + AUv3 (iOS + macOS)`, then `### CLAP plugin (macOS)`, then `## Tests` — in that order, with no duplicate `## Getting started`.

- [ ] **Step 7: Commit**

```bash
git add README.md
git commit -m "docs(readme): split getting-started into Swift/AUv3 and CLAP tracks"
```

---

## Self-Review

**1. Spec coverage**
- Decision 1 (add `install_clap.sh`) → Task 1. ✓
- Decision 2 (minimal CLAP setup, no `setup.sh`) → Task 2 CLAP track uses `rustup target add` only. ✓
- Decision 3 (AUv3 = note, no commands) → Task 2 AUv3 blockquote. ✓
- One H2 + chooser + two H3 structure → Task 2 Step 2. ✓
- CLAP install gap closed (`install_clap.sh` referenced) → Task 1 + Task 2 Step 4. ✓
- "Out of scope" (no `CLAUDE.md`/Rust/Swift/spec/license changes) → no task touches them. ✓

**2. Placeholder scan**
- No TBD/TODO. Every code/command step shows full content. ✓
- The README-replace step shows the entire block verbatim (no "similar to"). ✓
- shellcheck is conditional on `command -v`, not a placeholder. ✓

**3. Consistency**
- Script path `engine/scripts/install_clap.sh` identical in Task 1 (Files/Produces), Task 2 Step 2 (CLAP track), Task 2 Step 4 (grep). ✓
- Bundle name `stepforge_clap.clap` identical in script body, Step 4/5 asserts, README comment. ✓
- Headings `### Swift app + AUv3 (iOS + macOS)` / `### CLAP plugin (macOS)` identical in Step 2 content and Step 3/4 greps. ✓
- `cargo xtask bundle -p stepforge_clap --release` matches `CLAUDE.md` and the pkg name confirmed in `crates/clap_plugin/Cargo.toml`. ✓
- No types to drift (docs + bash only). ✓

No issues found.
