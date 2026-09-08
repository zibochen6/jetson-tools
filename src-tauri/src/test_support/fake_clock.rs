//! A manually-advanced clock for deterministic tests.
//!
//! Time-dependent logic (backoff, timeouts, health polling) must read time
//! through this type in tests so scenarios advance the clock instead of
//! sleeping. Production code keeps using `std::time` directly — this module
//! is test infrastructure only, matching the "Adapter" principle where the
//! clock seam is swapped at the test boundary.

use std::sync::atomic::{AtomicU64, Ordering};

/// Monotonic-ish fake clock. `advance` moves `now` forward; `now` returns
/// the current fake instant (milliseconds since an arbitrary epoch).
#[derive(Debug, Default)]
pub struct FakeClock {
    now_ms: AtomicU64,
}

impl FakeClock {
    pub fn new(start_ms: u64) -> Self {
        Self {
            now_ms: AtomicU64::new(start_ms),
        }
    }

    pub fn now_ms(&self) -> u64 {
        self.now_ms.load(Ordering::Relaxed)
    }

    /// Move the clock forward and return the new value.
    pub fn advance(&self, delta_ms: u64) -> u64 {
        self.now_ms.fetch_add(delta_ms, Ordering::Relaxed) + delta_ms
    }

    /// Convenience for backoff loops: "time after N steps of `step_ms`".
    pub fn after(&self, steps: u64, step_ms: u64) -> u64 {
        self.advance(steps * step_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advance_is_additive_and_visible() {
        let clock = FakeClock::new(1_000);
        assert_eq!(clock.now_ms(), 1_000);
        assert_eq!(clock.advance(500), 1_500);
        assert_eq!(clock.now_ms(), 1_500);
        assert_eq!(clock.after(3, 250), 2_250);
    }

    #[test]
    fn advance_never_goes_backwards() {
        let clock = FakeClock::new(0);
        let a = clock.advance(10);
        let b = clock.advance(5);
        assert!(b >= a);
    }

    #[test]
    fn can_model_a_backoff_schedule() {
        let clock = FakeClock::new(0);
        let schedule = [200, 400, 800];
        let mut observed = Vec::new();
        for step in schedule {
            observed.push(clock.advance(step));
        }
        assert_eq!(observed, vec![200, 600, 1400]);
    }
}
