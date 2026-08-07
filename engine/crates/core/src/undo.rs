//! Per-track, one-deep undo. Snapshot triggers: Roll/Vary/Cut/Paste/Trash.
//!
//! The snapshot captures `steps` + `length` + `speed_ratio` (the fields Paste can
//! change). `midi_note` is excluded — no mutating command changes it (working
//! agreement). A dedicated `SetTrackLength` command does NOT push undo (a length
//! change via that path is intentionally not undoable); only the algorithm/clipboard
//! mutating commands snapshot. In-memory only — `Undo` is engine state, never
//! serialized, so this is no `SESSION_FORMAT_VERSION` change.

use crate::models::{
    FollowAction, Pattern, Session, Step, Track, MAX_TRACKS, PATTERN_SLOTS, STEP_COUNT,
};

/// What one undo snapshot captures for a track.
#[derive(Clone, Copy)]
pub struct TrackSnapshot {
    pub steps: [Step; STEP_COUNT],
    pub length: usize,
    pub speed_ratio: f32,
}

impl TrackSnapshot {
    pub fn of(track: &Track) -> Self {
        Self {
            steps: track.steps,
            length: track.length,
            speed_ratio: track.speed_ratio,
        }
    }
}

/// What one pattern-level undo snapshot captures (#34): the full `tracks` plus
/// the `follow_action`. `id` is excluded — every mutating pattern op preserves
/// the target pattern's `id` (mirrors `PatternClipboard`), so restore overwrites
/// only `tracks` + `follow_action`. Heap-backed (`Vec<Track>`) → worker-thread
/// only, never RT. In-memory only; never serialized (no `SESSION_FORMAT_VERSION`
/// bump).
#[derive(Clone)]
pub struct PatternSnapshot {
    pub tracks: Vec<Track>,
    pub follow_action: FollowAction,
}

impl PatternSnapshot {
    pub fn of(p: &Pattern) -> Self {
        Self {
            tracks: p.tracks.clone(),
            follow_action: p.follow_action.clone(),
        }
    }
}

pub struct Undo {
    slots: [Option<TrackSnapshot>; MAX_TRACKS],
    pattern_slots: [Option<PatternSnapshot>; PATTERN_SLOTS],
}
impl Default for Undo {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| None),
            pattern_slots: std::array::from_fn(|_| None),
        }
    }
}

