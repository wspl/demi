//! What a test build adds for a black-box suite that runs `demi-backend` as a
//! process (`docs/internal/go-migration/design/g6-api-suite.md`): the tuning
//! file `DEMI_TEST_TUNING` and the control socket `DEMI_TEST_CONTROL`. A
//! release build has none of it.
//!
//! The tuning file is a JSON document that sets what no environment variable
//! sets: the tuning structs of the configuration, the models.dev address and
//! the user streams. A member left out keeps the product's default. The
//! control socket carries newline-delimited JSON, one request and one reply
//! per line, replies in the order the requests finish. A hold or a lease
//! belongs to the connection that took it and ends when it is released or
//! when the connection closes, so a failed test leaves none behind.
//!
//! The control socket accepts connections once the backend serves, so
//! its existence tells a suite that the backend is ready.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use demi_command_tree::NativeOperation;
use demi_core::Clock;
use demi_gates::{GateLease, Purpose};
use demi_web_api::auth::{Password, Role};
use demi_web_api::ids::{ConversationId, UserId};
use demi_web_api::text::EmailAddress;
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::auth::email_change::{AccountMail, MailError, VerificationMail};
use crate::config::BackendConfig;
use crate::storage::objects::counting::ObjectCounts;
use crate::{Backend, CommitHold, HelloStep, StepHold, SyncStep};

/// The variable that names the tuning file.
pub const TUNING_VARIABLE: &str = "DEMI_TEST_TUNING";

/// The variable that names the control socket.
pub const CONTROL_VARIABLE: &str = "DEMI_TEST_CONTROL";

/// Where the manual clock starts.
const CLOCK_START: &str = "2026-09-24T08:00:00Z";

/// The longest request line the control socket reads, its newline excluded.
const MAX_LINE_BYTES: usize = 1 << 20;

/// Why the test hooks could not be set up.
#[derive(Debug, thiserror::Error)]
pub enum TestControlError {
    #[error("{TUNING_VARIABLE} ({path}) cannot be used: {reason}")]
    Tuning { path: PathBuf, reason: String },
    #[error("{CONTROL_VARIABLE} ({path}) cannot be bound: {source}")]
    Bind { path: PathBuf, source: io::Error },
}

/// The tuning file. Every member is optional; a member that is left out keeps
/// the product's default, and an unknown one refuses the file.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Tuning {
    runners: Option<RunnersTuning>,
    lifecycle: Option<LifecycleTuning>,
    cloud: Option<CloudTuning>,
    conversations: Option<ConversationsTuning>,
    pages: Option<PagesTuning>,
    exposes: Option<ExposesTuning>,
    logins: Option<LoginsTuning>,
    /// Where the backend reads the models.dev document.
    models_dev_url: Option<url::Url>,
    /// The user streams a page may open, replacing the default one.
    user_streams: Option<BTreeMap<String, NativeOperation>>,
    /// Whether the backend has a mail sender: one that keeps what it sends
    /// for `mail.list`. Without one an email change answers `mail_unavailable`.
    mail: Option<bool>,
}

/// `RunnerTuning`; durations are milliseconds.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RunnersTuning {
    hello_deadline_ms: Option<u64>,
    claim_lifetime_ms: Option<u64>,
    claims_per_minute: Option<usize>,
    /// 0 turns liveness off.
    ping_ms: Option<u64>,
}

/// `LifecycleTuning`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LifecycleTuning {
    idle_window_ms: Option<u64>,
    idle_poll_ms: Option<u64>,
    /// 0 runs no retention pass by itself.
    retention_interval_ms: Option<u64>,
}

/// `CloudTuning`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CloudTuning {
    sweep_ms: Option<u64>,
    checkpoint_interval_ms: Option<u64>,
    lifetime_cap_ms: Option<u64>,
    runner_connection_ms: Option<u64>,
    crash_loop_deaths: Option<u32>,
    crash_loop_window_ms: Option<u64>,
    sync_timeout_ms: Option<u64>,
    reset_hold_ms: Option<u64>,
    system_quota: Option<u64>,
    home_quota: Option<u64>,
    capacity: Option<usize>,
}

