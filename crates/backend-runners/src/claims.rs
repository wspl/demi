//! Runners waiting to be paired (`backend.md` § Authentication and
//! ownership). An unpaired runner has no user yet, so it waits at the edge,
//! under the code it printed, until a signed-in user claims that code: the
//! claim creates the device and hands the waiting runner its token, and the
//! runner's socket moves into the user's shard. Each user may try ten codes
//! a minute.
//!
//! An agent pairs a computer it reaches without the user typing the code (`demi host
//! pair`): the code is proof enough, since it reaches only the runner's output and
//! its log, which only an account that runs commands on that computer reads, so an
//! agent holding it already controls the computer.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use demi_backend_database::StorageError;
use demi_backend_database::accounts::TokenHash;
use demi_backend_database::control::ControlService;
use demi_backend_database::devices::DeviceRecord;
use demi_runner_protocol::values::DeviceToken;
use demi_runner_protocol::wire::RunnerInfo;
use demi_web_api_protocol::devices::DeviceDto;
use demi_web_api_protocol::ids::UserId;
use tokio::sync::oneshot;

use demi_shared_gates::RateWindow;

use crate::codes::{ClaimCode, new_device_token};

/// The window claim attempts are counted in.
const ATTEMPT_WINDOW: Duration = Duration::from_secs(60);

/// The runners waiting to be paired, and each user's recent claims.
pub struct PendingClaims {
    /// A `std` mutex: the edge's threads share the waiting runners, and no
    /// section awaits.
    state: Mutex<State>,
    /// Each user's claims within the last minute.
    attempts: RateWindow<UserId>,
}

#[derive(Default)]
struct State {
    waiting: HashMap<ClaimCode, Waiting>,
    closed: bool,
}

struct Waiting {
    runner: RunnerInfo,
    grant: oneshot::Sender<ClaimGrant>,
}

/// A waiting runner a claim took: it is no longer anyone else's to claim.
pub struct PendingRunner {
    pub runner: RunnerInfo,
    grant: oneshot::Sender<ClaimGrant>,
}

/// Why a claim paired no device.
#[derive(Debug, thiserror::Error)]
pub enum ClaimRefusal {
    /// The user tried as many codes as a minute allows.
    #[error("too many claim attempts")]
    TooManyAttempts,
    /// No runner waits under the code: it expired, was claimed, or was
    /// never issued.
    #[error("unknown or expired pairing code")]
    UnknownCode,
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// What a claim hands its runner: the device made for it, the token only
/// the runner receives, and where the device's answer goes once the runner's
/// socket is bound in the user's shard.
pub struct ClaimGrant {
    pub device: DeviceRecord,
    pub token: DeviceToken,
    pub bound: oneshot::Sender<DeviceDto>,
}

impl PendingClaims {
    pub fn new(attempts_per_minute: usize) -> Self {
        Self {
            state: Mutex::default(),
            attempts: RateWindow::new(attempts_per_minute, ATTEMPT_WINDOW),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // No section panics while it holds the lock, so a poisoned one still
        // holds whole records.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Pairs the runner waiting under `code`, spelled as a person types it,
    /// with `user`: the device is made, its token goes to that runner alone,
    /// and the answer is the device once the runner's socket is bound in the
    /// user's shard. Add Device's `POST /devices/claim` and `demi host pair`
    /// both claim this way.
    pub async fn claim(
        &self,
        control: &ControlService,
        user: &UserId,
        code: &str,
    ) -> Result<DeviceDto, ClaimRefusal> {
        if !self.attempts.attempt(user) {
            return Err(ClaimRefusal::TooManyAttempts);
        }
        let pending = ClaimCode::parse(code)
            .and_then(|code| self.take(&code))
            .ok_or(ClaimRefusal::UnknownCode)?;
        let token = new_device_token();
        let device = control
            .create_device(
                user.clone(),
                pending.runner.name.clone(),
                pending.runner.platform,
                TokenHash::of(token.expose()),
            )
            .await?;
        let id = device.id.clone();
        match pending.grant(device, token).await {
            Ok(device) => Ok(device),
            Err(_) => {
                // The runner went away before it held its token: the device
                // it would have been goes too.
                control.delete_device(id).await?;
                Err(ClaimRefusal::UnknownCode)
            }
        }
    }

    /// The runner waiting under `code`, which only this claim holds from now
    /// on.
    fn take(&self, code: &ClaimCode) -> Option<PendingRunner> {
        let waiting = self.lock().waiting.remove(code)?;
        Some(PendingRunner {
            runner: waiting.runner,
            grant: waiting.grant,
        })
    }

    /// Puts `runner` up for claiming under `code`; none once the backend is
    /// shutting down.
    pub fn wait(
        &self,
        code: ClaimCode,
        runner: RunnerInfo,
    ) -> Option<oneshot::Receiver<ClaimGrant>> {
        let mut state = self.lock();
        if state.closed {
            return None;
        }
        let (grant, granted) = oneshot::channel();
        state.waiting.insert(code, Waiting { runner, grant });
        Some(granted)
    }

    /// Takes a code back: it expired, or its runner went away.
    pub fn withdraw(&self, code: &ClaimCode) {
        self.lock().waiting.remove(code);
    }

    /// Lets every waiting runner go, whose sockets then close, and puts up
    /// no more.
    pub fn close(&self) {
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
    pub fn grant(self, device: DeviceRecord, token: DeviceToken) -> oneshot::Receiver<DeviceDto> {
        let (bound, answer) = oneshot::channel();
        // A runner that went away dropped its end, and `answer` then resolves
        // without a device.
        let _ = self.grant.send(ClaimGrant {
            device,
            token,
            bound,
        });
        answer
    }
}
