//! The conversations' browsers (`browser.md` § Owners inside the service).
//! One owner task finds each invocation's conversation browser and forgets
//! it after the conversation's release. Each conversation browser is an
//! owner task of its own, with the one lifecycle of `browser.md` § Lifetime,
//! so one conversation's slow start or retirement never holds up another's
//! commands, and the runner's `status` never waits for either.

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
use demi_command_package_browser_protocol::OperationError;
use demi_command_protocol::{
    CommandLocale, Completion, ConversationRequest, ConversationStatus,
    MAX_MEDIUM_BYTES,
    StdoutTarget,
};
use demi_command_sdk::{ConversationContext, InvocationContext, Numbers, ServiceError};

use crate::protocol::{
    self, ActionProgress, BrowserErrorCode, BrowserFailure, BrowserOperation, CapabilitiesResult,
    CloseResult, ContentReadResult, DEFAULT_NODES, FailureDocument, ImageMime, InstallResult,
    OpenResult, ScreenshotResult, ShowResult, TabsResult,
};

/// Requests waiting for an owner; a full queue holds back their senders.
const REQUESTS: usize = 64;
/// Lines of an install's progress not printed yet; a full queue holds back
/// the install until the output takes them.
const PROGRESS_LINES: usize = 16;

enum CommandOutput {
    Json(serde_json::Value),
    /// A screenshot returned as a medium (`browser.md` § Images and large
    /// outputs), with the text that tells what it captured when the
    /// command's stdout is the job's output.
    Medium { text: Option<String>, png: Vec<u8> },
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
    /// how to install (`browser.md` § Browser distribution).
    Installation(String),
    Unavailable(String),
    Lost(String),
}

/// A conversation browser's one state (`browser.md` § Lifetime).
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
}

