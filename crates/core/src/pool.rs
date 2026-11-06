//! Shard threads and calls into a core. A shard thread is a single-threaded
//! `LocalRuntime` running the cores of every principal pinned to it; a
//! stable hash of the principal's id pins each to one thread. The edge
//! reaches a core with `shards.of(user).call(..)`: the closure is `Send`, the
//! future it starts runs on the core's thread and need not be, and the answer
//! is `Send`.
//!
//! A core starts at its first call, over the store the process's [`Opener`]
//! gives, and runs until the process closes it (`ShardRef::close`) or the
//! pool closes. A call that reaches a closing core is refused, except one the
//! core serves while it closes; the close waits for the calls the core runs.
//!
//! The pool is Demi's own small set of `LocalRuntime` threads fed by a job
//! channel. tokio-util's `LocalPoolHandle` is not used: it gives each worker a
//! runtime of its own, which a test's paused clock cannot reach.

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::num::NonZeroUsize;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use demi_core_store::StorageError;
use demi_core_store::store::CoreStore;
use demi_web_api_protocol::ids::UserId;
use futures_util::future::{BoxFuture, LocalBoxFuture};
use sha2::{Digest, Sha256};
use tokio::sync::{Notify, mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::Instrument as _;

use crate::conversation::claude_cli::ClaudeMachine;
use crate::cores::{CoreWork, CoreUnavailable, Cores};
use crate::device_requests::DeviceRequests;
use crate::extensions::Extensions;
use crate::services::CoreServices;
use crate::starter::Starter;
use crate::{Census, Core, web_client};

/// What a process gives the cores its pool starts: a server, the core of
/// each of its users; a program's process, the cores the program opened.
pub trait Opener: Send + Sync {
    /// The store of `user`'s core, which its first call starts it over; none
    /// when the process holds no core of `user`, whose calls are refused.
    fn store<'a>(&'a self, user: &'a UserId) -> BoxFuture<'a, Result<Option<CoreStore>, StorageError>>;

    /// The starter of `user`'s core, made for the core `core`.
    fn starter(&self, user: &UserId, core: Weak<Core>) -> Rc<dyn Starter>;

    /// What the starter keeps in `user`'s core.
    fn extensions(&self, user: &UserId) -> Extensions;
}

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
    fn run(&self, user: &UserId, work: CoreWork) -> BoxFuture<'static, Result<(), CoreUnavailable>> {
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
    pub async fn call<F, Fut, T>(&self, work: F) -> Result<T, CoreUnavailable>
    where
        F: FnOnce(Rc<Core>, CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = T> + 'static,
        T: Send + 'static,
    {
        self.run(work, false).await
    }

    /// A call the shard also serves while it closes, such as a runner's pipe
    /// request, which the shard's own shutdown steps may need.
    pub async fn call_while_closing<F, Fut, T>(&self, work: F) -> Result<T, CoreUnavailable>
    where
        F: FnOnce(Rc<Core>, CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = T> + 'static,
        T: Send + 'static,
    {
        self.run(work, true).await
    }

    async fn run<F, Fut, T>(&self, work: F, while_closing: bool) -> Result<T, CoreUnavailable>
    where
        F: FnOnce(Rc<Core>, CancellationToken) -> Fut + Send + 'static,
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
            .map_err(|_| CoreUnavailable::Closing)?;
        // An answer dropped unsent means the call panicked.
        answered.await.unwrap_or(Err(CoreUnavailable::Failed))
    }

    /// Closes the core as the pool's close closes each, and answers once it
    /// closed and the calls it ran ended; a core that is not running
    /// answers at once. Its next call starts it again, over the store the
    /// opener gives then.
    pub async fn close(&self) -> Result<(), CoreUnavailable> {
        let (closed, done) = oneshot::channel();
        self.queue
            .send(Message::CloseCore(self.user.clone(), closed))
            .await
            .map_err(|_| CoreUnavailable::Closing)?;
        // A thread that stops answers once it closed every core.
        done.await.map_err(|_| CoreUnavailable::Closing)
    }

    /// Moves `work`, such as serving a page's lane, into the user's
    /// shard, where it runs as the shard's task until it ends. It answers
    /// once the work is queued; a closing shard refuses it, and dropping the
    /// work drops what it holds.
    pub async fn adopt<F, Fut>(&self, work: F) -> Result<(), CoreUnavailable>
    where
        F: FnOnce(Rc<Core>) -> Fut + Send + 'static,
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
            .map_err(|_| CoreUnavailable::Closing)
    }
}

