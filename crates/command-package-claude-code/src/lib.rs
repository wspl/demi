//! The `demi.claude-code` command package: Demi's own verified copy of the Claude
//! Code CLI on the machine that runs it, which the runner installs.
//!
//! It is a package of its own: the runner knows nothing about Claude Code, and
//! removing the package and the backend's use of it leaves the rest of Demi as
//! it was. The CLI is an artifact of the package, installed as every artifact
//! is, through the runner's cache; there is no fallback to another version,
//! location or executable. The package picks the platform from the machine,
//! not from the runner's own target, because a runner built for musl runs on
//! glibc machines and the CLI's `linux-*` and `linux-*-musl` builds are not
//! interchangeable; a platform the release does not carry is
//! `unsupported_platform`, never another platform's build. It reads no pointers
//! and chooses no versions: the backend builds the release record, and Demi is
//! the only thing that changes the CLI's version (the CLI's own updater is off).
//!
//! It starts no CLI, and is never linked into the runner or another command
//! program.

pub mod install;
pub mod platform;
pub mod release;

use std::{future::Future, pin::Pin};

use bytes::Bytes;
use demi_command_package_claude_code_protocol::{Failure, Installed, Operation, Reply};
use demi_command_protocol::{CommandError, Completion, Invocation};
use demi_command_sdk::{Artifacts, Handler, Input, InvocationContext, ServiceError};
use serde::Serialize;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use install::EnsureError;

/// A release record is a few hundred bytes for each platform.
const MAX_INPUT_BYTES: usize = 64 * 1024;

/// The service holds no conversation state, so the trait's empty conversation
/// status and its `close` stand.
pub struct DemiClaude {
    /// The service's artifacts source, which it takes before it admits a
    /// call.
    artifacts: watch::Sender<Option<Artifacts>>,
}

impl Default for DemiClaude {
    fn default() -> Self {
        Self {
            artifacts: watch::Sender::new(None),
        }
    }
}

impl Handler for DemiClaude {
    type Metadata = Invocation;

    fn artifacts(&self, artifacts: Artifacts) {
        self.artifacts.send_replace(Some(artifacts));
    }

    fn operations(&self) -> Vec<String> {
        Operation::ALL
            .into_iter()
            .map(|operation| operation.name().to_owned())
            .collect()
    }

    fn invoke(
        &self,
        context: InvocationContext,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        let artifacts = self.artifacts.borrow().clone();
        Box::pin(async move {
            let InvocationContext {
                request,
                input,
                output,
                cancellation,
            } = context;
            let Some(operation) = Operation::parse(&request.operation) else {
                return Err(ServiceError::UnknownOperation(request.operation));
            };
            let result = match artifacts {
                None => Err(EnsureError::InstallFailed(
                    "demi-claude-code has no artifacts source".into(),
                )),
                Some(artifacts) => match operation {
                    Operation::Ensure => {
                        ensure(&artifacts, &request.invocation_id, input, &cancellation)
                            .await
                            .and_then(document)
                    }
                    Operation::Status => install::status(&artifacts).await.and_then(document),
                },
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
    artifacts: &Artifacts,
    invocation: &str,
    input: Input,
    cancel: &CancellationToken,
) -> Result<Installed, EnsureError> {
    let input = read_input(input, cancel).await?;
    let release = install::release(&input)?;
    tokio::select! {
        _ = cancel.cancelled() => Err(EnsureError::Cancelled),
        installed = install::ensure(artifacts, invocation, &release) => installed,
    }
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
