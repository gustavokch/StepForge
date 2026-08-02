//! Shared dismiss logic for the T11 track-level overlays (`ActionDrawer` +
//! `NotePickerSheet`). Both close on Esc or on a primary click outside their
//! rect; the decision lives here so the two widgets don't carry a verbatim
//! duplicate. Each widget still owns its own `close` (it writes its own
//! `target` back to `ctx.data`), so [`should_dismiss`] returns the decision and
//! the caller performs the close.

use egui::{Context, Key, Rect};

/// Whether an overlay with the given `rect` should close this frame.
///
/// `true` on Esc (any frame), or on a primary click that lands outside `rect`
/// on any frame EXCEPT the opening one. `pointer.primary_clicked()` is global
/// and not consumption-aware, so the header click that opened the overlay is
/// still "primary_clicked" when the overlay first renders this same frame —
/// without the `is_open_frame` guard, that click (which lands outside the
/// overlay rect for tracks below it) would self-dismiss the overlay
/// immediately. (The ratchet popover avoids this with a `!alt` modifier; a
/// plain-click open has no modifier, so it guards on the frame number instead.)
pub(crate) fn should_dismiss(ctx: &Context, rect: Rect, is_open_frame: bool) -> bool {
    ctx.input(|i| i.key_pressed(Key::Escape))
        || (!is_open_frame
            && ctx.input(|i| {
                i.pointer.primary_clicked()
                    && i.pointer.latest_pos().is_none_or(|p| !rect.contains(p))
            }))
}
