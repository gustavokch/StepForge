//! Phase 4 §T T13b — typography type-scale. Port of iOS
//! `app/StepForge/Theme/Typography.swift`'s 7 named SF styles into egui
//! `FontId`s (size + family). egui default fonts expose only normal/bold, so
//! iOS `medium → normal` and `semibold → bold` (applied per-call via
//! `.strong()` in the helper fns). Registered as named `TextStyle`s by
//! [`crate::apply_theme`] (Task 7).

use egui::{FontFamily, FontId, RichText, TextStyle};

use crate::theme::TEXT_PRIMARY;

// ---- Role names (registered as TextStyle::Name) ----
pub const NAME_BPM_LARGE: &str = "BpmLarge";
pub const NAME_MONO_VALUE: &str = "MonoValue";
pub const NAME_STEP_INDEX: &str = "StepIndex";
pub const NAME_TRACK_NAME: &str = "TrackName";
pub const NAME_CONTROL_LABEL: &str = "ControlLabel";
pub const NAME_SECTION_TAG: &str = "SectionTag";
pub const NAME_BADGE: &str = "Badge";

// ---- Sizes (px; iOS Dynamic Type → fixed px in egui) ----
pub const BPM_LARGE_SIZE: f32 = 28.0; // iOS title2
pub const MONO_VALUE_SIZE: f32 = 13.0;
pub const STEP_INDEX_SIZE: f32 = 10.0;
pub const TRACK_NAME_SIZE: f32 = 14.0; // iOS subheadline
pub const CONTROL_LABEL_SIZE: f32 = 12.0; // iOS caption
pub const SECTION_TAG_SIZE: f32 = 11.0; // iOS caption2
pub const BADGE_SIZE: f32 = 10.0;

/// The 7 (name, FontId) pairs installed into `ctx.style().text_styles` by
/// [`crate::apply_theme`].
pub fn font_ids() -> impl Iterator<Item = (String, FontId)> {
    [
        (
            NAME_BPM_LARGE,
            FontId::new(BPM_LARGE_SIZE, FontFamily::Monospace),
        ),
        (
            NAME_MONO_VALUE,
            FontId::new(MONO_VALUE_SIZE, FontFamily::Monospace),
        ),
        (
            NAME_STEP_INDEX,
            FontId::new(STEP_INDEX_SIZE, FontFamily::Monospace),
        ),
        (
            NAME_TRACK_NAME,
            FontId::new(TRACK_NAME_SIZE, FontFamily::Proportional),
        ),
        (
            NAME_CONTROL_LABEL,
            FontId::new(CONTROL_LABEL_SIZE, FontFamily::Proportional),
        ),
        (
            NAME_SECTION_TAG,
            FontId::new(SECTION_TAG_SIZE, FontFamily::Monospace),
        ),
        (NAME_BADGE, FontId::new(BADGE_SIZE, FontFamily::Monospace)),
    ]
    .into_iter()
    .map(|(n, f)| (n.to_string(), f))
}

// ---- Helper fns (size + family + color; bold roles add .strong()) ----

/// Large numeric transport readout (BPM). iOS `bpmLarge` — mono, bold.
pub fn bpm_large(text: &str) -> RichText {
    RichText::new(text)
        .text_style(TextStyle::Name(NAME_BPM_LARGE.into()))
        .strong()
        .color(TEXT_PRIMARY)
}
/// In-control numeric values. iOS `monoValue` — 13 semibold mono → bold.
pub fn mono_value(text: &str) -> RichText {
    RichText::new(text)
        .text_style(TextStyle::Name(NAME_MONO_VALUE.into()))
        .strong()
        .color(TEXT_PRIMARY)
}
/// 1..16 column index. iOS `stepIndex` — 10 medium mono → normal.
pub fn step_index(text: &str) -> RichText {
    RichText::new(text)
        .text_style(TextStyle::Name(NAME_STEP_INDEX.into()))
        .color(TEXT_PRIMARY)
}
/// Track / drum name. iOS `trackName` — subheadline semibold → bold.
pub fn track_name(text: &str) -> RichText {
    RichText::new(text)
        .text_style(TextStyle::Name(NAME_TRACK_NAME.into()))
        .strong()
        .color(TEXT_PRIMARY)
}
/// Pill control labels + section headers. iOS `controlLabel` — caption medium.
pub fn control_label(text: &str) -> RichText {
    RichText::new(text)
        .text_style(TextStyle::Name(NAME_CONTROL_LABEL.into()))
        .color(TEXT_PRIMARY)
}
/// Uppercase technical section tag. iOS `sectionTag` — caption2 semibold mono.
pub fn section_tag(text: &str) -> RichText {
    RichText::new(text)
        .text_style(TextStyle::Name(NAME_SECTION_TAG.into()))
        .strong()
        .color(TEXT_PRIMARY)
}
/// Small chip/badge. iOS `badge` — 10 bold mono.
pub fn badge(text: &str) -> RichText {
    RichText::new(text)
        .text_style(TextStyle::Name(NAME_BADGE.into()))
        .strong()
        .color(TEXT_PRIMARY)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 7 roles are emitted with their iOS-mapped size + family.
    #[test]
    fn font_ids_are_the_seven_ios_roles() {
        let map: std::collections::BTreeMap<String, FontId> = font_ids().collect();
        assert_eq!(map.len(), 7);
        assert_eq!(
            map[NAME_BPM_LARGE],
            FontId::new(28.0, FontFamily::Monospace)
        );
        assert_eq!(
            map[NAME_MONO_VALUE],
            FontId::new(13.0, FontFamily::Monospace)
        );
        assert_eq!(
            map[NAME_STEP_INDEX],
            FontId::new(10.0, FontFamily::Monospace)
        );
        assert_eq!(
            map[NAME_TRACK_NAME],
            FontId::new(14.0, FontFamily::Proportional)
        );
        assert_eq!(
            map[NAME_CONTROL_LABEL],
            FontId::new(12.0, FontFamily::Proportional)
        );
        assert_eq!(
            map[NAME_SECTION_TAG],
            FontId::new(11.0, FontFamily::Monospace)
        );
        assert_eq!(map[NAME_BADGE], FontId::new(10.0, FontFamily::Monospace));
    }
}