enum Message {
    Call(Box<dyn Job>),
    /// Answered with what the thread's shards hold now.
    Census(oneshot::Sender<Census>),
    /// Closes the core of the user; answered once it closed.
    CloseCore(UserId, oneshot::Sender<()>),
    /// Answered once every shard closed.
    Close(oneshot::Sender<()>),
}

/// Work on its way to a shard, which either runs it or refuses it.
trait Job: Send {
    fn user(&self) -> &UserId;
    /// Whether the shard runs it while it closes.
    fn while_closing(&self) -> bool;
    fn run(self: Box<Self>, shard: Rc<Core>) -> LocalBoxFuture<'static, ()>;
    fn refuse(self: Box<Self>);
}

struct CallJob<F, T> {
    user: UserId,
    work: F,
    cancel: CancellationToken,
    answer: oneshot::Sender<Result<T, CoreUnavailable>>,
    while_closing: bool,
    /// The caller's span, such as its request's, which the work logs in.
    span: tracing::Span,
}

impl<F, Fut, T> Job for CallJob<F, T>
where
    F: FnOnce(Rc<Core>, CancellationToken) -> Fut + Send + 'static,
    Fut: Future<Output = T> + 'static,
    T: Send + 'static,
{
    fn user(&self) -> &UserId {
        &self.user
    }

    fn while_closing(&self) -> bool {
        self.while_closing
    }

    fn run(self: Box<Self>, shard: Rc<Core>) -> LocalBoxFuture<'static, ()> {
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
        let _ = self.answer.send(Err(CoreUnavailable::Closing));
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
    F: FnOnce(Rc<Core>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + 'static,
{
    fn user(&self) -> &UserId {
        &self.user
    }

    fn while_closing(&self) -> bool {
        false
    }

    fn run(self: Box<Self>, shard: Rc<Core>) -> LocalBoxFuture<'static, ()> {
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
    /// The shard threads, which start each core over the store `opener`
    /// gives, with the services `services` every core shares.
    pub async fn start(
        placement: ShardPlacement,
        services: Arc<CoreServices>,
        opener: Arc<dyn Opener>,
    ) -> io::Result<Self> {
        let shared = services.clone();
        let mut queues = Vec::new();
        let mut workers = Vec::new();
        match placement {
            ShardPlacement::Inline => {
                let (queue, receiver) = mpsc::channel(QUEUE);
                queues.push(queue);
                workers.push(Worker::Inline(tokio::task::spawn_local(serve(
                    receiver, services, opener,
                ))));
            }
            ShardPlacement::Threads(count) => {
                for index in 0..count.get() {
                    let (queue, receiver) = mpsc::channel(QUEUE);
                    match spawn_thread(index, receiver, services.clone(), opener.clone()).await {
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
        // through the owner's shard (`crates/core/src/device_requests.rs`).
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
    services: Arc<CoreServices>,
    opener: Arc<dyn Opener>,
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
                        runtime.block_on(serve(receiver, services, opener));
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

/// A core a shard thread runs, with the calls it runs, which its close
/// waits for.
struct Running {
    core: Rc<Core>,
    calls: TaskTracker,
}

impl Running {
    fn run(&self, job: Box<dyn Job>) {
        self.calls.spawn_local(job.run(self.core.clone()));
    }
}

/// A core that closes, and who waits for its close.
struct Closing {
    running: Running,
    waiters: Vec<oneshot::Sender<()>>,
}

/// The cores of one shard thread.
#[derive(Default)]
struct ThreadCores {
    open: HashMap<UserId, Running>,
    closing: HashMap<UserId, Closing>,
}

impl ThreadCores {
    fn census(&self) -> Census {
        let mut census = Census::default();
        let closing = self.closing.values().map(|closing| &closing.running);
        for running in self.open.values().chain(closing) {
            census += running.core.census();
        }
        census
    }
}

/// Starts the close of `user`'s core, which `done` waits for, and tells
/// `ended` once it closed; a core that closes already gains a waiter, and
/// one not running answers at once.
fn close_core(
    cores: &Rc<RefCell<ThreadCores>>,
    ended: &Rc<Notify>,
    user: UserId,
    done: Option<oneshot::Sender<()>>,
) {
    let mut thread = cores.borrow_mut();
    if let Some(closing) = thread.closing.get_mut(&user) {
        closing.waiters.extend(done);
        return;
    }
    let Some(running) = thread.open.remove(&user) else {
        if let Some(done) = done {
            // A waiter that went away needs no answer.
            let _ = done.send(());
        }
        return;
    };
    let core = running.core.clone();
    let calls = running.calls.clone();
    thread.closing.insert(
        user.clone(),
        Closing {
            running,
            waiters: done.into_iter().collect(),
        },
    );
    drop(thread);
    let closed = Closed {
        cores: cores.clone(),
        ended: ended.clone(),
        user,
    };
    tokio::task::spawn_local(async move {
        let _closed = closed;
        core.close().await;
        calls.close();
        calls.wait().await;
    });
}

/// Ends a core's close as it is dropped, when the close ended or panicked,
/// so a thread that stops never waits for a close that will not end: the
/// core leaves the thread, and its waiters and the thread hear.
struct Closed {
    cores: Rc<RefCell<ThreadCores>>,
    ended: Rc<Notify>,
    user: UserId,
}

impl Drop for Closed {
    fn drop(&mut self) {
        let closed = self.cores.borrow_mut().closing.remove(&self.user);
        for waiter in closed.into_iter().flat_map(|closed| closed.waiters) {
            let _ = waiter.send(());
        }
        self.ended.notify_one();
    }
}

/// A shard thread's loop: it starts each core at its first call and runs
/// every call as a task of its own, and closes a core when its process asks,
/// until the pool closes. Closing, it refuses new calls, except those a core
/// serves while it closes, closes every core, and waits for the calls still
/// running.
async fn serve(mut queue: mpsc::Receiver<Message>, services: Arc<CoreServices>, opener: Arc<dyn Opener>) {
    let http = reqwest::Client::new();
    let web = web_client(&services);
    let cores = Rc::new(RefCell::new(ThreadCores::default()));
    let ended = Rc::new(Notify::new());
    let mut closer = None;
    while let Some(message) = queue.recv().await {
        match message {
            Message::Call(job) => {
                {
                    let thread = cores.borrow();
                    if let Some(running) = thread.open.get(job.user()) {
                        running.run(job);
                        continue;
                    }
                    if let Some(closing) = thread.closing.get(job.user()) {
                        if job.while_closing() {
                            closing.running.run(job);
                        } else {
                            job.refuse();
                        }
                        continue;
                    }
                }
                // The first call opens the core's store. Waiting for it holds
                // this thread's other cores' calls back only at that call.
                let store = match opener.store(job.user()).await {
                    Ok(Some(store)) => store,
                    Ok(None) => {
                        job.refuse();
                        continue;
                    }
                    Err(error) => {
                        tracing::error!(event = "shard.store_open_failed", user = %job.user(), error = &error as &dyn std::error::Error, "a core's store did not open");
                        job.refuse();
                        continue;
                    }
                };
                let user = job.user().clone();
                let core = Core::start(
                    user.clone(),
                    store,
                    services.clone(),
                    |core| opener.starter(&user, core),
                    opener.extensions(&user),
                    http.clone(),
                    web.clone(),
                );
                let running = Running {
                    core,
                    calls: TaskTracker::new(),
                };
                running.run(job);
                cores.borrow_mut().open.insert(user, running);
            }
            Message::Census(answer) => {
                let _ = answer.send(cores.borrow().census());
            }
            Message::CloseCore(user, done) => close_core(&cores, &ended, user, Some(done)),
            Message::Close(done) => {
                closer = Some(done);
                break;
            }
        }
    }
    let open: Vec<UserId> = cores.borrow().open.keys().cloned().collect();
    for user in open {
        close_core(&cores, &ended, user, None);
    }
    let mut senders = true;
    while !cores.borrow().closing.is_empty() {
        if !senders {
            ended.notified().await;
            continue;
        }
        tokio::select! {
            () = ended.notified() => {}
            message = queue.recv() => match message {
                // A closing thread starts no core.
                Some(Message::Call(job)) => match cores.borrow().closing.get(job.user()) {
                    Some(closing) if job.while_closing() => closing.running.run(job),
                    _ => job.refuse(),
                },
                Some(Message::CloseCore(user, done)) => close_core(&cores, &ended, user, Some(done)),
                // Only the pool closes, once.
                Some(Message::Close(done)) => drop(done),
                // A closing shard thread counts nothing.
                Some(Message::Census(answer)) => drop(answer),
                // Every sender is gone: nothing more arrives while the
                // cores close.
                None => senders = false,
            },
        }
    }
    queue.close();
    while let Some(message) = queue.recv().await {
        match message {
            Message::Call(job) => job.refuse(),
            Message::CloseCore(_, done) | Message::Close(done) => drop(done),
            Message::Census(answer) => drop(answer),
        }
    }
    if let Some(done) = closer {
        // The closer waits for this answer; if it went away, no one is left
        // to tell.
        let _ = done.send(());
    }
}
