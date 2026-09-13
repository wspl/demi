//! Installation status and coordinated upgrade drain.

use crate::tasks::TaskTable;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
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

pub struct Management {
    secret: String,
    release: String,
    phase: Mutex<Phase>,
    tasks: Mutex<Option<TaskTable>>,
    pub draining: CancellationToken,
    pub stop: CancellationToken,
}

impl Management {
    pub fn new(secret: String, release: String, stop: CancellationToken) -> Arc<Self> {
        Arc::new(Self {
            secret,
            release,
            phase: Mutex::new(Phase::Connecting),
            tasks: Mutex::new(None),
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
    pub fn attach(&self, tasks: TaskTable) {
        *self.tasks.lock().unwrap() = Some(tasks);
    }
    pub fn detach(&self) {
        self.tasks.lock().unwrap().take();
    }
    pub fn phase(&self) -> Phase {
        *self.phase.lock().unwrap()
    }
    pub fn set_phase(&self, phase: Phase) {
        *self.phase.lock().unwrap() = phase;
    }
    pub fn status(&self) -> Status {
        Status {
            release: self.release.clone(),
            phase: *self.phase.lock().unwrap(),
            draining: self.draining.is_cancelled(),
            jobs: self
                .tasks
                .lock()
                .unwrap()
                .as_ref()
                .map_or(0, TaskTable::count),
        }
    }
    pub async fn wait_jobs(&self) {
        let tasks = self.tasks.lock().unwrap().clone();
        if let Some(tasks) = tasks {
            tasks.wait_idle().await;
        }
    }
}
