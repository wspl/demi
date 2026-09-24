//! Shard threads and calls into a user's shard (`concurrency.md` § The user
//! shard). A shard thread is a single-threaded `LocalRuntime` hosting the
//! shards of every user pinned to it; a stable hash of the user id pins each
//! user to one thread. The edge reaches a user's shard with
//! `shards.of(user).call(..)`: the closure is `Send`, the future it starts
//! runs on the shard and need not be, and the answer is `Send`.

pub(crate) mod lease;

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::num::NonZeroUsize;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use demi_agent::AgentServer;
use demi_gates::KeyedSerialGate;
use demi_host_remote::{ARRIVAL, Pipes};
use demi_web_api::ids::UserId;
use futures_util::future::LocalBoxFuture;
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::backend::Services;
use crate::conversation::host_access::Conversations;
use crate::conversation::titles::Titles;
use crate::conversation::{self, ConversationHarness, ConversationParts};
use crate::lifecycle::conversations::ConversationWatches;
use crate::managed::Cloud;
use crate::runner::devices::Devices;
use crate::runner::router::CommandRouter;
use crate::usage::rate_limit::RequestRateLimit;

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
const QUEUE: usize = 256;

/// One user's shard: everything the backend decides for that user. Its
/// components arrive with the conversations, devices and Cloud that need
/// them.
pub(crate) struct Shard {
    user: UserId,
    /// The shard itself, for the tasks its methods start.
    this: Weak<Shard>,
    services: Arc<Services>,
    /// The HTTP client of the shard's thread, which the runtimes built here
    /// send with: hyper ties a pooled connection to the runtime that made it.
    http: reqwest::Client,
    /// The user's conversations, each with its file gate and transfers.
    conversations: Conversations,
    /// The user's devices, each with its runner connection.
    devices: Devices,
    /// The pipes of the user's devices.
    pipes: Pipes,
    /// Each agent node's commands, for the rpc calls of its jobs.
    commands: CommandRouter,
    /// Every task the shard spawns, which its close waits for.
    tasks: TaskTracker,
    /// Cancelled when the shard starts closing: it takes no new runner.
    closing: CancellationToken,
    /// The user's conversation trees.
    agent: Rc<AgentServer<ConversationHarness>>,
    /// The title requests of the user's conversations.
    titles: Titles,
    /// The conversation sockets being served, which the close ends before
    /// the agent shuts down, so no frame reaches it after.
    conversation_sockets: TaskTracker,
    /// Fork requests, one at a time per destination id in lowercase.
    forks: KeyedSerialGate<String>,
    /// The user's Cloud machine.
    cloud: Cloud,
    /// Each conversation's idle watch.
    idle_watches: ConversationWatches,
}

impl Shard {
    /// The shard of `user`, whose agent server reaches it through `shard`.
    fn new(shard: Weak<Shard>, user: UserId, services: Arc<Services>, http: reqwest::Client) -> Self {
        // The user's request rate limit, which every runtime of the user's
        // conversations counts against.
        let limit = services.conversation_tuning.requests_per_minute;
        let rate_limit = Rc::new(RefCell::new(RequestRateLimit::new(limit)));
        let ConversationParts { agent, titles } =
            conversation::conversation_parts(shard.clone(), user.clone(), services.clone(), http.clone(), rate_limit);
        Self {
            user,
            this: shard,
            services,
            http,
            conversations: Conversations::default(),
            devices: Devices::default(),
            pipes: Pipes::new(ARRIVAL),
            commands: CommandRouter::default(),
            tasks: TaskTracker::new(),
            closing: CancellationToken::new(),
            agent,
            titles,
            conversation_sockets: TaskTracker::new(),
            forks: KeyedSerialGate::new(),
            cloud: Cloud::default(),
            idle_watches: ConversationWatches::default(),
        }
    }

    /// The shard, for a task that outlives the call that starts it. A
    /// method of the shard runs while its caller holds it.
    pub(crate) fn this(&self) -> Rc<Shard> {
        self.this.upgrade().expect("a shard's method runs while the shard lives")
    }

    pub(crate) fn cloud(&self) -> &Cloud {
        &self.cloud
    }

    pub(crate) fn idle_watches(&self) -> &ConversationWatches {
        &self.idle_watches
    }

    pub(crate) fn user(&self) -> &UserId {
        &self.user
    }

    pub(crate) fn services(&self) -> &Services {
        &self.services
    }

    pub(crate) fn http(&self) -> &reqwest::Client {
        &self.http
    }

    pub(crate) fn conversations(&self) -> &Conversations {
        &self.conversations
    }

