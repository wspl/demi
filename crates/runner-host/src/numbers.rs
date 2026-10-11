//! The runner's numbers (`runner.md` § Watching files): one count of the
//! file system events the process received, which only grows, and the
//! epoch that names this run of the process. Every event takes the next
//! number as a watch receives it; every read takes the count before it
//! reads. So a change made after a read began has a higher number than the
//! read, and an event the read already saw has one no higher.
//!
//! The count is the process's, not a connection's: the backend's reads and
//! a direct channel's compare with the same subscriptions.

use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};

use demi_runner_protocol::wire::ReadAt;

static COUNT: AtomicU64 = AtomicU64::new(0);

/// A random name for this run, which a runner started again does not share.
static EPOCH: LazyLock<String> = LazyLock::new(|| {
    let random = uuid::Uuid::new_v4().as_u128() as u64;
    format!("{random:016x}")
});

/// The epoch of this run.
pub fn epoch() -> &'static str {
    &EPOCH
}

/// Where the count is now: what a read takes before it reads, and a
/// subscription when its watch starts to cover it.
pub fn now() -> u64 {
    // Sequentially consistent, with the increment below: a read that takes
    // the count after an event took its number sees that number.
    COUNT.load(Ordering::SeqCst)
}

/// The number of an event a watch just received.
pub(crate) fn take() -> u64 {
    COUNT.fetch_add(1, Ordering::SeqCst) + 1
}

/// The count now with the epoch, as a read's answer carries it.
pub fn read_at() -> ReadAt {
    ReadAt {
        epoch: EPOCH.clone(),
        number: now(),
    }
}
