//! Per-track, one-deep undo. Snapshot triggers: Roll/Vary/Cut/Paste/Trash.
//! A length change does NOT push undo (working agreement).

use crate::models::{Session, Step, MAX_TRACKS};

pub struct Undo {
    slots: [Option<[Step; crate::models::STEP_COUNT]>; MAX_TRACKS],
}
impl Default for Undo {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| None),
        }
    }
}

impl Undo {
    pub fn push(&mut self, track_idx: usize, track_steps: &[Step; crate::models::STEP_COUNT]) {
        if track_idx < MAX_TRACKS {
            self.slots[track_idx] = Some(*track_steps);
        }
    }
    /// Restore track `idx`'s steps if a snapshot exists. Returns true if restored.
    pub fn undo(&mut self, s: &mut Session, idx: usize) -> bool {
        let Some(steps) = self.slots[idx].take() else {
            return false;
        };
        if let Some(p) = s.patterns[s.active_pattern_index].as_mut() {
            if let Some(t) = p.tracks.get_mut(idx) {
                t.steps = steps;
                return true;
            }
        }
        false
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
        let snap = s.patterns[0].as_ref().unwrap().tracks[0].steps;
        u.push(0, &snap);
        // mutate
        s.patterns[0].as_mut().unwrap().tracks[0].steps[0].active = false;
        assert!(u.undo(&mut s, 0));
        assert!(s.patterns[0].as_ref().unwrap().tracks[0].steps[0].active);
        assert!(!u.available(0));
    }
}
