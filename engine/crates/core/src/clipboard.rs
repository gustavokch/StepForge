//! Track + session clipboards. TrackClipboard carries steps/length/speed_ratio
//! but NEVER midi_note (working agreement). PatternClipboard carries tracks +
//! follow_action but NEVER the pattern `id` (paste preserves the target's id —
//! avoids `PlaySpecific` Uuid collisions; mirrors how track paste spares
//! midi_note).

use crate::models::{FollowAction, Session, Step, Track, STEP_COUNT};

#[derive(Clone)]
pub struct TrackClipboard {
    pub steps: [Step; STEP_COUNT],
    pub length: usize,
    pub speed_ratio: f32,
}

/// Whole-pattern clipboard. Holds a clone of a pattern's tracks + follow_action
/// but NOT its `id` — paste writes these into the target, keeping the target's
/// id. (A whole-pattern clone is meant to overwrite the target's midi_notes too,
/// unlike a track paste — you're cloning an entire pattern's worth of tracks.)
#[derive(Clone)]
pub struct PatternClipboard {
    pub tracks: Vec<Track>,
    pub follow_action: FollowAction,
}

#[derive(Default)]
pub struct Clipboard {
    track: Option<TrackClipboard>,
    pattern: Option<PatternClipboard>,
}

impl Clipboard {
    pub fn cut(&mut self, s: &mut Session, idx: usize) {
        self.copy(s, idx);
        if let Some(p) = s.patterns[s.active_pattern_index].as_mut() {
            if let Some(t) = p.tracks.get_mut(idx) {
                t.steps = [Step::default(); STEP_COUNT];
            }
        }
    }
    pub fn copy(&mut self, s: &Session, idx: usize) {
        if let Some(t) = s.patterns[s.active_pattern_index]
            .as_ref()
            .and_then(|p| p.tracks.get(idx))
        {
            self.track = Some(TrackClipboard {
                steps: t.steps,
                length: t.length,
                speed_ratio: t.speed_ratio,
            });
        }
    }
    pub fn paste(&self, s: &mut Session, idx: usize) -> bool {
        let Some(cb) = &self.track else {
            return false;
        };
        if let Some(t) = s.patterns[s.active_pattern_index]
            .as_mut()
            .and_then(|p| p.tracks.get_mut(idx))
        {
            t.steps = cb.steps;
            t.length = cb.length;
            t.speed_ratio = cb.speed_ratio;
            // midi_note is deliberately NOT overwritten.
            return true;
        }
        false
    }

    // ---- Whole-pattern clipboard ----

