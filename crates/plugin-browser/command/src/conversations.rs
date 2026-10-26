//! The conversations' browsers.
//!
//! One owner task finds each invocation's conversation browser and forgets it
//! after the conversation's release. Each conversation browser is an owner task
//! of its own, so one conversation's slow start or retirement never holds up
//! another's commands, and the runner's `status` is answered from the published
//! state without waiting for either.
//!
//! ```text
//! Resident
//!   conversations ............. finds each invocation's conversation browser
//!    `-- conversation browser . one per conversation: absent | starting |
//!          |                    ready | closing; admits commands, runs release
//!          `-- Chrome environment: process tree, profile, CDP event pump
//!                |-- tab registry ..... tabs, handles, openers; snapshot
//!                |-- capture channel .. the capture extension's connection
//!                |-- live hub ......... viewers, captures, screen, viewports
//!                `-- each tab ......... operation lock and its session; debug
//!                                       connections; console; open dialog
//! ```
//!
//! Each piece of state has one owner task; others send requests or read its
//! snapshot. Every task started for an environment belongs to it, and
//! retirement joins all of them before it removes the profile: no browser work
//! outlives its environment.
//!
//! # Lifetime
//!
//! - The browser ends on exactly two signals it sees itself: the last tab
//!   closing and the conversation release. It has no idle timer and no notion
//!   of agent activity. The backend decides when to release, and an open tab, a
//!   loading page or a running download never postpones it.
//! - Retirement invalidates tabs, references and debugging handles together. A
//!   release cancels the commands that run (`cancelled`) before it retires. A
//!   command still running when another closes the last tab fails with
//!   `browser_lost`. Nothing is restored: the next `tab.new` starts fresh.
//! - A failed cleanup keeps another environment from starting over the
//!   unresolved owner. After a completed one the next `tab.new` may start a
//!   fresh browser.
//! - A resident shutdown retires all its browsers at once, so it takes as long
//!   as the slowest. A resident that ends first leaves them to the sweep in
//!   `tabs::environment`.

use std::{collections::HashMap, sync::Arc};

use bytes::Bytes;
use tokio::{
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
};
use tokio_util::{
    sync::CancellationToken,
    task::{TaskTracker, task_tracker::TaskTrackerToken},
};
use tracing::Instrument as _;

use demi_command_package_browser_chrome::driver::{
    installation::Chrome,
    numbers::TabNumbers,
    operation::{BrowserError, Result},
    output,
};
use demi_command_package_browser_chrome::page::actions::TabResult;
use demi_command_package_browser_chrome::tabs::{
    environment::{BrowserEnvironment, LaunchOptions, with_browser},
    registry::Closed,
};
use demi_command_package_browser_chrome::tabs::registry::Snapshot;
use demi_command_package_browser_protocol::OperationError;
use demi_command_protocol::{
    ColorScheme, CommandError, CommandLocale, Completion, ConversationRequest, ConversationStatus,
};
use demi_command_sdk::{ConversationContext, InvocationContext, Messages, Numbers, ResidentError};
use demi_command_package_browser_protocol::browser::{BrowserMessage, TabsChange};

use demi_shared_openh264::Codec;
use demi_shared_recording::{Answer, Library, Live, Reading};

use crate::protocol::{
    self, ActionProgress, RecordAction, RecordInput, RecordTarget, BrowserCreatedBy, BrowserErrorCode, BrowserFailure, BrowserOperation, CapabilitiesResult,
    ContentReadResult, DEFAULT_NODES, FailureDocument, ImageMime, InstallResult,
    ScreenshotResult, ShowResult, TabCloseResult, TabListResult, TabNewResult,
};
use demi_command_package_browser_chrome::traced::SpawnTraced as _;

/// Requests waiting for an owner; a full queue holds back their senders.
const REQUESTS: usize = 64;
/// Lines of an install's progress not printed yet; a full queue holds back
/// the install until the output takes them.
const PROGRESS_LINES: usize = 16;
/// The most bytes of standard output one write carries.
const OUTPUT_CHUNK: usize = 256 * 1024;

enum CommandOutput {
    Json(serde_json::Value),
    /// A recording's video without `--output`: its MP4's file, which is
    /// removed once read, and what it holds for stderr.
    Video { file: std::path::PathBuf, about: String },
    /// A screenshot without `--output`: its PNG's bytes for stdout, and what it captured for
    /// stderr.
    Image { captured: String, png: Vec<u8> },
}

type Readiness = Option<std::result::Result<Started, EnvironmentFailure>>;

/// A browser that started: its environment and its live view.
#[derive(Clone)]
struct Started {
    environment: BrowserEnvironment,
    live: Arc<demi_command_package_browser_chrome::live::hub::Hub>,
}

#[derive(Clone)]
enum EnvironmentFailure {
    /// The Host lacks Chrome or a library it loads, which the message says
    /// how to install.
    Installation(String),
    Unavailable(String),
    Lost(String),
}

/// A conversation browser's one state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lifecycle {
    Absent,
    Starting,
    Ready,
    Closing,
}

/// One conversation's browser, as its invocations reach it.
struct ConversationBrowser {
    requests: mpsc::Sender<Request>,
    lifecycle: watch::Receiver<Lifecycle>,
    /// Cancelled when the conversation's release arrives.
    cancellation: CancellationToken,
    /// The invocations admitted into the conversation.
    commands: TaskTracker,
    /// The conversation's tab numbers and its tabs closed on purpose, which
    /// outlast each browser.
    numbers: TabNumbers,
    /// The conversation's recordings, which outlast each browser, and the
    /// tabs the running ones record.
    recordings: Recordings,
}

/// The conversation's recordings on this Host.
struct Recordings {
    library: Arc<Library>,
    /// The recording each tab has, held while one starts so that a tab has
    /// one at a time.
    running: tokio::sync::Mutex<HashMap<String, Live>>,
}

/// What starts a conversation's browser: the starting caller's locale and
/// color scheme.
#[derive(Clone)]
pub(crate) struct Starting {
    pub(crate) locale: CommandLocale,
    pub(crate) color_scheme: ColorScheme,
}

enum Request {
    /// The browser, started first when `start` says who starts it.
    Environment {
        start: Option<Starting>,
        reply: oneshot::Sender<Result<Option<watch::Receiver<Readiness>>>>,
    },
    /// Retires `environment` if it still runs; answered once it is gone.
    Retire {
        environment: BrowserEnvironment,
        reply: oneshot::Sender<Result<()>>,
    },
    /// The conversation's release: retires the browser, and starts none
    /// again.
    Release { reply: oneshot::Sender<Result<()>> },
}

impl ConversationBrowser {
    /// The browser of `conversation`, whose tabs take their numbers from
    /// `numbers` across every browser it starts, and whose changes `messages`
    /// tells the plugin of while its owner runs.
    fn start(
        tasks: &TaskTracker,
        chrome: Chrome,
        conversation: String,
        numbers: TabNumbers,
        messages: watch::Receiver<Option<Messages>>,
        library: Arc<Library>,
    ) -> Arc<Self> {
        let (requests, receiver) = mpsc::channel(REQUESTS);
        let (published, lifecycle) = watch::channel(Lifecycle::Absent);
        let cancellation = CancellationToken::new();
        let owner = Owner {
            state: State::Absent,
            chrome,
            numbers: numbers.clone(),
            published,
            released: cancellation.clone(),
            tasks: tasks.clone(),
        };
        let owned = CancellationToken::new();
        // The owner's events name the conversation, which it does not hold.
        let span = tracing::info_span!("conversation_browser", conversation = %conversation);
        let announced = owned.clone();
        tasks.spawn_traced(
            async move {
                owner.run(receiver).await;
                owned.cancel();
            }
            .instrument(span),
        );
        let browser = Arc::new(Self {
            requests,
            lifecycle,
            cancellation,
            commands: TaskTracker::new(),
            numbers,
            recordings: Recordings {
                library,
                running: tokio::sync::Mutex::default(),
            },
        });
        tasks.spawn_traced(announce(conversation, browser.clone(), messages, announced));
        browser
    }

