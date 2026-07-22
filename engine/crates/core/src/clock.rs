//! Self-scheduled clock + pure timing math. The Clock trait + SteppableClock
//! live in core (safe); the prod InstantClock (unsafe RT-priority) lives in ffi.

use std::sync::atomic::{AtomicU64, Ordering};

pub trait Clock: Send + Sync {
    fn now_micros(&self) -> u64;
    fn sleep_until(&self, _deadline_micros: u64) {}
    fn elevate_priority(&self) {}
}

pub struct SteppableClock {
    now: AtomicU64,
}
impl SteppableClock {
    pub fn new() -> Self {
        Self {
            now: AtomicU64::new(0),
        }
    }
    pub fn advance_to(&self, micros: u64) {
        self.now.store(micros, Ordering::Relaxed);
    }
}
impl Default for SteppableClock {
    fn default() -> Self {
        Self::new()
    }
}
impl Clock for SteppableClock {
    fn now_micros(&self) -> u64 {
        self.now.load(Ordering::Relaxed)
    }
}

/// f32 speed_ratio -> Q16.16 (1.0 == 0x1_0000).
pub fn to_q16_16(ratio: f32) -> u32 {
    (ratio * 65536.0) as u32
}

/// Advance a Q16.16 accumulator by `ratio_q`. Returns (whole steps to fire, new accumulator).
pub fn advance_speed_ratio(acc: u32, ratio_q: u32) -> (u32, u32) {
    let acc = acc.wrapping_add(ratio_q);
    (acc >> 16, acc & 0xFFFF)
}

/// Effective swing %, additive (global + track), hard-capped below 50 (E2).
pub fn effective_swing(global_pct: f32, track_pct: f32) -> f32 {
    let v = global_pct + track_pct;
    v.clamp(0.0, 49.0)
}

/// xorshift64 — deterministic, allocation-free, no `rand` dependency.
pub struct Rng(pub u64);
impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x >> 32) as u32
    }
    pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
        let span = (hi - lo + 1).max(1) as u32;
        lo + (self.next_u32() % span) as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steppable_clock_returns_set_time() {
        let c = SteppableClock::new();
        assert_eq!(c.now_micros(), 0);
        c.advance_to(123_456);
        assert_eq!(c.now_micros(), 123_456);
    }

    #[test]
    fn speed_ratio_accumulator_fires_correct_steps() {
        let q = to_q16_16(2.0);
        let (steps, acc) = advance_speed_ratio(0, q);
        assert_eq!(steps, 2);
        assert_eq!(acc, 0);
        // ratio 0.5 fires a step every other tick
        let qh = to_q16_16(0.5);
        let (s1, a1) = advance_speed_ratio(0, qh);
        assert_eq!(s1, 0); // 0.5 -> floor=0 first tick, carry 0.5
        let (s2, _) = advance_speed_ratio(a1, qh);
        assert_eq!(s2, 1); // second tick fires 1
    }

    #[test]
    fn effective_swing_is_additive_and_capped() {
        assert_eq!(effective_swing(10.0, 5.0), 15.0);
        assert_eq!(effective_swing(40.0, 20.0), 49.0); // capped < 50
    }

    #[test]
    fn rng_is_deterministic() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..10 {
            assert_eq!(a.next_u32(), b.next_u32());
        }
        assert!(Rng::new(42).range(-5, 5) >= -5 && Rng::new(42).range(-5, 5) <= 5);
    }
}
