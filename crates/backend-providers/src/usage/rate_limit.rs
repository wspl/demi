//! The request rate limit: each user's conversations may start at most 120
//! provider requests in any 60 seconds.
//!
//! It is a ceiling that stops runaway loops, not a budget. It is kept in memory
//! with the user's other state on the user's shard, so a restart empties it.

use std::collections::VecDeque;
use std::time::Duration;

use tokio::time::Instant;

/// The requests a user may start in [`WINDOW`].
pub const REQUESTS_PER_WINDOW: usize = 120;

/// The window the limit counts requests in.
const WINDOW: Duration = Duration::from_secs(60);

/// One user's requests of the last minute.
#[derive(Debug)]
pub struct RequestRateLimit {
    limit: usize,
    /// When each request of the window started, oldest first.
    started: VecDeque<Instant>,
}

/// The request would exceed the limit; it never reaches the vendor and does
/// not count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Provider request rate limit reached ({limit} per minute)")]
pub struct RateLimited {
    pub limit: usize,
}

impl RequestRateLimit {
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            started: VecDeque::new(),
        }
    }

    /// Counts a request that starts now, or refuses it while the window
    /// holds the limit; requests are admitted again as the earliest leave
    /// the window.
    pub fn take(&mut self) -> Result<(), RateLimited> {
        let now = Instant::now();
        while self
            .started
            .front()
            .is_some_and(|started| now.duration_since(*started) >= WINDOW)
        {
            self.started.pop_front();
        }
        if self.started.len() >= self.limit {
            return Err(RateLimited { limit: self.limit });
        }
        self.started.push_back(now);
        Ok(())
    }
}
