import SwiftUI

/// Design tokens for the "Kinetic Studio" / Bitwig-inspired dark theme
/// (DESIGN.md + screen.png). Dark-first: depth comes from **tonal layering and
/// 1px strokes, not shadows**; radii stay sharp (4px); the signature primary
/// `#FF7F00` orange marks active state / primary actions only.
enum Theme {
    /// Graphite surface tiers — higher elevation = lighter tone.
    enum Surface {
        static let lowest   = Color(hex: 0x0E0E0E)   // app background
        static let low      = Color(hex: 0x1B1B1C)   // panels / recessed wells
        static let `default` = Color(hex: 0x202020)  // default container
        static let high     = Color(hex: 0x2A2A2A)   // raised controls / cells
        static let highest  = Color(hex: 0x353535)   // toolbar / top-most
    }

    // Borders
    static let borderWeak   = Color(hex: 0x353535)   // outline-variant
    static let borderStrong = Color(hex: 0x584235)
    static let borderAccent = Color(hex: 0xFF7F00)

    // Brand
    static let primary    = Color(hex: 0xFF7F00)     // active state / primary action
    static let primaryDim = Color(hex: 0xFFB688)     // primary-fixed-dim (peach)
    static let onPrimary  = Color(hex: 0x231300)     // dark text on orange

    // Text
    static let textPrimary   = Color.white
    static let textSecondary = Color(hex: 0xA0A0A0)
    static let textMuted     = Color(hex: 0x6E6E6E)

    /// Velocity hues — discrete colors for sunlight legibility (ui-ux-spec §2.3).
    static func velocity(_ zone: VelocityZone) -> Color {
        switch zone {
        case .accent: Color(hex: 0xFF7F00)   // orange — loudest
        case .mid:    Color(hex: 0xFFB688)   // peach
        case .low:    Color(hex: 0x98CBFF)   // steel blue — softest
        }
    }

    /// 4px grid spacing (DESIGN.md §Layout & Spacing).
    enum Spacing {
        static let xs: CGFloat = 4
        static let sm: CGFloat = 8
        static let md: CGFloat = 16
        static let lg: CGFloat = 24
        static let xl: CGFloat = 48
        static let gutter: CGFloat = 12
    }

    enum Radius {
        static let sm: CGFloat = 4    // buttons / cells / inputs
        static let md: CGFloat = 6
        static let lg: CGFloat = 8    // large panels
    }

    static let borderWidth: CGFloat = 1
}
