//! A device's link session, which outlives each connection of its runner
//! (`crates/runner-protocol/src/wire/session.rs`), and the calls that go on
//! across them.
//!
//! The session is the numbers of the messages each end wrote and received, and
//! the messages the runner did not confirm. A connection of the same runner
//! start, to the session the backend holds, carries it on: the runner's hello
//! names the session's number and the count of messages it received, and the
//! driver writes again what follows. Any other hello starts the session
//! anew, under a number the backend draws, which the answer names: what the old
//! one kept is dropped, and its calls end with the reason.
//!
//! A call lives here, not on a connection ([`CallEntry`]): the runner's
//! `rpc_call` starts a handler that goes on when the connection under it
//! drops, and the messages the handler sends the runner wait in the call backlog
//! for whichever connection serves next. A call ends without its exit only
//! when its session ends, its job ends or the device is revoked. The session
//! holds the standard outputs of its calls that the runner reads over a
//! WebSocket of their own, which go with it ([`SinkSession`]).

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    sync::{Arc, Mutex},
};

use demi_host_interface::{HostError, HostErrorKind};
use demi_runner_protocol::wire::{self, Inbound, Resume};
use tokio_util::task::TaskTracker;

use crate::{
    link::END_GRACE,
    pipes::{PipeRefusal, Pipes, SinkSession, Sinks},
    relay::{CallEntry, Stop},
};

/// Why the calls of a session end when its runner started anew.
pub const RUNNER_RESTARTED_CALLS: &str = "the Host's runner started anew";

/// How a hello's connection relates to the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Beginning {
    /// The connection carries the session on; the runner received this many
    /// of the backend's messages.
    Resumed { received: u64 },
    /// The session is a new one: nothing of the runner's was received in it.
    Started,
}

impl Beginning {
    /// How many of the runner's numbered messages the backend received in
    /// the session this connection carries.
    pub fn received(self) -> u64 {
        match self {
            Self::Resumed { received } => received,
            Self::Started => 0,
        }
    }
}

/// A device's session. Cloning it is cheap; the clones are the same session.
#[derive(Clone)]
pub struct DeviceSession(Rc<Inner>);

struct Inner {
    device: String,
    /// The number the backend drew when it started the session, which the
    /// runner's hello names to resume it.
    number: Cell<u64>,
    numbers: RefCell<wire::Session>,
    /// The runner start whose session this is.
    instance: Cell<Option<u64>>,
    calls: RefCell<HashMap<String, Rc<CallEntry>>>,
    /// The messages of calls, which wait for a connection to write them.
    backlog: wire::Backlog,
    /// The standard outputs of the calls that resume, which outlive their
    /// pipes' records until the runner closed them.
    sinks: Sinks,
    /// The calls' handlers.
    tasks: TaskTracker,
}

impl DeviceSession {
    /// A session that no runner has carried on yet, of `device`.
    pub fn new(device: &str) -> Self {
        Self(Rc::new(Inner {
            device: device.to_owned(),
            number: Cell::new(0),
            numbers: RefCell::new(wire::Session::new()),
            instance: Cell::new(None),
            calls: RefCell::default(),
            backlog: wire::Backlog::default(),
            sinks: Arc::new(Mutex::new(HashMap::new())),
            tasks: TaskTracker::new(),
        }))
    }

    /// The number of the session, which the answer to a hello names.
    pub fn number(&self) -> u64 {
        self.0.number.get()
    }

    /// Whether a hello of runner start `instance` with `resume` carries the
    /// session on: the same runner start, the session's number, and a count
    /// of received messages between what the runner confirmed and what the
    /// backend wrote.
    pub fn resumes(&self, instance: u64, resume: Option<Resume>) -> bool {
        let Some(resume) = resume else {
            return false;
        };
        self.0.instance.get() == Some(instance)
            && resume.session == self.0.number.get()
            && self.0.numbers.borrow().accepts(resume.received)
    }

    /// Takes a hello: carries the session on when `resumes` says so, and
    /// otherwise starts it anew under a new number, which ends the calls it
    /// had.
    pub fn begin(&self, instance: u64, resume: Option<Resume>) -> Beginning {
        if self.resumes(instance, resume) {
            let resume = resume.expect("a session resumes with a hello that names it");
            if self.0.numbers.borrow_mut().resume(resume.received).is_ok() {
                return Beginning::Resumed {
                    received: self.0.numbers.borrow().received(),
                };
            }
        }
        let known = self.0.instance.replace(Some(instance));
        let reason = match known {
            None => None,
            Some(previous) if previous != instance => Some(RUNNER_RESTARTED_CALLS),
            Some(_) => Some("the runner did not resume the link's session"),
        };
        self.restart(reason);
        self.0.number.set(uuid::Uuid::new_v4().as_u64_pair().0);
        Beginning::Started
    }

    fn restart(&self, reason: Option<&str>) {
        self.0.numbers.borrow_mut().restart();
        self.0.backlog.clear();
        self.forget_sinks();
        if let Some(reason) = reason {
            tracing::info!(event = "link.session_ended", cause = reason, "a runner's link session ended");
            self.stop_calls(reason);
        }
    }

