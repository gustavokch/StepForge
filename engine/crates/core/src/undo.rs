//! Per-track, one-deep undo. Snapshot triggers: Roll/Vary/Cut/Paste/Trash.
//!
//! The snapshot captures `steps` + `length` + `speed_ratio` (the fields Paste can
//! change). `midi_note` is excluded — no mutating command changes it (working
//! agreement). A dedicated `SetTrackLength` command does NOT push undo (a length
//! change via that path is intentionally not undoable); only the algorithm/clipboard
//! mutating commands snapshot. In-memory only — `Undo` is engine state, never
//! serialized, so this is no `SESSION_FORMAT_VERSION` change.

use crate::models::{Session, Step, Track, MAX_TRACKS, STEP_COUNT};

/// What one undo snapshot captures for a track.
#[derive(Clone, Copy)]
pub struct TrackSnapshot {
    pub steps: [Step; STEP_COUNT],
    pub length: usize,
    pub speed_ratio: f32,
}

impl TrackSnapshot {
    pub fn of(track: &Track) -> Self {
        Self { steps: track.steps, length: track.length, speed_ratio: track.speed_ratio }
    }
}

pub struct Undo {
    slots: [Option<TrackSnapshot>; MAX_TRACKS],
}
impl Default for Undo {
    fn default() -> Self {
        Self { slots: std::array::from_fn(|_| None) }
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
        let Some(p) = s.patterns.get_mut(s.active_pattern_index).and_then(|opt| opt.as_mut()) else {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Pattern, Step, VelocityZone};
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
}
