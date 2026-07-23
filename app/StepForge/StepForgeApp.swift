import SwiftUI

@main
struct StepForgeApp: App {
    /// Single engine bridge, owned for the app's lifetime. `@StateObject` keeps it
    /// alive across re-renders; injected into the view tree as an `@EnvironmentObject`
    /// so feature views observe `mirror` and submit commands through it.
    @StateObject private var bridge = EngineBridge()

    var body: some Scene {
        WindowGroup {
            RootView()
                .environmentObject(bridge)
        }
    }
}