    /// The conversation's tabs as the user's `browser.tab.list` lists them, all
    /// of them, at the list's number, and the latest tabs closed on purpose;
    /// no tab while no browser runs. It never starts a browser.
    async fn report(&self, cancel: &CancellationToken) -> Result<(u64, Vec<protocol::BrowserTab>, Vec<protocol::TabId>)> {
        let Some(environment) = self.environment(None, cancel).await? else {
            return Ok((self.numbers.list_number(), Vec::new(), self.numbers.closed()));
        };
        let (list, listing) = environment.numbered_unbounded(cancel).await?;
        Ok((list, tab_rows(&listing, 0, usize::MAX), self.numbers.closed()))
    }

    /// Admits an invocation, unless the conversation's release has begun.
    fn admit(&self) -> Option<TaskTrackerToken> {
        // Counted before the release is checked: a release cancels before
        // it waits for commands, so it waits for every command admitted.
        let command = self.commands.token();
        (!self.cancellation.is_cancelled()).then_some(command)
    }

    async fn ask<T>(
        &self,
        request: impl FnOnce(oneshot::Sender<Result<T>>) -> Request,
        cancel: &CancellationToken,
    ) -> Result<T> {
        let (reply, answer) = oneshot::channel();
        let asked = async {
            self.requests
                .send(request(reply))
                .await
                .map_err(|_| BrowserError::Closed)?;
            answer.await.map_err(|_| BrowserError::Closed)?
        };
        tokio::select! {
            _ = cancel.cancelled() => Err(BrowserError::Cancelled),
            answer = asked => answer,
        }
    }

    /// The conversation's browser. The first open starts it in the starting
    /// caller's locale, and concurrent opens join that start.
    pub async fn environment(
        &self,
        start: Option<&Starting>,
        cancel: &CancellationToken,
    ) -> Result<Option<BrowserEnvironment>> {
        let started = self.started(start, cancel).await?;
        Ok(started.map(|started| started.environment))
    }

    /// The conversation's browser with its live view, started as
    /// [`Self::environment`] starts it.
    async fn started(
        &self,
        start: Option<&Starting>,
        cancel: &CancellationToken,
    ) -> Result<Option<Started>> {
        let start = start.cloned();
        let answer = self
            .ask(|reply| Request::Environment { start, reply }, cancel)
            .await?;
        let Some(mut ready) = answer else {
            return Ok(None);
        };
        loop {
            if let Some(result) = ready.borrow_and_update().clone() {
                return result.map(Some).map_err(|failure| match failure {
                    EnvironmentFailure::Installation(message) => BrowserError::Installation(message),
                    EnvironmentFailure::Unavailable(message) => BrowserError::Unavailable(message),
                    EnvironmentFailure::Lost(message) => BrowserError::Connection(message),
                });
            }
            tokio::select! {
                _ = cancel.cancelled() => return Err(BrowserError::Cancelled),
                changed = ready.changed() => changed.map_err(|_| BrowserError::Closed)?,
            }
        }
    }

    /// Retires `environment` if it still runs, and answers once Chrome and
    /// its profile are gone. A retirement once asked for finishes, so the
    /// caller's cancellation does not reach it.
    pub async fn retire(&self, environment: &BrowserEnvironment) -> Result<()> {
        let environment = environment.clone();
        self.ask(
            |reply| Request::Retire { environment, reply },
            &CancellationToken::new(),
        )
        .await
    }

    /// Whether the conversation holds a browser: starting, running or
    /// retiring.
    fn holds(&self) -> bool {
        *self.lifecycle.borrow() != Lifecycle::Absent
    }

    /// The conversation's release: its commands are cancelled and joined,
    /// then its browser retires.
    async fn release(&self) -> Result<()> {
        self.cancellation.cancel();
        self.commands.close();
        // Once fenced, the release finishes even if its requester goes away.
        self.commands.wait().await;
        self.ask(
            |reply| Request::Release { reply },
            &CancellationToken::new(),
        )
        .await
    }

    /// Inspect retained debug owners without starting or querying Chrome after a timeout.
    async fn debugging_callers(&self, tab: &str, caller: Option<u64>) -> Vec<u64> {
        match self.environment(None, &CancellationToken::new()).await {
            Ok(Some(environment)) => environment.debugging_callers(tab, caller),
            Ok(None) | Err(_) => Vec::new(),
        }
    }
}

/// What a conversation browser's owner keeps.
struct Owner {
    state: State,
    chrome: Chrome,
    /// The conversation's tab numbers, which outlast each browser.
    numbers: TabNumbers,
    published: watch::Sender<Lifecycle>,
    /// Cancelled when the conversation's release begins: no browser starts
    /// after it.
    released: CancellationToken,
    tasks: TaskTracker,
}

enum State {
    Absent,
    Running(Running),
    /// Released, or its browser's cleanup failed: no browser starts again.
    Ended {
        cleanup: Option<String>,
    },
}

struct Running {
    ready: watch::Receiver<Readiness>,
    stop: CancellationToken,
    task: JoinHandle<Result<()>>,
}

impl Running {
    /// The environment, once it started.
    fn environment(&self) -> Option<BrowserEnvironment> {
        self.ready
            .borrow()
            .as_ref()
            .and_then(|ready| ready.as_ref().ok())
            .map(|started| started.environment.clone())
    }

    /// Whether this browser is on its way out: stopped, lost, emptied, or
    /// failed to start.
    fn leaving(&self) -> bool {
        self.stop.is_cancelled()
            || match self.ready.borrow().as_ref() {
                None => false,
                Some(Err(_)) => true,
                Some(Ok(started)) => {
                    started.environment.ended().is_cancelled()
                        || started.environment.emptied().is_cancelled()
                }
            }
    }
}

enum Event {
    Request(Option<Request>),
    Finished(std::result::Result<Result<()>, tokio::task::JoinError>),
    Changed,
}

impl Owner {
    async fn run(mut self, mut requests: mpsc::Receiver<Request>) {
        loop {
            self.publish();
            let event = match &mut self.state {
                State::Running(running) => {
                    // The end or emptying of a ready browser changes the
                    // lifecycle without a request; a token already cancelled
                    // is not waited for again.
                    let leaving = running
                        .environment()
                        .map(|environment| {
                            (environment.ended().clone(), environment.emptied().clone())
                        })
                        .filter(|(ended, emptied)| {
                            !ended.is_cancelled() || !emptied.is_cancelled()
                        });
                    tokio::select! {
                        request = requests.recv() => Event::Request(request),
                        finished = &mut running.task => Event::Finished(finished),
                        Ok(()) = running.ready.changed() => Event::Changed,
                        () = async {
                            match &leaving {
                                Some((ended, emptied)) => {
                                    tokio::select! {
                                        _ = ended.cancelled(), if !ended.is_cancelled() => {}
                                        _ = emptied.cancelled(), if !emptied.is_cancelled() => {}
                                        // Both ended since they were looked at.
                                        else => std::future::pending().await,
                                    }
                                }
                                None => std::future::pending().await,
                            }
                        } => Event::Changed,
                    }
                }
                State::Absent | State::Ended { .. } => Event::Request(requests.recv().await),
            };
            match event {
                Event::Request(Some(request)) => self.handle(request).await,
                // Every handle is gone: whatever still runs retires.
                Event::Request(None) => break,
                Event::Finished(finished) => {
                    if let Err(error) = self.finished(finished) {
                        tracing::warn!(
                            event = "browser_cleanup.failed",
                            %error,
                            "a conversation's browser did not finish its cleanup"
                        );
                    }
                }
                Event::Changed => {}
            }
        }
        if let Err(error) = self.retire().await {
            tracing::warn!(
                event = "browser_cleanup.failed",
                %error,
                "a conversation's browser did not finish its cleanup"
            );
        }
    }

