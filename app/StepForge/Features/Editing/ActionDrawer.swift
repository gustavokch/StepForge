import SwiftUI

/// Track action drawer (ui-ux-spec §3.2): Roll / Vary / Cut / Copy / Paste / Trash,
/// plus ✓ keep (dismiss) and ✕ revert (`Undo` for this track). Presented as a
/// compact sheet from the track header's "…". Roll/Vary use a default strength;
/// the inline ✓/✕ affordance matches the spec's keep/revert after Roll/Vary.
struct ActionDrawer: View {
    @EnvironmentObject private var bridge: EngineBridge
    @Environment(\.dismiss) private var dismiss
    let trackIdx: Int

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

            HStack(spacing: 6) {
                action("Vary", "wand.and.stars") { bridge.submit(.vary(trackIdx: trackIdx, strength: 0.5)) }
                action("Roll", "dice") { bridge.submit(.roll(trackIdx: trackIdx, strength: 0.6)) }
                action("Copy", "doc.on.doc") { bridge.submit(.copy(trackIdx: trackIdx)) }
                action("Cut", "scissors") { bridge.submit(.cut(trackIdx: trackIdx)) }
                action("Paste", "doc.on.clipboard") { bridge.submit(.paste(trackIdx: trackIdx)) }
                action("Clear", "trash") { bridge.submit(.trash(trackIdx: trackIdx)); Haptics.confirm() }
            }
        }
        .padding()
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Theme.Surface.default)
    }

    private func action(_ label: String, _ icon: String, _ perform: @escaping () -> Void) -> some View {
        Button(action: perform) {
            VStack(spacing: 4) {
                Image(systemName: icon).font(.title3)
                Text(label).font(Typography.sectionTag)
            }
            .foregroundStyle(Theme.textPrimary)
            .frame(maxWidth: .infinity)
            .padding(.vertical, 8)
            .raisedStyle()
        }
        .buttonStyle(.plain)
    }
}
