//! Conversation controllers are retained independently of individual invocations.

use std::{collections::HashMap, sync::Arc};

use bytes::Bytes;
use demi_command_service::{InvocationContext, ServiceError, protocol::Completion};
use tokio::sync::{Mutex, watch};
use tokio_util::sync::{CancellationToken, DropGuard};

use super::{
    BrowserEnvironment, BrowserError, LaunchOptions, Result, installation::Installation,
    protocol::BrowserCommand, with_browser,
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
}

impl Controller {
    /// First open starts this conversation's Chrome; concurrent opens join startup.
    pub async fn environment(
        &self,
        start: bool,
        cancel: &CancellationToken,
    ) -> Result<Option<BrowserEnvironment>> {
        let mut state = self.state.lock().await;
        if matches!(*state, State::Absent) && start {
            let stop = CancellationToken::new();
            let owner_stop = stop.clone();
            let installation = self.installation.clone();
            let (publish, ready) = watch::channel(None);
            let owner = tokio::spawn(async move {
                let result = async {
                    let executable = installation.executable(&owner_stop).await?;
                    with_browser(
                        LaunchOptions { executable },
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
    pub async fn release(&self) -> Result<()> {
        let mut state = self.state.lock().await;
        let previous = std::mem::replace(&mut *state, State::Released);
        // Keep the gate until retirement finishes: concurrent release acknowledgements
        // must not claim that Chrome has exited while the first caller is still waiting.
        if let State::Live { stop, owner, .. } = previous {
            stop.cancel();
            match owner.await? {
                Ok(()) | Err(BrowserError::Cancelled) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    /// Retire an empty browser only after all tab-acquisition batches have finished.
    async fn retire_empty(&self, environment: &BrowserEnvironment) -> Result<()> {
        if let Ok(_admission) = environment.acquisition.clone().try_write_owned()
            && environment
                .tabs(&CancellationToken::new(), super::operation::CONTROL_TIMEOUT)
                .await?
                .is_empty()
        {
            self.release().await?;
        }
        Ok(())
    }

    async fn live(&self) -> bool {
        match &*self.state.lock().await {
            State::Absent => true,
            State::Released => false,
            State::Live { owner, .. } => !owner.is_finished(),
        }
    }
}

#[derive(Default)]
pub(crate) struct Resources {
    controllers: Mutex<HashMap<String, Arc<Controller>>>,
    installation: Arc<Installation>,
}

impl Resources {
    pub async fn invoke(
        &self,
        mut context: InvocationContext,
    ) -> std::result::Result<Completion, ServiceError> {
        let operation = context
            .request
            .operation
            .strip_prefix("browser.")
            .ok_or_else(|| ServiceError::Handler("not a browser operation".into()))?
            .to_owned();
        let result = async {
            let command = BrowserCommand::parse(&operation, context.request.args.clone())
                .map_err(|error| BrowserError::Configuration(error.to_string()))?;
            let cancellation = context.cancellation.child_token();
            let _cancel_on_drop = cancellation.clone().drop_guard();
            let deadline = tokio::time::Instant::now() + command.timeout();
            let invocation = self.execute(&mut context, &command, &cancellation, deadline);
            tokio::pin!(invocation);
            match tokio::time::timeout_at(deadline, invocation.as_mut()).await {
                Ok(result) => result,
                Err(_) => {
                    // Cancel the active phase, then join its bounded input/file cleanup.
                    // Dropping the invocation here could strand a pressed button.
                    cancellation.cancel();
                    match invocation.await {
                        Err(error) if matches!(error.code(), "cancelled" | "timeout") => {
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
            CommandOutput::Json(value) => {
                super::output::render(&operation, value, context.request.json == Some(true))
            }
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
                if details.get("action").is_none() {
                    details["action"] = serde_json::json!("not_started");
                }
                if details.get("tab").is_none()
                    && let Some(tab) = context
                        .request
                        .args
                        .get("tab")
                        .and_then(serde_json::Value::as_str)
                {
                    details["tab"] = serde_json::json!(tab);
                }
                let bytes = if context.request.json == Some(true) {
                    serde_json::to_vec(
                        &serde_json::json!({ "error": { "code": code, "message": message, "details": details } }),
                    )?
                } else {
                    super::output::render_error(code, &message, &details).into_bytes()
                };
                context.output.stderr(Bytes::from(bytes)).await?;
                Ok(Completion {
                    exit_code: if code == "invalid_input" {
                        2
                    } else if code == "cancelled" {
                        130
                    } else {
                        1
                    },
                    error: None,
                })
            }
        }
    }

    async fn execute(
        &self,
        context: &mut InvocationContext,
        command: &BrowserCommand,
        cancellation: &CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<CommandOutput> {
        let controller = self.controller(context).await?;
        let environment = controller
            .environment(
                matches!(
                    command,
                    BrowserCommand::Open(_) | BrowserCommand::ContentFetch(_)
                ),
                cancellation,
            )
            .await?;
        let Some(environment) = environment else {
            if matches!(command, BrowserCommand::Tabs(_)) {
                return Ok(CommandOutput::Json(
                    serde_json::json!({ "tabs": [], "truncated": false }),
                ));
            }
            return Err(BrowserError::TabNotFound);
        };
        if let BrowserCommand::Open(input) = command {
            let caller = context.request.caller.as_ref().ok_or_else(|| {
                BrowserError::Configuration(
                    "browser caller is missing from trusted invocation".into(),
                )
            })?;
            let (tab, url) = environment
                .open_for(
                    &input.url,
                    caller,
                    input.load.as_deref().unwrap_or("domcontentloaded"),
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
                Ok(metadata) if metadata["url"] == url => metadata,
                _ => serde_json::json!({"tab": tab.id(), "url": url}),
            };
            return Ok(CommandOutput::Json(result));
        }
        if matches!(command, BrowserCommand::ContentFetch(_)) {
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
        if let BrowserCommand::Tabs(input) = command {
            let offset = input.offset.unwrap_or(0) as usize;
            let limit = input
                .limit
                .map_or(super::protocol::DEFAULT_NODES, |limit| limit as usize);
            let mut rows = Vec::new();
            for tab in tabs.iter().skip(offset).take(limit) {
                let mut row = tab.metadata(cancellation, command.timeout()).await?;
                let metadata = row.as_object_mut().expect("metadata is an object");
                metadata.remove("viewport");
                metadata.remove("tab");
                row["id"] = serde_json::json!(tab.id());
                row["createdBy"] = serde_json::to_value(&tab.created_by)
                    .map_err(|error| BrowserError::InvalidResult(error.to_string()))?;
                rows.push(row);
            }
            return Ok(CommandOutput::Json(
                serde_json::json!({ "tabs": rows, "truncated": tabs.len() > offset.saturating_add(limit) }),
            ));
        }
        let tab = tabs
            .iter()
            .find(|tab| Some(tab.id().as_str()) == command.tab())
            .ok_or(BrowserError::TabNotFound)?;
        {
            let family: Option<futures_util::future::BoxFuture<'_, Result<serde_json::Value>>> =
                match command {
                    BrowserCommand::Upload(_) => Some(Box::pin(super::upload::execute(
                        context,
                        &environment,
                        Some(tab),
                        command,
                        cancellation,
                        deadline,
                    ))),
                    BrowserCommand::Download(_) => Some(Box::pin(super::download::execute(
                        context,
                        &environment,
                        Some(tab),
                        command,
                        cancellation,
                        deadline,
                    ))),
                    BrowserCommand::ClipboardRead(_) | BrowserCommand::ClipboardWrite(_) => {
                        Some(Box::pin(super::clipboard::execute(
                            context,
                            &environment,
                            Some(tab),
                            command,
                            cancellation,
                            deadline,
                        )))
                    }
                    BrowserCommand::CdpTargets(_)
                    | BrowserCommand::CdpSend(_)
                    | BrowserCommand::CdpEvents(_) => Some(Box::pin(super::cdp::execute(
                        context,
                        &environment,
                        Some(tab),
                        command,
                        cancellation,
                        deadline,
                    ))),
                    BrowserCommand::AssetsList(_) | BrowserCommand::AssetsExport(_) => {
                        Some(Box::pin(super::assets::execute(
                            context,
                            &environment,
                            Some(tab),
                            command,
                            cancellation,
                            deadline,
                        )))
                    }
                    BrowserCommand::WebmcpList(_) | BrowserCommand::WebmcpCall(_) => {
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
        if let BrowserCommand::Screenshot(input) = command {
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
            return Ok(CommandOutput::Json(
                serde_json::json!({ "path": path, "mimeType": "image/png", "width": width, "height": height, "viewport": metadata["viewport"] }),
            ));
        }
        if matches!(command, BrowserCommand::Close(_)) {
            tab.close(cancellation, command.timeout()).await?;
            controller.retire_empty(&environment).await?;
            return Ok(CommandOutput::Json(
                serde_json::json!({ "closed": tab.id() }),
            ));
        }
        if let BrowserCommand::Probe(input) = command
            && let Some(output) = &input.output
        {
            super::output::preflight(&context.request.cwd, output, input.overwrite == Some(true))
                .await?;
            let mut references = tab
                .state
                .operations
                .try_lock()
                .map_err(|_| BrowserError::Busy)?;
            let mut result = tab
                .command_admitted(command, cancellation, deadline, &mut references)
                .await?;
            let bytes = super::operation::Operation::until(&tab.ended, cancellation, deadline)
                .run(tab.screenshot_bytes(false, None))
                .await?;
            let bytes = super::probe::annotate(bytes, &result)?;
            result["path"] = serde_json::json!(
                super::output::save_with_overwrite(
                    &context.request.cwd,
                    output,
                    bytes,
                    input.overwrite == Some(true),
                    cancellation,
                    deadline
                )
                .await?
            );
            return Ok(CommandOutput::Json(result));
        }
        let mut result = tab.command(command, cancellation, deadline).await?;
        if let BrowserCommand::ContentRead(input) = command
            && let Some(output) = &input.output
        {
            let content = result["content"].as_str().ok_or_else(|| {
                BrowserError::InvalidResult("content export did not return text".into())
            })?;
            let path = super::output::save_with_overwrite(
                &context.request.cwd,
                output,
                content.as_bytes().to_vec(),
                input.overwrite == Some(true),
                cancellation,
                deadline,
            )
            .await?;
            result["path"] = serde_json::json!(path);
            if let Some(result) = result.as_object_mut() {
                result.remove("content");
                result.remove("truncated");
            }
        }
        Ok(CommandOutput::Json(result))
    }

    /// Resolve only the trusted scope supplied by runner, never a command argument.
    pub(super) async fn controller(&self, context: &InvocationContext) -> Result<Arc<Controller>> {
        let scope = context
            .request
            .resource
            .as_ref()
            .filter(|scope| scope.kind == "browser")
            .ok_or_else(|| BrowserError::WrongHost)?;
        self.controllers
            .lock()
            .await
            .get(&scope.id)
            .cloned()
            .ok_or(BrowserError::Closed)
    }

    pub async fn resource(
        &self,
        context: InvocationContext,
    ) -> std::result::Result<Completion, ServiceError> {
        let scope = context
            .request
            .resource
            .as_ref()
            .filter(|scope| scope.kind == "browser")
            .ok_or_else(|| ServiceError::Handler("unsupported resource kind".into()))?;
        let controller = {
            let mut controllers = self.controllers.lock().await;
            if context.request.operation == "acquire" {
                controllers.entry(scope.id.clone()).or_insert_with(|| {
                    Arc::new(Controller {
                        state: Mutex::new(State::Absent),
                        installation: self.installation.clone(),
                    })
                });
            }
            controllers.get(&scope.id).cloned()
        };
        let live = match context.request.operation.as_str() {
            "acquire" | "status" => match controller {
                Some(controller) => controller.live().await,
                None => false,
            },
            "release" => {
                if let Some(controller) = controller {
                    controller
                        .release()
                        .await
                        .map_err(|error| ServiceError::Handler(error.to_string()))?;
                    self.controllers.lock().await.remove(&scope.id);
                }
                false
            }
            _ => return Err(ServiceError::Handler("unknown resource operation".into())),
        };
        let output = serde_json::json!({ "state": if live { "ready" } else { "released" } });
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
            if let Err(error) = controller.release().await {
                failure.get_or_insert(error);
            }
        }
        match failure {
            Some(error) => Err(ServiceError::Handler(error.to_string())),
            None => Ok(()),
        }
    }
}