/// `ConversationTuning`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConversationsTuning {
    outbox_frames: Option<usize>,
    requests_per_minute: Option<usize>,
    titles: Option<bool>,
}

/// `PageTuning`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PagesTuning {
    heartbeat_ms: Option<u64>,
    close_wait_ms: Option<u64>,
}

/// `ExposeTuning`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExposesTuning {
    idle_ms: Option<u64>,
}

/// `LoginTiming`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LoginsTuning {
    lifetime_ms: Option<u64>,
    retention_ms: Option<u64>,
}

fn millis(value: u64) -> Duration {
    Duration::from_millis(value)
}

impl Tuning {
    fn read(path: &Path) -> Result<Self, TestControlError> {
        let refused = |reason: String| TestControlError::Tuning { path: path.to_owned(), reason };
        let bytes = std::fs::read(path).map_err(|error| refused(error.to_string()))?;
        serde_json::from_slice(&bytes).map_err(|error| refused(error.to_string()))
    }

    /// Sets what the file names in `config`, but the mail sender, which the
    /// control sets up.
    fn apply(self, config: &mut BackendConfig) {
        if let Some(runners) = self.runners {
            let tuning = &mut config.runners;
            if let Some(value) = runners.hello_deadline_ms {
                tuning.hello_deadline = millis(value);
            }
            if let Some(value) = runners.claim_lifetime_ms {
                tuning.claim_lifetime = millis(value);
            }
            if let Some(value) = runners.claims_per_minute {
                tuning.claims_per_minute = value;
            }
            if let Some(value) = runners.ping_ms {
                tuning.ping = (value > 0).then(|| millis(value));
            }
        }
        if let Some(lifecycle) = self.lifecycle {
            let tuning = &mut config.lifecycle;
            if let Some(value) = lifecycle.idle_window_ms {
                tuning.idle_window = millis(value);
            }
            if let Some(value) = lifecycle.idle_poll_ms {
                tuning.idle_poll = millis(value);
            }
            if let Some(value) = lifecycle.retention_interval_ms {
                tuning.retention_interval = (value > 0).then(|| millis(value));
            }
        }
        if let Some(cloud) = self.cloud {
            let tuning = &mut config.cloud;
            if let Some(value) = cloud.sweep_ms {
                tuning.sweep = millis(value);
            }
            if let Some(value) = cloud.checkpoint_interval_ms {
                tuning.checkpoint_interval = millis(value);
            }
            if let Some(value) = cloud.lifetime_cap_ms {
                tuning.lifetime_cap = millis(value);
            }
            if let Some(value) = cloud.runner_connection_ms {
                tuning.runner_connection = millis(value);
            }
            if let Some(value) = cloud.crash_loop_deaths {
                tuning.crash_loop_deaths = value;
            }
            if let Some(value) = cloud.crash_loop_window_ms {
                tuning.crash_loop_window = millis(value);
            }
            if let Some(value) = cloud.sync_timeout_ms {
                tuning.sync_timeout = millis(value);
            }
            if let Some(value) = cloud.reset_hold_ms {
                tuning.reset_hold = millis(value);
            }
            if let Some(value) = cloud.system_quota {
                tuning.system_quota = value;
            }
            if let Some(value) = cloud.home_quota {
                tuning.home_quota = value;
            }
            if let Some(value) = cloud.capacity {
                tuning.capacity = value;
            }
        }
        if let Some(conversations) = self.conversations {
            let tuning = &mut config.conversations;
            if let Some(value) = conversations.outbox_frames {
                tuning.outbox_frames = value;
            }
            if let Some(value) = conversations.requests_per_minute {
                tuning.requests_per_minute = value;
            }
            if let Some(value) = conversations.titles {
                tuning.titles = value;
            }
        }
        if let Some(pages) = self.pages {
            let tuning = &mut config.pages;
            if let Some(value) = pages.heartbeat_ms {
                tuning.heartbeat = millis(value);
            }
            if let Some(value) = pages.close_wait_ms {
                tuning.close_wait = millis(value);
            }
        }
        if let Some(exposes) = self.exposes {
            if let Some(value) = exposes.idle_ms {
                config.exposes.idle = millis(value);
            }
        }
        if let Some(logins) = self.logins {
            let tuning = &mut config.logins;
            if let Some(value) = logins.lifetime_ms {
                tuning.lifetime = millis(value);
            }
            if let Some(value) = logins.retention_ms {
                tuning.retention = millis(value);
            }
        }
        if let Some(url) = self.models_dev_url {
            config.models_dev_url = url;
        }
        if let Some(streams) = self.user_streams {
            config.user_streams = streams;
        }
    }
}

