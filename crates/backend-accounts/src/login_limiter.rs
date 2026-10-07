//! The login lockout (`backend.md` § Authentication and ownership).

use std::collections::HashMap;
use std::sync::Mutex;

use demi_web_api_protocol::text::EmailAddress;
use tokio::time::{Duration, Instant};

/// Failures, each within `WINDOW` of the one before, that lock an address.
const LOCK_AFTER: u32 = 5;
/// How long a lock holds, and how long an address's failures are remembered.
const WINDOW: Duration = Duration::from_secs(60);

struct Failures {
    count: u32,
    locked_until: Option<Instant>,
    /// `WINDOW` after the last failure: a lock's end at the latest.
    forget_at: Instant,
}

/// Counts failed logins per email address, whether or not an account has
/// it. Five failures lock the address for a minute, during which even the
/// right password is refused; a success clears the count. An entry is
/// forgotten a minute after its last failure, so a spray of addresses holds
/// memory for a minute each. The count lives in this process's memory.
pub struct LoginLimiter {
    /// Edge threads share the map; every section is a few map operations and
    /// never awaits.
    failures: Mutex<HashMap<EmailAddress, Failures>>,
}

impl LoginLimiter {
    pub fn new() -> Self {
        Self {
            failures: Mutex::new(HashMap::new()),
        }
    }

    /// How long `email` stays locked; `None` while it is not.
    pub fn locked_for(&self, email: &EmailAddress) -> Option<Duration> {
        let now = Instant::now();
        let mut failures = self
            .failures
            .lock()
            .expect("the login limiter's lock is not poisoned");
        let Some(entry) = failures.get(email) else {
            return None;
        };
        if let Some(until) = entry.locked_until.filter(|until| *until > now) {
            return Some(until - now);
        }
        if entry.forget_at <= now {
            failures.remove(email);
        }
        None
    }

    pub fn failed(&self, email: &EmailAddress) {
        let now = Instant::now();
        let mut failures = self
            .failures
            .lock()
            .expect("the login limiter's lock is not poisoned");
        failures.retain(|_, entry| entry.forget_at > now);
        let entry = failures.entry(email.clone()).or_insert(Failures {
            count: 0,
            locked_until: None,
            forget_at: now,
        });
        entry.count += 1;
        if entry.count >= LOCK_AFTER {
            entry.count = 0;
            entry.locked_until = Some(now + WINDOW);
        }
        entry.forget_at = now + WINDOW;
    }

    pub fn succeeded(&self, email: &EmailAddress) {
        self.failures
            .lock()
            .expect("the login limiter's lock is not poisoned")
            .remove(email);
    }
}

impl Default for LoginLimiter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(name: &str) -> EmailAddress {
        EmailAddress::try_from(format!("{name}@example.test")).unwrap()
    }

    fn tracked(limiter: &LoginLimiter) -> usize {
        limiter.failures.lock().unwrap().len()
    }

    #[tokio::test(start_paused = true)]
    async fn the_limiter_forgets_an_address_after_the_lock_window() {
        let limiter = LoginLimiter::new();
        for name in ["a", "b", "c"] {
            limiter.failed(&address(name));
        }
        for _ in 1..LOCK_AFTER {
            limiter.failed(&address("a"));
        }
        assert!(limiter.locked_for(&address("a")).is_some());
        assert!(limiter.locked_for(&address("b")).is_none());
        assert_eq!(tracked(&limiter), 3);

        tokio::time::advance(WINDOW).await;
        assert!(limiter.locked_for(&address("a")).is_none());
        // A failure sweeps what the window forgot, so sprayed addresses do
        // not accumulate.
        limiter.failed(&address("d"));
        assert_eq!(tracked(&limiter), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn each_failure_extends_the_window_and_a_success_clears_it() {
        let limiter = LoginLimiter::new();
        let ana = address("ana");
        for _ in 1..LOCK_AFTER {
            limiter.failed(&ana);
            tokio::time::advance(WINDOW - Duration::from_secs(1)).await;
        }
        limiter.failed(&ana);
        assert_eq!(limiter.locked_for(&ana), Some(WINDOW));
        tokio::time::advance(WINDOW - Duration::from_secs(1)).await;
        assert_eq!(limiter.locked_for(&ana), Some(Duration::from_secs(1)));
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(limiter.locked_for(&ana).is_none());

        for _ in 1..LOCK_AFTER {
            limiter.failed(&ana);
        }
        limiter.succeeded(&ana);
        limiter.failed(&ana);
        assert!(limiter.locked_for(&ana).is_none());
    }
}
