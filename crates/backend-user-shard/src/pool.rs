//! Shard threads and calls into a user's shard. A shard thread is a single-threaded `LocalRuntime`
//! hosting the shards of every user pinned to it; a stable hash of the user id pins each user to
//! one thread. The edge reaches a user's shard with `shards.of(user).call(..)`: the closure is
//! `Send`, the future it starts runs on the shard and need not be, and the answer is `Send`.
//!
//! The pool is Demi's own small set of `LocalRuntime` threads fed by a job
//! channel. tokio-util's `LocalPoolHandle` is not used: it gives each worker a
//! runtime of its own, which a test's paused clock cannot reach.

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::num::NonZeroUsize;
use std::rc::Rc;
use std::sync::Arc;

use demi_backend_cloud::CloudSide;
use demi_core::conversation::claude_cli::ClaudeMachine;
use demi_core::extensions::Extensions;
use demi_core::shard::cores::{CoreWork, Cores, ShardUnavailable};
use demi_core::shard::device_requests::DeviceRequests;
use demi_core::shard::{Census, Shard, web_client};
use demi_web_api_protocol::ids::UserId;
use futures_util::future::{BoxFuture, LocalBoxFuture};
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::Instrument as _;

use crate::server_starter::{ServerSide, ServerStarter};
use crate::services::Services;

/// Where the shards run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShardPlacement {
    /// This many shard threads, each a `LocalRuntime` of its own.
    Threads(NonZeroUsize),
    /// On the runtime that starts the backend, which must then be a
    /// `LocalRuntime`: a test's paused clock reaches the shards.
    Inline,
}

/// Calls waiting for a shard thread before its callers wait to send more.
///
/// Bounded so a shard thread that falls behind makes its callers wait instead
/// of growing the queue without end.
const QUEUE: usize = 256;

/// The way into every user's shard. Cloning it is cheap.
#[derive(Clone)]
pub struct Shards {
    queues: Arc<[mpsc::Sender<Message>]>,
}

/// The way into one user's shard.
pub struct ShardRef<'a> {
    queue: &'a mpsc::Sender<Message>,
    user: &'a UserId,
}

impl Shards {
    pub fn of<'a>(&'a self, user: &'a UserId) -> ShardRef<'a> {
        let digest = Sha256::digest(user.as_str().as_bytes());
        let hash = u64::from_be_bytes(
            digest[..8]
                .try_into()
                .expect("a SHA-256 digest has 8 bytes"),
        );
        let count = u64::try_from(self.queues.len()).expect("the shard count fits u64");
        let index = usize::try_from(hash % count).expect("a shard index fits usize");
        ShardRef {
            queue: &self.queues[index],
            user,
        }
    }
}

impl Shards {
    /// What every shard holds now; a shard thread that has closed counts
    /// nothing.
    pub async fn census(&self) -> Census {
        let mut asked = Vec::new();
        for queue in self.queues.iter() {
            let (answer, answered) = oneshot::channel();
            if queue.send(Message::Census(answer)).await.is_ok() {
                asked.push(answered);
            }
        }
        let mut census = Census::default();
        for answered in futures_util::future::join_all(asked).await {
            census += answered.unwrap_or_default();
        }
        census
    }
}

impl Cores for Shards {
    fn run(&self, user: &UserId, work: CoreWork) -> BoxFuture<'static, Result<(), ShardUnavailable>> {
        let shards = self.clone();
        let user = user.clone();
        Box::pin(async move { shards.of(&user).call(work).await })
    }
}

