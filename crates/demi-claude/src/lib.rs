//! The `demi.claude` command package: Demi's own verified copy of the Claude
//! Code CLI on the machine that runs it (`claude-code.md` § The package).

pub mod install;
pub mod platform;
pub mod release;
pub mod version;

#[cfg(test)]
mod tests;

use std::{future::Future, pin::Pin, sync::Arc};

use bytes::Bytes;
use demi_claude_protocol::{Failure, Installed, OPERATIONS, Reply};
use demi_command_service::protocol::{CommandError, Completion, Invocation};
use demi_command_service::{Handler, Input, InvocationContext, ServiceError};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use install::{EnsureError, Installer};

/// A release record is a few hundred bytes for each platform.
const MAX_INPUT_BYTES: usize = 64 * 1024;

/// The service holds no conversation state, so the trait's empty conversation
/// status and its `close` stand.
#[derive(Default)]
pub struct DemiClaude {
    installer: Arc<Installer>,
}

impl DemiClaude {
    pub fn new(installer: Installer) -> Self {
        Self {
            installer: Arc::new(installer),
        }
    }
}

impl Handler for DemiClaude {
    type Metadata = Invocation;

    fn operations(&self) -> Vec<String> {
        OPERATIONS.iter().copied().map(String::from).collect()
    }

    fn invoke(
        &self,
        context: InvocationContext,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        let installer = self.installer.clone();
        Box::pin(async move {
            let InvocationContext {
                request,
                input,
                output,
                cancellation,
            } = context;
            let result = match request.operation.as_str() {
                "claude.ensure" => ensure(&installer, input, &cancellation)
                    .await
                    .and_then(document),
                "claude.status" => installer.status().await.and_then(document),
                operation => {
                    return Err(ServiceError::Handler(format!(
                        "unknown operation {operation}"
                    )));
                }
            };
            let (body, completion) = match result {
                Ok(body) => (
                    body,
                    Completion {
                        exit_code: 0,
                        error: None,
                    },
                ),
                Err(error) => {
                    // Only a cancellation has no code; it writes no document.
                    let Some(code) = error.code() else {
                        return Err(ServiceError::Cancelled);
                    };
                    let message = error.to_string();
                    let body = serde_json::to_vec(&Reply::<()>::Failed(Failure {
                        code,
                        message: message.clone(),
                    }))?;
                    (
                        body,
                        Completion {
                            exit_code: 1,
                            error: Some(CommandError {
                                code: code.to_string(),
                                message,
                            }),
                        },
                    )
                }
            };
            output.stdout(line(body)).await?;
            Ok(completion)
        })
    }
}

async fn ensure(
    installer: &Installer,
    input: Input,
    cancel: &CancellationToken,
) -> Result<Installed, EnsureError> {
    let input = read_input(input, cancel).await?;
    let release = installer.release(&input)?;
    installer.ensure(&release, cancel).await
}

/// Read the invocation's input to its end, refusing more than `MAX_INPUT_BYTES`.
async fn read_input(mut input: Input, cancel: &CancellationToken) -> Result<Vec<u8>, EnsureError> {
    let mut bytes = Vec::new();
    loop {
        let chunk = tokio::select! {
            _ = cancel.cancelled() => return Err(EnsureError::Cancelled),
            chunk = input.next() => chunk,
        };
        match chunk {
            Ok(Some(chunk)) if bytes.len() + chunk.len() <= MAX_INPUT_BYTES => {
                bytes.extend_from_slice(&chunk);
            }
            Ok(Some(_)) => {
                return Err(EnsureError::InvalidRelease(format!(
                    "input exceeds {MAX_INPUT_BYTES} bytes"
                )));
            }
            Ok(None) => return Ok(bytes),
            Err(ServiceError::Cancelled) => return Err(EnsureError::Cancelled),
            Err(error) => return Err(EnsureError::InvalidRelease(error.to_string())),
        }
    }
}

fn document<T: Serialize>(answer: T) -> Result<Vec<u8>, EnsureError> {
    serde_json::to_vec(&Reply::Done(answer))
        .map_err(|error| EnsureError::InstallFailed(error.to_string()))
}

fn line(mut body: Vec<u8>) -> Bytes {
    body.push(b'\n');
    Bytes::from(body)
}
