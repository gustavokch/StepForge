import SwiftUI

/// Settings sheet: MIDI routing, sync source, and global MIDI channel.
struct SettingsSheet: View {
    @EnvironmentObject private var bridge: EngineBridge
    @EnvironmentObject private var midiManager: MidiManager
    @Environment(\.dismiss) private var dismiss

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
                        VStack(alignment: .leading, spacing: 8) {
                            Text("No MIDI output destinations found.")
                                .foregroundStyle(Theme.textPrimary)
                            Text("StepForge is broadcasting to \"StepForge Virtual Out\". You can select this as an input in your DAW or MIDI monitor app without further configuration!")
                                .font(.caption)
                                .foregroundStyle(Theme.textMuted)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        .padding(.vertical, 4)
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
                    Picker("Sync Source", selection: Binding(
                        get: { bridge.mirror.syncSource },
                        set: { bridge.submit(.setSyncSource(source: $0)) }
                    )) {
                        ForEach(SyncSource.allCases, id: \.self) { source in
                            Text(source.label).tag(source)
                        }
                    }
                    .foregroundStyle(Theme.textPrimary)

                    // Selecting "Link" as the sync source now engages the Ableton
                    // Link session automatically (engine-side, via SetSyncSource),
                    // so a separate enable toggle is redundant. Peer count stays.
                    LabeledContent("Connected Link Peers", value: "\(bridge.mirror.linkPeers)")
                        .foregroundStyle(Theme.textPrimary)
                } header: { Text("Sync & Ableton Link") }
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
        .frame(minWidth: 400, minHeight: 400)
        .preferredColorScheme(.dark)
    }
}
