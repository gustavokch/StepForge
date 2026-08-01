# README Getting Started — split by surface

**Date:** 2026-08-01
**Scope:** Rewrite the `## Getting started` section of `README.md` into two
self-contained build/install tracks — one per shipping surface — and add one
helper script the CLAP track references.

## Problem

The current `## Getting started` (README lines 115–158) is a single flat list
that interleaves two unrelated build pipelines:

- the **SwiftUI/Rust app + AUv3** surface (Xcode, XcodeGen, rustup with iOS
  targets, the `SequencerEngine.xcframework`, `build_install_macos.sh`), and
- the **pure-Rust egui/CLAP** plugin surface (`cargo xtask bundle`, no Swift,
  no C ABI).

A reader who wants only the CLAP plugin is dragged through Xcode + iOS-target
setup it does not need. A reader who wants the app has to skip the CLAP block.
There is also a real gap: the CLAP track has a build command
(`cargo xtask bundle -p stepforge_clap --release`) but **no install step** — no
script copies the produced `.clap` into `~/Library/Audio/Plug-Ins/CLAP/`, and
the README stops at the bundle output path.

## Decisions (resolved during brainstorming)

1. **CLAP install — add a helper script.** Create
   `engine/scripts/install_clap.sh`, symmetric with `build_install_macos.sh`,
   rather than documenting a bare `cp` convention or stopping at the bundle.
   Rationale: matches the app surface's install story (one script), and a script
   can assert the bundle exists, `mkdir -p` the destination, and strip
   quarantine in one place.
2. **Setup split — minimal CLAP setup.** The CLAP track tells the reader to run
   `rustup target add aarch64-apple-darwin x86_64-apple-darwin` only. It does
   **not** require Xcode, XcodeGen, cbindgen, or iOS targets, so it does not
   point at `engine/scripts/setup.sh` (which over-provisions all of those for a
   CLAP-only developer). The Swift track keeps `setup.sh`.
3. **AUv3 — a note, not a separate step.** `StepForgeAU` is an app-extension
   built automatically inside the macOS target; it has no standalone build or
   install command. Getting-started covers this with one blockquote note. The
   fiddly registration behavior (rebuilding over a registered appex can
   tombstone `auval`) stays in **Known limits**, not getting-started.

## Structure

`## Getting started` becomes one H2 with a two-bullet chooser, then two H3
tracks. No duplicate H2 titles (keeps GitHub TOC/nav clean). `## Tests` and all
other existing sections are untouched.

```text
## Getting started
  chooser: two bullets, one per surface, anchor-link to its H3
  ### Swift app + AUv3 (iOS + macOS)
    one-time setup     brew install xcodegen; engine/scripts/setup.sh
    build engine       engine/scripts/build_engine.sh
    gen + build        cd app && xcodegen generate; xcodebuild iOS / macOS
    install macOS      ./build_install_macos.sh   (+ SKIP_RUST_CLEAN note)
    > AUv3 note        built inside macOS target; host registers on launch
  ### CLAP plugin (macOS)
    one-time setup     rustup target add <two macOS targets>
    build + test       cargo test / clippy -p stepforge_editor_egui;
                       cargo xtask bundle -p stepforge_clap --release
    install            engine/scripts/install_clap.sh
    load hint          restart / rescan host
```

Each track is self-contained: a reader following one track never needs to read
the other.

## Artifacts

### 1. `engine/scripts/install_clap.sh` (new)

Symmetric with `build_install_macos.sh`. Steps, in order:

1. `set -euo pipefail`; `export PATH="$HOME/.cargo/bin:$PATH"` (Homebrew-rust
   shadow fix, mirrors `setup.sh` / `build_engine.sh`).
2. Resolve `ENGINE_DIR` from the script's own location; `cd "$ENGINE_DIR"`.
3. Preflight: `command -v cargo` or fail with a clear message.
4. `cargo xtask bundle -p stepforge_clap --release`.
5. Assert
   `engine/target/bundled/stepforge_clap.clap` exists; fail otherwise.
6. `CLAP_DIR="$HOME/Library/Audio/Plug-Ins/CLAP"`; `mkdir -p "$CLAP_DIR"`.
7. `rm -rf` the previously installed bundle, then
   `ditto "$BUNDLE" "$CLAP_DIR/stepforge_clap.clap"` (preserves bundle
   structure — same tool the app script uses).
8. `xattr -cr` the installed bundle (unsigned local build; strip quarantine,
   same rationale as `build_install_macos.sh`).
9. Print the installed path and a "restart or rescan your host to load it"
   hint. Mirror the app script's `log` / `start` / `elapsed` style.

No flags. `cargo xtask bundle` is cargo-incremental, so there is no equivalent
of the app script's `SKIP_RUST_CLEAN` (which exists only because that script
wipes `engine/target`). Roughly 30 lines.

### 2. `README.md` `## Getting started` rewrite

Replaces lines 115–158 verbatim. The replacement body is the structure above,
rendered as: chooser bullets → two H3 tracks → fenced `bash` blocks per step →
one `>` AUv3 blockquote under the Swift track.

Notes on the rewrite:

- **Commands lifted verbatim** from the current README / `CLAUDE.md` wherever
  they already exist (`brew install xcodegen`, `setup.sh`, `build_engine.sh`,
  `xcodegen generate`, both `xcodebuild` invocations,
  `build_install_macos.sh`, the three CLAP `cargo` lines). The only genuinely
  new command line is `engine/scripts/install_clap.sh`.
- **`cargo test -p stepforge_editor_egui`** appears in the CLAP track for
  context, and remains in `## Tests` as part of the full test suite — this is
  not duplication, it is the CLAP track's natural build/verify step.
- **Chooser anchor links** use GitHub's auto-generated heading anchors.
  Heading text is kept anchor-safe and the links are verified by rendering the
  README during implementation.
- **AUv3** = one blockquote, no commands (Decision 3).
- **CLAP setup** = minimal `rustup target add` (Decision 2).

## Out of scope

- No changes to `CLAUDE.md`, the specs, or any Rust/Swift source.
- No CI, no signed-release story (already listed under Known limits).
- AUv3 registration troubleshooting stays in Known limits, not here.
- No VST3 getting-started (not shipped).
