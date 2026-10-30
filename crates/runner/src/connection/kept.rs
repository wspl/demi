//! What the runner keeps through a connection loss: its jobs with their
//! directories and execution contexts, and the requests its jobs' commands make
//! of the backend.
//!
//! A connection serves them while it lasts. Without one:
//!
//! - the jobs run on however long it takes;
//! - their commands' requests wait for the next connection, and their calls
//!   go on: a call's messages wait for it in the link's backlog, and its events
//!   come back over the next connection, which carries on the link's session
//!   from the message the backend got to;
//! - the messages the runner would have sent about them are not queued, since
//!   the next connection reads what it needs from the kept output.
//!
//! A laptop whose Wi-Fi drops for two minutes in the middle of a test suite
//! therefore delivers the suite's end and its output to the conversation as if
//! the connection had never dropped.
//!
//! # What stops them
//!
//! Only the runner's end stops them:
//!
//! - A backend that refuses the runner (the device was revoked or removed) has
//!   them stopped as a stop does, since nobody will read them again.
//! - A runner that is stopped, drained or removed stops them. One that
//!   crashes or updates leaves them running in their shells' processes, and
//!   the runner that starts next takes them over
//!   (`crates/runner-shell/src/remote/mod.rs`). A Cloud's stop ends them with
//!   the Cloud.
//!
//! The hello lists the jobs with the runner's instance, a number each start
//! draws, so that the backend tells a runner that kept its jobs from one that
//! started anew. A job the backend has no command for is stopped.
//!
//! The runner notices a loss itself: it pings the backend and counts a
//! connection that says nothing for `LINK_SILENCE` as lost.

use std::sync::Arc;

use demi_runner_jobs::{
    commands::contexts::ContextTable,
    connection::{ConnectionHandle, Ended, Live, Reach, Relay, Request},
    job_directories::JobDirectories,
    tasks::{Commands, JobConfig, JobTable, WorkId},
};
use demi_runner_host::worktrees::Worktrees;
use demi_runner_protocol::wire;
use tokio::{
    sync::{mpsc, watch},
    task::JoinSet,
};
use tokio_util::task::AbortOnDropHandle;

use super::{LinkState, Registered};

/// What the registration keeps from one connection to the next.
pub struct Kept {
    pub jobs: JobTable,
    pub directories: Arc<JobDirectories>,
    pub contexts: ContextTable,
    pub relay: Relay,
    pub watches: JoinSet<Ended>,
    /// The link's numbers, which each connection carries on.
    pub link: LinkState,
    /// How the jobs' commands reach the backend.
    pub handle: ConnectionHandle,
    /// The projects' worktrees, which the backlog reports ahead of a message
    /// that could follow their change.
    pub worktrees: Worktrees,
    requests: mpsc::UnboundedReceiver<Request>,
    reach: watch::Sender<Reach>,
    /// The connection the jobs' messages go to; none while none serves.
    output: watch::Sender<Option<mpsc::UnboundedSender<wire::Frame>>>,
    /// The queue the jobs' messages wait in for that connection, which a
    /// message about a job's context joins behind the job's exit.
    queue: mpsc::UnboundedSender<wire::Frame>,
    /// Passes the jobs' messages on to that connection.
    _forwarder: AbortOnDropHandle<()>,
}

/// What happened to what the registration keeps.
pub enum Event {
    Request(Request),
    Finished(WorkId),
    Watch(Ended),
}