impl Undo {
    /// Snapshot a track's steps/length/speed_ratio before a mutation.
    pub fn push(&mut self, track_idx: usize, track: &Track) {
        if track_idx < MAX_TRACKS {
            self.slots[track_idx] = Some(TrackSnapshot::of(track));
        }
    }
    /// Restore track `idx`'s steps/length/speed_ratio if a snapshot exists.
    /// Returns true if restored. Total: OOB idx or missing snapshot → false, no panic.
    pub fn undo(&mut self, s: &mut Session, idx: usize) -> bool {
        if idx >= MAX_TRACKS {
            return false;
        }
        let Some(snap) = self.slots[idx].take() else {
            return false;
        };
        let Some(p) = s
            .patterns
            .get_mut(s.active_pattern_index)
            .and_then(|opt| opt.as_mut())
        else {
            return false;
        };
        let Some(t) = p.tracks.get_mut(idx) else {
            return false;
        };
        t.steps = snap.steps;
        t.length = snap.length;
        t.speed_ratio = snap.speed_ratio;
        true
    }
    pub fn available(&self, idx: usize) -> bool {
        idx < MAX_TRACKS && self.slots[idx].is_some()
    }
    /// Drain all occupied slots, marking which indices were `Some` (now
    /// cleared). Used by `LoadSession` to reset undo state on a wholesale
    /// reload (#30): the previous session's per-track snapshots would otherwise
    /// restore old-session tracks onto the new one. Idempotent. Returns a
    /// stack-allocated `[bool; MAX_TRACKS]` (no heap) — this runs on the
    /// worker thread, never RT.
    pub fn take_occupied(&mut self) -> [bool; MAX_TRACKS] {
        let mut out = [false; MAX_TRACKS];
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if slot.is_some() {
                out[i] = true;
                *slot = None;
            }
        }
        out
    }

    /// Snapshot a whole pattern (tracks + follow_action) before a pattern-level
    /// mutation (#34). One-deep: overwrites any prior snapshot for `index`.
    /// Bounds-checked; OOB is a no-op. Mirrors the per-track `push`.
    pub fn push_pattern(&mut self, index: usize, p: &Pattern) {
        if index < PATTERN_SLOTS {
            self.pattern_slots[index] = Some(PatternSnapshot::of(p));
        }
    }

    /// Restore slot `index`'s tracks + follow_action if a snapshot exists
    /// (one-deep: the snapshot is consumed). Leaves the pattern's `id` and the
    /// slot's `Some`-ness untouched. Total: OOB index, missing snapshot, or a
    /// `None` slot → returns false, no panic.
    pub fn undo_pattern(&mut self, s: &mut Session, index: usize) -> bool {
        if index >= PATTERN_SLOTS {
            return false;
        }
        // Resolve the target slot first so a `None` slot never consumes a
        // snapshot (doesn't arise today — pattern ops keep slots Some — but
        // stay total).
        let Some(p) = s.patterns.get_mut(index).and_then(|opt| opt.as_mut()) else {
            return false;
        };
        let Some(snap) = self.pattern_slots[index].take() else {
            return false;
        };
        p.tracks = snap.tracks;
        p.follow_action = snap.follow_action;
        true
    }

    /// Whether slot `index` has a pattern snapshot. (Tests only — the UI is
    /// always-on, so this is not read by any surface.)
    pub fn available_pattern(&self, index: usize) -> bool {
        index < PATTERN_SLOTS && self.pattern_slots[index].is_some()
    }

    /// Drain all occupied pattern slots, marking which indices were `Some` (now
    /// cleared). Used by `LoadSession` to reset pattern undo on a wholesale
    /// reload (#30 parity with `take_occupied`). Stack `[bool; PATTERN_SLOTS]`.
    pub fn take_occupied_patterns(&mut self) -> [bool; PATTERN_SLOTS] {
        let mut out = [false; PATTERN_SLOTS];
        for (i, slot) in self.pattern_slots.iter_mut().enumerate() {
            if slot.is_some() {
                out[i] = true;
                *slot = None;
            }
        }
        out
    }

    /// Clear every per-track slot, returning which indices were occupied (now
    /// cleared). Used when a whole-pattern change targets the active pattern:
    /// the per-track snapshots are now stale w.r.t. that pattern (D6). The
    /// caller emits `UndoAvailable { false }` per occupied slot so the mirror
    /// drops stale per-track undo availability (parity with `take_occupied` on
    /// LoadSession).
    pub fn clear_tracks(&mut self) -> [bool; MAX_TRACKS] {
        let mut out = [false; MAX_TRACKS];
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if slot.is_some() {
                out[i] = true;
                *slot = None;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::Clipboard;
    use crate::models::{Pattern, Step, VelocityZone, PATTERN_SLOTS};
    use proptest::prelude::*;
    fn session_with_steps() -> Session {
        let mut s = Session::default();
        let mut p = Pattern::default();
        p.tracks[0].steps[0] = Step {
            active: true,
            velocity_zone: VelocityZone::Accent,
            ..Default::default()
        };
        s.patterns[0] = Some(p);
        s
    }
    #[test]
    fn undo_restores_steps() {
        let mut u = Undo::default();
        let mut s = session_with_steps();
        u.push(0, &s.patterns[0].as_ref().unwrap().tracks[0]);
        // mutate
        s.patterns[0].as_mut().unwrap().tracks[0].steps[0].active = false;
        assert!(u.undo(&mut s, 0));
        assert!(s.patterns[0].as_ref().unwrap().tracks[0].steps[0].active);
        assert!(!u.available(0));
    }
    #[test]
    fn undo_restores_length_and_speed_ratio() {
        // Paste changes steps+length+speed_ratio; Undo must restore all three.
        let mut u = Undo::default();
        let mut s = Session::default();
        let mut p = Pattern::default();
        p.tracks[0].length = 5;
        p.tracks[0].speed_ratio = 2.0;
        s.patterns[0] = Some(p);
        u.push(0, &s.patterns[0].as_ref().unwrap().tracks[0]);
        // mutate length + speed_ratio (as Paste would)
        {
            let t = &mut s.patterns[0].as_mut().unwrap().tracks[0];
            t.length = 9;
            t.speed_ratio = 0.5;
        }
        assert!(u.undo(&mut s, 0));
        let t = &s.patterns[0].as_ref().unwrap().tracks[0];
        assert_eq!(t.length, 5);
        assert_eq!(t.speed_ratio, 2.0);
    }
    #[test]
    fn undo_ignores_out_of_range_idx() {
        // A malformed Command::Undo { track_idx: 99 } must not panic the worker
        // (run_worker_loop's catch_unwind aside, `undo` itself must be total).
        let mut u = Undo::default();
        let mut s = session_with_steps();
        u.push(0, &s.patterns[0].as_ref().unwrap().tracks[0]);
        assert!(!u.undo(&mut s, 99)); // OOB — no panic, returns false
        assert!(u.available(0)); // snapshot at idx 0 was NOT consumed
    }
    #[test]
    fn take_occupied_marks_indices_and_clears() {
        // #30: LoadSession drains occupied undo slots so stale snapshots from
        // the previous session can't be restored onto the new one.
        let mut u = Undo::default();
        let s = session_with_steps();
        u.push(0, &s.patterns[0].as_ref().unwrap().tracks[0]);
        u.push(2, &s.patterns[0].as_ref().unwrap().tracks[0]);
        let occupied = u.take_occupied();
        assert!(occupied[0] && occupied[2], "flags the occupied indices");
        assert!(!occupied[1] && !occupied[3], "unoccupied slots stay false");
        assert!(!u.available(0) && !u.available(2), "slots now cleared");
        // A second drain is all-false (idempotent).
        assert!(
            u.take_occupied().iter().all(|f| !f),
            "second drain is empty"
        );
    }

    fn session_with_pattern_at(idx: usize) -> Session {
        // Session::default pre-fills every slot Some; overwrite [idx] with a
        // Pattern with one active step + a non-default follow_action so a
        // snapshot is observable.
        let mut s = Session::default();
        let mut p = Pattern::default();
        p.tracks[0].steps[3] = crate::models::Step {
            active: true,
            velocity_zone: VelocityZone::Accent,
            ..Default::default()
        };
        p.follow_action = crate::models::FollowAction {
            after_loops: 5,
            action: crate::models::FollowActionType::PlayNext,
        };
        s.patterns[idx] = Some(p);
        s
    }

    #[test]
    fn push_then_undo_pattern_restores_tracks_and_follow_action() {
        // #34: a pre-mutation PatternSnapshot restores tracks + follow_action,
        // preserves the slot's id, and keeps the slot Some (one-deep).
        let mut s = session_with_pattern_at(2);
        let id_before = s.patterns[2].as_ref().unwrap().id;
        let tracks_before = s.patterns[2].as_ref().unwrap().tracks.clone();
        let fa_before = s.patterns[2].as_ref().unwrap().follow_action.clone();

        let mut u = Undo::default();
        u.push_pattern(2, s.patterns[2].as_ref().unwrap());
        assert!(u.available_pattern(2), "snapshot present after push");

        // Mutate (clear), then undo.
        Clipboard::clear_pattern(&mut s, 2);
        assert!(u.undo_pattern(&mut s, 2), "undo must restore");

        let got = s.patterns[2].as_ref().expect("slot stays Some");
        assert_eq!(got.tracks, tracks_before, "tracks restored");
        assert_eq!(got.follow_action, fa_before, "follow_action restored");
        assert_eq!(got.id, id_before, "id preserved");
        assert!(!u.available_pattern(2), "snapshot consumed (one-deep)");
        // A second undo is a no-op.
        assert!(!u.undo_pattern(&mut s, 2), "second undo is a no-op");
    }

    #[test]
    fn undo_pattern_is_total_for_bad_index_or_empty_slot() {
        // OOB index, missing snapshot, and a None slot must all return false,
        // never panic.
        let mut u = Undo::default();
        let mut s = Session::default();
        assert!(
            !u.undo_pattern(&mut s, PATTERN_SLOTS),
            "OOB index is a no-op"
        );
        assert!(!u.undo_pattern(&mut s, 0), "missing snapshot is a no-op");
        // Set slot 1 to None → no-op even with a snapshot push at that index.
        s.patterns[1] = None;
        u.push_pattern(1, &Pattern::default()); // push needs a real Pattern
        assert!(!u.undo_pattern(&mut s, 1), "None target slot is a no-op");
    }

    #[test]
    fn take_occupied_patterns_drains_and_clears() {
        // LoadSession parity (#30): drain all occupied pattern slots, marking
        // which indices were Some (now cleared). Stack `[bool; PATTERN_SLOTS]`.
        let mut u = Undo::default();
        u.push_pattern(1, &Pattern::default());
        u.push_pattern(7, &Pattern::default());
        let occupied = u.take_occupied_patterns();
        assert!(occupied[1] && occupied[7], "marked indices 1 and 7");
        assert!(
            !u.available_pattern(1) && !u.available_pattern(7),
            "cleared"
        );
        // Idempotent.
        assert!(
            u.take_occupied_patterns().iter().all(|f| !f),
            "second drain is empty"
        );
    }

    #[test]
    fn clear_tracks_empties_per_track_slots() {
        // D6 helper: a whole-pattern change on the active pattern clears the
        // (now stale) per-track snapshots and reports which were occupied.
        let mut u = Undo::default();
        let t = crate::models::Track::default();
        u.push(0, &t);
        u.push(3, &t);
        assert!(u.available(0) && u.available(3));
        let cleared = u.clear_tracks();
        assert!(cleared[0] && cleared[3], "occupied slots flagged");
        assert!(!cleared[1] && !cleared[2], "unoccupied slots stay false");
        assert!(
            !u.available(0) && !u.available(3),
            "per-track slots cleared"
        );
    }

    proptest::proptest! {
        /// #34 invariant (working agreement: algorithm changes get property
        /// tests). After push_pattern → mutate → undo_pattern, the pattern's
        /// tracks + follow_action are restored and the id is unchanged.
        #[test]
        fn prop_pattern_undo_restores_state(
            n_active in 0usize..16,
            after_loops in 1u32..=16,
        ) {
            let mut s = session_with_pattern_at(4);
            {
                let p = s.patterns[4].as_mut().unwrap();
                for st in p.tracks[0].steps.iter_mut().take(n_active) {
                    st.active = true;
                }
                p.follow_action.after_loops = after_loops;
            }
            let before = s.patterns[4].clone().unwrap();

            let mut u = Undo::default();
            u.push_pattern(4, &before);
            Clipboard::clear_pattern(&mut s, 4);
            prop_assert!(u.undo_pattern(&mut s, 4));

            let got = s.patterns[4].as_ref().unwrap();
            prop_assert_eq!(&got.tracks, &before.tracks);
            prop_assert_eq!(&got.follow_action, &before.follow_action);
            prop_assert_eq!(got.id, before.id, "id invariant");
        }
    }
}
