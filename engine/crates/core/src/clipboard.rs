//! Track + session clipboards. TrackClipboard carries steps/length/speed_ratio
//! but NEVER midi_note (working agreement).

use crate::models::{Session, Step, STEP_COUNT};

#[derive(Clone)]
pub struct TrackClipboard {
    pub steps: [Step; STEP_COUNT],
    pub length: usize,
    pub speed_ratio: f32,
}

#[derive(Default)]
pub struct Clipboard {
    track: Option<TrackClipboard>,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Pattern, Step, VelocityZone};
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
}
