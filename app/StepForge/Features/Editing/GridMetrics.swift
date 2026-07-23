import SwiftUI

/// Size-class-aware grid geometry (ui-ux-spec §1.1 / §2.1). Cell size is derived
/// so exactly `visibleSteps` columns fit the width; all 16 are rendered, so when
/// `visibleSteps == 8` the remaining 8 scroll into view horizontally. Tracks stack
/// vertically; the grid is the single source of layout numbers for both the pinned
/// header column and the scrolling cells (so rows stay aligned).
struct GridMetrics: Equatable {
    let visibleSteps: Int
    let headerWidth: CGFloat
    let stepSize: CGFloat
    let stepGap: CGFloat
    let rowHeight: CGFloat
    let rowSpacing: CGFloat

    static func resolve(hSize: UserInterfaceSizeClass?,
                        vSize: UserInterfaceSizeClass?,
                        width: CGFloat,
                        visibleSteps: Int) -> GridMetrics {
        let isPad = (hSize == .regular && vSize == .regular)
        let isCompactLandscape = (hSize == .compact && vSize == .compact)
        let header: CGFloat = isPad ? 210 : (isCompactLandscape ? 132 : 120)
        let gap: CGFloat = 3
        let spacing: CGFloat = 4
        let occupied = gap * CGFloat(max(visibleSteps - 1, 0))
        let avail = max(0, width - header - occupied)
        let step = max(18, avail / CGFloat(visibleSteps))
        let row: CGFloat = isPad ? 54 : (isCompactLandscape ? 40 : 44)
        return GridMetrics(visibleSteps: visibleSteps, headerWidth: header, stepSize: step,
                           stepGap: gap, rowHeight: row, rowSpacing: spacing)
    }

    /// Phone portrait → 8 steps; landscape/iPad → 16 (mockup-faithful).
    static func defaultVisibleSteps(hSize: UserInterfaceSizeClass?, vSize: UserInterfaceSizeClass?) -> Int {
        (hSize == .compact && vSize == .regular) ? 8 : 16
    }
}

extension Array {
    /// Bounds-checked subscript (the mirror's track lists are mutated by events;
    /// never index blindly).
    subscript(safe index: Int) -> Element? {
        indices.contains(index) ? self[index] : nil
    }
}
