//! Installation status and coordinated upgrade drain.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
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
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    release: String,
    phase: Phase,
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
            draining: CancellationToken::new(),
            stop,
        })
    }

    /// Whether `request` carries the secret, compared in constant time.
    pub fn authorize(&self, request: &Request) -> bool {
        use subtle::ConstantTimeEq;
        request.secret.as_bytes().ct_eq(self.secret.as_bytes()).into()
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
