//! A job's withdrawal: how its conversation's host access stops it when its
//! Host is no longer one of the conversation's, as a detach makes it.

use std::cell::RefCell;

use tokio_util::sync::CancellationToken;

/// A request, made once, to stop a running job because its Host was withdrawn
/// from the conversation. Whoever runs the job awaits [`asked`](Self::asked)
/// and stops it as an abort does, with the reason in its end; the job's host
/// access holds the request beside the job, and makes it.
#[derive(Default)]
pub struct Withdrawal {
    reason: RefCell<Option<String>>,
    asked: CancellationToken,
}

impl Withdrawal {
    /// Asks for the job to stop for `reason`, which its command's end tells;
    /// a second request keeps the first one's reason.
    pub fn withdraw(&self, reason: &str) {
        let mut held = self.reason.borrow_mut();
        if held.is_none() {
            *held = Some(reason.to_owned());
            self.asked.cancel();
        }
    }

    /// Resolves with the reason once the job is asked to stop, at once when
    /// it was asked already.
    pub async fn asked(&self) -> String {
        self.asked.cancelled().await;
        self.reason.borrow().clone().unwrap_or_default()
    }

    /// The reason the job was asked to stop for, once it was.
    pub fn reason(&self) -> Option<String> {
        self.reason.borrow().clone()
    }
}
