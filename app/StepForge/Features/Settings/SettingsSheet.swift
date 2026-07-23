import SwiftUI

/// Settings sheet (Phase 1 placeholder): MIDI routing, sync source, and global
/// MIDI channel live here in Phase 3 (CoreMIDI discovery + Ableton Link + MIDI
/// Clock — all Swift-owned, Hard Rule 7). For now it surfaces live mirror state.
struct SettingsSheet: View {
    @EnvironmentObject private var bridge: EngineBridge
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                Section {
                    LabeledContent("BPM", value: String(format: "%.1f", bridge.mirror.bpm))
                    LabeledContent("MIDI Channel", value: "\(bridge.mirror.globalMidiChannel)")
                    LabeledContent("Sync Source", value: bridge.mirror.syncSource.label)
                    LabeledContent("Swing", value: "\(Int((bridge.mirror.globalSwingPct * 100).rounded()))%")
                } header: { Text("Session") }

                Section {
                    Text("MIDI device selection — Phase 3").foregroundStyle(.secondary)
                } header: { Text("MIDI Routing") }

                Section {
                    Text("Free / MIDI Clock / Link — Phase 3").foregroundStyle(.secondary)
                } header: { Text("Sync") }
            }
            .navigationTitle("Settings")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Done") { dismiss() }.tint(Theme.primary)
                }
            }
        }
        .preferredColorScheme(.dark)
    }
}
