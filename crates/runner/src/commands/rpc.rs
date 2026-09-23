//! Application callbacks (`commands.md`): a declared command the backend
//! implements runs as a call over the job's connection, with its input and
//! output on pipes. The connection's owner routes the call's inbound events
//! to it; the call itself sends its requests.

use demi_command_tree::Parsed;
use crate::connection::wire::{self as wire, PipeRef};
use crate::{
    commands::command_output::CommandOutput, commands::contexts::ExecutionContext,
    connection::ConnectionHandle, pipes::PipeClient,
};
use bytes::Bytes;
use demi_command_service::{Input, ServiceError};
use futures_util::StreamExt;
use std::{collections::BTreeMap, io, sync::Arc, time::Duration};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

/// A call's queue of inbound events; past it the call ends.
const EVENTS: usize = 8;

pub struct Request {
    pub context: Arc<ExecutionContext>,
    pub root: String,
    pub argv: Vec<String>,
    pub parsed: Parsed,
    pub cwd: String,
    pub env: BTreeMap<String, String>,
    pub live: bool,
    pub finite: bool,
}

/// What the backend tells a call.
pub enum CallEvent {
    Pipes {
        stdin: Option<PipeRef>,
        stdout: PipeRef,
    },
    Stderr(Bytes),
    Pull,
    Exit(u8),
}

struct CallTransport<'a> {
    id: &'a str,
    output: &'a mpsc::Sender<wire::Frame>,
    stop: &'a CancellationToken,
}

/// Ends a call that did not complete: the backend is told to cancel it.
struct Pending<'a> {
    id: String,
    connection: &'a ConnectionHandle,
    ended: CancellationToken,
    completed: bool,
}

impl Drop for Pending<'_> {
    fn drop(&mut self) {
        self.ended.cancel();
        if self.completed {
            return;
        }
        match wire::encode(&wire::Outbound::RpcCancel {
            call_id: self.id.clone(),
        }) {
            Ok(message) => {
                if self.connection.control.try_send(message).is_err() {
                    self.connection.closed().cancel();
                }
            }
            // A dropped cancellation message could leave callback work live.
            // Closing the transport makes backend teardown authoritative.
            _ => self.connection.closed().cancel(),
        }
    }
}

/// A hint belongs to one invocation and is cleared even if its future is dropped.
pub struct RunningHint {
    clear: Option<wire::Frame>,
    connection: ConnectionHandle,
}

impl Drop for RunningHint {
    fn drop(&mut self) {
        if let Some(message) = self.clear.take()
            && self.connection.control.try_send(message).is_err()
        {
            // Backend disconnection clears job hints if a clear cannot be queued.
            self.connection.closed().cancel();
        }
    }
}

/// Shows `hint` on the job while the returned guard lives.
pub async fn running_hint(
    connection: &ConnectionHandle,
    job_id: &str,
    hint: Option<&str>,
) -> Result<Option<RunningHint>, ServiceError> {
    let Some(hint) = hint else {
        return Ok(None);
    };
    let id = uuid::Uuid::new_v4().simple().to_string();
    let set = wire::encode(&wire::Outbound::JobRunningHint {
        job_id: job_id.into(),
        invocation_id: id.clone(),
        hint: Some(hint.into()),
    })
    .map_err(handler)?;
    let clear = wire::encode(&wire::Outbound::JobRunningHint {
        job_id: job_id.into(),
        invocation_id: id,
        hint: None,
    })
    .map_err(handler)?;
    let guard = RunningHint {
        clear: Some(clear),
        connection: connection.clone(),
    };
    connection
        .control
        .send(set)
        .await
        .map_err(|_| ServiceError::Cancelled)?;
    Ok(Some(guard))
}

