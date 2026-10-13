//! A limit on how often each key may try something: at most so many
//! attempts within a sliding window, such as ten pairing codes a user a
//! minute.

use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use tokio::time::Instant;

/// Counts each key's attempts within the last `window`. A key whose
/// attempts all passed out of the window is forgotten, so a spray of keys
/// holds memory for one window each. The counts live in this process's
/// memory.
pub struct RateWindow<K> {
    most: usize,
    window: Duration,
    /// A `std` mutex: callers on several threads share it, and no section
    /// awaits.
    attempts: Mutex<HashMap<K, VecDeque<Instant>>>,
}

impl<K: Eq + Hash + Clone> RateWindow<K> {
    pub fn new(most: usize, window: Duration) -> Self {
        Self {
            most,
            window,
            attempts: Mutex::default(),
        }
    }

    /// Counts an attempt of `key`, unless the key made the most the window
    /// allows already: then false, and nothing is counted.
    pub fn attempt(&self, key: &K) -> bool {
        let now = Instant::now();
        // No section panics while it holds the lock, so a poisoned one still
        // holds whole counts.
        let mut attempts = self.attempts.lock().unwrap_or_else(PoisonError::into_inner);
        attempts.retain(|_, times| {
            while times
                .front()
                .is_some_and(|at| now.duration_since(*at) >= self.window)
            {
                times.pop_front();
            }
            !times.is_empty()
        });
        let times = attempts.entry(key.clone()).or_default();
        if times.len() >= self.most {
            return false;
        }
        times.push_back(now);
        true
    }
}
