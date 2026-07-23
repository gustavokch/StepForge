# Native macOS Target Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a native macOS target (`StepForge-macOS`) to StepForge so the app compiles and runs natively on macOS 14.0+ (Sonoma) in addition to iOS 17.0+.

**Architecture:** Rust engine compiled for macOS host targets (`aarch64-apple-darwin`, `x86_64-apple-darwin`), merged into universal `libsequencer_engine_ffi_macos.a`, and packaged inside `SequencerEngine.xcframework`. XcodeGen (`project.yml`) updated to generate a `StepForge-macOS` app target sharing 100% of SwiftUI views, view models, and FFI bindings. `Haptics.swift` conditionally guarded for macOS compatibility.

**Tech Stack:** Rust (cargo, rustup, cbindgen, lipo), XcodeGen, Swift, SwiftUI, AppKit / UIKit conditional compilation, xcodebuild.

---

## Global Constraints
- Target macOS version: `14.0`
- Target iOS version: `17.0`
- Target architecture: Apple Silicon (`aarch64`) and Intel (`x86_64`)
- Swift version: `5.0`
- CoreMIDI ownership remains in Swift (`EngineBridge`) across both platforms
- Unsafe isolation rules remain unchanged

---

## Proposed Changes

### Component 1: Engine Toolchain & Build Pipeline (`engine/`)

#### [MODIFY] [rust-toolchain.toml](file:///Users/gus/Git/StepForge/engine/rust-toolchain.toml)
Add macOS target triples `aarch64-apple-darwin` and `x86_64-apple-darwin`.

#### [MODIFY] [setup.sh](file:///Users/gus/Git/StepForge/engine/scripts/setup.sh)
Update setup script to verify and install macOS targets via `rustup`.

#### [MODIFY] [build_engine.sh](file:///Users/gus/Git/StepForge/engine/scripts/build_engine.sh)
Compile macOS slices, `lipo` them into a universal simulator/host archive, and add the macOS library slice to `xcodebuild -create-xcframework`.

---

### Component 2: Swift Shell Platform Compatibility (`app/StepForge/`)

#### [MODIFY] [Haptics.swift](file:///Users/gus/Git/StepForge/app/StepForge/Gestures/Haptics.swift)
Add `#if os(iOS)` / `#if os(macOS)` conditional compilation guards so `UIKit` haptic feedback generators are used on iOS and safe fallback/AppKit haptics (`NSHapticFeedbackManager`) are used on macOS.

---

### Component 3: Xcode Project Generation (`app/`)

#### [MODIFY] [project.yml](file:///Users/gus/Git/StepForge/app/project.yml)
Declare `macOS: "14.0"` under `deploymentTarget` and add a new `StepForge-macOS` native target.

---

## Detailed Task Breakdown

### Task 1: Declare macOS targets in Rust toolchain & setup script

**Files:**
- Modify: `engine/rust-toolchain.toml:5-9`
- Modify: `engine/scripts/setup.sh:21-34`

- [ ] **Step 1: Update `engine/rust-toolchain.toml`**

Add `aarch64-apple-darwin` and `x86_64-apple-darwin` to targets:
```toml
[toolchain]
channel = "stable"
profile = "minimal"
components = ["rust-src"]
targets = [
    "aarch64-apple-ios",
    "aarch64-apple-ios-sim",
    "x86_64-apple-ios",
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
]
```

- [ ] **Step 2: Update `engine/scripts/setup.sh`**

Update `setup.sh` target list:
```bash
echo "[setup] adding iOS and macOS targets ..."
rustup target add --toolchain stable \
  aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios \
  aarch64-apple-darwin x86_64-apple-darwin

echo "[setup] installing cbindgen (if missing) ..."
if ! command -v cbindgen >/dev/null 2>&1; then
  cargo install cbindgen --locked
fi

echo "[setup] verifying targets ..."
for t in aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios aarch64-apple-darwin x86_64-apple-darwin; do
  rustup target list --installed | grep -q "$t" || { echo "ERROR: target $t missing" >&2; exit 1; }
done
```

- [ ] **Step 3: Run setup script and verify installation**

Run: `engine/scripts/setup.sh`
Expected: `[setup] done.` with all 5 targets verified.

- [ ] **Step 4: Commit setup changes**

```bash
git add engine/rust-toolchain.toml engine/scripts/setup.sh
git commit -m "build: add macOS target triples to Rust toolchain setup"
```

---

### Task 2: Build macOS engine slices into SequencerEngine.xcframework

**Files:**
- Modify: `engine/scripts/build_engine.sh`

- [ ] **Step 1: Modify `engine/scripts/build_engine.sh` to compile macOS targets & assemble universal archive**

