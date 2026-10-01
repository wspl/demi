//! A conversation's tab numbers (`browser.md` § One tab registry). The
//! service draws them from the conversation's `tab` sequence in the backend
//! eight at a time (`native-runtime.md` § Conversation numbers), so most
//! tabs take theirs without waiting; a number it never uses is a gap.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use demi_command_protocol::ServiceSequence;
use demi_command_sdk::Numbers;

use crate::driver::operation::{BrowserError, Result};

/// How many numbers one draw takes.
const DRAW: u32 = 8;
/// How long the backend has to answer a draw; past it, the step that needed
/// a number fails.
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

    /// The next number, drawing more first when none is at hand. A draw that
    /// fails or times out fails only this call, with why; the next call
    /// draws again.
    pub async fn next(&self) -> Result<u64> {
        if let Some(number) = self.spare().pop_front() {
            return Ok(number);
        }
        let numbers = self.0.numbers.as_ref().ok_or_else(|| {
            BrowserError::Unavailable("the service was given no tab numbers".into())
        })?;
        let draw = numbers.draw(&self.0.conversation, ServiceSequence::Tab, DRAW);
        let first = tokio::time::timeout(DRAW_TIMEOUT, draw)
            .await
            .map_err(|_| {
                BrowserError::Unavailable("the backend gave out no tab numbers in time".into())
            })?
            .map_err(|error| BrowserError::Unavailable(format!("no tab numbers: {error}")))?;
        let mut spare = self.spare();
        spare.extend(first..first + u64::from(DRAW));
        Ok(spare.pop_front().expect("a draw gives at least one number"))
    }

    fn spare(&self) -> MutexGuard<'_, VecDeque<u64>> {
        // No section panics while it holds the lock.
        self.0.spare.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