    fn publish(&self) {
        let lifecycle = match &self.state {
            State::Absent | State::Ended { .. } => Lifecycle::Absent,
            State::Running(running) if running.leaving() => Lifecycle::Closing,
            State::Running(running) if running.ready.borrow().is_none() => Lifecycle::Starting,
            State::Running(_) => Lifecycle::Ready,
        };
        let ended = lifecycle == Lifecycle::Absent && *self.published.borrow() != Lifecycle::Absent;
        self.published.send_if_modified(|published| {
            let changed = *published != lifecycle;
            *published = lifecycle;
            changed
        });
        if ended {
            self.numbers.browser_ended();
        }
    }

    async fn handle(&mut self, request: Request) {
        match request {
            Request::Environment { start, reply } => {
                let answer = self.environment(start).await;
                let _gone = reply.send(answer);
            }
            Request::Retire { environment, reply } => {
                let runs = matches!(
                    &self.state,
                    State::Running(running)
                        if running.environment().is_some_and(|current| current.same(&environment))
                );
                let answer = if runs {
                    self.retire().await
                } else {
                    self.cleanup_failure()
                };
                let _gone = reply.send(answer);
            }
            Request::Release { reply } => {
                let answer = self.retire().await;
                if matches!(self.state, State::Absent) {
                    self.state = State::Ended { cleanup: None };
                }
                self.publish();
                let _gone = reply.send(answer);
            }
        }
    }

    async fn environment(
        &mut self,
        start: Option<Starting>,
    ) -> Result<Option<watch::Receiver<Readiness>>> {
        if let State::Running(running) = &self.state
            && (running.task.is_finished() || (start.is_some() && running.leaving()))
        {
            // A browser that ended, or one on its way out when an open
            // starts the next, is joined first: a failed cleanup must not
            // leave an owner unresolved under a new browser.
            self.retire().await?;
        }
        match &self.state {
            State::Ended { .. } => Err(BrowserError::Closed),
            State::Running(running) => match running.environment() {
                // Retiring after its last tab closed: no tab remains.
                Some(environment)
                    if environment.emptied().is_cancelled()
                        || (environment.ended().is_cancelled()
                            && environment.failure().is_none()) =>
                {
                    Ok(None)
                }
                Some(environment) if environment.ended().is_cancelled() => Err(
                    BrowserError::Connection(environment.failure().unwrap_or_default()),
                ),
                // Starting, ready, or failed to start, which the readiness says.
                _ => Ok(Some(running.ready.clone())),
            },
            State::Absent => match start {
                // A command admitted before the release began still asks.
                Some(_) if self.released.is_cancelled() => Err(BrowserError::Cancelled),
                Some(starting) => {
                    let ready = self.start(starting);
                    Ok(Some(ready))
                }
                None => Ok(None),
            },
        }
    }

    /// Starts the conversation's Chrome in the starting caller's locale and
    /// color scheme,
    /// from the installation the Host holds.
    fn start(&mut self, starting: Starting) -> watch::Receiver<Readiness> {
        let stop = CancellationToken::new();
        let (publish, ready) = watch::channel(None);
        let chrome = self.chrome.clone();
        let numbers = self.numbers.clone();
        let owner_stop = stop.clone();
        let task = self.tasks.spawn_traced(async move {
            let publisher = publish.clone();
            let retire = owner_stop.clone();
            let result = async {
                let installation = chrome.installation().await?;
                with_browser(
                    LaunchOptions::pinned(installation, starting.locale, starting.color_scheme)?,
                    numbers,
                    owner_stop.clone(),
                    move |environment| async move {
                        let live =
                            Arc::new(demi_command_package_browser_chrome::live::hub::Hub::start(
                                &environment,
                            ));
                        publisher.send_replace(Some(Ok(Started {
                            environment: environment.clone(),
                            live,
                        })));
                        // The browser runs until it is retired or its last tab closed.
                        tokio::select! {
                            _ = retire.cancelled() => {}
                            _ = environment.emptied().cancelled() => {}
                        }
                        Ok(())
                    },
                )
                .await
            }
            .await;
            if let Err(error) = &result {
                let message = error.to_string();
                let failure = if matches!(error, BrowserError::Installation(_)) {
                    EnvironmentFailure::Installation(message)
                } else if publish.borrow().as_ref().is_some_and(|ready| ready.is_ok()) {
                    EnvironmentFailure::Lost(message)
                } else {
                    EnvironmentFailure::Unavailable(message)
                };
                publish.send_replace(Some(Err(failure)));
            }
            result
        });
        self.state = State::Running(Running {
            ready: ready.clone(),
            stop,
            task,
        });
        self.publish();
        ready
    }

    /// Stops the running browser and joins its retirement.
    async fn retire(&mut self) -> Result<()> {
        let State::Running(running) = &mut self.state else {
            return self.cleanup_failure();
        };
        running.stop.cancel();
        self.publish();
        let State::Running(running) = &mut self.state else {
            unreachable!("the browser still runs");
        };
        let finished = (&mut running.task).await;
        self.finished(finished)
    }

    /// Takes the browser task's end. Only a failed cleanup keeps the
    /// conversation from starting another browser; startup and transport
    /// failures reached their callers through the readiness.
    fn finished(
        &mut self,
        finished: std::result::Result<Result<()>, tokio::task::JoinError>,
    ) -> Result<()> {
        let failure = match finished {
            Ok(Err(
                error @ (BrowserError::Cleanup { .. } | BrowserError::ProfileRetained { .. }),
            )) => Some(error),
            Ok(Ok(()) | Err(_)) => None,
            Err(error) => Some(BrowserError::Task(error)),
        };
        match failure {
            Some(error) => {
                self.state = State::Ended {
                    cleanup: Some(error.to_string()),
                };
                Err(error)
            }
            None => {
                self.state = State::Absent;
                Ok(())
            }
        }
    }

    /// The failed cleanup that ended the conversation's browsers, if one did.
    fn cleanup_failure(&self) -> Result<()> {
        match &self.state {
            State::Ended {
                cleanup: Some(cleanup),
            } => Err(BrowserError::Unavailable(format!(
                "the browser's cleanup failed: {cleanup}"
            ))),
            _ => Ok(()),
        }
    }
}

/// Prints a `line` of how `install` goes in the output of its invocation,
/// unless it answers in JSON, whose output is the one document.
/// Where an install's progress lines go: `demi browser install`'s own output,
/// or the standard error of an operation that installs first.
#[derive(Clone, Copy)]
enum Lines {
    Stdout,
    Stderr,
}

