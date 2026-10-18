//! A test's capture of what it logs (`crates-and-packages.md`
//! § `log-capture`).
//!
//! A test captures its logs with a subscriber of its own, the default of its
//! thread, while other tests of the same process run beside it with none.
//! tracing-core decides once per log statement whether anyone listens, when
//! the statement first runs, and caches the answer for the whole process.
//! While a single dispatcher is registered it asks only the default of the
//! thread that ran the statement first: a statement another test's thread
//! ran first, with no subscriber, would never log again, in this test
//! either. A second dispatcher kept registered for the whole process, which
//! listens to nothing, makes it ask every dispatcher, so the statement is
//! checked at each event against the default of the thread that logs it.

use std::sync::OnceLock;

use tracing::Dispatch;
use tracing::subscriber::{DefaultGuard, NoSubscriber, Subscriber};

/// Makes `subscriber` the default of this thread until the guard is
/// dropped, hearing what this thread logs, and what threads that take this
/// thread's default for theirs log, as a backend's shard threads do.
pub fn capture(subscriber: impl Subscriber + Send + Sync + 'static) -> DefaultGuard {
    static LISTENING_TO_NOTHING: OnceLock<Dispatch> = OnceLock::new();
    LISTENING_TO_NOTHING.get_or_init(|| Dispatch::new(NoSubscriber::default()));
    tracing::subscriber::set_default(subscriber)
}
