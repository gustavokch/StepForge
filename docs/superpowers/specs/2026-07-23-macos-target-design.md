# Design Specification: Native macOS Target for StepForge

**Date:** 2026-07-23  
**Status:** Approved  
**Target Platform:** macOS 14.0 (Sonoma) & iOS 17.0  

---

## 1. Executive Summary

This specification outlines the addition of a native macOS target (`StepForge-macOS`) to the StepForge project. 
The Rust engine (`sequencer_engine_ffi`) will be compiled for macOS host architecture targets (`aarch64-apple-darwin` and `x86_64-apple-darwin`) and bundled into `SequencerEngine.xcframework` alongside the existing iOS target slices. The Xcode app configuration (`app/project.yml`) will define a native macOS target sharing 100% of the SwiftUI feature code, view models, and `EngineBridge` FFI layer with the iOS app. `UIKit`-specific dependencies (specifically haptics) will be conditionally compiled for macOS.

---

## 2. Architecture & Components

```mermaid
graph TD
    subgraph Rust Engine Workspace ("engine/")
        iOS_Slices["iOS Slices\n(aarch64-apple-ios, sim)"]
        Mac_Slices["macOS Slices\n(aarch64-apple-darwin, x86_64-apple-darwin)"]
        lipo_mac["lipo (macOS universal archive)"]
        lipo_sim["lipo (iOS simulator universal archive)"]
        XCFramework["SequencerEngine.xcframework\n(iOS Device + iOS Sim + macOS)"]
        
        iOS_Slices --> lipo_sim
        Mac_Slices --> lipo_mac
        lipo_sim --> XCFramework
        lipo_mac --> XCFramework
    end

    subgraph Xcode Project ("app/project.yml")
        AppTarget_iOS["StepForge (iOS App)"]
        AppTarget_macOS["StepForge-macOS (macOS App)"]
        SharedSwiftUI["Shared SwiftUI Views, EngineBridge, Mirror & Codec"]
        
        XCFramework --> AppTarget_iOS
        XCFramework --> AppTarget_macOS
        SharedSwiftUI --> AppTarget_iOS
        SharedSwiftUI --> AppTarget_macOS
    end
```

---

## 3. Detailed Component Modifications

### 3.1 Rust Toolchain & Target Setup (`engine/`)
- Update `engine/rust-toolchain.toml` to include macOS target triples:
  - `aarch64-apple-darwin`
  - `x86_64-apple-darwin`
- Update `engine/scripts/setup.sh` to install both macOS target triples via `rustup target add`.

### 3.2 Engine Build Script (`engine/scripts/build_engine.sh`)
- Add compilation steps for `aarch64-apple-darwin` and `x86_64-apple-darwin`.
- Merge both macOS slices into a universal macOS archive `target/xcframework-staging/libsequencer_engine_ffi_macos.a` using `lipo`.
- Update `xcodebuild -create-xcframework` invocation to include the universal macOS library:
  ```bash
  xcodebuild -create-xcframework \
    -library "$LIB_DEVICE"        -headers "$INCLUDE_DIR" \
    -library "$LIB_SIM_UNIVERSAL" -headers "$INCLUDE_DIR" \
    -library "$LIB_MAC_UNIVERSAL" -headers "$INCLUDE_DIR" \
    -output "$DIST_DIR/SequencerEngine.xcframework"
  ```

### 3.3 Xcode Project Generation (`app/project.yml`)
- Update deployment targets to declare `macOS: "14.0"`.
- Add a new native macOS target `StepForge-macOS`:
  - `type: application`
  - `platform: macOS`
  - `sources: StepForge`
  - `dependencies`:
    - `framework: ../engine/dist/SequencerEngine.xcframework` (embed: false)
    - `sdk: CoreMIDI.framework`
  - `preBuildScripts`:
    - `Build Rust Engine`: `bash "$SRCROOT/../engine/scripts/build_engine.sh"`
  - `settings`:
    - `PRODUCT_BUNDLE_IDENTIFIER`: `com.stepforge.app.mac`
    - `MACOSX_DEPLOYMENT_TARGET`: `14.0`
    - Header & framework search paths matching iOS target.

### 3.4 Swift Shell & Haptics (`app/StepForge/Gestures/Haptics.swift`)
- Modify `Haptics.swift` to conditionally compile `UIKit` / `UIImpactFeedbackGenerator` calls under `#if os(iOS)` or `#if canImport(UIKit)`.
- On macOS (`#if os(macOS)`), use `NSHapticFeedbackManager.defaultPerformer` or safe no-op functions so that calls to `Haptics.zoneCross()`, `Haptics.delete()`, and `Haptics.confirm()` compile cleanly on macOS.

---

## 4. Verification Plan

1. **Rust Engine Setup & Build Verification**:
   - Run `engine/scripts/setup.sh` to verify rustup target installation.
   - Run `engine/scripts/build_engine.sh` to produce `SequencerEngine.xcframework` containing iOS and macOS slices.
   - Inspect xcframework structure via `xcodebuild -create-xcframework` / `lipo -info` / `file` commands.

2. **Xcode Project & App Build Verification**:
   - Run `cd app && xcodegen generate` to generate `StepForge.xcodeproj`.
   - Build iOS target:
     `xcodebuild -project app/StepForge.xcodeproj -scheme StepForge -destination 'generic/platform=iOS Simulator' CODE_SIGNING_ALLOWED=NO build`
   - Build macOS target:
     `xcodebuild -project app/StepForge.xcodeproj -scheme StepForge-macOS -destination 'platform=macOS' CODE_SIGNING_ALLOWED=NO build`

3. **Runtime & Test Verification**:
   - Run `cargo test` in `engine/` (host tests on macOS).
   - Run iOS unit tests (`StepForgeTests`).
