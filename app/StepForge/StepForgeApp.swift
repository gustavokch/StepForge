import SwiftUI

@main
struct StepForgeApp: App {
    /// Single engine bridge, owned for the app's lifetime. `@StateObject` keeps it
    /// alive across re-renders; injected into the view tree as an `@EnvironmentObject`
    /// so feature views observe `mirror` and submit commands through it.
    @StateObject private var bridge = EngineBridge()

    /// Owns inbound CoreMIDI (the MIDI-Clock input client) for the app's lifetime.
    /// MUST outlive the Settings sheet: previously this lived as a `@StateObject`
    /// inside `SettingsSheet`, so the input client was disposed on sheet dismiss
    /// and inbound MIDI Clock sync only worked while Settings was open. Hoisting
    /// it here (and binding it to the bridge in `RootView`) keeps the input alive
    /// for the whole session (Defect 1+2 fix).
    @StateObject private var midiManager = MidiManager()

    var body: some Scene {
        WindowGroup {
            RootView()
                .environmentObject(bridge)
                .environmentObject(midiManager)
        }
    }
}
