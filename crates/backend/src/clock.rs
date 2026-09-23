//! Wall-clock time.

use jiff::Timestamp;

/// Where the backend reads wall-clock time: the instants it stores, such as
/// a session's expiry, and the times its answers show. Durations measured in
/// memory, such as the login lockout, use Tokio's clock instead
/// (`concurrency.md` § Tests and time).
pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> Timestamp;
}

/// The system's clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }
}