impl ShardRef<'_> {
    /// Runs `work` on the user's shard and answers its result. The call runs
    /// to completion even when this future is dropped; dropping it cancels
    /// the token `work` receives, which only its waits observe, never a step
    /// after a commit.
    pub async fn call<F, Fut, T>(&self, work: F) -> Result<T, ShardUnavailable>
    where
        F: FnOnce(Rc<Shard>, CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = T> + 'static,
        T: Send + 'static,
    {
        self.run(work, false).await
    }

    /// A call the shard also serves while it closes, such as a runner's pipe
    /// request, which the shard's own shutdown steps may need.
    pub async fn call_while_closing<F, Fut, T>(&self, work: F) -> Result<T, ShardUnavailable>
    where
        F: FnOnce(Rc<Shard>, CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = T> + 'static,
        T: Send + 'static,
    {
        self.run(work, true).await
    }

    async fn run<F, Fut, T>(&self, work: F, while_closing: bool) -> Result<T, ShardUnavailable>
    where
        F: FnOnce(Rc<Shard>, CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = T> + 'static,
        T: Send + 'static,
    {
        let cancel = CancellationToken::new();
        let _requester = cancel.clone().drop_guard();
        let (answer, answered) = oneshot::channel();
        let job = CallJob {
            user: self.user.clone(),
            work,
            cancel,
            answer,
            while_closing,
            span: tracing::Span::current(),
        };
        self.queue
            .send(Message::Call(Box::new(job)))
            .await
            .map_err(|_| ShardUnavailable::Closing)?;
        // An answer dropped unsent means the call panicked.
        answered.await.unwrap_or(Err(ShardUnavailable::Failed))
    }

    /// Moves `work`, such as serving a page's lane, into the user's
    /// shard, where it runs as the shard's task until it ends. It answers
    /// once the work is queued; a closing shard refuses it, and dropping the
    /// work drops what it holds.
    pub async fn adopt<F, Fut>(&self, work: F) -> Result<(), ShardUnavailable>
    where
        F: FnOnce(Rc<Shard>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        let job = AdoptJob {
            user: self.user.clone(),
            work,
            span: tracing::Span::current(),
        };
        self.queue
            .send(Message::Call(Box::new(job)))
            .await
            .map_err(|_| ShardUnavailable::Closing)
    }
}

enum Message {
    Call(Box<dyn Job>),
    /// Answered with what the thread's shards hold now.
    Census(oneshot::Sender<Census>),
    /// Answered once every shard closed.
    Close(oneshot::Sender<()>),
}

/// Work on its way to a shard, which either runs it or refuses it.
trait Job: Send {
    fn user(&self) -> &UserId;
    /// Whether the shard runs it while it closes.
    fn while_closing(&self) -> bool;
    fn run(self: Box<Self>, shard: Rc<Shard>) -> LocalBoxFuture<'static, ()>;
    fn refuse(self: Box<Self>);
}

struct CallJob<F, T> {
    user: UserId,
    work: F,
    cancel: CancellationToken,
    answer: oneshot::Sender<Result<T, ShardUnavailable>>,
    while_closing: bool,
    /// The caller's span, such as its request's, which the work logs in.
    span: tracing::Span,
}

impl<F, Fut, T> Job for CallJob<F, T>
where
    F: FnOnce(Rc<Shard>, CancellationToken) -> Fut + Send + 'static,
    Fut: Future<Output = T> + 'static,
    T: Send + 'static,
{
    fn user(&self) -> &UserId {
        &self.user
    }

    fn while_closing(&self) -> bool {
        self.while_closing
    }

    fn run(self: Box<Self>, shard: Rc<Shard>) -> LocalBoxFuture<'static, ()> {
        let CallJob {
            work,
            cancel,
            answer,
            span,
            ..
        } = *self;
        Box::pin(async move {
            let value = work(shard, cancel).instrument(span).await;
            // A requester that went away reads no answer; the call ran to
            // completion all the same.
            let _ = answer.send(Ok(value));
        })
    }

    fn refuse(self: Box<Self>) {
        // A requester that went away needs no refusal either.
        let _ = self.answer.send(Err(ShardUnavailable::Closing));
    }
}

struct AdoptJob<F> {
    user: UserId,
    work: F,
    /// The adopter's span, such as a lane's open's, which the
    /// work logs in for its whole life.
    span: tracing::Span,
}

impl<F, Fut> Job for AdoptJob<F>
where
    F: FnOnce(Rc<Shard>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + 'static,
{
    fn user(&self) -> &UserId {
        &self.user
    }

    fn while_closing(&self) -> bool {
        false
    }

    fn run(self: Box<Self>, shard: Rc<Shard>) -> LocalBoxFuture<'static, ()> {
        let AdoptJob { work, span, .. } = *self;
        Box::pin(async move {
            // The work belongs to the shard, whose close waits for it.
            let tasks = shard.tasks().clone();
            tasks.spawn_local(work(shard).instrument(span));
        })
    }

    fn refuse(self: Box<Self>) {
        // Dropping the work drops what it holds, such as a lane, which
        // ends it.
    }
}

/// The shard threads, or the inline shard loop, owned by the backend.
pub struct ShardPool {
    shards: Shards,
    workers: Vec<Worker>,
}

enum Worker {
    Thread(std::thread::JoinHandle<()>),
    Inline(tokio::task::JoinHandle<()>),
}

impl ShardPool {
    pub async fn start(placement: ShardPlacement, services: Arc<Services>) -> io::Result<Self> {
        let shared = services.clone();
        let mut queues = Vec::new();
        let mut workers = Vec::new();
        match placement {
            ShardPlacement::Inline => {
                let (queue, receiver) = mpsc::channel(QUEUE);
                queues.push(queue);
                workers.push(Worker::Inline(tokio::task::spawn_local(serve(
                    receiver, services,
                ))));
            }
            ShardPlacement::Threads(count) => {
                for index in 0..count.get() {
                    let (queue, receiver) = mpsc::channel(QUEUE);
                    match spawn_thread(index, receiver, services.clone()).await {
                        Ok(thread) => {
                            queues.push(queue);
                            workers.push(Worker::Thread(thread));
                        }
                        Err(error) => {
                            // The threads already running stop again.
                            drop(queue);
                            Self::stop(queues, workers).await;
                            return Err(error);
                        }
                    }
                }
            }
        }
        let shards = Shards {
            queues: queues.into(),
        };
        // The master's shard holds the Claude Code machine, which every
        // shard and the providers' sign-ins and probes reach through it
        // (`crates/backend-cloud/src/lib.rs`).
        let machine = ClaudeMachine::new(Arc::new(shards.clone()), shared.vault.clone());
        if !shared.claude_machine.set(machine) {
            tracing::error!(event = "claude_code_machine.set_twice", "the Claude Code machine was set twice; the first is kept");
        }
        // A provider entry's request through one of its owner's devices goes
        // through the owner's shard (`crates/backend-user-shard/src/shard/device_requests.rs`).
        shared.provider_registry.set_devices(DeviceRequests::new(Arc::new(shards.clone())));
        Ok(Self { shards, workers })
    }

    pub fn shards(&self) -> Shards {
        self.shards.clone()
    }

    /// Refuses new calls, waits for the running ones and stops the shard
    /// threads.
    pub async fn close(self) {
        Self::stop(self.shards.queues.to_vec(), self.workers).await;
    }

    /// Tells every shard to close, then waits for them all, so a shard that
    /// takes long to close keeps no other running.
    async fn stop(queues: Vec<mpsc::Sender<Message>>, workers: Vec<Worker>) {
        let mut closing = Vec::new();
        for queue in &queues {
            let (closed, done) = oneshot::channel();
            if queue.send(Message::Close(closed)).await.is_ok() {
                closing.push(done);
            }
        }
        // A shard loop that ended already has nothing left to stop.
        let _ = futures_util::future::join_all(closing).await;
        for worker in workers {
            match worker {
                Worker::Inline(task) => {
                    if let Err(error) = task.await {
                        tracing::error!(
                            event = "shard.loop_failed",
                            error = &error as &dyn std::error::Error,
                            "the shard loop failed"
                        );
                    }
                }
                Worker::Thread(thread) => {
                    let joined = tokio::task::spawn_blocking(move || thread.join()).await;
                    if !matches!(joined, Ok(Ok(()))) {
                        tracing::error!(event = "shard.thread_panicked", "a shard thread panicked");
                    }
                }
            }
        }
    }
}

/// Starts a shard thread and waits until its runtime runs.
async fn spawn_thread(
    index: usize,
    receiver: mpsc::Receiver<Message>,
    services: Arc<Services>,
) -> io::Result<std::thread::JoinHandle<()>> {
    let (started, running) = oneshot::channel();
    // The thread logs where the thread that starts the pool logs: the
    // process's log, or a test's own.
    let log = tracing::dispatcher::get_default(Clone::clone);
    let thread = std::thread::Builder::new()
        .name(format!("shard-{index}"))
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build_local(tokio::runtime::LocalOptions::default());
            match runtime {
                Ok(runtime) => {
                    // The starter waits for this answer; it cannot have gone.
                    let _ = started.send(Ok(()));
                    tracing::dispatcher::with_default(&log, || {
                        runtime.block_on(serve(receiver, services));
                    });
                }
                Err(error) => {
                    let _ = started.send(Err(error));
                }
            }
        })?;
    match running.await {
        Ok(Ok(())) => Ok(thread),
        Ok(Err(error)) => Err(error),
        Err(_) => Err(io::Error::other(
            "a shard thread ended before its runtime started",
        )),
    }
}

