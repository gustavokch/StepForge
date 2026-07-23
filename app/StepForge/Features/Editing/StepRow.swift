import SwiftUI

/// One track's 16 step cells — the horizontal-scroll content of a single row.
/// Takes narrow value slices so SwiftUI's structural diff re-renders only the
/// row whose data changed. `playhead` is the coalesced per-track step index.
struct StepRow: View {
    let track: Track
    let trackIdx: Int
    let metrics: GridMetrics
    let playhead: Int?
    let onRatchetRequest: (Int) -> Void

    var body: some View {
        HStack(spacing: metrics.stepGap) {
            ForEach(0..<16, id: \.self) { step in
                StepCell(
                    trackIdx: trackIdx,
                    stepIdx: step,
                    step: track.steps[step],
                    isPlaying: playhead == step,
                    isWithinLength: step < track.length,
                    onRatchetRequest: { onRatchetRequest(step) }
                )
                .equatable()   // value-equal cells (ignoring the gesture closures) skip body re-eval
                .frame(width: metrics.stepSize, height: metrics.rowHeight)
            }
        }
    }
}