```bash
echo "[build] compiling iOS & macOS slices (release) ..."
cargo build --release --target aarch64-apple-ios
cargo build --release --target aarch64-apple-ios-sim
cargo build --release --target x86_64-apple-ios
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin

LIB_DEVICE="target/aarch64-apple-ios/release/libsequencer_engine_ffi.a"
LIB_SIM_ARM="target/aarch64-apple-ios-sim/release/libsequencer_engine_ffi.a"
LIB_SIM_X86="target/x86_64-apple-ios/release/libsequencer_engine_ffi.a"
LIB_MAC_ARM="target/aarch64-apple-darwin/release/libsequencer_engine_ffi.a"
LIB_MAC_X86="target/x86_64-apple-darwin/release/libsequencer_engine_ffi.a"

echo "[build] merging simulator and macOS slices (lipo) ..."
mkdir -p target/xcframework-staging
LIB_SIM_UNIVERSAL="target/xcframework-staging/libsequencer_engine_ffi_sim.a"
LIB_MAC_UNIVERSAL="target/xcframework-staging/libsequencer_engine_ffi_mac.a"

lipo -create "$LIB_SIM_ARM" "$LIB_SIM_X86" -output "$LIB_SIM_UNIVERSAL"
lipo -create "$LIB_MAC_ARM" "$LIB_MAC_X86" -output "$LIB_MAC_UNIVERSAL"

echo "[build] assembling xcframework ..."
rm -rf "$DIST_DIR/SequencerEngine.xcframework"
xcodebuild -create-xcframework \
  -library "$LIB_DEVICE"        -headers "$INCLUDE_DIR" \
  -library "$LIB_SIM_UNIVERSAL" -headers "$INCLUDE_DIR" \
  -library "$LIB_MAC_UNIVERSAL" -headers "$INCLUDE_DIR" \
  -output "$DIST_DIR/SequencerEngine.xcframework"
```

- [ ] **Step 2: Run `build_engine.sh` and verify xcframework creation**

Run: `engine/scripts/build_engine.sh`
Expected: `[build] done -> engine/dist/SequencerEngine.xcframework`

- [ ] **Step 3: Inspect xcframework slices with lipo and file**

Run: `lipo -info engine/dist/SequencerEngine.xcframework/macos-arm64_x86_64/libsequencer_engine_ffi.a` (or appropriate slice path)
Expected: Architectural output containing `arm64` and `x86_64`.

- [ ] **Step 4: Commit build script updates**

```bash
git add engine/scripts/build_engine.sh
git commit -m "build: include universal macOS static library in SequencerEngine.xcframework"
```

---

### Task 3: Wrap iOS-specific Haptics APIs with platform conditional compilation

**Files:**
- Modify: `app/StepForge/Gestures/Haptics.swift`

- [ ] **Step 1: Update `Haptics.swift` for cross-platform compilation**

```swift
#if os(iOS)
import UIKit

/// Haptic feedback for iOS. Fired on MainActor from gesture handlers.
enum Haptics {
    private static let impact = UIImpactFeedbackGenerator(style: .light)
    private static let notice = UINotificationFeedbackGenerator()

    static func prepare() {
        impact.prepare(); notice.prepare()
    }

    static func zoneCross() { impact.impactOccurred() }
    static func delete() { impact.impactOccurred(intensity: 0.7) }
    static func confirm() { notice.notificationOccurred(.success) }
}
#elseif os(macOS)
import AppKit

/// Haptic feedback for macOS.
enum Haptics {
    static func prepare() {}

    static func zoneCross() {
        NSHapticFeedbackManager.defaultPerformer.perform(.alignment, performanceTime: .default)
    }

    static func delete() {
        NSHapticFeedbackManager.defaultPerformer.perform(.generic, performanceTime: .default)
    }

    static func confirm() {
        NSHapticFeedbackManager.defaultPerformer.perform(.generic, performanceTime: .default)
    }
}
#else
enum Haptics {
    static func prepare() {}
    static func zoneCross() {}
    static func delete() {}
    static func confirm() {}
}
#endif
```

- [ ] **Step 2: Commit Haptics updates**

```bash
git add app/StepForge/Gestures/Haptics.swift
git commit -m "feat: add macOS platform conditional implementation for Haptics"
```

---

### Task 4: Configure `StepForge-macOS` target in `app/project.yml`

**Files:**
- Modify: `app/project.yml`

- [ ] **Step 1: Update `app/project.yml` to include macOS deployment target and `StepForge-macOS` target**

