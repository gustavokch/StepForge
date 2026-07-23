import SwiftUI

/// Reusable tonal container (DESIGN.md §Components: cards/panels — no shadow,
/// 1px border, elevated surface). Convenience over `.panelStyle`.
struct Panel<Content: View>: View {
    var surface: Color = Theme.Surface.default
    var border: Color = Theme.borderWeak
    var radius: CGFloat = Theme.Radius.sm
    var padding: CGFloat = Theme.Spacing.sm
    @ViewBuilder var content: () -> Content

    var body: some View {
        content()
            .padding(padding)
            .panelStyle(surface, border: border, radius: radius)
    }
}