    pub(crate) fn devices(&self) -> &Devices {
        &self.devices
    }

    pub(crate) fn pipes(&self) -> &Pipes {
        &self.pipes
    }

    pub(crate) fn commands(&self) -> &CommandRouter {
        &self.commands
    }

    pub(crate) fn tasks(&self) -> &TaskTracker {
        &self.tasks
    }

    /// Whether the shard is closing and starts no new work.
    pub(crate) fn is_closing(&self) -> bool {
        self.closing.is_cancelled()
    }

    /// Resolves once the shard starts closing.
    pub(crate) fn closed(&self) -> tokio_util::sync::WaitForCancellationFuture<'_> {
        self.closing.cancelled()
    }

    pub(crate) fn agent(&self) -> &Rc<AgentServer<ConversationHarness>> {
        &self.agent
    }

    pub(crate) fn titles(&self) -> &Titles {
        &self.titles
    }

    pub(crate) fn conversation_sockets(&self) -> &TaskTracker {
        &self.conversation_sockets
    }

    pub(crate) fn forks(&self) -> &KeyedSerialGate<String> {
        &self.forks
    }

    /// Ends the user's work in order (`backend.md` § Startup and shutdown):
    /// the idle watches stop, and a retirement already running finishes;
    /// title requests are aborted; the conversation sockets end; open file
    /// transfers and user streams end, and stay closed; the agent turns are
    /// aborted while their runners are still connected; the Cloud is saved
    /// and stopped; the runner connections close, their work ends with
    /// them, and then the pipes fail. The answer says why the Cloud was not
    /// saved, when it was not.
    async fn close(&self) -> Option<String> {
        self.closing.cancel();
        self.stop_idle_watches();
        self.cloud.stop();
        self.titles.abort_all();
        self.conversation_sockets.close();
        self.conversation_sockets.wait().await;
        let _transfers_closed = self.conversations.end_transfers().await;
        self.agent.shutdown().await;
        let saved = self.close_cloud().await;
        self.devices.disconnect_all("backend shutting down");
        self.tasks.close();
        self.tasks.wait().await;
        self.pipes.close().await;
        saved.err().map(|error| format!("the Cloud of {}: {error}", self.user))
    }
}

/// Why a call produced no answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ShardUnavailable {
    /// The backend is shutting down and starts no new work.
    #[error("the backend is shutting down")]
    Closing,
    /// The call panicked; its shard goes on serving.
    #[error("the shard call failed")]
    Failed,
}

/// The way into every user's shard. Cloning it is cheap.
#[derive(Clone)]
pub(crate) struct Shards {
    queues: Arc<[mpsc::Sender<Message>]>,
}

/// The way into one user's shard.
pub(crate) struct ShardRef<'a> {
    queue: &'a mpsc::Sender<Message>,
    user: &'a UserId,
}

impl Shards {
    pub(crate) fn of<'a>(&'a self, user: &'a UserId) -> ShardRef<'a> {
        let digest = Sha256::digest(user.as_str().as_bytes());
        let hash = u64::from_be_bytes(digest[..8].try_into().expect("a SHA-256 digest has 8 bytes"));
        let count = u64::try_from(self.queues.len()).expect("the shard count fits u64");
        let index = usize::try_from(hash % count).expect("a shard index fits usize");
        ShardRef {
            queue: &self.queues[index],
            user,
        }
    }
}

impl ShardRef<'_> {
    /// Runs `work` on the user's shard and answers its result. The call runs
    /// to completion even when this future is dropped; dropping it cancels
    /// the token `work` receives, which only its waits observe, never a step
    /// after a commit.
    pub(crate) async fn call<F, Fut, T>(&self, work: F) -> Result<T, ShardUnavailable>
    where
        F: FnOnce(Rc<Shard>, CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = T> + 'static,
        T: Send + 'static,
    {
        self.run(work, false).await
    }

    /// A call the shard also serves while it closes, such as a runner's pipe
    /// request, which the shard's own shutdown steps may need.
    pub(crate) async fn call_while_closing<F, Fut, T>(&self, work: F) -> Result<T, ShardUnavailable>
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
        };
        self.queue
            .send(Message::Call(Box::new(job)))
            .await
            .map_err(|_| ShardUnavailable::Closing)?;
        // An answer dropped unsent means the call panicked.
        answered.await.unwrap_or(Err(ShardUnavailable::Failed))
    }

    /// Moves `work`, such as serving an upgraded socket, into the user's
    /// shard, where it runs as the shard's task until it ends. It answers
    /// once the work is queued; a closing shard refuses it, and dropping the
    /// work drops what it holds.
    pub(crate) async fn adopt<F, Fut>(&self, work: F) -> Result<(), ShardUnavailable>
    where
        F: FnOnce(Rc<Shard>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        let job = AdoptJob {
            user: self.user.clone(),
            work,
        };
        self.queue
            .send(Message::Call(Box::new(job)))
            .await
            .map_err(|_| ShardUnavailable::Closing)
    }
}