    /// Copy pattern `idx` (tracks + follow_action) to the pattern clipboard.
    pub fn copy_pattern(&mut self, s: &Session, idx: usize) {
        if let Some(p) = s.patterns.get(idx).and_then(|opt| opt.as_ref()) {
            self.pattern = Some(PatternClipboard {
                tracks: p.tracks.clone(),
                follow_action: p.follow_action.clone(),
            });
        }
    }
    /// Copy pattern `idx` to the clipboard, then clear its steps (slot stays
    /// `Some`). Equivalent to track `cut` generalized to every track. Returns
    /// `true` iff the slot was `Some` and the cut mutated it (clear_pattern
    /// bool); a `None`/out-of-range slot is a silent no-op returning `false`.
    pub fn cut_pattern(&mut self, s: &mut Session, idx: usize) -> bool {
        self.copy_pattern(s, idx);
        Self::clear_pattern(s, idx)
    }
    /// Paste the pattern clipboard into slot `idx`: overwrites tracks +
    /// follow_action, preserves the target's `id`. Returns `false` (no-op) if the
    /// clipboard is empty or the target slot is out of range / `None`.
    pub fn paste_pattern(&self, s: &mut Session, idx: usize) -> bool {
        let Some(cb) = &self.pattern else {
            return false;
        };
        if let Some(p) = s.patterns.get_mut(idx).and_then(|opt| opt.as_mut()) {
            p.tracks = cb.tracks.clone();
            p.follow_action = cb.follow_action.clone();
            // id is deliberately NOT overwritten.
            return true;
        }
        false
    }
    /// Clear every track's steps in pattern `idx` (all inactive). The slot stays
    /// `Some` — re-editable + RT-safe (no `None` active-pattern risk). Associated
    /// fn: touches no clipboard state. Returns `true` iff the slot was `Some`
    /// (and steps were reset); `None`/out-of-range returns `false` so the engine
    /// can gate publish on actual mutation.
    pub fn clear_pattern(s: &mut Session, idx: usize) -> bool {
        if let Some(p) = s.patterns.get_mut(idx).and_then(|opt| opt.as_mut()) {
            for t in p.tracks.iter_mut() {
                t.steps = [Step::default(); STEP_COUNT];
            }
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Pattern, Step, VelocityZone};
    use proptest::prelude::*;

    #[test]
    fn copy_then_paste_preserves_midi_note() {
        let mut s = Session::default();
        let mut p = Pattern::default();
        p.tracks[0].midi_note = 42;
        p.tracks[0].steps[3] = Step {
            active: true,
            velocity_zone: VelocityZone::Accent,
            ..Default::default()
        };
        p.tracks[1].midi_note = 99;
        s.patterns[0] = Some(p);
        let mut cb = Clipboard::default();
        cb.copy(&s, 0);
        assert!(cb.paste(&mut s, 1));
        assert_eq!(s.patterns[0].as_ref().unwrap().tracks[1].midi_note, 99); // preserved
        assert!(s.patterns[0].as_ref().unwrap().tracks[1].steps[3].active); // pasted
    }

    #[test]
    fn pattern_copy_then_paste_preserves_target_id_overwrites_tracks() {
        let mut s = Session::default();
        // Slot 2: source — an active step on track 0, follow_action PlayNext.
        {
            let src = s.patterns[2].as_mut().unwrap();
            src.tracks[0].steps[0] = Step {
                active: true,
                velocity_zone: VelocityZone::Accent,
                ..Default::default()
            };
            src.follow_action = FollowAction {
                after_loops: 3,
                action: crate::models::FollowActionType::PlayNext,
            };
        }
        let target_id = s.patterns[5].as_ref().unwrap().id;
        let mut cb = Clipboard::default();
        cb.copy_pattern(&s, 2);
        assert!(cb.paste_pattern(&mut s, 5));
        // Tracks + follow_action overwritten from the source.
        let got = s.patterns[5].as_ref().unwrap();
        assert!(got.tracks[0].steps[0].active);
        assert_eq!(got.follow_action.after_loops, 3);
        // Target id preserved (no Uuid collision → PlaySpecific stays unambiguous).
        assert_eq!(got.id, target_id);
    }

    #[test]
    fn pattern_paste_empty_clipboard_is_noop() {
        let mut s = Session::default();
        let before = s.patterns[5].clone();
        let cb = Clipboard::default();
        assert!(!cb.paste_pattern(&mut s, 5));
        assert_eq!(s.patterns[5], before); // untouched
    }

    #[test]
    fn pattern_clear_resets_steps_keeps_slot() {
        let mut s = Session::default();
        s.patterns[3].as_mut().unwrap().tracks[0].steps[0] = Step {
            active: true,
            velocity_zone: VelocityZone::Accent,
            ..Default::default()
        };
        assert!(
            Clipboard::clear_pattern(&mut s, 3),
            "clear on a Some slot must return true"
        );
        let p = s.patterns[3].as_ref().unwrap();
        assert!(p.tracks.iter().all(|t| t.steps.iter().all(|st| !st.active)));
        // Slot + tracks + midi_notes survive.
        assert_eq!(p.tracks.len(), crate::models::MIN_TRACKS);
    }

    #[test]
    fn pattern_cut_copies_then_clears() {
        let mut s = Session::default();
        s.patterns[1].as_mut().unwrap().tracks[0].steps[0] = Step {
            active: true,
            velocity_zone: VelocityZone::Accent,
            ..Default::default()
        };
        let mut cb = Clipboard::default();
        cb.cut_pattern(&mut s, 1);
        // Source cleared (slot stays Some, steps inactive).
        assert!(s.patterns[1]
            .as_ref()
            .unwrap()
            .tracks
            .iter()
            .all(|t| t.steps.iter().all(|st| !st.active)));
        // Clipboard holds the pre-cut tracks.
        assert!(cb.pattern.as_ref().unwrap().tracks[0].steps[0].active);
    }

    proptest::proptest! {
        /// Invariants for the whole-pattern clipboard (working agreement:
        /// algorithm changes get property tests). copy→paste preserves the
        /// target's id, never shares an id with the source, and the pasted
        /// tracks are byte-equal to the source's.
        #[test]
        fn prop_pattern_copy_paste(
            src in 0usize..9,
            dst in 0usize..9,
        ) {
            let mut s = Session::default();
            // Distinct, active content in the source so a paste is observable.
            let src_pat = s.patterns[src].as_mut().unwrap();
            src_pat.tracks[0].steps[3] = Step {
                active: true, velocity_zone: VelocityZone::Low, ..Default::default()
            };
            src_pat.follow_action = FollowAction {
                after_loops: 7,
                action: crate::models::FollowActionType::PlayPrevious,
            };
            let src_tracks = src_pat.tracks.clone();
            let src_fa = src_pat.follow_action.clone();
            let dst_id_before = s.patterns[dst].as_ref().unwrap().id;

            let mut cb = Clipboard::default();
            cb.copy_pattern(&s, src);
            let pasted = cb.paste_pattern(&mut s, dst);
            prop_assert!(pasted);

            let got = s.patterns[dst].as_ref().unwrap();
            // Tracks + follow_action cloned from the source.
            prop_assert_eq!(&got.tracks, &src_tracks);
            prop_assert_eq!(&got.follow_action, &src_fa);
            // Target id preserved — never the source's id (no collision).
            prop_assert_eq!(got.id, dst_id_before);
        }
    }
}
