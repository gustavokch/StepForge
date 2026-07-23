//! Roll: randomize micro_timing_offset (+ toggle some steps). Preserves
//! length / midi_note / speed_ratio. Caller pushes undo first.

use crate::clock::Rng;
use crate::models::{Track, STEP_COUNT};

pub fn roll(track: &mut Track, strength: f32, rng: &mut Rng) {
    let s = strength.clamp(0.0, 1.0);
    for i in 0..STEP_COUNT {
        // Toggle step active state with a chance proportional to strength
        // max 30% chance to flip at 1.0 strength
        if rng.range(0, 100) < (s * 30.0) as i32 {
            track.steps[i].active = !track.steps[i].active;
        }

        if track.steps[i].active {
            let off = (rng.range(-50, 50) as f32 / 100.0) * s; // ±0.5 * strength
            track.steps[i].micro_timing_offset = off;

            if rng.range(0, 100) < (s * 40.0) as i32 {
                let v = rng.range(0, 3);
                track.steps[i].velocity_zone = match v {
                    0 => crate::models::VelocityZone::Low,
                    1 => crate::models::VelocityZone::Mid,
                    _ => crate::models::VelocityZone::Accent,
                };
            }
        }
    }
    // length / midi_note / speed_ratio untouched by construction.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::Rng; // disambiguate from proptest::prelude::Rng (trait)
    use crate::models::{Step, Track, VelocityZone};
    use proptest::prelude::*;

    fn active_track() -> Track {
        let mut t = Track::default();
        t.steps[0] = Step {
            active: true,
            velocity_zone: VelocityZone::Mid,
            ..Default::default()
        };
        t.length = 7;
        t.midi_note = 55;
        t.speed_ratio = 2.0;
        t
    }

    proptest! {
        #[test]
        fn roll_preserves_invariants(seed in 0u64..10_000, strength in 0.0f32..1.0) {
            let mut t = active_track();
            let before = (t.length, t.midi_note, t.speed_ratio.to_bits());
            roll(&mut t, strength, &mut Rng::new(seed));
            let after = (t.length, t.midi_note, t.speed_ratio.to_bits());
            prop_assert_eq!(before, after);
        }
    }
}
