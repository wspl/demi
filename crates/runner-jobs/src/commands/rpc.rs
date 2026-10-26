//! Application callbacks (`crates/command-declarations/src/lib.rs`): a declared command the backend
//! implements runs as a call over the job's connection, with its input and
//! output on pipes. The connection's owner routes the call's inbound events
//! to it; the call itself sends its requests.
//!
//! A call outlives the connections it runs on: its messages wait in the
//! link's backlog for the next one, its standard output resumes where the
//! runner got to, and it ends with its exit or with the reason the link's
//! session ended (`crates/runner/src/connection/link_state.rs`). Its
//! standard input, when it is a pipe, is cut with the connection it goes
//! over, which fails the call.

use crate::{
    commands::command_output::CommandOutput,
    commands::contexts::ExecutionContext,
    connection::{ConnectionHandle, Live, UNREACHABLE},
};
use bytes::Bytes;
use demi_command_declarations::Parsed;
use demi_command_sdk::{Input, ResidentError};
use demi_runner_process::pipes::PipeClient;
use demi_runner_protocol::wire::{self, PipeRef};
use futures_util::StreamExt;
use std::{collections::BTreeMap, io, sync::Arc};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

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
    /// The link's session ended, for this reason, so the call cannot end
    /// with its exit.
    Lost(String),
}

struct CallTransport<'a> {
    id: &'a str,
    output: &'a wire::Backlog,
    stop: &'a CancellationToken,
}

/// Ends a call that did not complete: the backend is told to cancel it.
struct Pending<'a> {
    id: String,
    backlog: &'a wire::Backlog,
    ended: CancellationToken,
    completed: bool,
}

impl Drop for Pending<'_> {
    fn drop(&mut self) {
        self.ended.cancel();
        if self.completed {
            return;
        }
        // A cancellation of a call the backend does not know any more, as one
        // of an ended session, is ignored there.
        match wire::encode(&wire::Outbound::RpcCancel {
            call_id: self.id.clone(),
        }) {
            Ok(message) => self.backlog.push(message),
            Err(error) => {
                tracing::warn!(event = "reply.encode_failed", what = "rpc_cancel", %error, "a call's cancellation was not encoded");
            }
        }
    }
}

/// A hint belongs to one invocation and is cleared even if its future is dropped.
pub struct RunningHint {
    clear: Option<wire::Frame>,
    connection: Live,
}

impl Drop for RunningHint {
    fn drop(&mut self) {
        if let Some(message) = self.clear.take()
            && self.connection.control.send(message).is_err()
        {
            // Backend disconnection clears job hints if a clear cannot be queued.
            self.connection.closed.cancel();
        }
    }
}

/// Shows `hint` on the job while the returned guard lives, on the
/// connection that serves now; with none, the backend shows no hint.
pub async fn running_hint(
    connection: &ConnectionHandle,
    job_id: &str,
    hint: Option<&str>,
) -> Result<Option<RunningHint>, ResidentError> {
    let (Some(hint), Some(connection)) = (hint, connection.current()) else {
        return Ok(None);
    };
    let id = uuid::Uuid::new_v4().simple().to_string();
    let set = wire::encode(&wire::Outbound::JobRunningHint {
        job_id: job_id.into(),
        invocation_id: id.clone(),
        hint: Some(hint.into()),
    })
    .map_err(ResidentError::failed)?;
    let clear = wire::encode(&wire::Outbound::JobRunningHint {
        job_id: job_id.into(),
        invocation_id: id,
        hint: None,
    })
    .map_err(ResidentError::failed)?;
    let guard = RunningHint {
        clear: Some(clear),
        connection: connection.clone(),
    };
    connection
        .control
        .send(set)
        .map_err(|_| ResidentError::Cancelled)?;
    Ok(Some(guard))
}

