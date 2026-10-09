//! Installation status, coordinated upgrade drain and removal, which the
//! runner's local endpoint answers beside the declared commands.

use demi_command_protocol::{Completion, LocalInvocation};
use demi_command_sdk::{Handler, InvocationContext, ServiceError};
use demi_runner_jobs::commands::dispatch::{Dispatcher, RUNNER, completed, reported};
use serde::{Deserialize, Serialize};
use std::{future::Future, pin::Pin, sync::Arc};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

/// The local operation that asks for the installation's status or its
/// drain; its arguments are a [`Request`].
pub const MANAGE: &str = "manage";

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Connecting,
    ClaimPending,
    Online,
    Rejected,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub secret: String,
    pub action: Action,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Status,
    Drain,
    /// Asks the backend to revoke the device, then drains (`runner.md`
    /// § Installation, pairing and removal).
    Uninstall,
}

/// What came of asking the backend to revoke the device, which `uninstall`
/// reports (`runner.md` § Installation, pairing and removal).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum Revocation {
    /// The backend revoked the device, and the projects of these names went
    /// with it; their files stay.
    Revoked { projects: Vec<String> },
    /// The runner was never paired, so there was no device to revoke.
    Unpaired,
    /// The backend could not be asked in time: it lists the device until the
    /// user revokes it.
    Unreached,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Status {
    release: String,
    pub phase: Phase,
    draining: bool,
    jobs: usize,
}

/// What the registration and its connection change while they run.
#[derive(Clone, Copy)]
struct Snapshot {
    phase: Phase,
    jobs: usize,
}

/// The installation's status and its drain (`runner.md` § Connection and
/// identity). The registration sets the phase and the connection the job
/// count; the management endpoint reads a snapshot of both.
pub struct Management {
    secret: String,
    release: String,
    snapshot: watch::Sender<Snapshot>,
    /// Asked to remove itself: the runner asks its backend to revoke the
    /// device, and drains once it answered or a bounded time passed.
    pub removing: CancellationToken,
    /// What came of the revocation once removing; none before.
    revocation: watch::Sender<Option<Revocation>>,
    pub draining: CancellationToken,
    pub stop: CancellationToken,
}

impl Management {
    pub fn new(secret: String, release: String, stop: CancellationToken) -> Arc<Self> {
        Arc::new(Self {
            secret,
            release,
            snapshot: watch::Sender::new(Snapshot {
                phase: Phase::Connecting,
                jobs: 0,
            }),
            removing: CancellationToken::new(),
            revocation: watch::Sender::new(None),
            draining: CancellationToken::new(),
            stop,
        })
    }

    /// Whether `request` carries the secret, compared in constant time.
    pub fn authorize(&self, request: &Request) -> bool {
        use subtle::ConstantTimeEq;
        request
            .secret
            .as_bytes()
            .ct_eq(self.secret.as_bytes())
            .into()
    }

    pub fn phase(&self) -> Phase {
        self.snapshot.borrow().phase
    }

    pub fn set_phase(&self, phase: Phase) {
        self.snapshot.send_modify(|snapshot| snapshot.phase = phase);
    }

    pub fn set_jobs(&self, jobs: usize) {
        self.snapshot.send_modify(|snapshot| snapshot.jobs = jobs);
    }

    /// Records what came of the revocation; only the first outcome counts.
    pub fn settle(&self, revocation: Revocation) {
        self.revocation.send_if_modified(|current| {
            let first = current.is_none();
            if first {
                *current = Some(revocation);
            }
            first
        });
    }

    /// What came of the revocation, once it settled.
    pub fn revocation(&self) -> Option<Revocation> {
        self.revocation.borrow().clone()
    }

    pub fn status(&self) -> Status {
        let snapshot = *self.snapshot.borrow();
        Status {
            release: self.release.clone(),
            phase: snapshot.phase,
            draining: self.draining.is_cancelled(),
            jobs: snapshot.jobs,
        }
    }
}

impl Management {
    /// Answers one management request: its secret checked, the drain or the
    /// removal begun when it asks for one, and the status written on its
    /// standard output; a removal writes what came of the revocation once it
    /// settled instead.
    async fn answer(
        &self,
        context: InvocationContext<LocalInvocation>,
    ) -> Result<Completion, ServiceError> {
        let request: Request = serde_json::from_value(context.request.args)?;
        if !self.authorize(&request) {
            return Err(ServiceError::failed(InvalidSecret));
        }
        let answer = match request.action {
            Action::Status => serde_json::to_vec(&self.status())?,
            Action::Drain => {
                self.draining.cancel();
                serde_json::to_vec(&self.status())?
            }
            Action::Uninstall => {
                self.removing.cancel();
                let mut settled = self.revocation.subscribe();
                // The management keeps the sender while it answers.
                let revocation = settled
                    .wait_for(Option::is_some)
                    .await
                    .map_err(ServiceError::failed)?
                    .clone();
                serde_json::to_vec(&revocation)?
            }
        };
        context.output.stdout(answer.into()).await?;
        Ok(completed(0))
    }
}

#[derive(Debug, thiserror::Error)]
#[error("invalid management secret")]
struct InvalidSecret;

/// What the runner's local endpoint serves: the declared commands the
/// dispatcher runs, and the management requests.
pub struct Endpoint {
    pub dispatcher: Arc<Dispatcher>,
    pub management: Arc<Management>,
}

impl Handler for Endpoint {
    type Metadata = LocalInvocation;

    fn operations(&self) -> Vec<String> {
        let mut operations = self.dispatcher.operations();
        operations.push(MANAGE.into());
        operations
    }

    fn invoke(
        &self,
        context: InvocationContext<LocalInvocation>,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        if context.request.operation != MANAGE {
            return self.dispatcher.invoke(context);
        }
        let management = self.management.clone();
        Box::pin(async move {
            let output = context.output.clone();
            reported(management.answer(context).await, &output, RUNNER).await
        })
    }
}
