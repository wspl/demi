//! What the runner keeps through a connection loss (`runner.md` § Command
//! lifetime): its jobs with their directories and execution contexts, and
//! the requests its jobs' commands make of the backend. A connection serves
//! them while it lasts; without one, the jobs run on however long it takes,
//! their messages go nowhere, and their commands' requests wait for the next
//! connection. Only the runner's end stops them, as when the backend refuses
//! it.

use std::sync::Arc;

use demi_runner_jobs::{
    commands::contexts::ContextTable,
    connection::{ConnectionHandle, Ended, Live, Reach, Relay, Request},
    job_directories::JobDirectories,
    tasks::{Commands, JobConfig, JobTable, WorkId},
};
use demi_runner_protocol::wire;
use tokio::{
    sync::{mpsc, watch},
    task::JoinSet,
    time::Instant,
};
use tokio_util::task::AbortOnDropHandle;

use super::Registered;

/// The jobs' messages a connection has not taken yet.
const OUTPUT_QUEUE: usize = 8;

/// What the registration keeps from one connection to the next.
pub struct Kept {
    pub jobs: JobTable,
    pub directories: Arc<JobDirectories>,
    pub contexts: ContextTable,
    pub relay: Relay,
    pub watches: JoinSet<Ended>,
    /// How the jobs' commands reach the backend.
    pub handle: ConnectionHandle,
    requests: mpsc::Receiver<Request>,
    reach: watch::Sender<Reach>,
    /// The connection the jobs' messages go to; none while none serves.
    output: watch::Sender<Option<mpsc::Sender<wire::Frame>>>,
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
    /// The registration's jobs, none yet, under its job root, whose
    /// directories a runner that ended left are removed.
    pub async fn open(registered: &Registered) -> Self {
        let directories = JobDirectories::open(registered.jobs.clone()).await;
        let (reach, reaching) = watch::channel(Reach::Away);
        let (handle, requests) = ConnectionHandle::new(reaching);
        let (jobs_output, forwarded) = mpsc::channel(OUTPUT_QUEUE);
        let output = watch::Sender::new(None);
        let forwarder = tokio::spawn(forward(forwarded, output.subscribe()));
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
                services: registered.services.clone(),
                endpoint: registered.endpoint.clone(),
                home: registered.state.root.to_string_lossy().into_owned(),
            }),
        });
        Self {
            jobs,
            directories,
            contexts: ContextTable::new(registered.index.clone()),
            relay: Relay::default(),
            watches: JoinSet::new(),
            handle,
            requests,
            reach,
            output,
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
    /// of those that ended.
    pub fn connected(&mut self, live: Live, output: mpsc::Sender<wire::Frame>) -> Vec<wire::Frame> {
        self.output.send_replace(Some(output));
        self.reach.send_replace(Reach::Connected(live));
        self.directories.announcements()
    }

    /// The connection ended: the jobs run on, and their commands' calls in
    /// flight end with it.
    pub fn disconnected(&mut self) {
        self.output.send_replace(None);
        // Calls in flight end: their events go nowhere any more.
        self.relay = Relay::default();
        self.reach.send_if_modified(|reach| {
            if matches!(reach, Reach::Connected(_)) {
                *reach = Reach::Away;
                return true;
            }
            false
        });
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
        control: Option<&mpsc::Sender<wire::Frame>>,
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
                        tracing::warn!("a question to the backend does not encode: {error}");
                        return;
                    }
                };
                // Without a connection the asker sees its connection's end.
                if let Some(control) = control {
                    let _gone = control.send(frame).await;
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
            Event::Finished(id) => {
                if let WorkId::Job(job) = &id {
                    self.contexts.remove(job);
                }
                registered.management.set_jobs(self.jobs.job_count());
            }
            Event::Watch(ended) => self.relay.ended(ended),
        }
    }

    /// Stops every job as a stop does and waits for each to end, as a
    /// runner that replaces itself does (`runner.md` § Runner updates). A
    /// killed job ends a second later whatever its in-process work does
    /// (§ Cancellation and completion), so the update never waits on one.
    pub async fn stop_jobs(&mut self, registered: &Registered) {
        self.jobs.stop_jobs();
        // `KILL` goes once: a deadline that has passed would be ready again
        // at every turn of the loop.
        let mut kill_at = Some(Instant::now() + wire::STOP_GRACE);
        while !self.jobs.running_jobs().is_empty() {
            tokio::select! {
                () = sleep_until(kill_at) => {
                    kill_at = None;
                    self.jobs.kill_jobs();
                }
                Some(id) = self.jobs.finished() => {
                    self.handle(Event::Finished(id), None, registered).await;
                }
            }
        }
    }

    /// Ends every job and waits for each; the registration ends.
    pub async fn close(&mut self) {
        self.reach.send_replace(Reach::Unreached);
        self.jobs.close().await;
    }
}

/// Sleeps until `at`; never without one.
async fn sleep_until(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

/// Passes each job message on to the connection that serves, waiting for
/// room in its queue; without one, the message goes nowhere, since the next
/// connection reads what it needs from the jobs' kept output (`runner.md`
/// § Command lifetime).
async fn forward(
    mut messages: mpsc::Receiver<wire::Frame>,
    output: watch::Receiver<Option<mpsc::Sender<wire::Frame>>>,
) {
    while let Some(message) = messages.recv().await {
        let current = output.borrow().clone();
        if let Some(current) = current {
            // A connection that ended takes nothing more.
            let _gone = current.send(message).await;
        }
    }
}