async fn print_progress(context: &InvocationContext, line: String, lines: Lines) -> Result<()> {
    if context.request.json == Some(true) {
        return Ok(());
    }
    let line = Bytes::from(line);
    let printed = match lines {
        Lines::Stdout => context.output.stdout(line).await,
        Lines::Stderr => context.output.stderr(line).await,
    };
    // An output that takes nothing more is an invocation that ended.
    printed.map_err(|_| BrowserError::Cancelled)
}

/// What the tab's page offers, of every command family: the page's own families
/// and the CDP ones, which only the program has together.
async fn capabilities(
    tab: &demi_command_package_browser_chrome::tabs::tab::BrowserTab,
    cancel: &CancellationToken,
    deadline: tokio::time::Instant,
) -> Result<serde_json::Value> {
    let operation = tab.operation(cancel, deadline);
    let failure = |error| operation.failure(error, tab.id().as_str(), None);
    let _session = tab
        .state()
        .gate
        .try_checkout()
        .ok_or_else(|| failure(BrowserError::Busy))?;
    if tab.state().dialog.is_open() {
        return Err(failure(BrowserError::DialogBlocked));
    }
    let mut capabilities = operation
        .run(demi_command_package_browser_chrome::page::actions::capabilities(tab.page()))
        .await
        .map_err(&failure)?;
    let families = operation
        .run(demi_command_package_browser_chrome::cdp::capabilities(
            tab.page(),
        ))
        .await
        .map_err(&failure)?;
    capabilities.extend(families);
    output::value(CapabilitiesResult { capabilities })
}

/// A viewer reaches the conversation's browser, and learns that one started
/// or ended, through its lifecycle.
impl demi_command_package_browser_chrome::live::viewer::ViewedBrowser for ConversationBrowser {
    async fn running(
        &self,
        cancel: &CancellationToken,
    ) -> Result<
        Option<(
            BrowserEnvironment,
            Arc<demi_command_package_browser_chrome::live::hub::Hub>,
        )>,
    > {
        let started = self.started(None, cancel).await?;
        Ok(started.map(|started| (started.environment, started.live)))
    }

    fn changed(&self) -> impl std::future::Future<Output = ()> + Send + 'static {
        let mut lifecycle = self.lifecycle.clone();
        lifecycle.mark_unchanged();
        async move {
            // A lifecycle that ended with its browser's owner changed too.
            let _ended = lifecycle.changed().await;
        }
    }

    fn released(&self) -> &CancellationToken {
        &self.cancellation
    }

    fn list_number(&self) -> u64 {
        self.numbers.list_number()
    }
}

/// Requests to the conversations' owner.
enum Find {
    /// The conversation's browser, made on first use.
    Admit {
        conversation: String,
        reply: oneshot::Sender<Arc<ConversationBrowser>>,
    },
    /// The conversation's browser, if it has one.
    Lookup {
        conversation: String,
        reply: oneshot::Sender<Option<Arc<ConversationBrowser>>>,
    },
    /// The conversations that hold a browser.
    Status { reply: oneshot::Sender<Vec<String>> },
    /// The conversation's release finished: a later invocation makes a new
    /// browser.
    Forget {
        conversation: String,
        browser: Arc<ConversationBrowser>,
    },
    /// The resident shuts down: every browser, and no more.
    Close {
        reply: oneshot::Sender<Vec<Arc<ConversationBrowser>>>,
    },
}

/// Tells `plugin-browser` the whole of what `browser.tab.list` lists of
/// `conversation` each time it changes, once for each burst of changes: a
/// change made while a message is on its way is told by the next one. It
/// tells the list once more whenever the resident is given a sender again, and
/// when it starts, so a plugin that missed a report has the latest.
/// Ends once the conversation browser's owner ended at `owned`, after it
/// told the change its end made.
async fn announce(
    conversation: String,
    browser: Arc<ConversationBrowser>,
    mut messages: watch::Receiver<Option<Messages>>,
    owned: CancellationToken,
) {
    let mut changes = browser.numbers.watch_changes();
    loop {
        // The owner's end is told by one last report, of no tabs.
        let last = owned.is_cancelled();
        changes.borrow_and_update();
        // The resident is given its sender before its first call; one
        // driven without it, as a test's is, tells nobody.
        let sender = messages.borrow_and_update().clone();
        if let Some(sender) = sender {
            let report = match browser.report(&owned).await {
                Ok(report) => Some(report),
                // The owner ended while it listed: no browser, no tabs.
                Err(BrowserError::Cancelled | BrowserError::Closed) => {
                    Some((browser.numbers.list_number(), Vec::new(), browser.numbers.closed()))
                }
                Err(error) => {
                    // The next change tells the list again.
                    tracing::warn!(event = "browser_tabs.list_failed", %conversation, %error, "the conversation's tabs were not listed for the plugin");
                    None
                }
            };
            if let Some((revision, tabs, closed)) = report {
                let message = serde_json::to_value(BrowserMessage {
                    tabs: TabsChange::Tabs { revision, tabs, closed },
                })
                .expect("the browser's message is JSON");
                if let Err(error) = sender.send(&conversation, message).await {
                    // The next change, or the next sender, tells it again.
                    tracing::warn!(event = "browser_tabs.announce_failed", %conversation, %error, "the plugin was not told of the tabs");
                }
            }
        }
        if last {
            return;
        }
        tokio::select! {
            changed = changes.changed() => if changed.is_err() { return },
            sender = messages.changed() => if sender.is_err() { return },
            () = owned.cancelled() => {},
        }
    }
}

/// Each tab's record as `browser.tab.list` lists it, not the tab itself: from
/// `offset`, at most `limit` of them.
fn tab_rows(listing: &Snapshot, offset: usize, limit: usize) -> Vec<protocol::BrowserTab> {
    listing
        .tabs
        .iter()
        .skip(offset)
        .take(limit)
        .map(|listed| {
            let (title, url) = listed.shown();
            protocol::BrowserTab {
                id: listed.tab.id().clone(),
                title,
                url,
                created_by: listed.tab.created_by().clone(),
                loading: listed.tab.loading(),
                can_go_back: listed.tab.history().back,
                can_go_forward: listed.tab.history().forward,
                shows: listed.tab.shows(),
                favicon: listed.tab.favicon(),
            }
        })
        .collect()
}

/// The resident's browsers, one per conversation.
pub(crate) struct Conversations {
    requests: mpsc::Sender<Find>,
    /// The owner, every conversation browser's owner and their Chromes.
    tasks: TaskTracker,
    /// Where the conversations' tab numbers come from, once the resident has
    /// its numbers source.
    numbers: watch::Sender<Option<Numbers>>,
    /// Where the resident tells the plugin of the conversations' tab changes,
    /// once it has its message sender.
    messages: watch::Sender<Option<Messages>>,
    /// The Host's Chrome, which `install` installs for every conversation.
    chrome: Chrome,
}

/// Makes a conversation's library of recordings.
#[derive(Clone)]
struct Libraries {
    home: Option<std::path::PathBuf>,
    codec: Arc<dyn Codec>,
}

impl Libraries {
    fn of(&self, conversation: &str) -> Arc<Library> {
        Arc::new(Library::new(
            demi_command_protocol::recordings_directory(self.home.as_deref(), conversation),
            self.codec.clone(),
        ))
    }
}