/// Wall-clock time a test sets: it stands still until it is moved.
pub struct ManualClock(Mutex<Timestamp>);

impl ManualClock {
    /// A clock standing at the start every suite shares.
    pub fn new() -> Self {
        Self(Mutex::new(CLOCK_START.parse().expect("the clock's start is a timestamp")))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Timestamp> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn set(&self, at: Timestamp) {
        *self.lock() = at;
    }

    /// Moves the clock by `by` and answers its new time.
    pub fn advance(&self, by: SignedDuration) -> Result<Timestamp, String> {
        let mut now = self.lock();
        let moved = now.checked_add(by).map_err(|error| error.to_string())?;
        *now = moved;
        Ok(moved)
    }

    /// Sets the time to the system's, for a test that compares the backend's
    /// times with those the file system gives the objects it writes.
    pub fn follow_system(&self) {
        *self.lock() = Timestamp::now();
    }

    fn read(&self) -> Timestamp {
        *self.lock()
    }
}

impl Default for ManualClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for ManualClock {
    fn now(&self) -> demi_core::Timestamp {
        demi_core::Timestamp::truncate(self.read())
    }
}

/// Captures verification mail, or refuses it while `failing` is set.
#[derive(Default)]
pub struct Mailbox {
    sent: Mutex<Vec<VerificationMail>>,
    failing: AtomicBool,
}

impl Mailbox {
    /// The mail sent so far, in order.
    pub fn sent(&self) -> Vec<VerificationMail> {
        self.sent.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Makes the transport fail, or work again.
    pub fn fail(&self, failing: bool) {
        self.failing.store(failing, Ordering::SeqCst);
    }
}

impl AccountMail for Mailbox {
    fn send_verification(
        &self,
        mail: VerificationMail,
    ) -> futures_util::future::BoxFuture<'static, Result<(), MailError>> {
        let delivered = if self.failing.load(Ordering::SeqCst) {
            Err(MailError("the mail transport failed".to_owned()))
        } else {
            self.sent.lock().unwrap_or_else(PoisonError::into_inner).push(mail);
            Ok(())
        };
        Box::pin(std::future::ready(delivered))
    }
}

/// The hooks a test build gives a suite, set up before the backend starts and
/// served on the control socket once it does.
pub struct TestControl {
    socket: PathBuf,
    clock: Arc<ManualClock>,
    mail: Arc<Mailbox>,
    counts: ObjectCounts,
}

impl TestControl {
    /// Reads the tuning file `DEMI_TEST_TUNING` names into `config` and, when
    /// `DEMI_TEST_CONTROL` names a socket, puts the backend on a manual clock,
    /// over an object store that counts, and behind a mailbox when the tuning
    /// asks for mail, and answers the control that serves them.
    pub fn from_environment(config: &mut BackendConfig) -> Result<Option<Self>, TestControlError> {
        let tuning = match std::env::var_os(TUNING_VARIABLE) {
            Some(path) => Tuning::read(Path::new(&path))?,
            None => Tuning::default(),
        };
        let mail = tuning.mail.unwrap_or(false);
        tuning.apply(config);
        let Some(socket) = std::env::var_os(CONTROL_VARIABLE) else {
            return Ok(None);
        };
        let control = Self {
            socket: PathBuf::from(socket),
            clock: Arc::new(ManualClock::new()),
            mail: Arc::default(),
            counts: ObjectCounts::default(),
        };
        config.clock = control.clock.clone();
        if mail {
            config.account_mail = Some(control.mail.clone());
        }
        config.object_counts = Some(control.counts.clone());
        Ok(Some(control))
    }

    /// Binds the control socket, once the backend serves: a connection to it
    /// tells a suite that the backend is ready.
    pub fn bind(&self) -> Result<UnixListener, TestControlError> {
        // A socket a killed backend left behind.
        let _ = std::fs::remove_file(&self.socket);
        UnixListener::bind(&self.socket).map_err(|source| TestControlError::Bind { path: self.socket.clone(), source })
    }

