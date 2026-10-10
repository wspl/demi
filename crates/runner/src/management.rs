//! Installation status, coordinated upgrade drain and removal, which the
//! runner's local endpoint answers beside the declared commands.

use demi_command_protocol::{Completion, LocalInvocation};
use demi_command_sdk::{Handler, InvocationContext, ServiceError};
use demi_runner_jobs::commands::dispatch::{Dispatcher, RUNNER, completed, reported};
use demi_runner_protocol::console::{PAIRED, PAIRING_CODE, REMOVAL};
use serde::{Deserialize, Serialize};
use std::{
    fmt::Display,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, PoisonError},
};
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
    /// Writes the runner's pairing state to its log again, for an installer
    /// that finds it running (`runner.md` § Installation, pairing and
    /// removal); the answer is the status, which carries no code.
    Announce,
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

/// The lines the runner wrote to its console about its pairing, as it
/// wrote them, which it writes again when asked: the device it is paired
/// as, the code it waits with, and why it cannot connect. A connection
/// that brings a code or the pairing ends the failure, and one that fails
/// ends the code, which the backend then no longer holds.
#[derive(Default)]
struct Told {
    paired: Option<String>,
    code: Option<String>,
    failure: Option<String>,
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
    told: Mutex<Told>,
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
            told: Mutex::default(),
        })
    }

    fn told<T>(&self, change: impl FnOnce(&mut Told) -> T) -> T {
        change(&mut self.told.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// Tells the person at the console the pairing code the runner waits
    /// with; the code goes nowhere else, and the host log only says that one
    /// is waiting.
    pub fn tell_code(&self, code: impl Display) {
        let line = format!("{PAIRING_CODE}{code}");
        self.told(|told| {
            told.code = Some(line.clone());
            told.failure = None;
        });
        crate::console::line(line);
    }

    /// Tells the person at the console, the first time, that the device is
    /// paired as `name` and how to remove the runner again; an installer
    /// shows them these lines.
    pub fn tell_paired(&self, name: &str, removal: &str) {
        // One write, so that whoever reads the log sees both lines at once.
        let lines = format!("{PAIRED}{name}\n{REMOVAL}{removal}");
        let first = self.told(|told| {
            told.code = None;
            told.failure = None;
            told.paired.replace(lines.clone()).is_none()
        });
        if first {
            crate::console::line(lines);
        }
    }

    /// Keeps why the runner cannot connect, as its console line, which the
    /// registration writes.
    pub fn failed(&self, line: String) {
        self.told(|told| {
            told.code = None;
            told.failure = Some(line);
        });
    }

    /// Writes the lines of the runner's pairing state to its console again,
    /// as one write: the device it is paired as, the code it waits with, and
    /// why it cannot connect, whichever it has.
    pub fn tell_again(&self) {
        let lines = self.told(|told| {
            [&told.paired, &told.code, &told.failure]
                .into_iter()
                .flatten()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        });
        if !lines.is_empty() {
            crate::console::line(lines);
        }
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
            Action::Announce => {
                self.tell_again();
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