/// The conversations' owner: it maps each conversation to its browser. It
/// answers every request at once, so the runner's `status` never waits for
/// a browser's start or retirement.
async fn serve(
    mut requests: mpsc::Receiver<Find>,
    tasks: TaskTracker,
    chrome: Chrome,
    numbers: watch::Receiver<Option<Numbers>>,
    messages: watch::Receiver<Option<Messages>>,
    libraries: Libraries,
) {
    let mut browsers: HashMap<String, Arc<ConversationBrowser>> = HashMap::new();
    // A requester that left needs no answer.
    while let Some(request) = requests.recv().await {
        match request {
            Find::Admit {
                conversation,
                reply,
            } => {
                let browser = browsers
                    .entry(conversation.clone())
                    .or_insert_with(|| {
                        let tabs =
                            TabNumbers::from_source(numbers.borrow().clone(), conversation.clone());
                        let library = libraries.of(&conversation);
                        ConversationBrowser::start(&tasks, chrome.clone(), conversation, tabs, messages.clone(), library)
                    })
                    .clone();
                let _gone = reply.send(browser);
            }
            Find::Lookup {
                conversation,
                reply,
            } => {
                let _gone = reply.send(browsers.get(&conversation).cloned());
            }
            Find::Status { reply } => {
                let mut held: Vec<_> = browsers
                    .iter()
                    .filter(|(_, browser)| browser.holds())
                    .map(|(conversation, _)| conversation.clone())
                    .collect();
                held.sort();
                let _gone = reply.send(held);
            }
            Find::Forget {
                conversation,
                browser,
            } => {
                if browsers
                    .get(&conversation)
                    .is_some_and(|current| Arc::ptr_eq(current, &browser))
                {
                    browsers.remove(&conversation);
                }
            }
            Find::Close { reply } => {
                let _gone = reply.send(browsers.drain().map(|(_, browser)| browser).collect());
                break;
            }
        }
    }
}

impl Conversations {
    /// Starts the owner on the current Tokio runtime; the browsers start
    /// `chrome`.
    pub(crate) fn new(chrome: Chrome, home: Option<std::path::PathBuf>, codec: Arc<dyn Codec>) -> Self {
        let (requests, receiver) = mpsc::channel(REQUESTS);
        let tasks = TaskTracker::new();
        let (numbers, source) = watch::channel(None);
        let (messages, sender) = watch::channel(None);
        let libraries = Libraries { home, codec };
        tasks.spawn_traced(serve(receiver, tasks.clone(), chrome.clone(), source, sender, libraries));
        Self {
            requests,
            tasks,
            numbers,
            messages,
            chrome,
        }
    }

    /// Takes the resident's message sender: the conversations' tab changes
    /// go to the plugin through it.
    pub(crate) fn attach_messages(&self, messages: Messages) {
        self.messages.send_replace(Some(messages));
    }

    /// Takes the resident's numbers source: browsers made from now on number
    /// their tabs from it.
    pub(crate) fn attach_numbers(&self, numbers: Numbers) {
        self.numbers.send_replace(Some(numbers));
    }

    /// Asks the owner; none once the resident has shut down.
    async fn ask<T>(&self, request: impl FnOnce(oneshot::Sender<T>) -> Find) -> Option<T> {
        let (reply, answer) = oneshot::channel();
        self.requests.send(request(reply)).await.ok()?;
        answer.await.ok()
    }

    /// Runs one browser command, or reports why its input was refused.
    pub async fn invoke(
        &self,
        context: InvocationContext,
        operation: std::result::Result<BrowserOperation, OperationError>,
    ) -> std::result::Result<Completion, ResidentError> {
        let (browser, _command) = self.admit(&context).await?;
        let cancellation = context.cancellation.clone();
        let work = self.invoke_admitted(&browser, context, operation);
        tokio::pin!(work);
        tokio::select! {
            biased;
            _ = browser.cancellation.cancelled() => {
                // Cancel the invocation token, including stdin and blocked output,
                // and join its cleanup before dropping the admission token.
                cancellation.cancel();
                work.await
            }
            result = &mut work => result,
        }
    }

    /// Serves a live view of the conversation's browser. A view ends itself
    /// on release, so it can tell its page why.
    pub async fn live(
        &self,
        context: InvocationContext,
    ) -> std::result::Result<Completion, ResidentError> {
        let (browser, _command) = self.admit(&context).await?;
        demi_command_package_browser_chrome::live::viewer::serve(browser, context).await
    }

    /// Admits an invocation into its conversation's browser, made on first
    /// use. A browser whose release has begun admits nothing, and the
    /// release waits for every invocation it admitted.
    async fn admit(
        &self,
        context: &InvocationContext,
    ) -> std::result::Result<(Arc<ConversationBrowser>, TaskTrackerToken), ResidentError> {
        let conversation = context.request.context.conversation.clone();
        let browser = self
            .ask(|reply| Find::Admit {
                conversation,
                reply,
            })
            .await
            .ok_or(ResidentError::Cancelled)?;
        let command = browser.admit().ok_or(ResidentError::Cancelled)?;
        Ok((browser, command))
    }

