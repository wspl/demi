//! A value read from a vendor's service and kept for a while, such as the
//! newest release of a vendor's CLI (`claude-code.md` § Which version,
//! `models.md` § The Codex catalog): believed until it is older than its
//! lifetime, then read again in the background while the kept one still
//! answers, so that a reader that can use it never waits. Concurrent readers
//! share one read, and a failed read keeps what was read before.

use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use demi_shared_types::{Clock, Timestamp};
use jiff::SignedDuration;
use tokio::runtime::Handle;

/// The kept value of one read.
pub struct KeptRead<T> {
    /// How long a value read is believed.
    lifetime: SignedDuration,
    clock: Arc<dyn Clock>,
    /// Held across a read, so that concurrent readers share it, and tried
    /// to tell whether a read is under way.
    reading: tokio::sync::Mutex<()>,
    /// A `std` mutex: no section awaits.
    state: Mutex<State<T>>,
}

struct State<T> {
    /// The last value read and when.
    value: Option<(Timestamp, T)>,
    /// How many reads have succeeded, which tells a refresh whether one
    /// finished after it began.
    reads: u64,
}

/// What [`KeptRead::kept`] answers at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kept<T> {
    pub value: T,
    /// Whether it was read within its lifetime.
    pub fresh: bool,
}

impl<T: Clone> KeptRead<T> {
    /// Nothing read yet; a value read is believed for `lifetime` by `clock`.
    pub fn new(lifetime: SignedDuration, clock: Arc<dyn Clock>) -> Self {
        Self {
            lifetime,
            clock,
            reading: tokio::sync::Mutex::new(()),
            state: Mutex::new(State {
                value: None,
                reads: 0,
            }),
        }
    }

    /// The value read within its lifetime, or a new read by `read`, which a
    /// concurrent caller waits for and shares. With `refresh`, only a read
    /// that finished after the call began answers without a new one. A
    /// failed read keeps the value read before and fails this call alone.
    pub async fn read<F, Fut, E>(&self, refresh: bool, read: F) -> Result<T, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        let asked = self.state().reads;
        let _reading = self.reading.lock().await;
        {
            let state = self.state();
            if let Some((read_at, value)) = &state.value {
                let fresh = if refresh {
                    state.reads > asked
                } else {
                    self.fresh(*read_at)
                };
                if fresh {
                    return Ok(value.clone());
                }
            }
        }
        let value = read().await?;
        let mut state = self.state();
        state.value = Some((self.clock.now(), value.clone()));
        state.reads += 1;
        Ok(value)
    }

    /// The value last read, fresh or not, without starting a read; none
    /// before the first read succeeds.
    pub fn last(&self) -> Option<T> {
        let state = self.state();
        state.value.as_ref().map(|(_, value)| value.clone())
    }

    /// The value last read, at once, fresh or not; none before the first
    /// read succeeds. When it is older than its lifetime or missing, and no
    /// read is under way, `read_again` is spawned on `edge`: it reads by
    /// [`read`](Self::read) for a later call to answer with, and says what
    /// becomes of a failure. Otherwise it is dropped unpolled.
    pub fn kept(
        &self,
        edge: &Handle,
        read_again: impl Future<Output = ()> + Send + 'static,
    ) -> Option<Kept<T>> {
        let kept = self.state().value.as_ref().map(|(read_at, value)| Kept {
            value: value.clone(),
            fresh: self.fresh(*read_at),
        });
        let current = kept.as_ref().is_some_and(|kept| kept.fresh);
        // A read under way holds the lock and keeps what it reads.
        if !current && self.reading.try_lock().is_ok() {
            edge.spawn(read_again);
        }
        kept
    }

    fn fresh(&self, read_at: Timestamp) -> bool {
        self.clock.now().to_jiff().duration_since(read_at.to_jiff()) < self.lifetime
    }

    fn state(&self) -> MutexGuard<'_, State<T>> {
        // A section only reads or replaces the value.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
