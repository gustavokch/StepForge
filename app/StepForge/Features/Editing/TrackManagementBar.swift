import SwiftUI

/// Track add/remove (ui-ux-spec §3.3): `+` instantly adds (up to 8); `−` quick-tap
/// shows a hold-to-remove hint, long-press removes the bottom-most track (down to
/// 4). Both bounds (4…8) are enforced by disabling.
struct TrackManagementBar: View {
    @EnvironmentObject private var bridge: EngineBridge
    @State private var showRemoveHint = false

    private var tracks: [Track] { bridge.mirror.tracks }

    var body: some View {
        HStack(spacing: 6) {
            Text("\(tracks.count) / 8 TRACKS")
                .font(Typography.sectionTag)
                .foregroundStyle(Theme.textMuted)
            if showRemoveHint {
                Text("HOLD − TO REMOVE")
                    .font(Typography.sectionTag)
                    .foregroundStyle(Theme.primary)
                    .transition(.opacity)
            }
            Spacer()
            addButton
            removeButton
        }
        .padding(.horizontal, Theme.Spacing.sm)
        .padding(.vertical, 4)
        .animation(.easeInOut(duration: 0.15), value: showRemoveHint)
    }

    private var addButton: some View {
        Button {
            bridge.submit(.addTrack)
        } label: {
            Image(systemName: "plus")
                .font(.body.weight(.bold))
                .foregroundStyle(tracks.count >= 8 ? Theme.textMuted : Theme.textPrimary)
                .frame(width: 30, height: 24)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .raisedStyle()
        .disabled(tracks.count >= 8)
    }

    private var removeButton: some View {
        Button {
            withAnimation { showRemoveHint = true }
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { showRemoveHint = false }
        } label: {
            Image(systemName: "minus")
                .font(.body.weight(.bold))
                .foregroundStyle(tracks.count <= 4 ? Theme.textMuted : Theme.textPrimary)
                .frame(width: 30, height: 24)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .raisedStyle()
        .disabled(tracks.count <= 4)
        .simultaneousGesture(
            LongPressGesture(minimumDuration: 0.5).onEnded { _ in
                guard tracks.count > 4 else { return }   // honor the disabled floor (MIN_TRACKS)
                bridge.submit(.removeTrack)
                Haptics.confirm()
            }
        )
    }
}
