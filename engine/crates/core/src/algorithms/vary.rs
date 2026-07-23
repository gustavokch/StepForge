//! Vary: perturb non-accent active steps; LOCK accents. Falls back to Roll
//! when there are no accents. Preserves length / midi_note / speed_ratio.

use crate::algorithms::roll;
use crate::clock::Rng;
use crate::models::{Track, VelocityZone, STEP_COUNT};

pub fn vary(track: &mut Track, strength: f32, rng: &mut Rng) {
    let has_accent = track
        .steps
        .iter()
        .any(|s| s.active && s.velocity_zone == VelocityZone::Accent);
    if !has_accent {
        return roll::roll(track, strength, rng);
    }
    let s = strength.clamp(0.0, 1.0);
    for i in 0..STEP_COUNT {
        let is_accent =
            track.steps[i].active && track.steps[i].velocity_zone == VelocityZone::Accent;
        if !is_accent {
            if rng.range(0, 100) < (s * 30.0) as i32 {
                track.steps[i].active = !track.steps[i].active;
            }
            if track.steps[i].active {
                let off = (rng.range(-50, 50) as f32 / 100.0) * s;
                track.steps[i].micro_timing_offset = off;
                
                if rng.range(0, 100) < (s * 40.0) as i32 {
                    let v = rng.range(0, 2);
                    track.steps[i].velocity_zone = match v {
                        0 => crate::models::VelocityZone::Low,
                        _ => crate::models::VelocityZone::Mid,
                    };
                }
            }
        }
        // accent steps untouched
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::Rng; // disambiguate from proptest::prelude::Rng (trait)
    use crate::models::{Step, Track, VelocityZone};
    use proptest::prelude::*;

    fn track_with_accent() -> Track {
        let mut t = Track::default();
        t.steps[0] = Step {
            active: true,
            velocity_zone: VelocityZone::Accent,
            ..Default::default()
        };
        t.steps[2] = Step {
            active: true,
            velocity_zone: VelocityZone::Mid,
            ..Default::default()
        };
        t.length = 5;
        t.midi_note = 40;
        t.speed_ratio = 0.5;
        t
    }

    #[test]
    fn vary_locks_accents() {
        let mut t = track_with_accent();
        let accent_before = t.steps[0];
        vary(&mut t, 1.0, &mut Rng::new(3));
        assert_eq!(t.steps[0], accent_before, "accent step unchanged");
    }

    #[test]
    fn vary_falls_back_to_roll_with_no_accents() {
        let mut t = Track::default();
        t.steps[0] = Step {
            active: true,
            velocity_zone: VelocityZone::Mid,
            ..Default::default()
        };
        vary(&mut t, 1.0, &mut Rng::new(9)); // no accent -> Roll path, must not panic
        assert!(t.steps[0].micro_timing_offset.abs() <= 0.5);
    }

    proptest! {
        #[test]
        fn vary_preserves_invariants(seed in 0u64..10_000, strength in 0.0f32..1.0) {
            let mut t = track_with_accent();
            let before = (t.length, t.midi_note, t.speed_ratio.to_bits());
            vary(&mut t, strength, &mut Rng::new(seed));
            prop_assert_eq!(before, (t.length, t.midi_note, t.speed_ratio.to_bits()));
        }
    }
}