    /// Serves the control socket, bound by [`bind`](Self::bind), over
    /// `backend`.
    pub fn serve(self, listener: UnixListener, backend: Backend) -> ControlServer {
        let shared = Arc::new(Shared { backend, clock: self.clock, mail: self.mail, counts: self.counts });
        let stop = CancellationToken::new();
        let accepting = tokio::spawn(accept(listener, shared.clone(), stop.clone()));
        ControlServer { shared, stop, accepting, socket: self.socket }
    }
}

/// The control socket while it serves.
pub struct ControlServer {
    shared: Arc<Shared>,
    stop: CancellationToken,
    accepting: tokio::task::JoinHandle<()>,
    socket: PathBuf,
}

impl ControlServer {
    /// Closes the socket and every connection, which releases their holds
    /// and leases, and answers the backend for its shutdown.
    pub async fn stop(self) -> Backend {
        self.stop.cancel();
        // The accepting task ends after its connections did, and none of
        // them panics.
        let _ = self.accepting.await;
        let _ = std::fs::remove_file(&self.socket);
        match Arc::try_unwrap(self.shared) {
            Ok(shared) => shared.backend,
            Err(_) => unreachable!("every connection ended with the accepting task"),
        }
    }
}

struct Shared {
    backend: Backend,
    clock: Arc<ManualClock>,
    mail: Arc<Mailbox>,
    counts: ObjectCounts,
}

async fn accept(listener: UnixListener, shared: Arc<Shared>, stop: CancellationToken) {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            () = stop.cancelled() => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    connections.spawn(connection(stream, shared.clone(), stop.clone()));
                }
                Err(error) => {
                    tracing::error!(error = &error as &dyn std::error::Error, "the test control cannot accept");
                    break;
                }
            },
        }
    }
    // Every connection ends on the stop, or ended already.
    stop.cancel();
    while connections.join_next().await.is_some() {}
}

/// A request: an id the client chooses, which its reply names, and the
/// operation.
#[derive(Debug, Deserialize)]
struct Request {
    id: String,
    #[serde(flatten)]
    call: Call,
}

/// The operations of the control (`g6-api-suite.md` § The test control
/// socket). Keys a message does not declare are ignored.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", content = "params")]
enum Call {
    /// Moves the manual clock by `byMs`, which may be negative.
    #[serde(rename = "clock.advance")]
    ClockAdvance(ClockAdvance),
    /// Sets the manual clock.
    #[serde(rename = "clock.set")]
    ClockSet(ClockSet),
    /// Adds an account without a route of the API.
    #[serde(rename = "users.add")]
    UsersAdd(UsersAdd),
    /// Enters a conversation's file gate; the reply names the lease.
    #[serde(rename = "gate.enter")]
    GateEnter(GateEnter),
    /// Waits until `count` entrants wait at a conversation's file gate.
    #[serde(rename = "gate.waiting")]
    GateWaiting(GateWaiting),
    /// Holds a flow; the reply names the hold.
    #[serde(rename = "hold")]
    Hold(Hold),
    /// Waits until `count` passes reached a hold.
    #[serde(rename = "hold.wait")]
    HoldWait(HoldWait),
    /// Ends a hold or a lease of this connection.
    #[serde(rename = "release")]
    Release(Release),
    /// Runs a user's retention pass and answers once it ended.
    #[serde(rename = "retention.run")]
    RetentionRun(RetentionRun),
    /// What reached the object store.
    #[serde(rename = "objects.count")]
    ObjectsCount(Empty),
    /// The verification mail sent so far.
    #[serde(rename = "mail.list")]
    MailList(Empty),
    /// Makes the mail transport fail, or work again.
    #[serde(rename = "mail.fail")]
    MailFail(MailFail),
}