enum Message {
    Call(Box<dyn Job>),
    /// Answered with why each shard's Cloud was not saved.
    Close(oneshot::Sender<Vec<String>>),
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
            work, cancel, answer, ..
        } = *self;
        Box::pin(async move {
            let value = work(shard, cancel).await;
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
        let work = self.work;
        Box::pin(async move {
            // The work belongs to the shard, whose close waits for it.
            let tasks = shard.tasks.clone();
            tasks.spawn_local(work(shard));
        })
    }

    fn refuse(self: Box<Self>) {
        // Dropping the work drops what it holds, such as a socket, which
        // closes it.
    }
}

/// The shard threads, or the inline shard loop, owned by the backend.
pub(crate) struct ShardPool {
    shards: Shards,
    workers: Vec<Worker>,
}

enum Worker {
    Thread(std::thread::JoinHandle<()>),
    Inline(tokio::task::JoinHandle<()>),
}

impl ShardPool {
    pub(crate) async fn start(placement: ShardPlacement, services: Arc<Services>) -> io::Result<Self> {
        let mut queues = Vec::new();
        let mut workers = Vec::new();
        match placement {
            ShardPlacement::Inline => {
                let (queue, receiver) = mpsc::channel(QUEUE);
                queues.push(queue);
                workers.push(Worker::Inline(tokio::task::spawn_local(serve(receiver, services))));
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
        Ok(Self {
            shards: Shards {
                queues: queues.into(),
            },
            workers,
        })
    }

    pub(crate) fn shards(&self) -> Shards {
        self.shards.clone()
    }

    /// Refuses new calls, waits for the running ones and stops the shard
    /// threads; answers why a shard's Cloud was not saved, for each that was
    /// not.
    pub(crate) async fn close(self) -> Vec<String> {
        Self::stop(self.shards.queues.to_vec(), self.workers).await
    }

    async fn stop(queues: Vec<mpsc::Sender<Message>>, workers: Vec<Worker>) -> Vec<String> {
        let mut failures = Vec::new();
        for queue in &queues {
            let (closed, done) = oneshot::channel();
            if queue.send(Message::Close(closed)).await.is_ok() {
                // A shard loop that ended already has nothing left to stop.
                failures.extend(done.await.unwrap_or_default());
            }
        }
        for worker in workers {
            match worker {
                Worker::Inline(task) => {
                    if let Err(error) = task.await {
                        tracing::error!(error = &error as &dyn std::error::Error, "the shard loop failed");
                    }
                }
                Worker::Thread(thread) => {
                    let joined = tokio::task::spawn_blocking(move || thread.join()).await;
                    if !matches!(joined, Ok(Ok(()))) {
                        tracing::error!("a shard thread panicked");
                    }
                }
            }
        }
        failures
    }
}

/// Starts a shard thread and waits until its runtime runs.
async fn spawn_thread(
    index: usize,
    receiver: mpsc::Receiver<Message>,
    services: Arc<Services>,
) -> io::Result<std::thread::JoinHandle<()>> {
    let (started, running) = oneshot::channel();
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
                    runtime.block_on(serve(receiver, services));
                }
                Err(error) => {
                    let _ = started.send(Err(error));
                }
            }
        })?;
    match running.await {
        Ok(Ok(())) => Ok(thread),
        Ok(Err(error)) => Err(error),
        Err(_) => Err(io::Error::other("a shard thread ended before its runtime started")),
    }
}

