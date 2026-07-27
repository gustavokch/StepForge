#if os(macOS)
import SwiftUI

/// Plugin editor: a trimmed `RootView` hosted inside the AU window. Drops the
/// app-shell concerns that don't apply inside a host-driven AU:
///   - no `MidiManager` (the host routes MIDI; the AU emits via
///     `midiOutputEventBlock` and consumes via `AURenderEvent`s),
///   - no `EngineLifecycle`/scene-phase handoff (the AU owns the engine; the
///     host's open/close drives `allocateRenderResources`/`deallocate…`),
///   - no Settings sheet (SettingsSheet references `MidiManager` and is excluded
///     from the AU target; full plugin settings land in a later phase),
///   - no separate PluginTransportBar at this level — `EditingView` renders it
///     via the `\.usePluginTransport` env flag (single source of transport,
///     `visibleSteps` lives where the grid reads it).
///
/// Bound to the borrowed `EngineBridge` the AU owns (injected via
/// `\.environmentObject` by `StepForgeEditorViewController`). Gestures submit
/// commands; the bridge's ~120 Hz drain refreshes the mirror that SwiftUI reads.
struct PluginEditorView: View {
    @EnvironmentObject private var bridge: EngineBridge
    @State private var mode: AppMode = .editing

    var body: some View {
        VStack(spacing: 6) {
            Group {
                switch mode {
                case .editing: EditingView()
                case .performance: PerformanceView()
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .background(Theme.Surface.lowest)
        .environment(\.usePluginTransport, true)
        .preferredColorScheme(.dark)
        .tint(Theme.primary)
        .frame(minWidth: 520, minHeight: 360)
        .toolbar { ToolbarItem(placement: .navigation) { modeToggle } }
    }

    private var modeToggle: some View {
        Button {
            withAnimation(.easeInOut(duration: 0.18)) {
                mode = (mode == .editing ? .performance : .editing)
            }
        } label: {
            Image(systemName: mode == .editing ? "square.grid.2x2.fill" : "pencil")
                .foregroundStyle(mode == .editing ? Theme.primary : Theme.textSecondary)
        }
    }
}
#endif