    async fn invoke_admitted(
        &self,
        browser: &ConversationBrowser,
        mut context: InvocationContext,
        operation: std::result::Result<BrowserOperation, OperationError>,
    ) -> std::result::Result<Completion, ResidentError> {
        let json = context.request.json == Some(true);
        let began = std::time::Instant::now();
        // How long the call waited for its browser, when it needed one.
        let mut environment_ms = None;
        let result = async {
            let command =
                operation.map_err(|error| BrowserError::Configuration(error.to_string()))?;
            self.install_first(&command, &mut context).await?;
            let cancellation = context.cancellation.child_token();
            let _cancel_on_drop = cancellation.clone().drop_guard();
            let deadline = tokio::time::Instant::now() + command.timeout();
            let invocation = self.execute(
                browser,
                &mut context,
                &command,
                &cancellation,
                deadline,
                &mut environment_ms,
            );
            tokio::pin!(invocation);
            let produced = match tokio::time::timeout_at(deadline, invocation.as_mut()).await {
                Ok(result) => result,
                Err(_) => {
                    // Cancel the active phase, then join its bounded input/file cleanup.
                    // Dropping the invocation here could strand a pressed button.
                    cancellation.cancel();
                    match invocation.await {
                        Err(error)
                            if matches!(
                                error.code(),
                                BrowserErrorCode::Cancelled | BrowserErrorCode::Timeout
                            ) =>
                        {
                            Err(BrowserError::Action {
                                details: Box::new(error.details()),
                                source: Box::new(BrowserError::Timeout),
                            })
                        }
                        result => result,
                    }
                }
            }?;
            match produced {
                CommandOutput::Json(value) => Ok((output::render(&command, value, json)?, None)),
                CommandOutput::Image { captured, png } => Ok((png, Some(captured))),
                CommandOutput::Video { file, about } => {
                    let bytes = tokio::fs::read(&file).await;
                    let _removed = tokio::fs::remove_file(&file).await;
                    Ok((bytes.map_err(BrowserError::Io)?, Some(about)))
                }
            }
        }
        .await;
        let failed = result
            .as_ref()
            .err()
            // The code as the protocol writes it, such as `tab_not_found`.
            .and_then(|error: &BrowserError| serde_json::to_value(error.code()).ok())
            .and_then(|code| code.as_str().map(str::to_owned));
        tracing::info!(
            event = "browser_call.ended",
            operation = context.request.operation.as_str(),
            ms = began.elapsed().as_millis() as u64,
            environment_ms,
            error = failed.as_deref(),
            "a browser call ended"
        );
        match result {
            Ok((stdout, captured)) => {
                if let Some(captured) = captured {
                    context.output.stderr(Bytes::from(captured)).await?;
                }
                // A video is larger than a frame of the output.
                let stdout = Bytes::from(stdout);
                let mut at = 0;
                while at < stdout.len() {
                    let end = (at + OUTPUT_CHUNK).min(stdout.len());
                    context.output.stdout(stdout.slice(at..end)).await?;
                    at = end;
                }
                if stdout.is_empty() {
                    context.output.stdout(stdout).await?;
                }
                Ok(Completion {
                    exit_code: 0,
                    error: None,
                })
            }
            Err(BrowserError::Cancelled) => Err(ResidentError::Cancelled),
            Err(error) => {
                let code = error.code();
                let message = error.to_string();
                let mut details = error.details();
                details.action.get_or_insert(ActionProgress::NotStarted);
                if details.tab.is_none() {
                    details.tab = context
                        .request
                        .args
                        .get("tab")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned);
                }
                if code == BrowserErrorCode::Timeout
                    && let Some(tab) = details.tab.as_deref()
                {
                    let callers = browser
                        .debugging_callers(tab, context.request.context.caller.agent_number())
                        .await;
                    if !callers.is_empty() {
                        details.debugging_callers = Some(callers);
                    }
                }
                let exit_code = match code {
                    BrowserErrorCode::InvalidInput => 2,
                    BrowserErrorCode::Cancelled => 130,
                    _ => 1,
                };
                if json {
                    let document = serde_json::to_vec(&FailureDocument {
                        error: BrowserFailure {
                            code,
                            message,
                            details: Some(details),
                        },
                    })?;
                    context.output.stderr(Bytes::from(document)).await?;
                    return Ok(Completion {
                        exit_code,
                        error: None,
                    });
                }
                // The context first; the dispatcher writes the error as the
                // last line, after the command's path.
                let rendered = output::render_error(code, &message, &details);
                if !rendered.context.is_empty() {
                    context.output.stderr(Bytes::from(rendered.context)).await?;
                }
                Ok(Completion {
                    exit_code,
                    error: Some(CommandError {
                        code: code.to_string(),
                        message: rendered.error,
                    }),
                })
            }
        }
    }

    /// Installs the pinned Chrome, and on Linux the Chrome runtime, for
    /// `context`'s invocation, which prints a line in its own output as each
    /// tenth of a download arrives and as it is unpacked, unless it answers
    /// in JSON.
    async fn install(&self, context: &mut InvocationContext, lines: Lines) -> Result<InstallResult> {
        let (progress, mut reports) = mpsc::channel(PROGRESS_LINES);
        let installing = self.chrome.install(&context.request.invocation_id, progress);
        tokio::pin!(installing);
        let installed = loop {
            tokio::select! {
                biased;
                Some(report) = reports.recv() => print_progress(context, report, lines).await?,
                installed = &mut installing => break installed,
            }
        };
        // What the runner reported before it answered.
        while let Ok(report) = reports.try_recv() {
            print_progress(context, report, lines).await?;
        }
        installed
    }

    /// Installs Chrome for an operation that starts the conversation's
    /// browser on a Host that has none, before the operation's own time
    /// runs: the agent's first `tab new` works instead of failing, and a tab
    /// the user opens in the panel opens once Chrome is in place. Its lines
    /// go to standard error, as the operation's output is its own.
    async fn install_first(&self, command: &BrowserOperation, context: &mut InvocationContext) -> Result<()> {
        let starts = matches!(
            command,
            BrowserOperation::TabNew(_) | BrowserOperation::ContentFetch(_)
        );
        if !starts || self.chrome.installation().await.is_ok() {
            return Ok(());
        }
        self.install(context, Lines::Stderr).await.map(drop)
    }

    async fn execute(
        &self,
        browser: &ConversationBrowser,
        context: &mut InvocationContext,
        command: &BrowserOperation,
        cancellation: &CancellationToken,
        deadline: tokio::time::Instant,
        environment_ms: &mut Option<u64>,
    ) -> Result<CommandOutput> {
        if matches!(command, BrowserOperation::Install(_)) {
            // The Host's browser, not the conversation's: nothing starts.
            let installed = self.install(context, Lines::Stdout).await?;
            return Ok(CommandOutput::Json(output::value(installed)?));
        }
        // Only the user's page reads the tab list's number and its tabs closed
        // on purpose, and only it stops a load.
        let user = context.request.context.caller.agent_number().is_none();
        if matches!(command, BrowserOperation::Stop(_)) && !user {
            // The agent waits for its commands rather than stopping a load.
            return Err(BrowserError::Configuration(
                "stop is the user's: an agent's command waits for its page instead".into(),
            ));
        }
        // Reading a recording needs no browser: it outlasts its tab.
        if let BrowserOperation::Record(input) = command
            && demi_command_package_browser_protocol::browser::BrowserInput::tab(input).is_none()
        {
            return record(browser, context, input).await;
        }
        let starts = matches!(
            command,
            BrowserOperation::TabNew(_) | BrowserOperation::ContentFetch(_)
        );
        let starting = starts.then(|| Starting {
            locale: context.request.context.locale.clone(),
            color_scheme: context.request.context.color_scheme,
        });
        let waiting = std::time::Instant::now();
        let environment = browser.environment(starting.as_ref(), cancellation).await;
        *environment_ms = Some(waiting.elapsed().as_millis() as u64);
        let environment = environment?;
        let Some(environment) = environment else {
            if matches!(command, BrowserOperation::TabList(_)) {
                return Ok(CommandOutput::Json(output::value(TabListResult {
                    // No browser runs: the conversation's latest list number.
                    list: user.then(|| browser.numbers.list_number()),
                    tabs: Vec::new(),
                    truncated: false,
                    closed: user.then(|| browser.numbers.closed()),
                })?));
            }
            return Err(BrowserError::TabNotFound);
        };
        if matches!(command, BrowserOperation::TabNew(_)) {
            let created_by = if user {
                BrowserCreatedBy::User {}
            } else {
                BrowserCreatedBy::Agent {
                    number: demi_command_package_browser_chrome::driver::operation::agent(context)?,
                }
            };
            let tab = environment
                .new_tab(created_by, cancellation, deadline)
                .await?;
            // The user's new tab answers at once: the work panel shows its
            // loading. The agent's names its viewport when the tab reports it
            // in time; the tab is made either way.
            let viewport = if user {
                None
            } else {
                demi_command_package_browser_chrome::page::actions::metadata(
                    &tab,
                    cancellation,
                    deadline.saturating_duration_since(tokio::time::Instant::now()),
                )
                .await
                .ok()
                .map(|metadata| metadata.viewport)
            };
            return Ok(CommandOutput::Json(output::value(TabNewResult {
                tab: tab.id().clone(),
                viewport,
            })?));
        }
        if matches!(command, BrowserOperation::ContentFetch(_)) {
            let result = demi_command_package_browser_chrome::page::fetch::execute(
                context,
                &environment,
                None,
                command,
                cancellation,
                deadline,
            )
            .await;
            // The batch's tabs are closed; the browser retires with its last one.
            let retired = if environment.emptied().is_cancelled() {
                browser.retire(&environment).await
            } else {
                Ok(())
            };
            return demi_command_package_browser_chrome::driver::operation::after_cleanup(
                result, retired,
            )
            .map(CommandOutput::Json);
        }
        if let BrowserOperation::TabList(input) = command {
            let (list, listing) = environment.numbered(cancellation, command.timeout()).await?;
            let offset = input.offset.unwrap_or(0);
            let limit = input.limit.unwrap_or(DEFAULT_NODES);
            let rows = tab_rows(&listing, offset, limit);
            return Ok(CommandOutput::Json(output::value(TabListResult {
                list: user.then_some(list),
                tabs: rows,
                truncated: listing.tabs.len() > offset.saturating_add(limit),
                closed: user.then(|| browser.numbers.closed()),
            })?));
        }
        let id = command.tab().ok_or(BrowserError::TabNotFound)?;
        let tab = &environment.tab(id, cancellation, command.timeout()).await?;
        if matches!(command, BrowserOperation::Show(_)) {
            // Only the request is recorded: the user's panel shows the tab
            // once the job ends.
            tab.show();
            return Ok(CommandOutput::Json(output::value(ShowResult {
                tab: tab.id().clone(),
            })?));
        }
        if let BrowserOperation::Record(input) = command {
            return start_recording(browser, &environment, tab, input).await;
        }
        if matches!(command, BrowserOperation::Stop(_)) {
            let operation = tab.operation(cancellation, deadline);
            let stopped =
                demi_command_package_browser_chrome::tabs::navigation::stop(tab, &operation)
                    .await?;
            return Ok(CommandOutput::Json(output::value(stopped)?));
        }
        if user
            && matches!(
                command,
                BrowserOperation::Open(_)
                    | BrowserOperation::Reload(_)
                    | BrowserOperation::Back(_)
                    | BrowserOperation::Forward(_)
            )
        {
            let operation = tab.operation(cancellation, deadline);
            let result = demi_command_package_browser_chrome::tabs::navigation::steer(
                tab, command, &operation,
            )
            .await?;
            return Ok(CommandOutput::Json(output::value(result)?));
        }
        {
            let family: Option<futures_util::future::BoxFuture<'_, Result<serde_json::Value>>> =
                match command {
                    BrowserOperation::Upload(_) => Some(Box::pin(
                        demi_command_package_browser_chrome::page::upload::execute(
                            context,
                            &environment,
                            Some(tab),
                            command,
                            cancellation,
                            deadline,
                        ),
                    )),
                    BrowserOperation::Download(_) => Some(Box::pin(
                        demi_command_package_browser_chrome::page::download::execute(
                            context,
                            &environment,
                            Some(tab),
                            command,
                            cancellation,
                            deadline,
                        ),
                    )),
                    BrowserOperation::ClipboardRead(_) | BrowserOperation::ClipboardWrite(_) => {
                        Some(Box::pin(
                            demi_command_package_browser_chrome::page::clipboard::execute(
                                context,
                                &environment,
                                Some(tab),
                                command,
                                cancellation,
                                deadline,
                            ),
                        ))
                    }
                    BrowserOperation::CdpTargets(_)
                    | BrowserOperation::CdpDetach(_)
                    | BrowserOperation::CdpSend(_)
                    | BrowserOperation::CdpEvents(_) => Some(Box::pin(
                        demi_command_package_browser_chrome::cdp::commands::execute(
                            context,
                            &environment,
                            Some(tab),
                            command,
                            cancellation,
                            deadline,
                        ),
                    )),
                    BrowserOperation::AssetsList(_) | BrowserOperation::AssetsExport(_) => Some(
                        Box::pin(demi_command_package_browser_chrome::page::assets::execute(
                            context,
                            &environment,
                            Some(tab),
                            command,
                            cancellation,
                            deadline,
                        )),
                    ),
                    BrowserOperation::WebmcpList(_) | BrowserOperation::WebmcpCall(_) => Some(
                        Box::pin(demi_command_package_browser_chrome::cdp::webmcp::execute(
                            context,
                            &environment,
                            Some(tab),
                            command,
                            cancellation,
                            deadline,
                        )),
                    ),
                    _ => None,
                };
            if let Some(family) = family {
                return family.await.map(CommandOutput::Json);
            }
        }
        if let BrowserOperation::Screenshot(input) = command {
            if input.output.is_none() && context.request.json == Some(true) {
                return Err(BrowserError::Configuration(
                    "screenshot --json requires --output".into(),
                ));
            }
            if let Some(output) = &input.output {
                demi_command_package_browser_chrome::driver::output::preflight(
                    &context.request.cwd,
                    output,
                    input.overwrite == Some(true),
                )
                .await?;
            }
            let bytes = demi_command_package_browser_chrome::page::screenshot::capture(
                tab,
                input.full_page == Some(true),
                input.clip.as_deref(),
                cancellation,
                command.timeout(),
            )
            .await?;
            let decoded = png::Decoder::new(std::io::Cursor::new(&bytes))
                .read_info()
                .map_err(|error| BrowserError::InvalidResult(error.to_string()))?;
            let width = decoded.info().width;
            let height = decoded.info().height;
            drop(decoded);
            let metadata = demi_command_package_browser_chrome::page::actions::metadata(
                tab,
                cancellation,
                command.timeout(),
            )
            .await?;
            let Some(output) = &input.output else {
                let captured = demi_command_package_browser_chrome::driver::text::captured(
                    tab.id().as_str(),
                    width,
                    height,
                    &metadata.viewport,
                );
                return Ok(CommandOutput::Image {
                    captured,
                    png: bytes,
                });
            };
            let path = demi_command_package_browser_chrome::driver::output::save_with_overwrite(
                &context.request.cwd,
                output,
                bytes,
                input.overwrite == Some(true),
                cancellation,
                deadline,
            )
            .await?;
            return Ok(CommandOutput::Json(output::value(ScreenshotResult {
                path,
                mime_type: ImageMime::Png,
                width,
                height,
                viewport: metadata.viewport,
            })?));
        }
        if matches!(command, BrowserOperation::TabClose(_)) {
            // Closing the last tab stops Chrome and removes its profile
            // before the close is acknowledged.
            let closed = tab.close_request(cancellation, command.timeout()).await?;
            if matches!(closed, Closed::Environment) || environment.emptied().is_cancelled() {
                browser.retire(&environment).await?;
            }
            return Ok(CommandOutput::Json(output::value(TabCloseResult {
                closed: tab.id().clone(),
            })?));
        }
        if matches!(command, BrowserOperation::Capabilities(_)) {
            return capabilities(tab, cancellation, deadline)
                .await
                .map(CommandOutput::Json);
        }
        if let BrowserOperation::Probe(input) = command
            && let Some(file) = &input.output
        {
            output::preflight(&context.request.cwd, file, input.overwrite == Some(true)).await?;
            let mut session = tab.state().gate.try_checkout().ok_or(BrowserError::Busy)?;
            let TabResult::Probe(mut result) =
                demi_command_package_browser_chrome::page::actions::command_admitted(
                    tab,
                    command,
                    cancellation,
                    deadline,
                    &mut session.references,
                )
                .await?
            else {
                return Err(BrowserError::InvalidResult(
                    "probe returned another result".into(),
                ));
            };
            let bytes = tab
                .operation(cancellation, deadline)
                .run(demi_command_package_browser_chrome::page::screenshot::bytes(tab, false, None))
                .await?;
            // Decoding and encoding a screenshot takes milliseconds of CPU.
            let outlined = result.clone();
            let bytes = tokio::task::spawn_blocking(move || {
                demi_command_package_browser_chrome::page::probe::annotate(bytes, &outlined)
            })
            .await??;
            result.path = Some(
                output::save_with_overwrite(
                    &context.request.cwd,
                    file,
                    bytes,
                    input.overwrite == Some(true),
                    cancellation,
                    deadline,
                )
                .await?,
            );
            return Ok(CommandOutput::Json(output::value(result)?));
        }
        let result = demi_command_package_browser_chrome::page::actions::command(
            tab,
            command,
            cancellation,
            deadline,
        )
        .await?;
        if let BrowserOperation::Open(input) = command
            && input.show == Some(true)
        {
            // The page opened: the user's panel shows the tab once the job
            // ends, as show does.
            tab.show();
        }
        if let BrowserOperation::ContentRead(input) = command
            && let Some(file) = &input.output
        {
            let TabResult::ContentRead(ContentReadResult::Inline {
                url,
                title,
                format,
                content,
                ..
            }) = result
            else {
                return Err(BrowserError::InvalidResult(
                    "content export did not return text".into(),
                ));
            };
            let path = output::save_with_overwrite(
                &context.request.cwd,
                file,
                content.into_bytes(),
                input.overwrite == Some(true),
                cancellation,
                deadline,
            )
            .await?;
            return Ok(CommandOutput::Json(output::value(
                ContentReadResult::File {
                    url,
                    title,
                    format,
                    path,
                },
            )?));
        }
        Ok(CommandOutput::Json(output::value(result)?))
    }

    /// Release only the trusted conversation selected by the parent resident port.
    pub async fn conversation(
        &self,
        context: ConversationContext,
    ) -> std::result::Result<Completion, ResidentError> {
        let output = match &context.request {
            ConversationRequest::Status {} => {
                let conversations = self
                    .ask(|reply| Find::Status { reply })
                    .await
                    .unwrap_or_default();
                serde_json::to_value(ConversationStatus { conversations })?
            }
            ConversationRequest::Release { conversation } => {
                let lookup = conversation.clone();
                let browser = self
                    .ask(|reply| Find::Lookup {
                        conversation: lookup,
                        reply,
                    })
                    .await
                    .flatten();
                // An unknown conversation is harmless to release.
                if let Some(browser) = browser {
                    // The browser's recordings end, complete, before it goes.
                    let released = browser.release().await;
                    let forget = Find::Forget {
                        conversation: conversation.clone(),
                        browser,
                    };
                    // A resident that shut down forgets everything anyway.
                    let _closed = self.requests.send(forget).await;
                    released.map_err(ResidentError::failed)?;
                }
                serde_json::json!({})
            }
        };
        context
            .output
            .stdout(Bytes::from(serde_json::to_vec(&output)?))
            .await?;
        Ok(Completion {
            exit_code: 0,
            error: None,
        })
    }

    /// Releases every conversation. Their browsers retire at the same time,
    /// so shutting down takes as long as the slowest.
    pub async fn close(&self) -> std::result::Result<(), ResidentError> {
        let browsers = self
            .ask(|reply| Find::Close { reply })
            .await
            .unwrap_or_default();
        let released =
            futures_util::future::join_all(browsers.iter().map(|browser| browser.release())).await;
        drop(browsers);
        // Each conversation browser's owner ends once its handles are gone.
        self.tasks.close();
        self.tasks.wait().await;
        match released.into_iter().find_map(std::result::Result::err) {
            Some(error) => Err(ResidentError::failed(error)),
            None => Ok(()),
        }
    }
}

