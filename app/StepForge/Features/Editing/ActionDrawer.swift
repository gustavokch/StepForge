import SwiftUI

/// Track action drawer (ui-ux-spec §3.2): Roll / Vary / Cut / Copy / Paste / Trash,
/// plus ✓ keep (dismiss) and ✕ revert (`Undo` for this track). Presented as a
/// compact sheet from the track header's "…". Includes mini strength sliders for Roll/Vary.
struct ActionDrawer: View {
    @EnvironmentObject private var bridge: EngineBridge
    @Environment(\.dismiss) private var dismiss
    let trackIdx: Int

    @State private var rollStrength: Float = 0.6
    @State private var varyStrength: Float = 0.5

    private var trackName: String {
        DrumNames.name(for: bridge.mirror.tracks[safe: trackIdx]?.midiNote ?? 0)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: Theme.Spacing.sm) {
            HStack {
                SectionLabel("Actions · \(trackName)")
                Spacer()
                Button { dismiss() } label: {
                    Image(systemName: "checkmark")
                        .foregroundStyle(Theme.primary)
                        .frame(width: 28, height: 28)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Keep")
                Button {
                    bridge.submit(.undo(trackIdx: trackIdx))
                    dismiss()
                } label: {
                    Image(systemName: "arrow.uturn.backward")
                        .foregroundStyle(Theme.textSecondary)
                        .frame(width: 28, height: 28)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Revert (undo)")
            }

            HStack(spacing: Theme.Spacing.md) {
                VStack(alignment: .leading, spacing: 2) {
                    HStack {
                        Text("VARY STRENGTH")
                            .font(Typography.sectionTag)
                            .foregroundStyle(Theme.textMuted)
                        Spacer()
                        Text("\(Int((varyStrength * 100).rounded()))%")
                            .font(Typography.monoValue)
                            .foregroundStyle(Theme.textSecondary)
                    }
                    Slider(value: $varyStrength, in: 0...1)
                        .tint(Theme.primary)
                }

                VStack(alignment: .leading, spacing: 2) {
                    HStack {
                        Text("ROLL STRENGTH")
                            .font(Typography.sectionTag)
                            .foregroundStyle(Theme.textMuted)
                        Spacer()
                        Text("\(Int((rollStrength * 100).rounded()))%")
                            .font(Typography.monoValue)
                            .foregroundStyle(Theme.textSecondary)
                    }
                    Slider(value: $rollStrength, in: 0...1)
                        .tint(Theme.primary)
                }
            }

            HStack(spacing: 6) {
                TileButton("Vary", "wand.and.stars") { bridge.submit(.vary(trackIdx: trackIdx, strength: varyStrength)) }
                TileButton("Roll", "dice") { bridge.submit(.roll(trackIdx: trackIdx, strength: rollStrength)) }
                TileButton("Copy", "doc.on.doc") { bridge.submit(.copy(trackIdx: trackIdx)) }
                TileButton("Cut", "scissors") { bridge.submit(.cut(trackIdx: trackIdx)) }
                TileButton("Paste", "doc.on.clipboard") { bridge.submit(.paste(trackIdx: trackIdx)) }
                TileButton("Clear", "trash") { bridge.submit(.trash(trackIdx: trackIdx)); Haptics.confirm() }
            }
        }
        .padding()
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Theme.Surface.default)
    }
}
