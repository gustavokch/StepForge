import SwiftUI

/// The track area: a vertical scroll of rows, each a pinned `TrackHeader` (left,
/// outside the horizontal scroll) beside that track's 16 `StepCell`s (in a single
/// shared horizontal scroll so columns stay aligned across rows). Reads tracks from
/// `bridge.mirror`; gesture callbacks bubble long-press (ratchet) and header "…"
/// (action drawer) up to `EditingView`. Wraps vertical scroll in `ScrollViewReader`
/// to auto-scroll when new tracks are added.
struct TrackList: View {
    @EnvironmentObject private var bridge: EngineBridge
    @Environment(\.horizontalSizeClass) private var hSize
    @Environment(\.verticalSizeClass) private var vSize

    let visibleSteps: Int
    @Binding var ratchetTarget: TrackStepRef?
    @Binding var drawerTarget: DrawerTarget?

    var body: some View {
        GeometryReader { geo in
            let metrics = GridMetrics.resolve(hSize: hSize, vSize: vSize,
                                              width: geo.size.width, visibleSteps: visibleSteps)
            let tracks = bridge.mirror.tracks
            if tracks.isEmpty {
                emptyState
            } else {
                grid(metrics: metrics, tracks: tracks)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func grid(metrics: GridMetrics, tracks: [Track]) -> some View {
        ScrollViewReader { proxy in
            ScrollView(.vertical, showsIndicators: false) {
                HStack(alignment: .top, spacing: 0) {
                    // Pinned header column — fixed width, NOT in the horizontal scroll.
                    VStack(spacing: metrics.rowSpacing) {
                        ForEach(Array(tracks.enumerated()), id: \.element.id) { idx, track in
                            TrackHeader(track: track, trackIdx: idx) {
                                drawerTarget = DrawerTarget(track: idx)
                            }
                            .frame(width: metrics.headerWidth, height: metrics.rowHeight)
                            .id(track.id)
                        }
                    }

                    // One shared horizontal scroller for every row's cells.
                    ScrollView(.horizontal, showsIndicators: false) {
                        VStack(spacing: metrics.rowSpacing) {
                            ForEach(Array(tracks.enumerated()), id: \.element.id) { idx, track in
                                StepRow(track: track, trackIdx: idx, metrics: metrics,
                                        playhead: bridge.mirror.playheads[idx]) { step in
                                    ratchetTarget = TrackStepRef(track: idx, step: step)
                                }
                                .frame(height: metrics.rowHeight)
                            }
                        }
                    }
                }
                .padding(.vertical, Theme.Spacing.xs)
            }
            .onChange(of: tracks.count) { oldCount, newCount in
                if newCount > oldCount, let last = tracks.last {
                    withAnimation {
                        proxy.scrollTo(last.id, anchor: .bottom)
                    }
                }
            }
        }
    }

    private var emptyState: some View {
        VStack(spacing: Theme.Spacing.sm) {
            Image(systemName: "music.note").font(.system(size: 40)).foregroundStyle(Theme.textMuted)
            Text("No tracks").font(Typography.controlLabel).foregroundStyle(Theme.textMuted)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