impl Kept {
    /// The registration's jobs under its job root: those a runner before
    /// this one left whose shells still run, which it takes over, and no
    /// other. The directories a runner that ended left of any other job are
    /// removed.
    pub async fn open(registered: &Registered) -> Self {
        let (directories, left) = JobDirectories::open(registered.jobs.clone()).await;
        let (reach, reaching) = watch::channel(Reach::Away);
        let link = LinkState::default();
        let worktrees = Worktrees::default();
        let reporter = worktrees.clone();
        link.backlog().set_reporter(Box::new(move |backlog| reporter.report(backlog)));
        let (handle, requests) = ConnectionHandle::new(reaching, link.backlog().clone());
        let (jobs_output, forwarded) = mpsc::unbounded_channel();
        let output = watch::Sender::new(None);
        let forwarder = tokio::spawn(forward(forwarded, output.subscribe()));
        let queue = jobs_output.clone();
        let jobs = JobTable::new(JobConfig {
            output: jobs_output,
            directories: directories.clone(),
            scratch: registered.scratch.clone(),
            pipes: registered.pipes.clone(),
            shell: registered.shell.clone(),
            commands: Some(Commands {
                dispatcher: registered.dispatcher.clone(),
                connection: handle.clone(),
                paths: registered.paths.clone(),
                residents: registered.residents.clone(),
                endpoint: registered.endpoint.clone(),
                home: registered.state.root.to_string_lossy().into_owned(),
            }),
            leaving: Arc::default(),
        });
        let mut jobs = jobs;
        let mut contexts = ContextTable::new(registered.index.clone());
        for path in left {
            match jobs.reattach(path.clone(), &mut contexts).await {
                // The job's life logs its takeover.
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(event = "job.reattach_failed", directory = %path.display(), %error, "a job a runner before this one started was not taken over");
                    directories.remove_left(path).await;
                }
            }
        }
        // Then the shells of jobs that have ended, which run on for what
        // their jobs left running.
        registered.shell.reattach_left().await;
        registered.management.set_jobs(jobs.job_count());
        Self {
            jobs,
            directories,
            contexts,
            relay: Relay::default(),
            watches: JoinSet::new(),
            link,
            handle,
            worktrees,
            requests,
            reach,
            output,
            queue,
            _forwarder: AbortOnDropHandle::new(forwarder),
        }
    }

    /// The jobs as a hello lists them.
    pub fn hello_jobs(&self) -> Vec<wire::KeptJob> {
        self.directories.kept(&self.jobs.running_jobs())
    }

    /// A connection serves from now on: the jobs' messages and their
    /// commands' requests go to it. Answers what the connection sends again
    /// of the jobs not released: their media's announcements and the exits
    /// of those that ended, and the residents' latest messages.
    pub fn connected(&mut self, live: Live, output: mpsc::UnboundedSender<wire::Frame>) -> Vec<wire::Frame> {
        self.output.send_replace(Some(output));
        self.reach.send_replace(Reach::Connected(live));
        let mut frames = self.directories.announcements();
        frames.extend(self.handle.told());
        frames
    }

    /// The connection ended: the jobs run on, and so do their commands'
    /// calls, whose events come over the next connection.
    pub fn disconnected(&mut self) {
        self.output.send_replace(None);
        self.reach.send_if_modified(|reach| {
            if matches!(reach, Reach::Connected(_)) {
                *reach = Reach::Away;
                return true;
            }
            false
        });
    }

    /// The link's session ended, as a backend that started again ends it:
    /// the calls in flight end with `reason`.
    pub fn session_ended(&mut self, reason: &str) {
        self.relay.lose_calls(reason);
    }

    /// The next thing that happened; cancel-safe.
    pub async fn next(&mut self) -> Event {
        tokio::select! {
            biased;
            Some(request) = self.requests.recv() => Event::Request(request),
            Some(id) = self.jobs.finished() => Event::Finished(id),
            Some(ended) = self.watches.join_next() => {
                Event::Watch(ended.expect("relay watches do not panic"))
            }
        }
    }

    /// Does what `event` asks; a question goes to the backend on `control`,
    /// the connection that serves, and waits for its asker's next try
    /// without one.
    pub async fn handle(
        &mut self,
        event: Event,
        control: Option<&mpsc::UnboundedSender<wire::Frame>>,
        registered: &Registered,
    ) {
        match event {
            Event::Request(Request::Call { id, events, ended }) => {
                self.relay.call(id, events, ended, &mut self.watches);
            }
            Event::Request(Request::Ask {
                question,
                abandoned,
            }) => {
                let frame = match self.relay.ask(question, abandoned, &mut self.watches) {
                    Ok(frame) => frame,
                    Err(error) => {
                        tracing::warn!(event = "reply.encode_failed", what = "question", %error, "a question to the backend was not encoded");
                        return;
                    }
                };
                // Without a connection the asker sees its connection's end.
                if let Some(control) = control {
                    let _gone = control.send(frame);
                }
            }
            Event::Request(Request::Context {
                context,
                leases,
                reply,
            }) => {
                // A job that gave up no longer needs to know.
                let _gone = reply.send(self.contexts.insert(context, leases));
            }
            Event::Request(Request::ContextReleased { job_id }) => {
                if self.contexts.released(&job_id) {
                    self.context_ended(job_id).await;
                }
            }
            Event::Finished(id) => {
                if let WorkId::Job(job) = &id
                    && self.contexts.remove(job)
                {
                    self.context_ended(job.clone()).await;
                }
                registered.management.set_jobs(self.jobs.job_count());
            }
            Event::Watch(ended) => self.relay.ended(ended),
        }
    }

    /// Tells the backend that the context of job `job_id` ended, in the queue
    /// that holds the job's exit, which it follows.
    async fn context_ended(&self, job_id: String) {
        match wire::encode(&wire::Outbound::JobContextEnded { job_id }) {
            // The queue's forwarder ends with the registration.
            Ok(frame) => {
                let _ended = self.queue.send(frame);
            }
            Err(error) => tracing::warn!(event = "reply.encode_failed", what = "job_context_end", %error, "a job context's end was not encoded"),
        }
    }

    /// Leaves every job to the runner that starts next, which takes it over
    /// (`crates/runner-shell/src/remote/mod.rs`); the registration ends.
    pub async fn leave(&mut self) {
        self.reach.send_replace(Reach::Unreached);
        self.contexts.leave();
        self.jobs.leave().await;
    }

    /// Ends every job and waits for each; the registration ends.
    pub async fn close(&mut self) {
        self.reach.send_replace(Reach::Unreached);
        self.jobs.close().await;
    }
}

/// Passes each job message on to the connection that serves; without one, the
/// message goes nowhere, since the next connection reads what it needs from
/// the jobs' kept output.
async fn forward(
    mut messages: mpsc::UnboundedReceiver<wire::Frame>,
    output: watch::Receiver<Option<mpsc::UnboundedSender<wire::Frame>>>,
) {
    while let Some(message) = messages.recv().await {
        let current = output.borrow().clone();
        if let Some(current) = current {
            // A connection that ended takes nothing more.
            let _gone = current.send(message);
        }
    }
}
