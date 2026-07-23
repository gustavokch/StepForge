import SwiftUI

/// Tonal panel: surface fill + 1px border, no shadow (DESIGN.md §Elevation &
/// §Shapes — depth via stroke, hard edges).
struct PanelBackground: ViewModifier {
    var surface: Color
    var border: Color
    var radius: CGFloat

    func body(content: Content) -> some View {
        content
            .background(surface)
            .overlay(
                RoundedRectangle(cornerRadius: radius, style: .continuous)
                    .stroke(border, lineWidth: Theme.borderWidth)
                    .allowsHitTesting(false)
            )
            .clipShape(RoundedRectangle(cornerRadius: radius, style: .continuous))
    }
}

extension View {
    /// Container panel (default surface + weak border).
    func panelStyle(_ surface: Color = Theme.Surface.default,
                    border: Color = Theme.borderWeak,
                    radius: CGFloat = Theme.Radius.sm) -> some View {
        modifier(PanelBackground(surface: surface, border: border, radius: radius))
    }

    /// Recessed "well" — darker than its parent + inset border (DESIGN.md §Elevation).
    func wellStyle(radius: CGFloat = Theme.Radius.sm) -> some View {
        modifier(PanelBackground(surface: Theme.Surface.low,
                                 border: Theme.borderWeak,
                                 radius: radius))
    }

    /// Raised control surface (toolbar buttons, toggles).
    func raisedStyle(radius: CGFloat = Theme.Radius.sm) -> some View {
        modifier(PanelBackground(surface: Theme.Surface.high,
                                 border: Theme.borderWeak,
                                 radius: radius))
    }

    /// Small uppercase technical chip / badge.
    func chipStyle(foreground: Color = Theme.textPrimary,
                   background: Color = Theme.Surface.high,
                   border: Color = Theme.borderWeak) -> some View {
        self
            .font(Typography.badge)
            .foregroundStyle(foreground)
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .background(background)
            .overlay(Capsule().stroke(border, lineWidth: Theme.borderWidth).allowsHitTesting(false))
            .clipShape(Capsule())
    }
}