/// A shard thread's loop: it creates each user's shard on the user's first
/// call and runs every call as a task of its own until the pool closes.
/// Closing, it refuses new calls, except those a shard serves while it
/// closes, closes every shard, and waits for the calls still running.
async fn serve(mut queue: mpsc::Receiver<Message>, services: Arc<Services>) {
    let calls = TaskTracker::new();
    let http = reqwest::Client::new();
    let mut shards: HashMap<UserId, Rc<Shard>> = HashMap::new();
    let mut closer = None;
    while let Some(message) = queue.recv().await {
        match message {
            Message::Call(job) => {
                let shard = shards
                    .entry(job.user().clone())
                    .or_insert_with_key(|user| {
                        Rc::new_cyclic(|shard| Shard::new(shard.clone(), user.clone(), services.clone(), http.clone()))
                    })
                    .clone();
                calls.spawn_local(job.run(shard));
            }
            Message::Close(done) => {
                closer = Some(done);
                break;
            }
        }
    }
    let closing = futures_util::future::join_all(shards.values().map(|shard| shard.close()));
    tokio::pin!(closing);
    let failures: Vec<String> = loop {
        tokio::select! {
            closed = &mut closing => break closed.into_iter().flatten().collect(),
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
                // Every sender is gone: nothing more arrives while the
                // shards close.
                None => break (&mut closing).await.into_iter().flatten().collect(),
            },
        }
    };
    queue.close();
    while let Some(message) = queue.recv().await {
        match message {
            Message::Call(job) => job.refuse(),
            Message::Close(done) => drop(done),
        }
    }
    calls.close();
    calls.wait().await;
    if let Some(done) = closer {
        // The closer waits for this answer; if it went away, no one is left
        // to tell.
        let _ = done.send(failures);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    async fn pool(placement: ShardPlacement) -> (ShardPool, tempfile::TempDir) {
        let data = tempfile::tempdir().unwrap();
        let services = Services::start_for_tests(data.path()).await;
        (ShardPool::start(placement, services).await.unwrap(), data)
    }

    fn user(name: &str) -> UserId {
        UserId::try_from(name.to_owned()).unwrap()
    }

    #[tokio::test]
    async fn a_user_always_reaches_the_same_shard_on_its_thread() {
        let (pool, _data) = pool(ShardPlacement::Threads(NonZeroUsize::new(3).unwrap())).await;
        let shards = pool.shards();
        let ana = user("ana");
        let place = |shard: Rc<Shard>, _| async move {
            (std::thread::current().name().map(str::to_owned), Rc::as_ptr(&shard) as usize, shard.user().clone())
        };
        let first = shards.of(&ana).call(place).await.unwrap();
        let second = shards.of(&ana).call(place).await.unwrap();
        assert_eq!(first, second);
        assert!(first.0.unwrap().starts_with("shard-"));
        assert_eq!(first.2, ana);
        pool.close().await;
    }

    #[tokio::test(flavor = "local")]
    async fn a_call_whose_requester_leaves_runs_to_completion_with_its_token_cancelled() {
        let (pool, _data) = pool(ShardPlacement::Inline).await;
        let shards = pool.shards();
        let (finished, observed) = oneshot::channel();
        let ana = user("ana");
        let shard = shards.of(&ana);
        let call = shard.call(move |_, cancel| async move {
            cancel.cancelled().await;
            finished.send("cancelled, then finished").unwrap();
        });
        assert!(tokio::time::timeout(Duration::from_millis(50), call).await.is_err());
        assert_eq!(observed.await.unwrap(), "cancelled, then finished");
        pool.close().await;
    }

    #[tokio::test(flavor = "local")]
    async fn a_panicking_call_fails_and_its_shard_goes_on() {
        let (pool, _data) = pool(ShardPlacement::Inline).await;
        let shards = pool.shards();
        let ana = user("ana");
        let failed = shards.of(&ana).call(|_, _| async { panic!("the call fails") }).await;
        assert_eq!(failed, Err::<(), _>(ShardUnavailable::Failed));
        assert_eq!(shards.of(&ana).call(|_, _| async { 7 }).await, Ok(7));
        pool.close().await;
    }

    #[tokio::test(flavor = "local")]
    async fn closing_waits_for_running_calls_and_refuses_later_ones() {
        let (pool, _data) = pool(ShardPlacement::Inline).await;
        let shards = pool.shards();
        let ana = user("ana");
        let (started, running) = oneshot::channel();
        let (release, released) = oneshot::channel::<()>();
        let running_call = {
            let shards = shards.clone();
            let ana = ana.clone();
            tokio::task::spawn_local(async move {
                shards
                    .of(&ana)
                    .call(move |_, _| async move {
                        started.send(()).unwrap();
                        released.await.unwrap();
                        "done"
                    })
                    .await
            })
        };
        running.await.unwrap();
        let closing = tokio::task::spawn_local(pool.close());
        tokio::task::yield_now().await;
        assert!(!closing.is_finished());
        release.send(()).unwrap();
        assert_eq!(running_call.await.unwrap(), Ok("done"));
        closing.await.unwrap();
        assert_eq!(
            shards.of(&ana).call(|_, _| async {}).await,
            Err(ShardUnavailable::Closing)
        );
    }
}
