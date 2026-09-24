//! The conversations' browsers (`browser.md` § Owners inside the service).
//! One owner task finds each invocation's conversation browser and forgets
//! it after the conversation's release. Each conversation browser is an
//! owner task of its own, with the one lifecycle of `browser.md` § Lifetime,
//! so one conversation's slow start or retirement never holds up another's
//! commands, and the runner's `status` never waits for either.

use std::{collections::HashMap, sync::Arc};

use bytes::Bytes;
use demi_builtin_protocol::DecodeError;
use demi_command_service::{
    ConversationContext, InvocationContext, ServiceError,
    protocol::{CommandLocale, Completion, ConversationRequest, ConversationStatus},
};
use tokio::{
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
};
use tokio_util::{
    sync::CancellationToken,
    task::{TaskTracker, task_tracker::TaskTrackerToken},
};

use super::{
    BrowserEnvironment, BrowserError, LaunchOptions, Result,
    actions::TabResult,
    installation::Installation,
    output,
    protocol::{
        self, ActionProgress, BrowserErrorCode, BrowserFailure, BrowserOperation, CloseResult,
        ContentReadResult, DEFAULT_NODES, FailureDocument, ImageMime, OpenResult,
        ScreenshotResult, TabsResult,
    },
    registry::Closed,
    with_browser,
};

/// Requests waiting for an owner; a full queue holds back their senders.
const REQUESTS: usize = 64;

enum CommandOutput {
    Json(serde_json::Value),
    Png(Vec<u8>),
}

type Readiness = Option<std::result::Result<BrowserEnvironment, EnvironmentFailure>>;

#[derive(Clone)]
enum EnvironmentFailure {
    Unavailable(String),
    Lost(String),
}

/// A conversation browser's one state (`browser.md` § Lifetime).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Lifecycle {
    Absent,
    Starting,
    Ready,
    Closing,
}

/// One conversation's browser, as its invocations reach it.
pub(super) struct ConversationBrowser {
    requests: mpsc::Sender<Request>,
    lifecycle: watch::Receiver<Lifecycle>,
    /// Cancelled when the conversation's release arrives.
    pub(super) cancellation: CancellationToken,
    /// The invocations admitted into the conversation.
    commands: TaskTracker,
}

