//! Thread-safe ordered event recorder for scenario tests.
//!
//! Scenarios emit named events (`tunnel.created`, `rdp.connected`, ...) and
//! assert on the sequence afterwards. Ordering is by monotonic sequence
//! number, so two threads racing to record still produce a strict total
//! order — exactly what lifecycle-order assertions (the cleanup order
//! contract) need.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedEvent {
    pub seq: u64,
    pub name: String,
}

impl fmt::Display for RecordedEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{} {}", self.seq, self.name)
    }
}

/// An `Arc`-shareable event log.
#[derive(Debug, Default)]
pub struct EventRecorder {
    next_seq: AtomicU64,
    events: Mutex<Vec<RecordedEvent>>,
}

impl EventRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&self, name: impl Into<String>) -> RecordedEvent {
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed) + 1;
        let event = RecordedEvent {
            seq,
            name: name.into(),
        };
        self.events
            .lock()
            .expect("event recorder poisoned")
            .push(event.clone());
        event
    }

    /// All recorded events, in recording order.
    pub fn events(&self) -> Vec<RecordedEvent> {
        self.events.lock().expect("event recorder poisoned").clone()
    }

    /// Names in recording order (the common assertion shape).
    pub fn names(&self) -> Vec<String> {
        self.events().into_iter().map(|e| e.name).collect()
    }

    /// True when every expected name appears, in this relative order,
    /// without any constraint on intervening events.
    pub fn contains_in_order(&self, expected: &[&str]) -> bool {
        let names = self.names();
        let mut cursor = 0;
        for want in expected {
            match names[cursor..].iter().position(|n| n == want) {
                Some(idx) => cursor += idx + 1,
                None => return false,
            }
        }
        true
    }

    /// True when the named event was never recorded.
    pub fn never(&self, forbidden: &str) -> bool {
        !self.names().iter().any(|n| n == forbidden)
    }

    /// Count of a given event name.
    pub fn count(&self, name: &str) -> usize {
        self.names().iter().filter(|n| n == &name).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordering_is_total_and_stable() {
        let recorder = EventRecorder::new();
        recorder.record("stop_rdp");
        recorder.record("detach_surface");
        recorder.record("close_tunnel");
        assert_eq!(
            recorder.names(),
            vec!["stop_rdp", "detach_surface", "close_tunnel"]
        );
        assert!(recorder.contains_in_order(&["detach_surface", "close_tunnel"]));
        assert!(recorder.contains_in_order(&["stop_rdp", "close_tunnel"]));
        assert!(!recorder.contains_in_order(&["close_tunnel", "stop_rdp"]));
        assert!(recorder.never("release_secrets"));
        assert_eq!(recorder.count("close_tunnel"), 1);
    }

    #[test]
    fn contains_in_order_ignores_intervening_events() {
        let recorder = EventRecorder::new();
        for name in ["a", "noise", "b", "noise", "c"] {
            recorder.record(name);
        }
        assert!(recorder.contains_in_order(&["a", "b", "c"]));
    }

    #[test]
    fn cross_thread_records_have_unique_seq() {
        use std::sync::Arc;
        let recorder = Arc::new(EventRecorder::new());
        let mut handles = Vec::new();
        for i in 0..8 {
            let r = recorder.clone();
            handles.push(std::thread::spawn(move || {
                for j in 0..100 {
                    r.record(format!("t{i}-e{j}"));
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let events = recorder.events();
        assert_eq!(events.len(), 800);
        let seqs: Vec<u64> = events.iter().map(|e| e.seq).collect();
        let mut sorted = seqs.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 800, "seq numbers must be unique");
    }
}
