//! The login lockout (`backend.md` § Authentication and ownership).

use std::collections::HashMap;
use std::sync::Mutex;

use demi_web_api::text::EmailAddress;
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
pub(crate) struct LoginLimiter {
    /// Edge threads share the map; every section is a few map operations and
    /// never awaits.
    failures: Mutex<HashMap<EmailAddress, Failures>>,
}

impl LoginLimiter {
    pub(crate) fn new() -> Self {
        Self {
            failures: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn locked(&self, email: &EmailAddress) -> bool {
        let now = Instant::now();
        let mut failures = self.failures.lock().expect("the login limiter's lock is not poisoned");
        let Some(entry) = failures.get(email) else {
            return false;
        };
        if entry.locked_until.is_some_and(|until| until > now) {
            return true;
        }
        if entry.forget_at <= now {
            failures.remove(email);
        }
        false
    }

    pub(crate) fn failed(&self, email: &EmailAddress) {
        let now = Instant::now();
        let mut failures = self.failures.lock().expect("the login limiter's lock is not poisoned");
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

    pub(crate) fn succeeded(&self, email: &EmailAddress) {
        self.failures
            .lock()
            .expect("the login limiter's lock is not poisoned")
            .remove(email);
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
        assert!(limiter.locked(&address("a")));
        assert!(!limiter.locked(&address("b")));
        assert_eq!(tracked(&limiter), 3);

        tokio::time::advance(WINDOW).await;
        assert!(!limiter.locked(&address("a")));
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
        assert!(limiter.locked(&ana));
        tokio::time::advance(WINDOW - Duration::from_secs(1)).await;
        assert!(limiter.locked(&ana));
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(!limiter.locked(&ana));

        for _ in 1..LOCK_AFTER {
            limiter.failed(&ana);
        }
        limiter.succeeded(&ana);
        limiter.failed(&ana);
        assert!(!limiter.locked(&ana));
    }
}
