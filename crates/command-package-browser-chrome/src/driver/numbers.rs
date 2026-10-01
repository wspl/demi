//! A conversation's tab numbers (`browser.md` § One tab registry). The
//! service draws them from the conversation's `tab` sequence in the backend a
//! few at a time (`native-runtime.md` § Conversation numbers), so most tabs
//! take theirs without waiting; a number it never uses is a gap.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use demi_command_protocol::{MAX_NUMBERS, ServiceSequence};
use demi_command_sdk::Numbers;

use crate::driver::operation::{BrowserError, Result};

/// How many numbers one draw takes when fewer are needed.
const DRAW: u32 = 8;
/// How long the backend has to answer a draw.
const DRAW_TIMEOUT: Duration = Duration::from_secs(15);

/// One conversation's tab numbers at hand, and where more come from. Cloning
/// shares them.
#[derive(Clone)]
pub struct TabNumbers(Arc<Inner>);

struct Inner {
    /// Where more come from; none for a service that was given no numbers
    /// source.
    numbers: Option<Numbers>,
    conversation: String,
    spare: Mutex<VecDeque<u64>>,
}

impl TabNumbers {
    /// The tab numbers of `conversation`, drawn from `numbers`.
    pub fn new(numbers: Numbers, conversation: String) -> Self {
        Self::from_source(Some(numbers), conversation)
    }

    /// The tab numbers of `conversation`, drawn from `numbers` once the
    /// service has a numbers source.
    pub fn from_source(numbers: Option<Numbers>, conversation: String) -> Self {
        Self(Arc::new(Inner {
            numbers,
            conversation,
            spare: Mutex::new(VecDeque::new()),
        }))
    }

    /// Numbers at hand from the start and no source, for the registry's
    /// bookkeeping tests.
    #[cfg(any(test, feature = "testing"))]
    pub fn preset(numbers: impl IntoIterator<Item = u64>) -> Self {
        let preset = Self::from_source(None, "conversation".into());
        preset.spare().extend(numbers);
        preset
    }

    /// Makes sure at least `needed` numbers are at hand, drawing more when
    /// fewer are.
    pub async fn stock(&self, needed: usize) -> Result<()> {
        loop {
            let missing = needed.saturating_sub(self.spare().len());
            if missing == 0 {
                return Ok(());
            }
            let count = u32::try_from(missing)
                .unwrap_or(MAX_NUMBERS)
                .clamp(DRAW, MAX_NUMBERS);
            let numbers = self.0.numbers.as_ref().ok_or_else(|| {
                BrowserError::Unavailable("the service was given no tab numbers".into())
            })?;
            let draw = numbers.draw(&self.0.conversation, ServiceSequence::Tab, count);
            let first = tokio::time::timeout(DRAW_TIMEOUT, draw)
                .await
                .map_err(|_| {
                    BrowserError::Unavailable("the backend gave out no tab numbers in time".into())
                })?
                .map_err(|error| BrowserError::Unavailable(format!("no tab numbers: {error}")))?;
            self.spare().extend(first..first + u64::from(count));
        }
    }

    /// The next number at hand; the caller stocked it.
    pub fn take(&self) -> Option<u64> {
        self.spare().pop_front()
    }

    fn spare(&self) -> MutexGuard<'_, VecDeque<u64>> {
        // No section panics while it holds the lock.
        self.0.spare.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