/// Runs one call on the context's connection and returns its exit code.
pub async fn invoke(
    pipes: &PipeClient,
    request: Request,
    input: Input,
    output: CommandOutput,
    cancel: CancellationToken,
) -> Result<u8, ServiceError> {
    let connection = request.context.connection.clone();
    let id = uuid::Uuid::new_v4().simple().to_string();
    let stop = cancel.child_token();
    let _stop_guard = stop.clone().drop_guard();
    let (events, receiver) = mpsc::channel(EVENTS);
    if !connection
        .register_call(id.clone(), events, stop.clone())
        .await
    {
        return Err(ServiceError::Cancelled);
    }
    let mut pending = Pending {
        id: id.clone(),
        connection: &connection,
        ended: stop.clone(),
        completed: false,
    };
    let message = wire::encode(&wire::Outbound::RpcCall {
        job_id: request.context.job_id.clone(),
        call_id: id.clone(),
        root: request.root.clone(),
        path: request.parsed.path.clone(),
        argv: request.argv.clone(),
        args: request.parsed.values.clone().into_iter().collect(),
        json: request.parsed.json,
        cwd: request.cwd.clone(),
        env: request.env.clone(),
        stdin: request.finite,
    })
    .map_err(handler)?;
    let result = async {
        connection
            .control
            .send(message)
            .await
            .map_err(|_| ServiceError::Cancelled)?;
        if !request.live {
            connection
                .control
                .send(stdin_end(&id)?)
                .await
                .map_err(|_| ServiceError::Cancelled)?;
        }
        let transport = CallTransport {
            id: &id,
            output: &connection.control,
            stop: &stop,
        };
        exchange(pipes, &request, receiver, input, output, &transport).await
    };
    let result = tokio::select! {
        biased;
        _ = request.context.cancel.cancelled() => Err(ServiceError::Cancelled),
        _ = stop.cancelled() => Err(ServiceError::Cancelled),
        result = result => result,
    };
    pending.completed = result.is_ok();
    result
}

async fn exchange(
    pipes: &PipeClient,
    request: &Request,
    mut events: mpsc::Receiver<CallEvent>,
    input: Input,
    output: CommandOutput,
    transport: &CallTransport<'_>,
) -> Result<u8, ServiceError> {
    let &CallTransport {
        id,
        output: connection,
        stop,
    } = transport;
    let (stdin_sender, stdin_receiver) = oneshot::channel();
    let (stdout_sender, stdout_receiver) = oneshot::channel();
    let (pull, demanded) = mpsc::channel(1);
    let control = async {
        let mut pipe_senders = Some((stdin_sender, stdout_sender));
        while let Some(event) = events.recv().await {
            match event {
                CallEvent::Pipes { stdin, stdout } => {
                    let (stdin_sender, stdout_sender) = pipe_senders
                        .take()
                        .ok_or_else(|| ServiceError::Handler("duplicate RPC pipes".into()))?;
                    if stdin.is_some() != request.finite {
                        return Err(ServiceError::Handler(
                            "RPC input pipe disagrees with invocation".into(),
                        ));
                    }
                    stdin_sender
                        .send(stdin)
                        .map_err(|_| ServiceError::Cancelled)?;
                    stdout_sender
                        .send(Some(stdout))
                        .map_err(|_| ServiceError::Cancelled)?;
                }
                CallEvent::Stderr(bytes) => output.stderr(bytes).await?,
                CallEvent::Pull if request.live => pull.try_send(()).map_err(|_| {
                    ServiceError::Handler("overlapping RPC stdin demands".into())
                })?,
                CallEvent::Pull => connection
                    .send(stdin_end(id)?)
                    .await
                    .map_err(|_| ServiceError::Cancelled)?,
                CallEvent::Exit(code) => {
                    if let Some((stdin, stdout)) = pipe_senders.take() {
                        if code == 0 {
                            return Err(ServiceError::Handler(
                                "RPC success arrived before pipe descriptors".into(),
                            ));
                        }
                        stdin.send(None).map_err(|_| ServiceError::Cancelled)?;
                        stdout.send(None).map_err(|_| ServiceError::Cancelled)?;
                    }
                    return Ok(code);
                }
            }
        }
        Err(ServiceError::Cancelled)
    };
    let download = async {
        let Some(reference): Option<PipeRef> =
            stdout_receiver.await.map_err(|_| ServiceError::Cancelled)?
        else {
            return Ok(());
        };
        let result = async {
            let mut stream = pipes.get(&reference.url, stop.clone()).await?;
            while let Some(bytes) = stream.next().await {
                output.stdout(bytes?).await.map_err(io::Error::other)?;
            }
            Ok::<_, io::Error>(())
        }
        .await;
        report(connection, &reference, &result, stop).await?;
        result.map_err(handler)
    };
    let input = send_input(pipes, request.live, stdin_receiver, demanded, input, transport);
    let complete = async {
        let (code, ()) = tokio::try_join!(control, download)?;
        Ok(code)
    };
    tokio::pin!(complete);
    tokio::select! {
        result = &mut complete => result,
        result = input => {
            match result {
                Ok(()) => complete.await,
                Err(error) => {
                    // A callback may finish successfully without consuming its
                    // finite pipe; the broker then closes that unused upload.
                    // Give its ordered exit a bounded opportunity to arrive.
                    tokio::time::timeout(Duration::from_secs(15), complete).await.unwrap_or(Err(error))
                }
            }
        }
    }
}

