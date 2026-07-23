import SwiftUI

/// Settings sheet: MIDI routing, sync source, and global MIDI channel.
struct SettingsSheet: View {
    @EnvironmentObject private var bridge: EngineBridge
    @Environment(\.dismiss) private var dismiss
    @StateObject private var midiManager = MidiManager()

    private var globalChannelBinding: Binding<UInt8> {
        Binding(
            get: { bridge.mirror.globalMidiChannel },
            set: { channel in
                bridge.submit(.setGlobalMidiChannel(channel: channel))
            }
        )
    }

    var body: some View {
        NavigationStack {
            List {
                Section {
                    LabeledContent("BPM", value: String(format: "%.1f", bridge.mirror.bpm))
                        .foregroundStyle(Theme.textPrimary)
                    Picker("Global MIDI Channel", selection: globalChannelBinding) {
                        ForEach(UInt8(1)...UInt8(16), id: \.self) { ch in
                            Text("Channel \(ch)\(ch == 10 ? " (GM Drums)" : "")")
                                .tag(ch)
                        }
                    }
                    .foregroundStyle(Theme.textPrimary)
                    LabeledContent("Sync Source", value: bridge.mirror.syncSource.label)
                        .foregroundStyle(Theme.textPrimary)
                    LabeledContent("Swing", value: "\(Int((bridge.mirror.globalSwingPct * 100).rounded()))%")
                        .foregroundStyle(Theme.textPrimary)
                } header: { Text("Session") }

                Section {
                    if midiManager.destinations.isEmpty {
                        Text("No MIDI output destinations found").foregroundStyle(Theme.textMuted)
                    } else {
                        ForEach(midiManager.destinations) { dest in
                            Toggle(
                                dest.name,
                                isOn: Binding(
                                    get: { midiManager.selectedIDs.contains(dest.id) },
                                    set: { _ in midiManager.toggleDestination(dest.id, on: bridge) }
                                )
                            )
                            .tint(Theme.primary)
                        }
                    }

                    Button {
                        midiManager.refreshDestinations()
                    } label: {
                        HStack {
                            Image(systemName: "arrow.clockwise")
                            Text("Refresh Destinations")
                        }
                    }
                    .tint(Theme.primary)
                } header: { Text("MIDI Routing") }

                Section {
                    Text("Free / MIDI Clock / Link — Phase 3").foregroundStyle(Theme.textMuted)
                } header: { Text("Sync") }
            }
            #if os(macOS)
            .listStyle(.inset)
            #else
            .listStyle(.insetGrouped)
            #endif
            .scrollContentBackground(.hidden)
            .background(Theme.Surface.lowest.ignoresSafeArea())
            .navigationTitle("Settings")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }.tint(Theme.primary)
                }
            }
        }
        .preferredColorScheme(.dark)
    }
}