    /// Ends the session: every call stops with `reason`, and what waits for
    /// the runner is dropped. The device was revoked.
    pub fn end(&self, reason: &str) {
        self.0.instance.set(None);
        self.restart(Some(reason));
    }

    /// Stops every call with `reason` and waits for their handlers, which
    /// have [`END_GRACE`] to finish, such as one committing its change, and
    /// what still runs then is aborted. The backend shuts down.
    pub async fn drain(&self, reason: &str) {
        self.stop_calls(reason);
        self.0.tasks.close();
        self.0.tasks.wait().await;
    }

    fn stop_calls(&self, reason: &str) {
        let calls: Vec<_> = self.0.calls.borrow_mut().drain().map(|(_, call)| call).collect();
        for call in calls {
            tracing::info!(event = "call.lost", call_job = call.job_id.as_str(), reason, "a runner's call ended with its session");
            call.stop(Stop::Ended(reason.into()));
        }
    }

    /// Lets go of the standard outputs of the calls that resume: their session
    /// ended, so nobody reads them again.
    fn forget_sinks(&self) {
        let forgotten: Vec<_> = self
            .0
            .sinks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain()
            .map(|(_, sink)| sink)
            .collect();
        for sink in forgotten {
            sink.cancel();
        }
    }

    /// The numbers of the messages written and received, which the connection
    /// that serves reads and writes through.
    pub(crate) fn numbers(&self) -> &RefCell<wire::Session> {
        &self.0.numbers
    }

    /// Runs a call's handler: it belongs to the device, which the connection
    /// that started the call is not. Once the call stops it has [`END_GRACE`]
    /// to finish, and what still runs then is aborted, after which `aborted`
    /// ends the call as the handler would have, so its caller is answered.
    pub(crate) fn spawn_call(
        &self,
        name: String,
        stopped: tokio_util::sync::CancellationToken,
        task: impl std::future::Future<Output = ()> + 'static,
        aborted: impl FnOnce() + 'static,
    ) {
        let device = self.0.device.clone();
        self.0.tasks.spawn_local(async move {
            let mut aborted_handler = false;
            tokio::select! {
                biased;
                () = task => {}
                () = async {
                    stopped.cancelled().await;
                    tokio::time::sleep(END_GRACE).await;
                } => {
                    aborted_handler = true;
                    tracing::warn!(
                        event = "link_task.aborted",
                        host_id = %device,
                        task = name.as_str(),
                        grace_ms = END_GRACE.as_millis() as u64,
                        "a call's handler was aborted after its call ended"
                    );
                }
            }
            // Only an aborted handler leaves the call unanswered.
            if aborted_handler {
                aborted();
            }
        });
    }

    /// The standard output of call `id` as the runner reads it: the session
    /// that holds it, which the pipe's sink end is taken for the first time.
    pub fn call_output(&self, pipes: &Pipes, id: &str) -> Result<Arc<SinkSession>, PipeRefusal> {
        let known = self
            .0
            .sinks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(id)
            .cloned();
        if let Some(sink) = known {
            return Ok(sink);
        }
        let sink = pipes.claim_call_sink(id, &self.0.device, &self.0.sinks)?;
        self.0
            .sinks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id.into(), sink.clone());
        Ok(sink)
    }

    /// Registers the call `id`; false when the runner started it already.
    pub(crate) fn add_call(&self, id: String, call: Rc<CallEntry>) -> bool {
        match self.0.calls.borrow_mut().entry(id) {
            std::collections::hash_map::Entry::Occupied(_) => false,
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(call);
                true
            }
        }
    }

    pub(crate) fn call(&self, id: &str) -> Option<Rc<CallEntry>> {
        self.0.calls.borrow().get(id).cloned()
    }

    pub(crate) fn remove_call(&self, id: &str) -> Option<Rc<CallEntry>> {
        self.0.calls.borrow_mut().remove(id)
    }

    /// The calls of job `job`.
    pub(crate) fn calls_of(&self, job: &str) -> Vec<Rc<CallEntry>> {
        self.0
            .calls
            .borrow()
            .values()
            .filter(|call| call.job_id == job)
            .cloned()
            .collect()
    }

    /// Every call, which a connection checks against the jobs its hello
    /// lists.
    pub(crate) fn calls(&self) -> Vec<Rc<CallEntry>> {
        self.0.calls.borrow().values().cloned().collect()
    }

    /// Queues `message` for the runner behind the messages of calls before
    /// it: it waits for a connection to write it, however long that takes.
    pub(crate) fn send(&self, message: &Inbound) -> Result<(), HostError> {
        let frame = wire::encode(message)
            .map_err(|error| HostError::new(HostErrorKind::Protocol, error.to_string()))?;
        self.0.backlog.push(frame);
        Ok(())
    }

    /// The oldest message of a call, waiting for one; cancel-safe.
    pub(crate) async fn next_message(&self) -> wire::Frame {
        self.0.backlog.next().await
    }
}
