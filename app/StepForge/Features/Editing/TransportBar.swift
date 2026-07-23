import SwiftUI

/// Top transport row (ui-ux-spec §1.1 Row 1): play/stop, BPM (type to edit),
/// sync-source badge, and the 8/16 zoom toggle. Styled as the highest surface tier.
struct TransportBar: View {
    @EnvironmentObject private var bridge: EngineBridge
    @Binding var visibleSteps: Int

    @State private var bpmText: String = ""
    @FocusState private var bpmFocused: Bool

    var body: some View {
        HStack(spacing: Theme.Spacing.sm) {
            playStop
            bpmControl
            syncBadge
            Spacer(minLength: Theme.Spacing.xs)
            zoomToggle
        }
        .padding(.horizontal, Theme.Spacing.sm)
        .padding(.vertical, 6)
        .panelStyle(Theme.Surface.highest)
        .onAppear { syncBpmText(force: true) }
        .onChange(of: bridge.mirror.bpm) { _, new in syncBpmText(force: false, bpm: new) }
    }

    private var playStop: some View {
        Button {
            bridge.submit(bridge.mirror.playing ? .stop : .play)
        } label: {
            Image(systemName: bridge.mirror.playing ? "stop.fill" : "play.fill")
                .font(.title3)
                .foregroundStyle(bridge.mirror.playing ? Theme.primary : Theme.textPrimary)
                .frame(width: 34, height: 30)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .raisedStyle()
    }

    private var bpmControl: some View {
        HStack(spacing: 4) {
            Text("BPM").font(Typography.controlLabel).foregroundStyle(Theme.textMuted)
            TextField("120", text: $bpmText)
                .font(Typography.bpmLarge)
                .foregroundStyle(Theme.textPrimary)
                .multilineTextAlignment(.leading)
                .frame(width: 74)
                #if os(iOS)
                .keyboardType(.decimalPad)
                #endif
                .focused($bpmFocused)
                .onSubmit(commitBpm)
                .onChange(of: bpmFocused) { _, focused in if !focused { commitBpm() } }
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .raisedStyle()
    }

    private var syncBadge: some View {
        Button { cycleSync() } label: {
            Label(bridge.mirror.syncSource.label, systemImage: syncIcon)
                .labelStyle(.titleAndIcon)
                .font(Typography.controlLabel)
                .foregroundStyle(Theme.textSecondary)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .padding(.horizontal, 8)
        .padding(.vertical, 6)
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

    // MARK: Actions

    private func commitBpm() {
        if let v = Double(bpmText) { bridge.submit(.setBpm(bpm: v)) }
        bpmFocused = false
    }

    /// Quick cycle (full sync UI lands in Phase 3 Settings).
    private func cycleSync() {
        let order: [SyncSource] = [.free, .link, .midiClock]
        let i = order.firstIndex(of: bridge.mirror.syncSource) ?? 0
        bridge.submit(.setSyncSource(source: order[(i + 1) % order.count]))
    }

    private var syncIcon: String {
        switch bridge.mirror.syncSource {
        case .free: "infinity"
        case .link: "link"
        case .midiClock: "metronome"
        }
    }

    private func syncBpmText(force: Bool, bpm: Double? = nil) {
        let value = bpm ?? bridge.mirror.bpm
        guard force || !bpmFocused else { return }
        bpmText = String(format: "%.1f", value)
    }
}