async fn send_input(
    pipes: &PipeClient,
    live: bool,
    reference: oneshot::Receiver<Option<PipeRef>>,
    mut demanded: mpsc::Receiver<()>,
    mut input: Input,
    transport: &CallTransport<'_>,
) -> Result<(), ServiceError> {
    let &CallTransport {
        id,
        output: connection,
        stop,
    } = transport;
    if live {
        let _reference = reference.await.map_err(|_| ServiceError::Cancelled)?;
        while demanded.recv().await.is_some() {
            let message = match input.next().await? {
                Some(bytes) => wire::encode(&wire::Outbound::RpcStdin {
                    call_id: id.into(),
                    bytes: wire::WireBytes(bytes.to_vec()),
                }),
                None => {
                    connection
                        .send(stdin_end(id)?)
                        .await
                        .map_err(|_| ServiceError::Cancelled)?;
                    return Ok(());
                }
            }
            .map_err(handler)?;
            connection
                .send(message)
                .await
                .map_err(|_| ServiceError::Cancelled)?;
        }
        return Ok(());
    }
    if let Some(reference) = reference.await.map_err(|_| ServiceError::Cancelled)? {
        let token = stop.clone();
        let body =
            futures_util::stream::unfold((input, token), |(mut input, stop)| async move {
                tokio::select! {
                    _ = stop.cancelled() => None,
                    bytes = input.next() => match bytes {
                        Ok(Some(bytes)) => Some((Ok(bytes), (input, stop))),
                        Ok(None) => None,
                        Err(error) => Some((Err(io::Error::other(error)), (input, stop))),
                    }
                }
            });
        let result = pipes.put(&reference.url, body, stop).await;
        report(connection, &reference, &result, stop).await?;
        result.map_err(handler)?;
    }
    Ok(())
}

async fn report(
    output: &mpsc::Sender<wire::Frame>,
    reference: &PipeRef,
    result: &io::Result<()>,
    stop: &CancellationToken,
) -> Result<(), ServiceError> {
    let message = wire::encode(&wire::Outbound::PipeDone {
        pipe_id: reference.id.clone(),
        ok: result.is_ok(),
        error: result.as_ref().err().map(ToString::to_string),
    })
    .map_err(handler)?;
    tokio::select! {
        _ = stop.cancelled() => Err(ServiceError::Cancelled),
        result = output.send(message) => result.map_err(|_| ServiceError::Cancelled),
    }
}
fn handler(error: impl std::fmt::Display) -> ServiceError {
    ServiceError::Handler(error.to_string())
}

/// The frame that ends a call's standard input.
fn stdin_end(call_id: &str) -> Result<wire::Frame, ServiceError> {
    wire::encode(&wire::Outbound::RpcStdinEnd {
        call_id: call_id.into(),
    })
    .map_err(handler)
}
