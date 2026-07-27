import SwiftUI

/// Root shell: a slim top app bar (wordmark + mode toggle + settings) over the
/// active mode's content (ui-ux-spec §1 — Editing default ⇄ Performance, toggled
/// by a corner icon; MIDI/sync behind a Settings sheet). Owns the scene-phase →
/// engine-lifecycle handoff (Hard Rule 5).
struct RootView: View {
    @EnvironmentObject private var bridge: EngineBridge
    @EnvironmentObject private var midiManager: MidiManager
    @Environment(\.scenePhase) private var scenePhase

    @State private var mode: AppMode = .editing
    @State private var showSettings = false

    var body: some View {
        VStack(spacing: 6) {
            appBar
            Group {
                switch mode {
                case .editing: EditingView()
                case .performance: PerformanceView()
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .background(Theme.Surface.lowest)
        .preferredColorScheme(.dark)
        .tint(Theme.primary)
        .onAppear {
            EngineLifecycle.handle(scenePhase, on: bridge)
            // Bind the inbound-MIDI owner to the bridge so `handleMidiInput` can
            // forward clock/start/stop as commands. Without this the weak `bridge`
            // ref stayed nil and every inbound packet was dropped (Defect 1 fix).
            midiManager.bind(to: bridge)
        }
        .onChange(of: scenePhase) { _, phase in
            EngineLifecycle.handle(phase, on: bridge)
        }
        .sheet(isPresented: $showSettings) {
            SettingsSheet()
                .environmentObject(bridge)
                .environmentObject(midiManager)
        }
    }

    private var appBar: some View {
        HStack(spacing: Theme.Spacing.sm) {
            Text("StepForge")
                .font(Typography.trackName)
                .foregroundStyle(Theme.textPrimary)
            Spacer()
            modeToggle
            settingsButton
        }
        .padding(.horizontal, Theme.Spacing.sm)
        .padding(.vertical, 4)
    }

    private var modeToggle: some View {
        Button {
            withAnimation(.easeInOut(duration: 0.18)) { mode = (mode == .editing ? .performance : .editing) }
        } label: {
            Image(systemName: mode == .editing ? "square.grid.2x2.fill" : "pencil")
                .font(.body)
                .foregroundStyle(mode == .editing ? Theme.primary : Theme.textSecondary)
                .frame(width: 30, height: 28)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .raisedStyle()
        .accessibilityLabel(mode == .editing ? "Switch to Performance" : "Switch to Editing")
    }

    private var settingsButton: some View {
        Button { showSettings = true } label: {
            Image(systemName: "slider.horizontal.3")
                .font(.body)
                .foregroundStyle(Theme.textSecondary)
                .frame(width: 30, height: 28)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .raisedStyle()
        .accessibilityLabel("Settings")
    }
}
