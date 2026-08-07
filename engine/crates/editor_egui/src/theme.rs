//! Phase 4 §T T13b — design tokens. Faithful port of the iOS
//! `app/StepForge/Theme/Theme.swift` into egui `Color32` / `f32` / `u8`
//! constants. Dark-only (a light variant needs mode-aware tokens; out of
//! scope — see the spec's non-goals). Replaces the inline `grid.rs:39-50`
//! palette; imported by every widget as `crate::theme::{...}`.

use egui::Color32;

// ---- Surface (5 graphite tiers; higher elevation = lighter) ----
pub const SURFACE_LOWEST: Color32 = Color32::from_rgb(0x0E, 0x0E, 0x0E);
pub const SURFACE_LOW: Color32 = Color32::from_rgb(0x1B, 0x1B, 0x1C);
pub const SURFACE_DEFAULT: Color32 = Color32::from_rgb(0x20, 0x20, 0x20);
pub const SURFACE_HIGH: Color32 = Color32::from_rgb(0x2A, 0x2A, 0x2A);
pub const SURFACE_HIGHEST: Color32 = Color32::from_rgb(0x35, 0x35, 0x35);

// ---- Border ----
pub const BORDER_WEAK: Color32 = Color32::from_rgb(0x35, 0x35, 0x35);
pub const BORDER_STRONG: Color32 = Color32::from_rgb(0x58, 0x42, 0x35);
pub const BORDER_ACCENT: Color32 = Color32::from_rgb(0xFF, 0x7F, 0x00);

// ---- Brand ----
pub const PRIMARY: Color32 = Color32::from_rgb(0xFF, 0x7F, 0x00);
pub const PRIMARY_DIM: Color32 = Color32::from_rgb(0xFF, 0xB6, 0x88);
pub const ON_PRIMARY: Color32 = Color32::from_rgb(0x23, 0x13, 0x00);

// ---- Text ----
pub const TEXT_PRIMARY: Color32 = Color32::WHITE;
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(0xA0, 0xA0, 0xA0);
pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x6E, 0x6E, 0x6E);

// ---- Velocity zones ----
pub const ZONE_ACCENT: Color32 = Color32::from_rgb(0xFF, 0x7F, 0x00);
pub const ZONE_MID: Color32 = Color32::from_rgb(0xFF, 0xB6, 0x88);
pub const ZONE_LOW: Color32 = Color32::from_rgb(0x98, 0xCB, 0xFF);

// ---- Semantic ----
/// Engine-error / danger text (replaces the stray `Color32::LIGHT_RED` in lib.rs).
pub const DANGER: Color32 = Color32::from_rgb(0xE5, 0x4B, 0x4B);

// ---- Spacing (4px grid; f32 for egui Vec2) — port of iOS `Theme.Spacing` ----
pub struct Spacing;
impl Spacing {
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 8.0;
    pub const MD: f32 = 16.0;
    pub const LG: f32 = 24.0;
    pub const XL: f32 = 48.0;
    pub const GUTTER: f32 = 12.0;
}

// ---- Radius (u8 for egui CornerRadius) — port of iOS `Theme.Radius` ----
pub struct Radius;
impl Radius {
    pub const SM: u8 = 4;
    pub const MD: u8 = 6;
    pub const LG: u8 = 8;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Asserts every token equals the iOS `Theme.swift` source value (the
    /// port-fidelity guard). If iOS changes, update BOTH sides deliberately.
    #[test]
    fn palette_matches_ios_hex() {
        // Surface tiers
        assert_eq!(SURFACE_LOWEST, Color32::from_rgb(0x0E, 0x0E, 0x0E));
        assert_eq!(SURFACE_LOW, Color32::from_rgb(0x1B, 0x1B, 0x1C));
        assert_eq!(SURFACE_DEFAULT, Color32::from_rgb(0x20, 0x20, 0x20));
        assert_eq!(SURFACE_HIGH, Color32::from_rgb(0x2A, 0x2A, 0x2A));
        assert_eq!(SURFACE_HIGHEST, Color32::from_rgb(0x35, 0x35, 0x35));
        // Border
        assert_eq!(BORDER_WEAK, Color32::from_rgb(0x35, 0x35, 0x35));
        assert_eq!(BORDER_STRONG, Color32::from_rgb(0x58, 0x42, 0x35));
        assert_eq!(BORDER_ACCENT, Color32::from_rgb(0xFF, 0x7F, 0x00));
        // Brand
        assert_eq!(PRIMARY, Color32::from_rgb(0xFF, 0x7F, 0x00));
        assert_eq!(PRIMARY_DIM, Color32::from_rgb(0xFF, 0xB6, 0x88));
        assert_eq!(ON_PRIMARY, Color32::from_rgb(0x23, 0x13, 0x00));
        // Text
        assert_eq!(TEXT_PRIMARY, Color32::WHITE);
        assert_eq!(TEXT_SECONDARY, Color32::from_rgb(0xA0, 0xA0, 0xA0));
        assert_eq!(TEXT_MUTED, Color32::from_rgb(0x6E, 0x6E, 0x6E));
        // Velocity zones
        assert_eq!(ZONE_ACCENT, Color32::from_rgb(0xFF, 0x7F, 0x00));
        assert_eq!(ZONE_MID, Color32::from_rgb(0xFF, 0xB6, 0x88));
        assert_eq!(ZONE_LOW, Color32::from_rgb(0x98, 0xCB, 0xFF));
    }

    #[test]
    fn spacing_and_radius_match_ios() {
        assert_eq!(Spacing::XS, 4.0);
        assert_eq!(Spacing::SM, 8.0);
        assert_eq!(Spacing::MD, 16.0);
        assert_eq!(Spacing::LG, 24.0);
        assert_eq!(Spacing::XL, 48.0);
        assert_eq!(Spacing::GUTTER, 12.0);
        assert_eq!(Radius::SM, 4);
        assert_eq!(Radius::MD, 6);
        assert_eq!(Radius::LG, 8);
    }
}