/// Runs one call over the connections that serve and returns its exit code;
/// while the runner is away, a call not yet sent waits for the next
/// connection, and one in flight goes on. It fails once the runner ends
/// (`crates/runner/src/connection/kept.rs`) or its session does.
pub async fn invoke(
    pipes: &PipeClient,
    request: Request,
    input: Input,
    output: &mut CommandOutput<'_>,
    cancel: CancellationToken,
) -> Result<u8, ResidentError> {
    let handle = request.context.connection.clone();
    tokio::select! {
        biased;
        _ = request.context.cancel.cancelled() => return Err(ResidentError::Cancelled),
        _ = cancel.cancelled() => return Err(ResidentError::Cancelled),
        connected = handle.connected() => connected
            .map_err(|_| ResidentError::failed(io::Error::other(UNREACHABLE)))?,
    };
    let backlog = handle.backlog().clone();
    let id = uuid::Uuid::new_v4().simple().to_string();
    let stop = cancel.child_token();
    let _stop_guard = stop.clone().drop_guard();
    let (events, receiver) = mpsc::unbounded_channel();
    if !handle
        .register_call(id.clone(), events, stop.clone())
    {
        return Err(ResidentError::Cancelled);
    }
    let mut pending = Pending {
        id: id.clone(),
        backlog: &backlog,
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
    .map_err(ResidentError::failed)?;
    let result = async {
        // What the job changed of the Host's projects goes first, so the
        // backend knows a worktree the command names (`Backlog::report`).
        backlog.report();
        backlog.push(message);
        if !request.live {
            backlog.push(stdin_end(&id)?);
        }
        let transport = CallTransport {
            id: &id,
            output: &backlog,
            stop: &stop,
        };
        exchange(pipes, &request, receiver, input, output, &transport).await
    };
    // The call goes on across the connections it runs on, whichever of them
    // carry its messages.
    let result = tokio::select! {
        biased;
        _ = request.context.cancel.cancelled() => Err(ResidentError::Cancelled),
        _ = stop.cancelled() => Err(ResidentError::Cancelled),
        result = result => result,
    };
    pending.completed = result.is_ok();
    result
}

async fn exchange(
    pipes: &PipeClient,
    request: &Request,
    mut events: mpsc::UnboundedReceiver<CallEvent>,
    input: Input,
    output: &mut CommandOutput<'_>,
    transport: &CallTransport<'_>,
) -> Result<u8, ResidentError> {
    let errors = output.errors();
    let &CallTransport {
        id,
        output: connection,
        stop,
    } = transport;
    let (stdin_sender, stdin_receiver) = oneshot::channel();
    let (stdout_sender, stdout_receiver) = oneshot::channel();
    // One demand at a time: the backend asks for input only after it has
    // taken the last, and a second is a protocol violation.
    let (pull, demanded) = mpsc::channel(1);
    let control = async {
        let mut pipe_senders = Some((stdin_sender, stdout_sender));
        while let Some(event) = events.recv().await {
            match event {
                CallEvent::Pipes { stdin, stdout } => {
                    let (stdin_sender, stdout_sender) = pipe_senders
                        .take()
                        .ok_or_else(|| ResidentError::failed(RpcError::DuplicatePipes))?;
                    if stdin.is_some() != request.finite {
                        return Err(ResidentError::failed(RpcError::InputDisagrees));
                    }
                    stdin_sender
                        .send(stdin)
                        .map_err(|_| ResidentError::Cancelled)?;
                    stdout_sender
                        .send(Some(stdout))
                        .map_err(|_| ResidentError::Cancelled)?;
                }
                CallEvent::Stderr(bytes) => errors.stderr(bytes).await?,
                CallEvent::Pull if request.live => pull
                    .try_send(())
                    .map_err(|_| ResidentError::failed(RpcError::OverlappingDemands))?,
                CallEvent::Pull => connection.push(stdin_end(id)?),
                CallEvent::Lost(reason) => {
                    return Err(ResidentError::failed(io::Error::other(reason)));
                }
                CallEvent::Exit(code) => {
                    if let Some((stdin, stdout)) = pipe_senders.take() {
                        if code == 0 {
                            return Err(ResidentError::failed(RpcError::SuccessBeforePipes));
                        }
                        stdin.send(None).map_err(|_| ResidentError::Cancelled)?;
                        stdout.send(None).map_err(|_| ResidentError::Cancelled)?;
                    }
                    return Ok(code);
                }
            }
        }
        Err(ResidentError::Cancelled)
    };
    let download = async {
        let Some(reference): Option<PipeRef> =
            stdout_receiver.await.map_err(|_| ResidentError::Cancelled)?
        else {
            return Ok(());
        };
        download(pipes, connection, stop, output, &reference).await
    };
    let input = send_input(
        pipes,
        request.live,
        stdin_receiver,
        demanded,
        input,
        transport,
    );
    let complete = async {
        let (code, ()) = tokio::try_join!(control, download)?;
        Ok(code)
    };
    // The call's exit and its whole stdout own its lifetime: its input
    // ending, read or unread, ends nothing else, however long the stdout
    // takes to reach a slow process (`crates/host-interface/src/rpc.rs`).
    // Only an input that cannot be read or sent ends the call early.
    tokio::pin!(complete);
    tokio::select! {
        biased;
        result = &mut complete => result,
        result = input => {
            result?;
            complete.await
        }
    }
}

/// Reads the call's stdout pipe `reference` to its end into `output`, and
/// reports how the pipe ended.
///
/// A failure of the pipe itself, such as a stdout handed to a job on another
/// device that went away, also ends the call on the backend, whose exit then
/// ends it with the cause on stderr (`crates/host-interface/src/rpc.rs`). Only
/// a failure to write the output ends it here.
async fn download(
    pipes: &PipeClient,
    connection: &wire::Backlog,
    stop: &CancellationToken,
    output: &mut CommandOutput<'_>,
    reference: &PipeRef,
) -> Result<(), ResidentError> {
    let mut unwritten = None;
    let result = async {
        let mut stream = pipes.resumed(reference, stop.clone()).await?;
        while let Some(chunk) = stream.next().await {
            if let Err(error) = output.stdout(chunk?).await {
                let reported = io::Error::other(error.to_string());
                unwritten = Some(error);
                return Err(reported);
            }
        }
        Ok::<_, io::Error>(())
    }
    .await;
    report(connection, reference, &result)?;
    match unwritten {
        Some(error) => Err(ResidentError::failed(io::Error::other(error))),
        None => Ok(()),
    }
}

async fn send_input(
    pipes: &PipeClient,
    live: bool,
    reference: oneshot::Receiver<Option<PipeRef>>,
    mut demanded: mpsc::Receiver<()>,
    mut input: Input,
    transport: &CallTransport<'_>,
) -> Result<(), ResidentError> {
    let &CallTransport {
        id,
        output: connection,
        stop,
    } = transport;
    if live {
        let _reference = reference.await.map_err(|_| ResidentError::Cancelled)?;
        while demanded.recv().await.is_some() {
            let message = match input.next().await? {
                Some(bytes) => wire::encode(&wire::Outbound::RpcStdin {
                    call_id: id.into(),
                    bytes: wire::WireBytes(bytes.to_vec()),
                }),
                None => {
                    connection.push(stdin_end(id)?);
                    return Ok(());
                }
            }
            .map_err(ResidentError::failed)?;
            connection.push(message);
        }
        return Ok(());
    }
    if let Some(reference) = reference.await.map_err(|_| ResidentError::Cancelled)? {
        let token = stop.clone();
        let body = futures_util::stream::unfold((input, token), |(mut input, stop)| async move {
            tokio::select! {
                _ = stop.cancelled() => None,
                bytes = input.next() => match bytes {
                    Ok(Some(bytes)) => Some((Ok(bytes), (input, stop))),
                    Ok(None) => None,
                    Err(error) => Some((Err(io::Error::other(error)), (input, stop))),
                }
            }
        });
        // A stream pipe: the call's input must reach the backend as it is
        // read, however long it takes (`crates/runner/src/main.rs`).
        let result = pipes.stream(&reference, body, stop).await;
        // A failed upload ends nothing here: the backend hears of it from
        // the report, which fails a pipe still open and with it the call,
        // whose exit carries the cause; a pipe already over, as the input of
        // a call that ended is, stays as it ended.
        report(connection, &reference, &result)?;
    }
    Ok(())
}

fn report(
    output: &wire::Backlog,
    reference: &PipeRef,
    result: &io::Result<()>,
) -> Result<(), ResidentError> {
    let message = wire::encode(&wire::Outbound::PipeDone {
        pipe_id: reference.id.clone(),
        ok: result.is_ok(),
        error: result.as_ref().err().map(ToString::to_string),
    })
    .map_err(ResidentError::failed)?;
    output.push(message);
    Ok(())
}
/// A call's relay broke the rpc protocol (`crates/host-interface/src/rpc.rs`).
#[derive(Debug, thiserror::Error)]
enum RpcError {
    #[error("duplicate RPC pipes")]
    DuplicatePipes,
    #[error("RPC input pipe disagrees with invocation")]
    InputDisagrees,
    #[error("overlapping RPC stdin demands")]
    OverlappingDemands,
    #[error("RPC success arrived before pipe descriptors")]
    SuccessBeforePipes,
}

/// The frame that ends a call's standard input.
fn stdin_end(call_id: &str) -> Result<wire::Frame, ResidentError> {
    wire::encode(&wire::Outbound::RpcStdinEnd {
        call_id: call_id.into(),
    })
    .map_err(ResidentError::failed)
}