enum Request {
    /// The browser, started first when `start` names the starting caller's
    /// locale.
    Environment {
        start: Option<CommandLocale>,
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
    fn start(tasks: &TaskTracker, installation: Arc<Installation>) -> Arc<Self> {
        let (requests, receiver) = mpsc::channel(REQUESTS);
        let (published, lifecycle) = watch::channel(Lifecycle::Absent);
        let cancellation = CancellationToken::new();
        let owner = Owner {
            state: State::Absent,
            installation,
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
        start: Option<&CommandLocale>,
        cancel: &CancellationToken,
    ) -> Result<Option<BrowserEnvironment>> {
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

    /// Watches the lifecycle, so a waiting viewer learns that a browser
    /// started.
    pub fn lifecycle(&self) -> watch::Receiver<Lifecycle> {
        let mut lifecycle = self.lifecycle.clone();
        lifecycle.mark_unchanged();
        lifecycle
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
        self.ask(|reply| Request::Release { reply }, &CancellationToken::new())
            .await
    }

    /// Inspect retained debug owners without starting or querying Chrome after a timeout.
    async fn debugging_callers(&self, tab: &str, caller: Option<&str>) -> Vec<String> {
        match self.environment(None, &CancellationToken::new()).await {
            Ok(Some(environment)) => environment.debugging_callers(tab, caller),
            Ok(None) | Err(_) => Vec::new(),
        }
    }
}

/// What a conversation browser's owner keeps.
struct Owner {
    state: State,
    installation: Arc<Installation>,
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
    Ended { cleanup: Option<String> },
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
            .cloned()
    }

    /// Whether this browser is on its way out: stopped, lost, emptied, or
    /// failed to start.
    fn leaving(&self) -> bool {
        self.stop.is_cancelled()
            || match self.ready.borrow().as_ref() {
                None => false,
                Some(Err(_)) => true,
                Some(Ok(environment)) => {
                    environment.ended.is_cancelled() || environment.emptied().is_cancelled()
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
                        .map(|environment| (environment.ended.clone(), environment.emptied().clone()))
                        .filter(|(ended, emptied)| !ended.is_cancelled() || !emptied.is_cancelled());
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
                        tracing::warn!("a conversation's browser did not finish its cleanup: {error}");
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
        start: Option<CommandLocale>,
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
                        || (environment.ended.is_cancelled() && environment.failure().is_none()) =>
                {
                    Ok(None)
                }
                Some(environment) if environment.ended.is_cancelled() => Err(
                    BrowserError::Connection(environment.failure().unwrap_or_default()),
                ),
                // Starting, ready, or failed to start, which the readiness says.
                _ => Ok(Some(running.ready.clone())),
            },
            State::Absent => match start {
                // A command admitted before the release began still asks.
                Some(_) if self.released.is_cancelled() => Err(BrowserError::Cancelled),
                Some(locale) => {
                    let ready = self.start(locale);
                    Ok(Some(ready))
                }
                None => Ok(None),
            },
        }
    }

    /// Starts the conversation's Chrome in the starting caller's locale.
    fn start(&mut self, locale: CommandLocale) -> watch::Receiver<Readiness> {
        let stop = CancellationToken::new();
        let (publish, ready) = watch::channel(None);
        let installation = self.installation.clone();
        let owner_stop = stop.clone();
        let task = self.tasks.spawn(async move {
            let publisher = publish.clone();
            let retire = owner_stop.clone();
            let result = async {
                let executable = installation.executable(&owner_stop).await?;
                with_browser(
                    LaunchOptions::pinned(executable, locale)?,
                    owner_stop.clone(),
                    move |environment| async move {
                        publisher.send_replace(Some(Ok(environment.clone())));
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
                let failure = if publish.borrow().as_ref().is_some_and(|ready| ready.is_ok()) {
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
            Ok(Err(error @ (BrowserError::Cleanup { .. } | BrowserError::ProfileRetained { .. }))) => {
                Some(error)
            }
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

/// The agent node a browser command acts for: tabs, debugging sessions and
/// temporary tabs belong to it (`native-runtime.md` § Command context).
pub(super) fn agent(context: &InvocationContext) -> Result<&str> {
    context
        .request
        .context
        .caller
        .node()
        .ok_or_else(|| BrowserError::Configuration("browser commands act for an agent".into()))
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
}

impl Default for Conversations {
    /// Starts the owner on the current Tokio runtime.
    fn default() -> Self {
        let (requests, receiver) = mpsc::channel(REQUESTS);
        let tasks = TaskTracker::new();
        tasks.spawn(serve(
            receiver,
            tasks.clone(),
            Arc::new(Installation::default()),
        ));
        Self { requests, tasks }
    }
}

/// The conversations' owner: it maps each conversation to its browser. It
/// answers every request at once, so the runner's `status` never waits for
/// a browser's start or retirement.
async fn serve(mut requests: mpsc::Receiver<Find>, tasks: TaskTracker, installation: Arc<Installation>) {
    let mut browsers: HashMap<String, Arc<ConversationBrowser>> = HashMap::new();
    // A requester that left needs no answer.
    while let Some(request) = requests.recv().await {
        match request {
            Find::Admit {
                conversation,
                reply,
            } => {
                let browser = browsers
                    .entry(conversation)
                    .or_insert_with(|| ConversationBrowser::start(&tasks, installation.clone()))
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
        operation: std::result::Result<BrowserOperation, DecodeError>,
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
        super::live::serve(browser, context).await
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
        operation: std::result::Result<BrowserOperation, DecodeError>,
    ) -> std::result::Result<Completion, ServiceError> {
        let result = async {
            let command =
                operation.map_err(|error| BrowserError::Configuration(error.to_string()))?;
            let cancellation = context.cancellation.child_token();
            let _cancel_on_drop = cancellation.clone().drop_guard();
            let deadline = tokio::time::Instant::now() + command.timeout();
            let invocation =
                self.execute(browser, &mut context, &command, &cancellation, deadline);
            tokio::pin!(invocation);
            match tokio::time::timeout_at(deadline, invocation.as_mut()).await {
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
                                details: error.details(),
                                source: Box::new(BrowserError::Timeout),
                            })
                        }
                        result => result,
                    }
                }
            }
        }
        .await
        .and_then(|value| match value {
            CommandOutput::Json(value) => output::render(value, context.request.json == Some(true)),
            CommandOutput::Png(bytes) => Ok(bytes),
        });
        match result {
            Ok(bytes) => {
                context.output.stdout(Bytes::from(bytes)).await?;
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
                        .debugging_callers(tab, context.request.context.caller.node())
                        .await;
                    if !callers.is_empty() {
                        details.debugging_callers = Some(callers);
                    }
                }
                let bytes = if context.request.json == Some(true) {
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

    async fn execute(
        &self,
        browser: &ConversationBrowser,
        context: &mut InvocationContext,
        command: &BrowserOperation,
        cancellation: &CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<CommandOutput> {
        let starts = matches!(
            command,
            BrowserOperation::Open(_) | BrowserOperation::ContentFetch(_)
        );
        let environment = browser
            .environment(
                starts.then_some(&context.request.context.locale),
                cancellation,
            )
            .await?;
        let Some(environment) = environment else {
            if matches!(command, BrowserOperation::Tabs(_)) {
                return Ok(CommandOutput::Json(output::value(TabsResult {
                    tabs: Vec::new(),
                    truncated: false,
                })?));
            }
            return Err(BrowserError::TabNotFound);
        };
        if let BrowserOperation::Open(input) = command
            && context.request.context.caller.node().is_none()
        {
            // The user's new tab (`web-api.md` § Conversation browser tabs): the
            // work panel shows its loading, so opening does not wait for the page.
            let url = (input.url != "about:blank").then_some(input.url.as_str());
            let tab = environment.open_user(url, cancellation, deadline).await?;
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
                    agent(context)?,
                    input.load.unwrap_or_default(),
                    cancellation,
                    deadline,
                )
                .await?;
            // Navigation completed. Metadata is optional and cannot undo that input.
            let result = match tab
                .metadata(
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
            let result =
                super::fetch::execute(context, &environment, None, command, cancellation, deadline)
                    .await;
            // The batch's tabs are closed; the browser retires with its last one.
            let retired = if environment.emptied().is_cancelled() {
                browser.retire(&environment).await
            } else {
                Ok(())
            };
            return super::operation::after_cleanup(result, retired).map(CommandOutput::Json);
        }
        if let BrowserOperation::Tabs(input) = command {
            let listing = environment.listed(cancellation, command.timeout()).await?;
            let offset = input.offset.unwrap_or(0);
            let limit = input.limit.unwrap_or(DEFAULT_NODES);
            // Each tab's record as the list shows it, not the tab itself.
            let rows = listing
                .tabs
                .iter()
                .skip(offset)
                .take(limit)
                .map(|listed| protocol::BrowserTab {
                    id: listed.tab.id().clone(),
                    title: listed.title.clone(),
                    url: listed.url.clone(),
                    created_by: listed.tab.created_by.clone(),
                })
                .collect();
            return Ok(CommandOutput::Json(output::value(TabsResult {
                tabs: rows,
                truncated: listing.tabs.len() > offset.saturating_add(limit),
            })?));
        }
        let id = command.tab().ok_or(BrowserError::TabNotFound)?;
        let tab = &environment
            .tab(id, cancellation, command.timeout())
            .await?;
        if context.request.context.caller.node().is_none()
            && matches!(
                command,
                BrowserOperation::Goto(_)
                    | BrowserOperation::Reload(_)
                    | BrowserOperation::Back(_)
                    | BrowserOperation::Forward(_)
            )
        {
            let operation = super::operation::Operation::for_tab(tab, cancellation, deadline);
            let result = super::navigation::steer(tab, command, &operation).await?;
            return Ok(CommandOutput::Json(output::value(result)?));
        }
        {
            let family: Option<futures_util::future::BoxFuture<'_, Result<serde_json::Value>>> =
                match command {
                    BrowserOperation::Upload(_) => Some(Box::pin(super::upload::execute(
                        context,
                        &environment,
                        Some(tab),
                        command,
                        cancellation,
                        deadline,
                    ))),
                    BrowserOperation::Download(_) => Some(Box::pin(super::download::execute(
                        context,
                        &environment,
                        Some(tab),
                        command,
                        cancellation,
                        deadline,
                    ))),
                    BrowserOperation::ClipboardRead(_) | BrowserOperation::ClipboardWrite(_) => {
                        Some(Box::pin(super::clipboard::execute(
                            context,
                            &environment,
                            Some(tab),
                            command,
                            cancellation,
                            deadline,
                        )))
                    }
                    BrowserOperation::CdpTargets(_)
                    | BrowserOperation::CdpDetach(_)
                    | BrowserOperation::CdpSend(_)
                    | BrowserOperation::CdpEvents(_) => Some(Box::pin(super::cdp::execute(
                        context,
                        &environment,
                        Some(tab),
                        command,
                        cancellation,
                        deadline,
                    ))),
                    BrowserOperation::AssetsList(_) | BrowserOperation::AssetsExport(_) => {
                        Some(Box::pin(super::assets::execute(
                            context,
                            &environment,
                            Some(tab),
                            command,
                            cancellation,
                            deadline,
                        )))
                    }
                    BrowserOperation::WebmcpList(_) | BrowserOperation::WebmcpCall(_) => {
                        Some(Box::pin(super::webmcp::execute(
                            context,
                            &environment,
                            Some(tab),
                            command,
                            cancellation,
                            deadline,
                        )))
                    }
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
                super::output::preflight(
                    &context.request.cwd,
                    output,
                    input.overwrite == Some(true),
                )
                .await?;
            }
            let bytes = tab
                .capture(
                    input.full_page == Some(true),
                    input.clip.as_deref(),
                    cancellation,
                    command.timeout(),
                )
                .await?;
            let Some(output) = &input.output else {
                return Ok(CommandOutput::Png(bytes));
            };
            let decoded = png::Decoder::new(std::io::Cursor::new(&bytes))
                .read_info()
                .map_err(|error| BrowserError::InvalidResult(error.to_string()))?;
            let width = decoded.info().width;
            let height = decoded.info().height;
            drop(decoded);
            let metadata = tab.metadata(cancellation, command.timeout()).await?;
            let path = super::output::save_with_overwrite(
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
        if let BrowserOperation::Probe(input) = command
            && let Some(file) = &input.output
        {
            output::preflight(&context.request.cwd, file, input.overwrite == Some(true)).await?;
            let mut session = tab.state.gate.try_checkout().ok_or(BrowserError::Busy)?;
            let TabResult::Probe(mut result) = tab
                .command_admitted(command, cancellation, deadline, &mut session.references)
                .await?
            else {
                return Err(BrowserError::InvalidResult("probe returned another result".into()));
            };
            let bytes = super::operation::Operation::for_tab(tab, cancellation, deadline)
                .run(tab.screenshot_bytes(false, None))
                .await?;
            // Decoding and encoding a screenshot takes milliseconds of CPU
            // (`concurrency.md` § Blocking work).
            let outlined = result.clone();
            let bytes =
                tokio::task::spawn_blocking(move || super::probe::annotate(bytes, &outlined))
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
        let result = tab.command(command, cancellation, deadline).await?;
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
            return Ok(CommandOutput::Json(output::value(ContentReadResult::File {
                url,
                title,
                format,
                path,
            })?));
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
