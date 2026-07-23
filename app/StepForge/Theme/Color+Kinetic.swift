import SwiftUI

extension Color {
    /// `Color(hex: 0xRRGGBB)` — opaque sRGB. DESIGN.md "Kinetic Studio" palette.
    init(hex: UInt32) {
        let r = Double((hex >> 16) & 0xFF) / 255.0
        let g = Double((hex >> 8) & 0xFF) / 255.0
        let b = Double(hex & 0xFF) / 255.0
        self.init(.sRGB, red: r, green: g, blue: b, opacity: 1)
    }
}