/// `record` of a recording: stops it, lists its frames, or exports them. The
/// recording is the conversation's on this Host, and outlasts its tab and
/// its browser. `stop` also takes the tab the recording runs on, as a device's
/// does.
async fn record(
    browser: &ConversationBrowser,
    context: &InvocationContext,
    input: &RecordInput,
) -> Result<CommandOutput> {
    let id = match (&input.target, input.action) {
        (RecordTarget::Recording(id), _) => id.clone(),
        (RecordTarget::Tab(tab), RecordAction::Stop) => {
            let running = browser.recordings.running.lock().await;
            match running.get(tab.as_str()).filter(|live| !live.ended().is_cancelled()) {
                Some(live) => live.id().clone(),
                None => return Err(BrowserError::NoRecording(format!("{tab} is not recording"))),
            }
        }
        _ => {
            return Err(BrowserError::Configuration(format!(
                "{} works on a recording, such as demi browser record r1 {}; start returns its ID",
                input.action, input.action
            )));
        }
    };
    let id = &id;
    if input.action == RecordAction::Start {
        return Err(BrowserError::Configuration(
            "start records a tab, such as demi browser record t1 start".into(),
        ));
    }
    let output = match &input.output {
        Some(output) => Some(
            demi_command_sdk::paths::resolve(&context.request.cwd, output)
                .map_err(|error| BrowserError::Configuration(error.to_string()))?,
        ),
        None => None,
    };
    if input.mp4 == Some(true) && output.is_none() && context.request.json == Some(true) {
        return Err(BrowserError::Configuration(
            "record export --mp4 --json requires --output".into(),
        ));
    }
    let library = &browser.recordings.library;
    // Where the images go unless told: the caller's scratch directory, which
    // the job's environment names.
    let scratch = match context.request.env.get("DEMI_SCRATCH") {
        Some(scratch) => std::path::PathBuf::from(scratch),
        None => library.directory().parent().map_or_else(std::env::temp_dir, std::path::Path::to_path_buf),
    };
    let reading = Reading {
        command: "demi browser record",
        frames: input.frames.as_ref(),
        whole: input.whole == Some(true),
        region: input.region,
        mp4: input.mp4 == Some(true),
        output: output.as_deref(),
        overwrite: input.overwrite == Some(true),
        scratch: &scratch,
    };
    let answer = match input.action {
        RecordAction::Start => unreachable!("start was refused above"),
        RecordAction::Stop => library.stop(id).await?,
        RecordAction::Frames => library.frames(id, &reading).await?,
        RecordAction::Export => library.export(id, &reading).await?,
    };
    Ok(match answer {
        Answer::Result { result, .. } => CommandOutput::Json(output::value(result)?),
        Answer::Video { file, about } => CommandOutput::Video { file, about },
    })
}

