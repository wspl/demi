//! Conversation controllers are retained independently of individual invocations.

use std::{collections::HashMap, sync::Arc};

use bytes::Bytes;
use demi_command_service::{InvocationContext, ServiceError, protocol::Completion};
use tokio::sync::{Mutex, watch};
use tokio_util::sync::{CancellationToken, DropGuard};

use super::{
    BrowserEnvironment, BrowserError, LaunchOptions, Result, installation::Installation,
    operation::after_cleanup, protocol::BrowserCommand, with_browser,
};

enum CommandOutput {
    Json(serde_json::Value),
    Png(Vec<u8>),
}

type Readiness = Option<std::result::Result<BrowserEnvironment, String>>;

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
                    publish.send_replace(Some(Err(error.to_string())));
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
                return result.map(Some).map_err(BrowserError::Configuration);
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

    async fn live(&self) -> bool {
        match &*self.state.lock().await {
            State::Absent => true,
            State::Released => false,
            State::Live { owner, .. } => !owner.is_finished(),
        }
    }

    async fn failure(&self) -> Option<String> {
        match &*self.state.lock().await {
            State::Live { ready, .. } => match &*ready.borrow() {
                Some(Ok(environment)) => environment.failure(),
                Some(Err(error)) => Some(error.clone()),
                None => None,
            },
            _ => None,
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
        context: InvocationContext,
    ) -> std::result::Result<Completion, ServiceError> {
        let operation = context
            .request
            .operation
            .strip_prefix("browser.")
            .ok_or_else(|| ServiceError::Handler("not a browser operation".into()))?;
        let result = async {
            let command = BrowserCommand::parse(operation, context.request.args.clone())
                .map_err(|error| BrowserError::Configuration(error.to_string()))?;
            let cancellation = context.cancellation.child_token();
            let _cancel_on_drop = cancellation.clone().drop_guard();
            let invocation = self.execute(&context, &command, &cancellation);
            tokio::pin!(invocation);
            match tokio::time::timeout(command.timeout(), invocation.as_mut()).await {
                Ok(result) => result,
                Err(_) => {
                    // Cancel the active phase, then join its bounded input/file cleanup.
                    // Dropping the invocation here could strand a pressed button.
                    cancellation.cancel();
                    let cleanup = match invocation.await {
                        Err(BrowserError::Cancelled | BrowserError::Timeout) => Ok(()),
                        result => result.map(|_| ()),
                    };
                    after_cleanup(Err(BrowserError::Timeout), cleanup)
                }
            }
        }
        .await;
        match result {
            Ok(value) => {
                let bytes = match value {
                    CommandOutput::Json(value) => {
                        super::output::render(operation, value, context.request.json == Some(true))
                            .map_err(|error| ServiceError::Handler(error.to_string()))?
                    }
                    CommandOutput::Png(bytes) => bytes,
                };
                context.output.stdout(Bytes::from(bytes)).await?;
                Ok(Completion {
                    exit_code: 0,
                    error: None,
                })
            }
            Err(BrowserError::Cancelled) => Err(ServiceError::Cancelled),
            Err(error) => {
                let code = match error {
                    BrowserError::Busy => "tab_busy",
                    BrowserError::DialogBlocked => "dialog_blocked",
                    BrowserError::DialogNotFound => "dialog_not_found",
                    BrowserError::InvalidDialogAction => "invalid_dialog_action",
                    BrowserError::StaleReference => "stale_ref",
                    BrowserError::Closed => "browser_closed",
                    BrowserError::Timeout => "timeout",
                    BrowserError::Ambiguous(_) => "ambiguous_target",
                    BrowserError::Configuration(_) => "invalid_arguments",
                    _ => "browser_failed",
                };
                let message = if let Ok(controller) = self.controller(&context).await {
                    controller
                        .failure()
                        .await
                        .unwrap_or_else(|| error.to_string())
                } else {
                    error.to_string()
                };
                let bytes = if context.request.json == Some(true) {
                    serde_json::to_vec(
                        &serde_json::json!({ "error": { "code": code, "message": message } }),
                    )?
                } else {
                    format!("{code}: {message:?}\n").into_bytes()
                };
                context.output.stderr(Bytes::from(bytes)).await?;
                Ok(Completion {
                    exit_code: if code == "invalid_arguments" { 2 } else { 1 },
                    error: None,
                })
            }
        }
    }

    async fn execute(
        &self,
        context: &InvocationContext,
        command: &BrowserCommand,
        cancellation: &CancellationToken,
    ) -> Result<CommandOutput> {
        let controller = self.controller(context).await?;
        let environment = controller
            .environment(matches!(command, BrowserCommand::Open(_)), cancellation)
            .await?;
        let Some(environment) = environment else {
            if matches!(command, BrowserCommand::Tabs(_)) {
                return Ok(CommandOutput::Json(
                    serde_json::json!({ "tabs": [], "truncated": false }),
                ));
            }
            return Err(BrowserError::Closed);
        };
        if let BrowserCommand::Open(input) = command {
            let caller = context.request.caller.as_ref().ok_or_else(|| {
                BrowserError::Configuration(
                    "browser caller is missing from trusted invocation".into(),
                )
            })?;
            let tab = environment
                .open_for(&input.url, caller, cancellation, command.timeout())
                .await?;
            return tab
                .metadata(cancellation, command.timeout())
                .await
                .map(CommandOutput::Json);
        }
        let tabs = environment.tabs(cancellation, command.timeout()).await?;
        if matches!(command, BrowserCommand::Tabs(_)) {
            let mut rows = Vec::new();
            for tab in &tabs {
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
                serde_json::json!({ "tabs": rows, "truncated": false }),
            ));
        }
        let tab = tabs
            .iter()
            .find(|tab| Some(tab.id().as_str()) == command.tab())
            .ok_or(BrowserError::Closed)?;
        if let BrowserCommand::Screenshot(input) = command {
            if input.output.is_none() && context.request.json == Some(true) {
                return Err(BrowserError::Configuration(
                    "screenshot --json requires --output".into(),
                ));
            }
            let bytes = tab
                .capture(
                    input.full_page == Some(true),
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
            let path =
                super::output::save(&context.request.cwd, output, bytes, cancellation).await?;
            return Ok(CommandOutput::Json(
                serde_json::json!({ "path": path, "mimeType": "image/png", "width": width, "height": height, "viewport": metadata["viewport"] }),
            ));
        }
        if matches!(command, BrowserCommand::Close(_)) {
            tab.close(cancellation, command.timeout()).await?;
            if environment
                .tabs(cancellation, command.timeout())
                .await?
                .is_empty()
            {
                controller.release().await?;
            }
            return Ok(CommandOutput::Json(
                serde_json::json!({ "closed": tab.id() }),
            ));
        }
        let mut result = tab.command(command, cancellation).await?;
        if let BrowserCommand::ContentRead(input) = command {
            if let Some(output) = &input.output {
                let content = result["content"].as_str().ok_or_else(|| {
                    BrowserError::InvalidResult("content export did not return text".into())
                })?;
                let path = super::output::save(
                    &context.request.cwd,
                    output,
                    content.as_bytes().to_vec(),
                    cancellation,
                )
                .await?;
                result["path"] = serde_json::json!(path);
                result["content"] = serde_json::json!("");
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
            .ok_or_else(|| {
                BrowserError::Configuration("wrong_host: browser scope is missing".into())
            })?;
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
