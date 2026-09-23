//! Conversation controllers are retained independently of individual invocations.

use std::{collections::HashMap, sync::Arc};

use bytes::Bytes;
use demi_command_service::{
    ConversationContext, InvocationContext, ServiceError,
    protocol::{CommandLocale, Completion, ConversationRequest, ConversationStatus},
};
use tokio::sync::{Mutex, watch};
use tokio_util::{
    sync::{CancellationToken, DropGuard},
    task::TaskTracker,
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
    with_browser,
};

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

enum State {
    Absent,
    Live {
        ready: watch::Receiver<Readiness>,
        stop: CancellationToken,
        _stop_on_drop: DropGuard,
        owner: tokio::task::JoinHandle<Result<()>>,
    },
    Released,
}

pub(super) struct Controller {
    state: Mutex<State>,
    installation: Arc<Installation>,
    /// Cancelled when the conversation's release arrives.
    pub(super) cancellation: CancellationToken,
    commands: TaskTracker,
    /// Counts the environments started, so a waiting viewer learns of one.
    started: watch::Sender<u64>,
}

impl Controller {
    /// First open starts this conversation's Chrome in the starting caller's
    /// locale; concurrent opens join startup.
    pub async fn environment(
        &self,
        start: Option<&CommandLocale>,
        cancel: &CancellationToken,
    ) -> Result<Option<BrowserEnvironment>> {
        let mut state = self.state.lock().await;
        if cancel.is_cancelled() {
            return Err(BrowserError::Cancelled);
        }
        if start.is_some() && matches!(&*state, State::Live { owner, .. } if owner.is_finished()) {
            Self::retire_locked(&mut state, State::Absent).await?;
        }
        if let (State::Absent, Some(locale)) = (&*state, start) {
            let locale = locale.clone();
            let stop = CancellationToken::new();
            let owner_stop = stop.clone();
            let installation = self.installation.clone();
            let (publish, ready) = watch::channel(None);
            let owner = tokio::spawn(async move {
                let result = async {
                    let executable = installation.executable(&owner_stop).await?;
                    with_browser(
                        LaunchOptions::pinned(executable, locale)?,
                        owner_stop.clone(),
                        |environment| async {
                            publish.send_replace(Some(Ok(environment)));
                            owner_stop.cancelled().await;
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
            *state = State::Live {
                ready,
                _stop_on_drop: stop.clone().drop_guard(),
                stop,
                owner,
            };
            self.started.send_modify(|started| *started += 1);
        }
        let mut ready = match &*state {
            State::Absent => return Ok(None),
            State::Released => return Err(BrowserError::Closed),
            State::Live { ready, .. } => ready.clone(),
        };
        drop(state);
        loop {
            if let Some(result) = ready.borrow_and_update().clone() {
                return result.map(Some).map_err(|failure| match failure {
                    EnvironmentFailure::Unavailable(message) => BrowserError::Unavailable(message),
                    EnvironmentFailure::Lost(message) => BrowserError::Connection(message),
                });
            }
            tokio::select! {
                _ = cancel.cancelled() => return Err(BrowserError::Cancelled),
                result = ready.changed() => result.map_err(|_| BrowserError::Closed)?,
            }
        }
    }

    /// Fence new operations before awaiting the browser's joined retirement.
    async fn retire(&self, next: State, expected: Option<&BrowserEnvironment>) -> Result<()> {
        let mut state = self.state.lock().await;
        if let Some(expected) = expected {
            let matches = match &*state {
                State::Live { ready, .. } => ready.borrow().as_ref().is_some_and(|result| {
                    result
                        .as_ref()
                        .is_ok_and(|environment| environment.browser.ptr_eq(&expected.browser))
                }),
                _ => false,
            };
            // A delayed last-tab close must not retire a newly opened generation.
            if !matches {
                return Ok(());
            }
        }
        Self::retire_locked(&mut state, next).await
    }

    /// Join the browser owner under the lifecycle gate before admitting a replacement.
    async fn retire_locked(state: &mut State, next: State) -> Result<()> {
        let previous = std::mem::replace(state, State::Released);
        // Keep the gate until retirement finishes: concurrent release acknowledgements
        // must not claim that Chrome has exited while the first caller is still waiting.
        if let State::Live { stop, owner, .. } = previous {
            stop.cancel();
            match owner.await? {
                Err(
                    error @ (BrowserError::Cleanup { .. } | BrowserError::ProfileRetained { .. }),
                ) => {
                    return Err(error);
                }
                // Startup and transport failures were published to their callers.
                // The joined owner has cleaned up; only cleanup failures fence reuse.
                Ok(()) | Err(_) => {}
            }
        }
        *state = next;
        Ok(())
    }

    /// Close a conversation tab, retiring its entire browser when it is the last.
    async fn close_tab(
        &self,
        environment: &BrowserEnvironment,
        tab: &super::BrowserTab,
        cancellation: &CancellationToken,
        timeout: std::time::Duration,
    ) -> Result<()> {
        if let Ok(_admission) = environment.acquisition.clone().try_write_owned() {
            let tabs = environment.tabs(cancellation, timeout).await?;
            if tabs.len() == 1 && tabs[0].id() == tab.id() {
                // CloseTarget can acknowledge while a concurrent navigation keeps
                // the final page alive. Retire the owned browser directly, while
                // admission prevents another caller from adding a tab to it.
                return self.retire(State::Absent, Some(environment)).await;
            }
        }
        tab.close(cancellation, timeout).await?;
        self.retire_empty(environment).await
    }

    /// Watches for the next environment to start.
    pub(super) fn started(&self) -> watch::Receiver<u64> {
        self.started.subscribe()
    }

    /// Retire an empty browser only after all tab-acquisition batches have finished.
    pub(super) async fn retire_empty(&self, environment: &BrowserEnvironment) -> Result<()> {
        if let Ok(_admission) = environment.acquisition.clone().try_write_owned()
            && environment
                .tabs(&CancellationToken::new(), super::operation::CONTROL_TIMEOUT)
                .await?
                .is_empty()
        {
            self.retire(State::Absent, Some(environment)).await?;
        }
        Ok(())
    }

    async fn live(&self) -> bool {
        match &*self.state.lock().await {
            State::Absent | State::Released => false,
            State::Live { owner, .. } => !owner.is_finished(),
        }
    }

    /// Inspect retained debug owners without starting or querying Chrome after a timeout.
    async fn debugging_callers(&self, tab: &str, caller: Option<&str>) -> Vec<String> {
        let environment = match &*self.state.lock().await {
            State::Live { ready, .. } => ready
                .borrow()
                .as_ref()
                .and_then(|result| result.as_ref().ok())
                .cloned(),
            _ => None,
        };
        match environment {
            Some(environment) => environment.debugging_callers(tab, caller).await,
            None => Vec::new(),
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

#[derive(Default)]
pub(crate) struct Conversations {
    controllers: Mutex<HashMap<String, Arc<Controller>>>,
    installation: Arc<Installation>,
}

impl Conversations {
    pub async fn invoke(
        &self,
        context: InvocationContext,
    ) -> std::result::Result<Completion, ServiceError> {
        // Registration and the release fence share the map lock: release can never
        // miss an admitted command, and a fenced controller admits no new work.
        let (controller, _command) = {
            let mut controllers = self.controllers.lock().await;
            let controller = controllers
                .entry(context.request.context.conversation.clone())
                .or_insert_with(|| {
                    Arc::new(Controller {
                        state: Mutex::new(State::Absent),
                        installation: self.installation.clone(),
                        cancellation: CancellationToken::new(),
                        commands: TaskTracker::new(),
                        started: watch::channel(0).0,
                    })
                })
                .clone();
            if controller.cancellation.is_cancelled() {
                return Err(ServiceError::Cancelled);
            }
            let command = controller.commands.token();
            (controller, command)
        };
        // A view ends itself on release, so it can tell its page why.
        if context.request.operation == super::live::OPERATION {
            return super::live::serve(controller, context).await;
        }
        let cancellation = context.cancellation.clone();
        let work = self.invoke_admitted(&controller, context);
        tokio::pin!(work);
        tokio::select! {
            biased;
            _ = controller.cancellation.cancelled() => {
                // Cancel the invocation token, including stdin and blocked output,
                // and join its cleanup before dropping the admission token.
                cancellation.cancel();
                work.await
            }
            result = &mut work => result,
        }
    }

    async fn invoke_admitted(
        &self,
        controller: &Controller,
        mut context: InvocationContext,
    ) -> std::result::Result<Completion, ServiceError> {
        let operation = context
            .request
            .operation
            .strip_prefix("browser.")
            .ok_or_else(|| ServiceError::Handler("not a browser operation".into()))?
            .to_owned();
        let result = async {
            let command = BrowserOperation::parse(&operation, context.request.args.clone())
                .map_err(|error| BrowserError::Configuration(error.to_string()))?;
            let cancellation = context.cancellation.child_token();
            let _cancel_on_drop = cancellation.clone().drop_guard();
            let deadline = tokio::time::Instant::now() + command.timeout();
            let invocation =
                self.execute(controller, &mut context, &command, &cancellation, deadline);
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
                    let callers = controller
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
        controller: &Controller,
        context: &mut InvocationContext,
        command: &BrowserOperation,
        cancellation: &CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<CommandOutput> {
        let starts = matches!(
            command,
            BrowserOperation::Open(_) | BrowserOperation::ContentFetch(_)
        );
        let environment = controller
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
            return super::operation::after_cleanup(
                result,
                controller.retire_empty(&environment).await,
            )
            .map(CommandOutput::Json);
        }
        let tabs = environment.tabs(cancellation, command.timeout()).await?;
        if let BrowserOperation::Tabs(input) = command {
            let offset = input.offset.unwrap_or(0);
            let limit = input.limit.unwrap_or(DEFAULT_NODES);
            let mut rows = Vec::new();
            for tab in tabs.iter().skip(offset).take(limit) {
                let metadata = tab.metadata(cancellation, command.timeout()).await?;
                // The tab's record as the list shows it, not the tab itself.
                rows.push(protocol::BrowserTab {
                    id: tab.id().clone(),
                    title: metadata.title,
                    url: metadata.url,
                    created_by: tab.created_by.clone(),
                });
            }
            return Ok(CommandOutput::Json(output::value(TabsResult {
                tabs: rows,
                truncated: tabs.len() > offset.saturating_add(limit),
            })?));
        }
        let tab = tabs
            .iter()
            .find(|tab| Some(tab.id()) == command.tab())
            .ok_or(BrowserError::TabNotFound)?;
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
            controller
                .close_tab(&environment, tab, cancellation, command.timeout())
                .await?;
            return Ok(CommandOutput::Json(output::value(CloseResult {
                closed: tab.id().clone(),
            })?));
        }
        if let BrowserOperation::Probe(input) = command
            && let Some(file) = &input.output
        {
            output::preflight(&context.request.cwd, file, input.overwrite == Some(true)).await?;
            let mut references = tab
                .state
                .operations
                .try_lock()
                .map_err(|_| BrowserError::Busy)?;
            let TabResult::Probe(mut result) = tab
                .command_admitted(command, cancellation, deadline, &mut references)
                .await?
            else {
                return Err(BrowserError::InvalidResult("probe returned another result".into()));
            };
            let bytes = super::operation::Operation::for_tab(tab, cancellation, deadline)
                .run(tab.screenshot_bytes(false, None))
                .await?;
            let bytes = super::probe::annotate(bytes, &result)?;
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
                let controllers: Vec<_> = self
                    .controllers
                    .lock()
                    .await
                    .iter()
                    .map(|(id, controller)| (id.clone(), controller.clone()))
                    .collect();
                let mut conversations = Vec::new();
                for (id, controller) in controllers {
                    if controller.live().await {
                        conversations.push(id);
                    }
                }
                conversations.sort();
                serde_json::to_value(ConversationStatus { conversations })?
            }
            ConversationRequest::Release { conversation } => {
                let controller = {
                    let controllers = self.controllers.lock().await;
                    let controller = controllers.get(conversation).cloned();
                    if let Some(controller) = &controller {
                        controller.cancellation.cancel();
                        controller.commands.close();
                    }
                    controller
                };
                if let Some(controller) = controller {
                    // Once fenced, finish release even if the requester goes away.
                    controller.commands.wait().await;
                    controller
                        .retire(State::Released, None)
                        .await
                        .map_err(|error| ServiceError::Handler(error.to_string()))?;
                    let mut controllers = self.controllers.lock().await;
                    if controllers
                        .get(conversation)
                        .is_some_and(|current| Arc::ptr_eq(current, &controller))
                    {
                        controllers.remove(conversation);
                    }
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

    pub async fn close(&self) -> std::result::Result<(), ServiceError> {
        let controllers = std::mem::take(&mut *self.controllers.lock().await);
        let mut failure = None;
        for controller in controllers.into_values() {
            controller.cancellation.cancel();
            controller.commands.close();
            controller.commands.wait().await;
            if let Err(error) = controller.retire(State::Released, None).await {
                failure.get_or_insert(error);
            }
        }
        match failure {
            Some(error) => Err(ServiceError::Handler(error.to_string())),
            None => Ok(()),
        }
    }
}
