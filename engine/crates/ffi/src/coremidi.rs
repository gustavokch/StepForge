//! CoreMIDI bindings + prod clock. The ONLY crate doing `unsafe`. RT-priority
//! syscall + MIDISend live here (Hard Rules 1, 6, 7).

use sequencer_engine::clock::Clock;
use std::time::Instant;

pub struct InstantClock {
    start: Instant,
}
impl InstantClock {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}
impl Default for InstantClock {
    fn default() -> Self {
        Self::new()
    }
}
impl Clock for InstantClock {
    fn now_micros(&self) -> u64 {
        self.start.elapsed().as_micros() as u64
    }
    fn sleep_until(&self, deadline_micros: u64) {
        let now = self.now_micros();
        if deadline_micros > now {
            let remaining = deadline_micros - now;
            // coarse sleep until ~1ms before, then spin the rest (sub-ms accuracy)
            if remaining > 1_000 {
                std::thread::sleep(std::time::Duration::from_micros(remaining - 1_000));
            }
            while self.now_micros() < deadline_micros {
                std::hint::spin_loop();
            }
        }
    }
    fn elevate_priority(&self) {
        // Called EXACTLY ONCE at RT-thread spawn (never in the per-tick loop).
        // Best-effort QoS elevation; failures are non-fatal (timing still correct).
        #[cfg(target_os = "ios")]
        unsafe {
            elevate_thread_rt_ios();
        }
        #[cfg(not(target_os = "ios"))]
        {
            let _ = 0;
        }
    }
}

#[cfg(target_os = "ios")]
unsafe fn elevate_thread_rt_ios() {
    // pthread_set_qos_class_self(QOS_CLASS_USER_INTERACTIVE, 0) via the sys crate
    // or libc; kept behind cfg so host tests don't link it. Implementation in Task 19
    // alongside the CoreMIDI worker (same ffi block). For now a no-op stub.
}

pub fn flush_all_notes_off(_channel: u8) {} // replaced by the worker in Task 19

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn instant_clock_is_monotonic_nonzero() {
        let c = InstantClock::new();
        let a = c.now_micros();
        std::thread::sleep(std::time::Duration::from_millis(1));
        let b = c.now_micros();
        assert!(b > a, "clock must be monotonic");
    }
    #[test]
    fn elevate_priority_does_not_panic() {
        let c = InstantClock::new();
        c.elevate_priority(); // once at spawn
    }
}
