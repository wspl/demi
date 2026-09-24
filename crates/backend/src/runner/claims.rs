//! Runners waiting to be paired (`backend.md` § Authentication and
//! ownership). An unpaired runner has no user yet, so it waits at the edge,
//! under the code it printed, until a signed-in user claims that code: the
//! claim creates the device and hands the waiting runner its token, and the
//! runner's socket moves into the user's shard. Each user may try ten codes
//! a minute.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use demi_runner_protocol::values::DeviceToken;
use demi_runner_protocol::wire::RunnerInfo;
use demi_web_api::devices::DeviceDto;
use demi_web_api::ids::UserId;
use tokio::sync::oneshot;
use tokio::time::Instant;

use super::codes::ClaimCode;
use crate::storage::devices::DeviceRecord;

/// The window claim attempts are counted in.
const ATTEMPT_WINDOW: Duration = Duration::from_secs(60);

/// The runners waiting to be paired, and each user's recent claims.
pub(crate) struct PendingClaims {
    /// A `std` mutex: the edge's threads share the waiting runners and the
    /// attempt windows, and no section awaits.
    state: Mutex<State>,
    attempts_per_minute: usize,
}

#[derive(Default)]
struct State {
    waiting: HashMap<ClaimCode, Waiting>,
    attempts: HashMap<UserId, VecDeque<Instant>>,
    closed: bool,
}

struct Waiting {
    runner: RunnerInfo,
    grant: oneshot::Sender<ClaimGrant>,
}

/// A waiting runner a claim took: it is no longer anyone else's to claim.
pub(crate) struct PendingRunner {
    pub(crate) runner: RunnerInfo,
    grant: oneshot::Sender<ClaimGrant>,
}

/// What a claim hands its runner: the device made for it, the token only
/// the runner receives, and where the device's answer goes once the runner's
/// socket is bound in the user's shard.
pub(crate) struct ClaimGrant {
    pub(crate) device: DeviceRecord,
    pub(crate) token: DeviceToken,
    pub(crate) bound: oneshot::Sender<DeviceDto>,
}

impl PendingClaims {
    pub(crate) fn new(attempts_per_minute: usize) -> Self {
        Self {
            state: Mutex::default(),
            attempts_per_minute,
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // No section panics while it holds the lock, so a poisoned one still
        // holds whole records.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Counts a claim attempt of `user`, unless the user made the most a
    /// minute allows already: then false, and nothing is counted.
    pub(crate) fn attempt(&self, user: &UserId) -> bool {
        let now = Instant::now();
        let mut state = self.lock();
        let attempts = state.attempts.entry(user.clone()).or_default();
        while attempts.front().is_some_and(|at| now.duration_since(*at) >= ATTEMPT_WINDOW) {
            attempts.pop_front();
        }
        if attempts.len() >= self.attempts_per_minute {
            return false;
        }
        attempts.push_back(now);
        true
    }

    /// The runner waiting under `code`, which only this claim holds from now
    /// on.
    pub(crate) fn take(&self, code: &ClaimCode) -> Option<PendingRunner> {
        let waiting = self.lock().waiting.remove(code)?;
        Some(PendingRunner {
            runner: waiting.runner,
            grant: waiting.grant,
        })
    }

    /// Puts `runner` up for claiming under `code`; none once the backend is
    /// shutting down.
    pub(super) fn wait(&self, code: ClaimCode, runner: RunnerInfo) -> Option<oneshot::Receiver<ClaimGrant>> {
        let mut state = self.lock();
        if state.closed {
            return None;
        }
        let (grant, granted) = oneshot::channel();
        state.waiting.insert(code, Waiting { runner, grant });
        Some(granted)
    }

    /// Takes a code back: it expired, or its runner went away.
    pub(super) fn withdraw(&self, code: &ClaimCode) {
        self.lock().waiting.remove(code);
    }

    /// Lets every waiting runner go, whose sockets then close, and puts up
    /// no more.
    pub(crate) fn close(&self) {
        let waiting = {
            let mut state = self.lock();
            state.closed = true;
            std::mem::take(&mut state.waiting)
        };
        drop(waiting);
    }
}

impl PendingRunner {
    /// Hands the runner its device and token. The answer is the device as it
    /// is once the runner is bound; none when the runner went away before.
    pub(crate) fn grant(self, device: DeviceRecord, token: DeviceToken) -> oneshot::Receiver<DeviceDto> {
        let (bound, answer) = oneshot::channel();
        // A runner that went away dropped its end, and `answer` then resolves
        // without a device.
        let _ = self.grant.send(ClaimGrant { device, token, bound });
        answer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(name: &str) -> UserId {
        UserId::try_from(name).unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn each_user_may_try_so_many_codes_a_minute() {
        let claims = PendingClaims::new(2);
        let (ana, ben) = (user("ana"), user("ben"));
        assert!(claims.attempt(&ana));
        tokio::time::advance(Duration::from_secs(30)).await;
        assert!(claims.attempt(&ana));
        assert!(!claims.attempt(&ana));
        assert!(claims.attempt(&ben));
        // The first attempt leaves the window, and one more fits.
        tokio::time::advance(Duration::from_secs(30)).await;
        assert!(claims.attempt(&ana));
        assert!(!claims.attempt(&ana));
    }
}
