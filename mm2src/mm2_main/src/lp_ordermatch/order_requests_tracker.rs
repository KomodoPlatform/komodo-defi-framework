#![allow(dead_code)]
use std::{collections::hash_map::{HashMap, RawEntryMut},
          num::NonZeroUsize,
          time::Duration};

use instant::Instant;

const ONE_SECOND: Duration = Duration::from_secs(1);

/// Stores the timestamps of order requests sent to specific peer
pub struct OrderRequestsTracker {
    requested_at: HashMap<String, Vec<Instant>>,
    limit_per_sec: NonZeroUsize,
}

impl Default for OrderRequestsTracker {
    fn default() -> OrderRequestsTracker { OrderRequestsTracker::new(NonZeroUsize::new(5).unwrap()) }
}

impl OrderRequestsTracker {
    /// Create new tracker with `limit` requests per second
    pub fn new(limit_per_sec: NonZeroUsize) -> OrderRequestsTracker {
        OrderRequestsTracker {
            requested_at: HashMap::new(),
            limit_per_sec,
        }
    }

    pub fn peer_requested(&mut self, peer: &str) {
        let now = Instant::now();
        let peer_requested_at = match self.requested_at.raw_entry_mut().from_key(peer) {
            RawEntryMut::Occupied(e) => e.into_mut(),
            RawEntryMut::Vacant(e) => {
                let tuple = e.insert(peer.to_string(), Vec::with_capacity(self.limit_per_sec.get()));
                tuple.1
            },
        };
        if peer_requested_at.len() >= self.limit_per_sec.get() {
            peer_requested_at.pop();
        }
        peer_requested_at.insert(0, now);
    }

    pub fn limit_reached(&self, peer: &str) -> bool {
        match self.requested_at.get(peer) {
            Some(requested) => {
                if requested.len() < self.limit_per_sec.get() {
                    false
                } else {
                    let min = requested.last().expect("last() can not be None as len > 0");
                    let now = Instant::now();
                    now.duration_since(*min) < ONE_SECOND
                }
            },
            None => false,
        }
    }
}

#[cfg(test)]
mod order_requests_tracker_tests {
    use std::{thread::sleep, time::Duration};

    use super::*;

    // TODO investigate why this fails on MacOS
    #[test]
    #[ignore]
    fn test_limit_reached_true() {
        let limit = NonZeroUsize::new(5).unwrap();
        let mut tracker = OrderRequestsTracker::new(limit);
        let peer = "peer";
        for _ in 0..5 {
            tracker.peer_requested(peer);
            sleep(Duration::from_millis(100));
        }

        assert!(tracker.limit_reached(peer));
    }

    #[test]
    fn test_limit_reached_false() {
        let limit = NonZeroUsize::new(5).unwrap();
        let mut tracker = OrderRequestsTracker::new(limit);
        let peer = "peer";
        for _ in 0..5 {
            tracker.peer_requested(peer);
            sleep(Duration::from_millis(201));
        }

        assert!(!tracker.limit_reached(peer));
    }
}