/// What starts a conversation's browser: the starting caller's locale.
#[derive(Clone)]
pub(crate) struct Starting {
    pub(crate) locale: CommandLocale,
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
    /// The browser of a conversation whose tabs take their numbers from
    /// `numbers`, across every browser it starts.
    fn start(tasks: &TaskTracker, chrome: Chrome, numbers: TabNumbers) -> Arc<Self> {
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
        tasks.spawn(owner.run(receiver));
        Arc::new(Self {
            requests,
            lifecycle,
            cancellation,
            commands: TaskTracker::new(),
            numbers,
        })
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
    /// retiring (`browser.md` § Conversation-scoped state).
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
                            "a conversation's browser did not finish its cleanup: {error}"
                        );
                    }
                }
                Event::Changed => {}
            }
        }
        if let Err(error) = self.retire().await {
            tracing::warn!("a conversation's browser did not finish its cleanup: {error}");
        }
    }

    fn publish(&self) {
        let lifecycle = match &self.state {
            State::Absent | State::Ended { .. } => Lifecycle::Absent,
            State::Running(running) if running.leaving() => Lifecycle::Closing,
            State::Running(running) if running.ready.borrow().is_none() => Lifecycle::Starting,
            State::Running(_) => Lifecycle::Ready,
        };
        self.published.send_if_modified(|published| {
            let changed = *published != lifecycle;
            *published = lifecycle;
            changed
        });
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

    /// Starts the conversation's Chrome in the starting caller's locale,
    /// from the installation the Host holds (`browser.md` § Browser
    /// distribution).
    fn start(&mut self, starting: Starting) -> watch::Receiver<Readiness> {
        let stop = CancellationToken::new();
        let (publish, ready) = watch::channel(None);
        let chrome = self.chrome.clone();
        let numbers = self.numbers.clone();
        let owner_stop = stop.clone();
        let task = self.tasks.spawn(async move {
            let publisher = publish.clone();
            let retire = owner_stop.clone();
            let result = async {
                let installation = chrome.installation().await?;
                with_browser(
                    LaunchOptions::pinned(installation, starting.locale)?,
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
async fn print_progress(context: &InvocationContext, line: String) -> Result<()> {
    if context.request.json == Some(true) {
        return Ok(());
    }
    // An output that takes nothing more is an invocation that ended.
    context
        .output
        .stdout(Bytes::from(line))
        .await
        .map_err(|_| BrowserError::Cancelled)
}

/// What the tab's page offers, of every command family (`browser.md` §
/// Capabilities and WebMCP): the page's own families and the CDP ones, which
/// only the program has together.
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
    /// The service shuts down: every browser, and no more.
    Close {
        reply: oneshot::Sender<Vec<Arc<ConversationBrowser>>>,
    },
}

/// The service's browsers, one per conversation.
pub(crate) struct Conversations {
    requests: mpsc::Sender<Find>,
    /// The owner, every conversation browser's owner and their Chromes.
    tasks: TaskTracker,
    /// Where the conversations' tab numbers come from, once the service has
    /// its numbers source.
    numbers: watch::Sender<Option<Numbers>>,
    /// The Host's Chrome, which `install` installs for every conversation.
    chrome: Chrome,
}

/// The conversations' owner: it maps each conversation to its browser. It
/// answers every request at once, so the runner's `status` never waits for
/// a browser's start or retirement.
async fn serve(
    mut requests: mpsc::Receiver<Find>,
    tasks: TaskTracker,
    chrome: Chrome,
    numbers: watch::Receiver<Option<Numbers>>,
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
                        let tabs = TabNumbers::from_source(numbers.borrow().clone(), conversation);
                        ConversationBrowser::start(&tasks, chrome.clone(), tabs)
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
    pub(crate) fn new(chrome: Chrome) -> Self {
        let (requests, receiver) = mpsc::channel(REQUESTS);
        let tasks = TaskTracker::new();
        let (numbers, source) = watch::channel(None);
        tasks.spawn(serve(receiver, tasks.clone(), chrome.clone(), source));
        Self {
            requests,
            tasks,
            numbers,
            chrome,
        }
    }

    /// Takes the service's numbers source: browsers made from now on number
    /// their tabs from it.
    pub(crate) fn attach_numbers(&self, numbers: Numbers) {
        self.numbers.send_replace(Some(numbers));
    }

    /// Asks the owner; none once the service has shut down.
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
    ) -> std::result::Result<Completion, ServiceError> {
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
    ) -> std::result::Result<Completion, ServiceError> {
        let (browser, _command) = self.admit(&context).await?;
        demi_command_package_browser_chrome::live::viewer::serve(browser, context).await
    }

    /// Admits an invocation into its conversation's browser, made on first
    /// use. A browser whose release has begun admits nothing, and the
    /// release waits for every invocation it admitted.
    async fn admit(
        &self,
        context: &InvocationContext,
    ) -> std::result::Result<(Arc<ConversationBrowser>, TaskTrackerToken), ServiceError> {
        let conversation = context.request.context.conversation.clone();
        let browser = self
            .ask(|reply| Find::Admit {
                conversation,
                reply,
            })
            .await
            .ok_or(ServiceError::Cancelled)?;
        let command = browser.admit().ok_or(ServiceError::Cancelled)?;
        Ok((browser, command))
    }

    async fn invoke_admitted(
        &self,
        browser: &ConversationBrowser,
        mut context: InvocationContext,
        operation: std::result::Result<BrowserOperation, OperationError>,
    ) -> std::result::Result<Completion, ServiceError> {
        let json = context.request.json == Some(true);
        // Only a job's command returns media; another call's PNG is its
        // stdout.
        let returns_media = context.request.stdout.is_some();
        let result = async {
            let command =
                operation.map_err(|error| BrowserError::Configuration(error.to_string()))?;
            let cancellation = context.cancellation.child_token();
            let _cancel_on_drop = cancellation.clone().drop_guard();
            let deadline = tokio::time::Instant::now() + command.timeout();
            let invocation = self.execute(browser, &mut context, &command, &cancellation, deadline);
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
                // A medium is at most 16 MiB (`runtime.md` § Bounds and cut
                // output); a file has no such bound.
                CommandOutput::Medium { png, .. }
                    if returns_media && png.len() as u64 > MAX_MEDIUM_BYTES =>
                {
                    Err(BrowserError::ScreenshotTooLarge(png.len()))
                }
                CommandOutput::Medium { text, png } => {
                    Ok((text.unwrap_or_default().into_bytes(), Some(png)))
                }
            }
        }
        .await;
        match result {
            Ok((text, medium)) => {
                context.output.stdout(Bytes::from(text)).await?;
                match medium {
                    Some(png) if returns_media => {
                        context.output.medium(Bytes::from(png)).await?;
                    }
                    // A call that is no job's command returns no media: the
                    // PNG is its stdout.
                    Some(png) => context.output.stdout(Bytes::from(png)).await?,
                    None => {}
                }
                Ok(Completion {
                    exit_code: 0,
                    error: None,
                })
            }
            Err(BrowserError::Cancelled) => Err(ServiceError::Cancelled),
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
                let bytes = if json {
                    serde_json::to_vec(&FailureDocument {
                        error: BrowserFailure {
                            code,
                            message,
                            details: Some(details),
                        },
                    })?
                } else {
                    output::render_error(code, &message, &details).into_bytes()
                };
                context.output.stderr(Bytes::from(bytes)).await?;
                Ok(Completion {
                    exit_code: match code {
                        BrowserErrorCode::InvalidInput => 2,
                        BrowserErrorCode::Cancelled => 130,
                        _ => 1,
                    },
                    error: None,
                })
            }
        }
    }

    /// Installs the pinned Chrome, and on Linux the Chrome runtime, for
    /// `context`'s invocation, which prints a line in its own output as each
    /// tenth of a download arrives and as it is unpacked, unless it answers
    /// in JSON (`browser.md` § Installation).
    async fn install(&self, context: &mut InvocationContext) -> Result<InstallResult> {
        let (progress, mut reports) = mpsc::channel(PROGRESS_LINES);
        let installing = self.chrome.install(&context.request.invocation_id, progress);
        tokio::pin!(installing);
        let installed = loop {
            tokio::select! {
                biased;
                Some(report) = reports.recv() => print_progress(context, report).await?,
                installed = &mut installing => break installed,
            }
        };
        // What the runner reported before it answered.
        while let Ok(report) = reports.try_recv() {
            print_progress(context, report).await?;
        }
        installed
    }

    async fn execute(
        &self,
        browser: &ConversationBrowser,
        context: &mut InvocationContext,
        command: &BrowserOperation,
        cancellation: &CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<CommandOutput> {
        if matches!(command, BrowserOperation::Install(_)) {
            // The Host's browser, not the conversation's: nothing starts.
            let installed = self.install(context).await?;
            return Ok(CommandOutput::Json(output::value(installed)?));
        }
        // Only the user's page reads the tab list's number and its tabs closed
        // on purpose, and only it stops a load (`live-view.md` § The tab methods).
        let user = context.request.context.caller.agent_number().is_none();
        if matches!(command, BrowserOperation::Stop(_)) && !user {
            // The agent waits for its commands rather than stopping a load.
            return Err(BrowserError::Configuration(
                "stop is the user's: an agent's command waits for its page instead".into(),
            ));
        }
        let starts = matches!(
            command,
            BrowserOperation::Open(_) | BrowserOperation::ContentFetch(_)
        );
        let starting = starts.then(|| Starting {
            locale: context.request.context.locale.clone(),
        });
        let environment = browser.environment(starting.as_ref(), cancellation).await?;
        let Some(environment) = environment else {
            if matches!(command, BrowserOperation::Tabs(_)) {
                return Ok(CommandOutput::Json(output::value(TabsResult {
                    // No browser runs: the conversation's latest list number.
                    list: user.then(|| browser.numbers.list_number()),
                    tabs: Vec::new(),
                    truncated: false,
                    closed: user.then(|| browser.numbers.closed()),
                })?));
            }
            return Err(BrowserError::TabNotFound);
        };
        if let BrowserOperation::Open(input) = command
            && user
        {
            // The user's new tab (`live-view.md` § The tab methods): the
            // work panel shows its loading, so opening does not wait for the page.
            let url = (input.url != "about:blank").then_some(input.url.as_str());
            let tab = environment.open_user(url, cancellation, deadline).await?;
            if input.show == Some(true) {
                tab.show();
            }
            return Ok(CommandOutput::Json(output::value(OpenResult {
                tab: tab.id().clone(),
                url: url.unwrap_or("about:blank").to_owned(),
                title: None,
                viewport: None,
            })?));
        }
        if let BrowserOperation::Open(input) = command {
            let (tab, url) = environment
                .open_for(
                    &input.url,
                    demi_command_package_browser_chrome::driver::operation::agent(context)?,
                    input.load.unwrap_or_default(),
                    cancellation,
                    deadline,
                )
                .await?;
            if input.show == Some(true) {
                tab.show();
            }
            // Navigation completed. Metadata is optional and cannot undo that input.
            let result = match demi_command_package_browser_chrome::page::actions::metadata(
                &tab,
                cancellation,
                deadline.saturating_duration_since(tokio::time::Instant::now()),
            )
            .await
            {
                Ok(metadata) if metadata.url == url => OpenResult {
                    tab: metadata.tab,
                    url: metadata.url,
                    title: Some(metadata.title),
                    viewport: Some(metadata.viewport),
                },
                _ => OpenResult {
                    tab: tab.id().clone(),
                    url,
                    title: None,
                    viewport: None,
                },
            };
            return Ok(CommandOutput::Json(output::value(result)?));
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
        if let BrowserOperation::Tabs(input) = command {
            let (list, listing) = environment.numbered(cancellation, command.timeout()).await?;
            let offset = input.offset.unwrap_or(0);
            let limit = input.limit.unwrap_or(DEFAULT_NODES);
            // Each tab's record as the list shows it, not the tab itself.
            let rows = listing
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
                .collect();
            return Ok(CommandOutput::Json(output::value(TabsResult {
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
            // once the job ends (`live-view.md` § Showing a tab).
            tab.show();
            return Ok(CommandOutput::Json(output::value(ShowResult {
                tab: tab.id().clone(),
            })?));
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
                BrowserOperation::Goto(_)
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
            // Only a screenshot that goes to the job's output, or to a file,
            // tells what it captured.
            let told = input.output.is_some() || context.request.stdout == Some(StdoutTarget::Job);
            if !told {
                return Ok(CommandOutput::Medium {
                    text: None,
                    png: bytes,
                });
            }
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
                let text = demi_command_package_browser_chrome::driver::text::captured(
                    tab.id().as_str(),
                    width,
                    height,
                    &metadata.viewport,
                );
                return Ok(CommandOutput::Medium {
                    text: Some(text),
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
        if matches!(command, BrowserOperation::Close(_)) {
            // Closing the last tab stops Chrome and removes its profile
            // before the close is acknowledged (`browser.md` § Lifetime).
            let closed = tab.close_request(cancellation, command.timeout()).await?;
            if matches!(closed, Closed::Environment) || environment.emptied().is_cancelled() {
                browser.retire(&environment).await?;
            }
            return Ok(CommandOutput::Json(output::value(CloseResult {
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
            // Decoding and encoding a screenshot takes milliseconds of CPU
            // (`concurrency.md` § Blocking work).
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

    /// Release only the trusted conversation selected by the parent service port.
    pub async fn conversation(
        &self,
        context: ConversationContext,
    ) -> std::result::Result<Completion, ServiceError> {
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
                    let released = browser.release().await;
                    let forget = Find::Forget {
                        conversation: conversation.clone(),
                        browser,
                    };
                    // A service that shut down forgets everything anyway.
                    let _closed = self.requests.send(forget).await;
                    released.map_err(ServiceError::failed)?;
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
    /// so shutting down takes as long as the slowest (`browser.md` § Release).
    pub async fn close(&self) -> std::result::Result<(), ServiceError> {
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
            Some(error) => Err(ServiceError::failed(error)),
            None => Ok(()),
        }
    }
}
