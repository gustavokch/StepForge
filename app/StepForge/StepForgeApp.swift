import SwiftUI

@main
struct StepForgeApp: App {
    var body: some Scene {
        WindowGroup {
            ContentView()
        }
    }
}

struct ContentView: View {
    @State private var engineReady = false
    private let bridge = EngineBridge()

    var body: some View {
        VStack(spacing: 20) {
            Image(systemName: "drum.fill")
                .font(.system(size: 64))
            Text("StepForge")
                .font(.largeTitle.bold())
            Text(engineReady ? "engine: ready" : "engine: starting")
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color.black.opacity(0.05))
        .onAppear {
            bridge.start()
            engineReady = bridge.hasHandle
        }
    }
}