/// `record <tab> start`: begins recording the tab, which ends by itself at the
/// duration, the limits, or when the tab closes.
async fn start_recording(
    browser: &ConversationBrowser,
    environment: &BrowserEnvironment,
    tab: &demi_command_package_browser_chrome::tabs::tab::BrowserTab,
    input: &RecordInput,
) -> Result<CommandOutput> {
    if input.action != RecordAction::Start {
        return Err(BrowserError::Configuration(format!(
            "{} works on a recording, such as demi browser record r1 {}; start returns its ID",
            input.action, input.action
        )));
    }
    let recordings = &browser.recordings;
    let mut running = recordings.running.lock().await;
    if running.get(tab.id().as_str()).is_some_and(|live| !live.ended().is_cancelled()) {
        return Err(BrowserError::Recording(tab.id().to_string()));
    }
    let live = recordings
        .library
        .begin(tab.id().as_str(), input.duration.map(|duration| duration.get()))
        .await?;
    running.insert(tab.id().to_string(), live.clone());
    drop(running);
    environment.recording(live.clone());
    demi_command_package_browser_chrome::live::record::start(
        tab.clone(),
        environment.captures().clone(),
        live.clone(),
        environment.tasks(),
    );
    let Answer::Result { result, .. } = demi_shared_recording::started("demi browser record", &live)? else {
        unreachable!("start answers a result")
    };
    Ok(CommandOutput::Json(output::value(result)?))
}