/// A shard thread's loop: it creates each user's shard on the user's first
/// call and runs every call as a task of its own until the pool closes.
/// Closing, it refuses new calls, except those a shard serves while it
/// closes, closes every shard, and waits for the calls still running.
async fn serve(mut queue: mpsc::Receiver<Message>, services: Arc<Services>) {
    let calls = TaskTracker::new();
    let http = reqwest::Client::new();
    let web = web_client(&services);
    let mut shards: HashMap<UserId, Rc<Shard>> = HashMap::new();
    let mut closer = None;
    while let Some(message) = queue.recv().await {
        match message {
            Message::Call(job) => {
                // The first call opens the user's store. Waiting for it holds
                // this thread's other users' calls back only at that call,
                // and the start opened every user's store that existed.
                let store = if shards.contains_key(job.user()) {
                    None
                } else {
                    match services.stores.of(job.user()).await {
                        Ok(store) => Some(store),
                        Err(error) => {
                            tracing::error!(event = "shard.store_open_failed", user = %job.user(), error = &error as &dyn std::error::Error, "a user's core store did not open");
                            job.refuse();
                            continue;
                        }
                    }
                };
                let shard = shards
                    .entry(job.user().clone())
                    .or_insert_with_key(|user| {
                        let store = store.expect("a user with no shard had the store opened above");
                        Shard::start(
                            user.clone(),
                            store,
                            services.core.clone(),
                            |shard| Rc::new(ServerStarter::new(user.clone(), services.clone(), shard)),
                            Extensions::default()
                                .with(ServerSide::new(services.clone()))
                                .with(CloudSide::new(services.control.clone(), services.cloud.clone())),
                            http.clone(),
                            web.clone(),
                        )
                    })
                    .clone();
                calls.spawn_local(job.run(shard));
            }
            Message::Census(answer) => {
                let mut census = Census::default();
                for shard in shards.values() {
                    census += shard.census();
                }
                let _ = answer.send(census);
            }
            Message::Close(done) => {
                closer = Some(done);
                break;
            }
        }
    }
    let closing = futures_util::future::join_all(shards.values().map(|shard| shard.close()));
    tokio::pin!(closing);
    loop {
        tokio::select! {
            _ = &mut closing => break,
            message = queue.recv() => match message {
                // A shard that closes takes no new user.
                Some(Message::Call(job)) => match shards.get(job.user()) {
                    Some(shard) if job.while_closing() => {
                        calls.spawn_local(job.run(shard.clone()));
                    }
                    _ => job.refuse(),
                },
                // Only the pool closes, once.
                Some(Message::Close(done)) => drop(done),
                // A closing shard thread counts nothing.
                Some(Message::Census(answer)) => drop(answer),
                // Every sender is gone: nothing more arrives while the
                // shards close.
                None => {
                    (&mut closing).await;
                    break;
                }
            },
        }
    }
    queue.close();
    while let Some(message) = queue.recv().await {
        match message {
            Message::Call(job) => job.refuse(),
            Message::Close(done) => drop(done),
            Message::Census(answer) => drop(answer),
        }
    }
    calls.close();
    calls.wait().await;
    if let Some(done) = closer {
        // The closer waits for this answer; if it went away, no one is left
        // to tell.
        let _ = done.send(());
    }
}
