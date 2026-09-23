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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub secret: String,
    pub action: Action,
}
#[derive(Deserialize)]
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

    pub fn authorize(&self, request: &Request) -> bool {
        // Secrets have fixed public length; compare every byte for a same-length value.
        request.secret.len() == self.secret.len()
            && request
                .secret
                .bytes()
                .zip(self.secret.bytes())
                .fold(0, |difference, (left, right)| difference | (left ^ right))
                == 0
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
