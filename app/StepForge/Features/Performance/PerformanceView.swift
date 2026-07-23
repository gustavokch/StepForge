import SwiftUI

/// Performance mode (ui-ux-spec §1.2) — live pattern jamming. Phase 1 placeholder;
/// the 3×3 pattern picker, retrigger shortcuts, and quantize-grain control land in
/// Phase 2. Reads transport state from the mirror so play/stop still reflect.
struct PerformanceView: View {
    @EnvironmentObject private var bridge: EngineBridge

    var body: some View {
        VStack(spacing: Theme.Spacing.md) {
            Spacer()
            Image(systemName: "music.note")
                .font(.system(size: 48))
                .foregroundStyle(bridge.mirror.playing ? Theme.primary : Theme.textMuted)
            Text("Performance")
                .font(Typography.trackName)
                .foregroundStyle(Theme.textPrimary)
            Text("Pattern picker — Phase 2")
                .font(Typography.sectionTag)
                .foregroundStyle(Theme.textMuted)
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Theme.Surface.lowest)
    }
}
