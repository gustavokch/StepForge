import SwiftUI

/// Environment flag (default `false`) that swaps `TransportBar` →
/// `PluginTransportBar` inside `EditingView`. Additive: standalone bit-for-bit
/// unchanged (default false ⇒ standalone keeps the play/stop/BPM/sync bar).
///
/// Defined here (next to the consumer) rather than in `EditingView` so the
/// plugin transport + its env key ship as one unit. Pure SwiftUI — no platform
/// guards — so it compiles into the iOS/macOS/AU targets alike (EditingView
/// references it unconditionally across all three).
private struct PluginTransportKey: EnvironmentKey {
    static let defaultValue: Bool = false
}

extension EnvironmentValues {
    var usePluginTransport: Bool {
        get { self[PluginTransportKey.self] }
        set { self[PluginTransportKey.self] = newValue }
    }
}

/// Plugin-mode transport row: read-only "Following host" readout (the host owns
/// transport in AU mode) + the 8/16 zoom toggle. Drops the standalone
/// play/stop/BPM-input/sync-source controls — those write transport the host
/// owns, so emitting them from inside a host-driven AU would be a no-op at best
/// and a UX lie at worst. The host's transport state still flows through the
/// mirror (drained on the bridge's ~120 Hz timer), so BPM/step track live.
struct PluginTransportBar: View {
    @EnvironmentObject private var bridge: EngineBridge
    @Binding var visibleSteps: Int

    var body: some View {
        HStack(spacing: Theme.Spacing.sm) {
            followingHostReadout
            Spacer(minLength: Theme.Spacing.xs)
            zoomToggle
        }
        .padding(.horizontal, Theme.Spacing.sm)
        .padding(.vertical, 6)
        .panelStyle(Theme.Surface.highest)
    }

    private var followingHostReadout: some View {
        HStack(spacing: 8) {
            Image(systemName: "music.note")
                .foregroundStyle(bridge.mirror.playing ? Theme.primary : Theme.textMuted)
            VStack(alignment: .leading, spacing: 0) {
                Text("Following host")
                    .font(Typography.controlLabel)
                    .foregroundStyle(Theme.textMuted)
                Text(String(format: "%.1f BPM · step %d",
                            bridge.mirror.bpm,
                            bridge.mirror.playheadStep ?? 0))
                    .font(Typography.bpmLarge)
                    .foregroundStyle(Theme.textPrimary)
            }
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .raisedStyle()
    }

    private var zoomToggle: some View {
        Picker("Zoom", selection: $visibleSteps) {
            Text("8").tag(8)
            Text("16").tag(16)
        }
        .pickerStyle(.segmented)
        .frame(width: 74)
        .labelsHidden()
    }
}