```yaml
name: StepForge
options:
  bundleIdPrefix: com.stepforge
  deploymentTarget:
    iOS: "17.0"
    macOS: "14.0"
  createIntermediateGroups: true
settings:
  base:
    SWIFT_VERSION: "5.0"
    CODE_SIGNING_ALLOWED: "NO"
    CODE_SIGNING_REQUIRED: "NO"
targets:
  StepForge:
    type: application
    platform: iOS
    sources:
      - path: StepForge
    settings:
      base:
        PRODUCT_BUNDLE_IDENTIFIER: com.stepforge.app
        TARGETED_DEVICE_FAMILY: "1,2"
        GENERATE_INFOPLIST_FILE: "YES"
        INFOPLIST_KEY_UILaunchScreen_Generation: "YES"
        INFOPLIST_KEY_UIApplicationSceneManifest_Generation: "YES"
        INFOPLIST_KEY_UIUserInterfaceStyle: "Dark"
        SWIFT_OBJC_BRIDGING_HEADER: "$(SRCROOT)/../engine/include/sequencer_engine.h"
        USER_HEADER_SEARCH_PATHS: "$(inherited) $(SRCROOT)/../engine/include"
        HEADER_SEARCH_PATHS: "$(inherited) $(SRCROOT)/../engine/include"
        FRAMEWORK_SEARCH_PATHS: "$(inherited) $(SRCROOT)/../engine/dist"
    dependencies:
      - framework: ../engine/dist/SequencerEngine.xcframework
        embed: false
      - sdk: CoreMIDI.framework
    preBuildScripts:
      - script: 'bash "$SRCROOT/../engine/scripts/build_engine.sh"'
        name: Build Rust Engine
        shell: /bin/sh
  StepForge-macOS:
    type: application
    platform: macOS
    sources:
      - path: StepForge
    settings:
      base:
        PRODUCT_BUNDLE_IDENTIFIER: com.stepforge.app.mac
        MACOSX_DEPLOYMENT_TARGET: "14.0"
        GENERATE_INFOPLIST_FILE: "YES"
        SWIFT_OBJC_BRIDGING_HEADER: "$(SRCROOT)/../engine/include/sequencer_engine.h"
        USER_HEADER_SEARCH_PATHS: "$(inherited) $(SRCROOT)/../engine/include"
        HEADER_SEARCH_PATHS: "$(inherited) $(SRCROOT)/../engine/include"
        FRAMEWORK_SEARCH_PATHS: "$(inherited) $(SRCROOT)/../engine/dist"
    dependencies:
      - framework: ../engine/dist/SequencerEngine.xcframework
        embed: false
      - sdk: CoreMIDI.framework
    preBuildScripts:
      - script: 'bash "$SRCROOT/../engine/scripts/build_engine.sh"'
        name: Build Rust Engine
        shell: /bin/sh
  StepForgeTests:
    type: bundle.unit-test
    platform: iOS
    sources:
      - path: StepForgeTests
    dependencies:
      - target: StepForge
```

- [ ] **Step 2: Generate project using XcodeGen**

Run: `cd app && xcodegen generate`
Expected: `Created project at /Users/gus/Git/StepForge/app/StepForge.xcodeproj`

- [ ] **Step 3: Commit project configuration**

```bash
git add app/project.yml
git commit -m "build: add StepForge-macOS target definition to project.yml"
```

---

### Task 5: Verify build for iOS Simulator & Native macOS

- [ ] **Step 1: Build iOS Simulator Target**

Run: `xcodebuild -project app/StepForge.xcodeproj -scheme StepForge -destination 'generic/platform=iOS Simulator' CODE_SIGNING_ALLOWED=NO build`
Expected: `** BUILD SUCCEEDED **`

- [ ] **Step 2: Build Native macOS Target**

Run: `xcodebuild -project app/StepForge.xcodeproj -scheme StepForge-macOS -destination 'platform=macOS' CODE_SIGNING_ALLOWED=NO build`
Expected: `** BUILD SUCCEEDED **`

- [ ] **Step 3: Run Rust Engine Unit Tests**

Run: `cd engine && cargo test`
Expected: All engine host tests PASS on macOS.

---

## Verification Plan

### Automated Tests
1. `engine/scripts/setup.sh` - verify rustup target list.
2. `engine/scripts/build_engine.sh` - build multi-target xcframework.
3. `cd app && xcodegen generate` - generate Xcode project.
4. `xcodebuild -project app/StepForge.xcodeproj -scheme StepForge -destination 'generic/platform=iOS Simulator' CODE_SIGNING_ALLOWED=NO build` - verify iOS build.
5. `xcodebuild -project app/StepForge.xcodeproj -scheme StepForge-macOS -destination 'platform=macOS' CODE_SIGNING_ALLOWED=NO build` - verify macOS build.
6. `cd engine && cargo test` - verify Rust host tests pass.

### Manual Verification
1. Launch native macOS build artifact (`StepForge-macOS.app`) or inspect build products.
2. Verify MIDI discovery and playback work natively on macOS.
