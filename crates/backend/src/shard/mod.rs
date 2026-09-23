//! Shard threads and calls into a user's shard (`concurrency.md` § The user
//! shard). A shard thread is a single-threaded `LocalRuntime` hosting the
//! shards of every user pinned to it; a stable hash of the user id pins each
//! user to one thread. The edge reaches a user's shard with
//! `shards.of(user).call(..)`: the closure is `Send`, the future it starts
//! runs on the shard and need not be, and the answer is `Send`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::num::NonZeroUsize;
use std::rc::Rc;
use std::sync::Arc;

use demi_web_api::ids::UserId;
use futures_util::future::LocalBoxFuture;
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::backend::Services;
use crate::usage::rate_limit::{REQUESTS_PER_WINDOW, RequestRateLimit};

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
    services: Arc<Services>,
    /// The HTTP client of the shard's thread, which the runtimes built here
    /// send with: hyper ties a pooled connection to the runtime that made it.
    http: reqwest::Client,
    /// The user's request rate limit, which every runtime of the user's
    /// conversations counts against.
    rate_limit: Rc<RefCell<RequestRateLimit>>,
}

impl Shard {
    pub(crate) fn user(&self) -> &UserId {
        &self.user
    }

    pub(crate) fn services(&self) -> &Services {
        &self.services
    }

    pub(crate) fn http(&self) -> &reqwest::Client {
        &self.http
    }

    #[expect(dead_code, reason = "the conversations' metered runtimes count against it")]
    pub(crate) fn rate_limit(&self) -> &Rc<RefCell<RequestRateLimit>> {
        &self.rate_limit
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
        let cancel = CancellationToken::new();
        let _requester = cancel.clone().drop_guard();
        let (answer, answered) = oneshot::channel();
        let job = CallJob {
            user: self.user.clone(),
            work,
            cancel,
            answer,
        };
        self.queue
            .send(Message::Call(Box::new(job)))
            .await
            .map_err(|_| ShardUnavailable::Closing)?;
        // An answer dropped unsent means the call panicked.
        answered.await.unwrap_or(Err(ShardUnavailable::Failed))
    }
}

enum Message {
    Call(Box<dyn Job>),
    Close(oneshot::Sender<()>),
}

/// A call on its way to a shard, which either runs it or refuses it.
trait Job: Send {
    fn user(&self) -> &UserId;
    fn run(self: Box<Self>, shard: Rc<Shard>) -> LocalBoxFuture<'static, ()>;
    fn refuse(self: Box<Self>);
}

struct CallJob<F, T> {
    user: UserId,
    work: F,
    cancel: CancellationToken,
    answer: oneshot::Sender<Result<T, ShardUnavailable>>,
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
    /// threads.
    pub(crate) async fn close(self) {
        Self::stop(self.shards.queues.to_vec(), self.workers).await;
    }

    async fn stop(queues: Vec<mpsc::Sender<Message>>, workers: Vec<Worker>) {
        for queue in &queues {
            let (closed, done) = oneshot::channel();
            if queue.send(Message::Close(closed)).await.is_ok() {
                // A shard loop that ended already has nothing left to stop.
                let _ = done.await;
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
                        Rc::new(Shard {
                            user: user.clone(),
                            services: services.clone(),
                            http: http.clone(),
                            rate_limit: Rc::new(RefCell::new(RequestRateLimit::new(REQUESTS_PER_WINDOW))),
                        })
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
    queue.close();
    while let Some(message) = queue.recv().await {
        match message {
            Message::Call(job) => job.refuse(),
            // Only the pool closes, once.
            Message::Close(done) => drop(done),
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