#[derive(Debug, Deserialize)]
struct Empty {}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClockAdvance {
    by_ms: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClockSet {
    at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct UsersAdd {
    email: String,
    password: String,
    role: Role,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PurposeName {
    Demand,
    Maintenance,
}

#[derive(Debug, Deserialize)]
struct GateEnter {
    user: String,
    conversation: String,
    purpose: PurposeName,
}

#[derive(Debug, Deserialize)]
struct GateWaiting {
    user: String,
    conversation: String,
    count: usize,
}

#[derive(Debug, Deserialize)]
struct Hold {
    target: HoldTarget,
}

/// What a hold stops: the commits of the conversations' saves, a runner's
/// hello at a step, or a page's synchronization channel at a step.
#[derive(Debug, Clone, Copy, Deserialize)]
enum HoldTarget {
    #[serde(rename = "commits")]
    Commits,
    #[serde(rename = "hello:token_lookup")]
    HelloTokenLookup,
    #[serde(rename = "hello:bind")]
    HelloBind,
    #[serde(rename = "sync:snapshot")]
    SyncSnapshot,
    #[serde(rename = "sync:changes")]
    SyncChanges,
}

#[derive(Debug, Deserialize)]
struct HoldWait {
    hold: String,
    count: usize,
}

#[derive(Debug, Deserialize)]
struct Release {
    id: String,
}

#[derive(Debug, Deserialize)]
struct RetentionRun {
    user: String,
}

#[derive(Debug, Deserialize)]
struct MailFail {
    failing: bool,
}

/// A reply: the request's result, or why it failed.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Reply {
    Ok { id: String, result: Value },
    Error { id: String, message: String },
}

/// What a connection holds until it releases it or ends.
enum Held {
    /// Read by no one: a lease counts while it is held, and ends when dropped.
    Lease(#[expect(dead_code, reason = "held for its drop")] GateLease),
    Commits(CommitHold),
    Step(StepHold),
}

/// A hold or a lease of one connection, and the requests that wait at it.
struct Entry {
    held: Held,
    /// Set once the connection released the entry, which ends every wait.
    released: CancellationToken,
    waiting: TaskTracker,
}

/// The holds and leases of one connection. Dropping it releases them all.
#[derive(Default)]
struct Holdings {
    next: AtomicU64,
    held: Mutex<HashMap<String, Arc<Entry>>>,
}

impl Holdings {
    fn keep(&self, held: Held) -> String {
        let id = (self.next.fetch_add(1, Ordering::SeqCst) + 1).to_string();
        let entry = Entry { held, released: CancellationToken::new(), waiting: TaskTracker::new() };
        self.held.lock().unwrap_or_else(PoisonError::into_inner).insert(id.clone(), Arc::new(entry));
        id
    }

    fn find(&self, id: &str) -> Result<Arc<Entry>, String> {
        self.held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .cloned()
            .ok_or_else(|| format!("this connection holds no {id}"))
    }

    /// Ends the hold or lease, and the waits at it; answers once it is free.
    async fn release(&self, id: &str) -> Result<(), String> {
        let entry = self
            .held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id)
            .ok_or_else(|| format!("this connection holds no {id}"))?;
        entry.released.cancel();
        entry.waiting.close();
        // A wait keeps the entry, and with it the hold, until it noticed.
        entry.waiting.wait().await;
        Ok(())
    }
}

/// Waits until `count` passes reached the hold in `entry`, or the entry is
/// released.
async fn wait_at(entry: &Entry, hold: &str, count: usize) -> Result<(), String> {
    let reached: Pin<Box<dyn Future<Output = ()> + Send + '_>> = match &entry.held {
        Held::Commits(commits) => Box::pin(commits.until_waiting(count)),
        Held::Step(step) => Box::pin(step.until_arrived(count)),
        Held::Lease(_) => return Err(format!("{hold} is a lease, which nothing waits at")),
    };
    tokio::select! {
        () = entry.released.cancelled() => Err(format!("{hold} was released while it was waited at")),
        () = reached => Ok(()),
    }
}

/// One control connection: requests run concurrently and are answered as
/// they finish. A line that is not a request drops the connection.
async fn connection(stream: UnixStream, shared: Arc<Shared>, stop: CancellationToken) {
    let (read, mut write) = stream.into_split();
    let (replies, mut outgoing) = mpsc::channel::<Vec<u8>>(64);
    let writer = tokio::spawn(async move {
        while let Some(line) = outgoing.recv().await {
            if write.write_all(&line).await.is_err() {
                return;
            }
        }
    });
    let holdings = Arc::new(Holdings::default());
    let mut requests = JoinSet::new();
    let mut lines = BufReader::new(read).lines();
    loop {
        let line = tokio::select! {
            () = stop.cancelled() => break,
            line = lines.next_line() => match line {
                Ok(Some(line)) => line,
                _ => break,
            },
        };
        if line.is_empty() {
            continue;
        }
        if line.len() > MAX_LINE_BYTES {
            break;
        }
        let Ok(request) = serde_json::from_str::<Request>(&line) else {
            break;
        };
        let (shared, holdings, replies) = (shared.clone(), holdings.clone(), replies.clone());
        requests.spawn(async move {
            let reply = match run(&shared, &holdings, request.call).await {
                Ok(result) => Reply::Ok { id: request.id, result },
                Err(message) => Reply::Error { id: request.id, message },
            };
            let mut line = serde_json::to_vec(&reply).expect("a reply is JSON");
            line.push(b'\n');
            // A connection that closed discards its replies.
            let _ = replies.send(line).await;
        });
    }
    // A request still waiting ends with the connection, and with it the
    // holds it took.
    requests.shutdown().await;
    drop(holdings);
    drop(replies);
    // The writer ends with its channel; it fails only on a closed socket.
    let _ = writer.await;
}

type Outcome<'a> = Pin<Box<dyn Future<Output = Result<Value, String>> + Send + 'a>>;

fn run<'a>(shared: &'a Shared, holdings: &'a Holdings, call: Call) -> Outcome<'a> {
    Box::pin(async move {
        match call {
            Call::ClockAdvance(ClockAdvance { by_ms }) => {
                let moved = shared.clock.advance(SignedDuration::from_millis(by_ms))?;
                Ok(json!({ "atMs": moved.as_millisecond() }))
            }
            Call::ClockSet(ClockSet { at_ms }) => {
                let at = Timestamp::from_millisecond(at_ms).map_err(|error| error.to_string())?;
                shared.clock.set(at);
                Ok(json!({ "atMs": at.as_millisecond() }))
            }
            Call::UsersAdd(add) => add_user(shared, add).await,
            Call::GateEnter(enter) => {
                let gate = file_gate(shared, &enter.user, &enter.conversation).await?;
                let purpose = match enter.purpose {
                    PurposeName::Demand => Purpose::Demand,
                    PurposeName::Maintenance => Purpose::Maintenance,
                };
                let lease = gate.enter(purpose).await;
                Ok(json!({ "id": holdings.keep(Held::Lease(lease)) }))
            }
            Call::GateWaiting(waiting) => {
                let gate = file_gate(shared, &waiting.user, &waiting.conversation).await?;
                let mut watched = gate.waiting();
                watched.wait_for(|entrants| *entrants >= waiting.count).await.map_err(|error| error.to_string())?;
                Ok(json!({}))
            }
            Call::Hold(Hold { target }) => {
                let held = match target {
                    HoldTarget::Commits => Held::Commits(shared.backend.hold_commits()),
                    HoldTarget::HelloTokenLookup => Held::Step(shared.backend.hold_hellos(HelloStep::TokenLookup)),
                    HoldTarget::HelloBind => Held::Step(shared.backend.hold_hellos(HelloStep::Bind)),
                    HoldTarget::SyncSnapshot => Held::Step(shared.backend.hold_sync(SyncStep::Snapshot)),
                    HoldTarget::SyncChanges => Held::Step(shared.backend.hold_sync(SyncStep::Changes)),
                };
                Ok(json!({ "id": holdings.keep(held) }))
            }
            Call::HoldWait(HoldWait { hold, count }) => {
                let entry = holdings.find(&hold)?;
                entry.waiting.track_future(wait_at(&entry, &hold, count)).await?;
                Ok(json!({}))
            }
            Call::Release(Release { id }) => {
                holdings.release(&id).await?;
                Ok(json!({}))
            }
            Call::RetentionRun(RetentionRun { user }) => {
                let user = UserId::try_from(user).map_err(|error| error.to_string())?;
                shared.backend.run_retention(&user).await;
                Ok(json!({}))
            }
            Call::ObjectsCount(Empty {}) => {
                let tally = shared.counts.tally();
                Ok(json!({
                    "puts": tally.puts,
                    "bytesPut": tally.bytes_put,
                    "gets": tally.gets,
                    "heads": tally.heads,
                    "mostGetsAtOnce": tally.most_gets_at_once,
                    "lists": tally.lists,
                    "deletes": tally.deletes,
                }))
            }
            Call::MailList(Empty {}) => {
                let mail: Vec<Value> = shared
                    .mail
                    .sent()
                    .iter()
                    .map(|mail| {
                        json!({
                            "email": mail.email.as_str(),
                            "code": mail.code,
                            "expiresAtMs": mail.expires_at.as_millisecond(),
                        })
                    })
                    .collect();
                Ok(json!({ "mail": mail }))
            }
            Call::MailFail(MailFail { failing }) => {
                shared.mail.fail(failing);
                Ok(json!({}))
            }
        }
    })
}

async fn file_gate(shared: &Shared, user: &str, conversation: &str) -> Result<demi_gates::ActivityGate, String> {
    let user = UserId::try_from(user).map_err(|error| error.to_string())?;
    let conversation = ConversationId::try_from(conversation).map_err(|error| error.to_string())?;
    Ok(shared.backend.file_gate(&user, &conversation).await)
}

async fn add_user(shared: &Shared, add: UsersAdd) -> Result<Value, String> {
    let email = EmailAddress::try_from(add.email).map_err(|error| error.to_string())?;
    let created = shared.backend.add_user(email, Password::from(add.password), add.role).await?;
    Ok(json!({ "id": created.as_str() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tuned(text: &str) -> BackendConfig {
        let path = std::env::temp_dir().join(format!("demi-tuning-{}-{}.json", std::process::id(), text.len()));
        std::fs::write(&path, text).unwrap();
        let tuning = Tuning::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        let mut config = BackendConfig::new(
            PathBuf::from("data"),
            "127.0.0.1:0".parse().unwrap(),
            demi_web_api::settings::InstanceMode::Shared,
            PathBuf::from("machines.sock"),
        );
        tuning.apply(&mut config);
        config
    }

    #[test]
    fn the_tuning_file_sets_what_it_names_and_leaves_the_rest_at_the_default() {
        let config = tuned(
            r#"{ "runners": { "pingMs": 0, "claimsPerMinute": 2 }, "lifecycle": { "retentionIntervalMs": 0 },
                 "conversations": { "titles": false }, "pages": { "heartbeatMs": 50 } }"#,
        );
        assert_eq!(config.runners.ping, None);
        assert_eq!(config.runners.claims_per_minute, 2);
        assert_eq!(config.runners.hello_deadline, Duration::from_secs(30));
        assert_eq!(config.lifecycle.retention_interval, None);
        assert!(!config.conversations.titles);
        assert_eq!(config.pages.heartbeat, Duration::from_millis(50));
        assert_eq!(config.pages.close_wait, Duration::from_secs(1));
    }

    #[test]
    fn a_member_the_tuning_does_not_declare_refuses_the_file() {
        let path = std::env::temp_dir().join(format!("demi-tuning-unknown-{}.json", std::process::id()));
        std::fs::write(&path, r#"{ "runners": { "pings": 1 } }"#).unwrap();
        let refused = Tuning::read(&path).unwrap_err();
        std::fs::remove_file(&path).unwrap();
        assert!(refused.to_string().contains("unknown field `pings`"), "{refused}");
    }

    #[test]
    fn a_request_names_its_operation_and_a_line_that_is_none_is_refused() {
        let request: Request = serde_json::from_str(r#"{"id":"7","op":"clock.advance","params":{"byMs":-5}}"#).unwrap();
        assert_eq!(request.id, "7");
        assert!(matches!(request.call, Call::ClockAdvance(ClockAdvance { by_ms: -5 })));
        let mail: Request = serde_json::from_str(r#"{"id":"8","op":"mail.list","params":{}}"#).unwrap();
        assert!(matches!(mail.call, Call::MailList(_)));
        assert!(serde_json::from_str::<Request>(r#"{"id":"9","op":"nothing","params":{}}"#).is_err());
    }
}
