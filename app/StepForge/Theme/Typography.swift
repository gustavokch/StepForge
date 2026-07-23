import SwiftUI

/// SF-based typography (DESIGN.md specifies Inter/Geist; per session decision we
/// use native **SF Pro + SF Mono** for Dynamic Type + HIG). Semantic styles scale
/// with Dynamic Type; `.monospaced` is used for technical labels/values (BPM,
/// step numbers, badges) — the "Geist-like" technical role.
enum Typography {
    /// Large numeric transport readout (BPM).
    static let bpmLarge    = Font.system(.title2, design: .monospaced).weight(.bold)
    /// In-control numeric values (swing %, grain, etc.).
    static let monoValue   = Font.system(size: 13, weight: .semibold, design: .monospaced)
    /// 1…16 column index above the grid.
    static let stepIndex   = Font.system(size: 10, weight: .medium, design: .monospaced)
    /// Track / drum name in the header.
    static let trackName   = Font.system(.subheadline, design: .default).weight(.semibold)
    /// Pill control labels (TEMPO, GROOVE, …) and section headers.
    static let controlLabel = Font.system(.caption, design: .default).weight(.medium)
    /// Uppercase technical section tag.
    static let sectionTag  = Font.system(.caption2, design: .monospaced).weight(.semibold)
    /// Small chip/badge text (speed ratio, follow action).
    static let badge       = Font.system(size: 10, weight: .bold, design: .monospaced)
}
