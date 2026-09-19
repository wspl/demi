//! Execution-target implementations for Demi builtin commands.

pub mod browser;

mod files;
mod patch;

use std::{future::Future, pin::Pin, sync::Arc};

use bytes::Bytes;
use demi_command_service::protocol::{CommandError, Completion, Invocation};
use demi_command_service::{ConversationContext, Handler, InvocationContext, ServiceError};
use tokio::{io::AsyncReadExt, sync::Mutex};

#[derive(Default)]
pub struct DemiCommands {
    mutations: Arc<Mutex<()>>,
    browsers: Arc<browser::Conversations>,
}

impl Handler for DemiCommands {
    type Metadata = Invocation;

    fn conversation(
        &self,
        context: ConversationContext,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        let browsers = self.browsers.clone();
        Box::pin(async move { browsers.conversation(context).await })
    }

    fn close(&self) -> Pin<Box<dyn Future<Output = Result<(), ServiceError>> + Send>> {
        let browsers = self.browsers.clone();
        Box::pin(async move { browsers.close().await })
    }

    fn operations(&self) -> Vec<String> {
        ["file.read", "file.create", "file.edit", "file.patch"]
            .into_iter()
            .chain(browser::OPERATIONS.iter().copied())
            .chain([browser::LIVE_OPERATION])
            .map(String::from)
            .collect()
    }

    fn invoke(
        &self,
        context: InvocationContext,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        if context.request.operation.starts_with("browser.") {
            let browsers = self.browsers.clone();
            return Box::pin(async move { browsers.invoke(context).await });
        }
        let mutations = self.mutations.clone();
        Box::pin(async move {
            let result = if context.request.operation == "file.read" {
                read(&context).await
            } else {
                let guard = tokio::select! {
                    _ = context.cancellation.cancelled() => return Err(ServiceError::Cancelled),
                    guard = mutations.lock_owned() => guard,
                };
                let request = context.request.clone();
                let cancellation = context.cancellation.clone();
                let result = tokio::task::spawn_blocking(move || {
                    // The lock covers planning, writes and rollback, including cancellation.
                    let _guard = guard;
                    let recorder = request.edits.clone().and_then(|context| {
                        match demi_command_service::edits::Recorder::new(context) {
                            Ok(recorder) => Some(recorder),
                            Err(error) => {
                                eprintln!("edit recording failed: {error}");
                                None
                            }
                        }
                    });
                    let mut recording = recorder.as_ref().and_then(|recorder| recorder.begin());
                    files::mutate(&request, &cancellation, recording.as_mut())
                })
                .await
                .map_err(|error| ServiceError::Handler(error.to_string()))?;
                match result {
                    Ok(message) => context.output.stdout(Bytes::from(message)).await,
                    Err(error) => Err(ServiceError::Handler(error)),
                }
            };
            match result {
                Ok(()) => Ok(Completion {
                    exit_code: 0,
                    error: None,
                }),
                Err(ServiceError::Cancelled) => Err(ServiceError::Cancelled),
                Err(error) => Ok(Completion {
                    exit_code: 1,
                    error: Some(CommandError {
                        code: "command_failed".into(),
                        message: error.to_string(),
                    }),
                }),
            }
        })
    }
}

async fn read(context: &InvocationContext) -> Result<(), ServiceError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        path: String,
    }
    let args: Args = serde_json::from_value(context.request.args.clone())?;
    let path =
        files::resolve_path(&context.request.cwd, &args.path).map_err(ServiceError::Handler)?;
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|error| ServiceError::Handler(error.to_string()))?;
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let count = tokio::select! {
            _ = context.cancellation.cancelled() => return Err(ServiceError::Cancelled),
            result = file.read(&mut buffer) => result.map_err(|error| ServiceError::Handler(error.to_string()))?,
        };
        if count == 0 {
            return Ok(());
        }
        context
            .output
            .stdout(Bytes::copy_from_slice(&buffer[..count]))
            .await?;
    }
}
