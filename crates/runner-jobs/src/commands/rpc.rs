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
//!
//! A call the backend marked resumable (`Inbound::RpcResumable`) outlives
//! the session too: it waits for what outlives the backend, such as a
//! vendor's media job. When the session ends, the call is sent again to the
//! next one, under its id and marked `resumed`, unless the runner delivered
//! some of its output meanwhile, which the call would write again.
//!
//! It outlives the runner as well: the caller that asked for it is told the
//! call's id (`demi_command_protocol::Record::Resumable`), and once an
//! update replaced the runner, sends the command again naming it, which the
//! next runner sends as that call, `resumed`.

use crate::{
    commands::command_output::CommandOutput,
    commands::contexts::ExecutionContext,
    connection::{ConnectionHandle, Live, UNREACHABLE},
};
use bytes::Bytes;
use demi_command_declarations::Parsed;
use demi_command_protocol::{MAX_MEDIUM_BYTES, MediumFacts};
use demi_command_sdk::{Input, ResidentError};
use demi_runner_process::pipes::PipeClient;
use demi_runner_protocol::wire::{self, PipeRef};
use futures_util::StreamExt;
use std::{
    collections::BTreeMap,
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

pub struct Request {
    pub context: Arc<ExecutionContext>,
    pub root: String,
    pub argv: Vec<String>,
    pub parsed: Parsed,
    pub cwd: String,
    pub env: BTreeMap<String, String>,
    /// Whether the call's input goes to the backend on a pipe; a leaf that
    /// takes stdin as an argument has it read already.
    pub finite: bool,
    /// Whether the caller is told the call's id once it is resumable.
    pub tells: bool,
    /// The call a runner before this one sent, which this one resumes.
    pub resume: Option<String>,
}

/// What the backend tells a call.
pub enum CallEvent {
    Pipes {
        stdin: Option<PipeRef>,
        stdout: PipeRef,
    },
    Stderr(Bytes),
    /// A medium the call hands its job, its bytes in the pipe.
    Medium(MediumFacts, PipeRef),
    Exit(u8),
    /// The link's session ended, for this reason, so the call cannot end
    /// with its exit.
    Lost(String),
    /// The link's session ended and the call is resumable: it is sent again
    /// to the next one.
    Resume(String),
    /// The backend marked the call resumable.
    Resumable,
}

/// How one session's part of a call ended.
enum Part {
    Exit(u8),
    /// The session ended, for this reason, and the call goes on in the next.
    Resume(String),
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
    let resumed = request.resume.is_some();
    let id = request.resume.clone().unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
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
    let message = |resumed: bool| {
        wire::encode(&wire::Outbound::RpcCall {
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
            resumed,
        })
        .map_err(ResidentError::failed)
    };
    let mut receiver = receiver;
    let result = async {
        // What the job changed of the Host's projects goes first, so the
        // backend knows a worktree the command names (`Backlog::report`).
        backlog.report();
        backlog.push(message(resumed)?);
        let transport = CallTransport {
            id: &id,
            output: &backlog,
            stop: &stop,
        };
        let mut input = Some(input);
        let delivered = AtomicBool::new(false);
        loop {
            let part = exchange(pipes, &request, &mut receiver, &mut input, output, &transport, &delivered).await?;
            match part {
                Part::Exit(code) => return Ok(code),
                // Output delivered would be written again.
                Part::Resume(reason) if delivered.load(Ordering::Relaxed) => {
                    return Err(ResidentError::failed(io::Error::other(reason)));
                }
                Part::Resume(reason) => {
                    tracing::info!(event = "rpc_call.resumed", call = %id, cause = %reason, "a call is sent again to the backend's new session");
                    backlog.push(message(true)?);
                }
            }
        }
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

/// One session's part of a call: its pipes, its events and its standard
/// output, until its exit or the session's end. `delivered` is set once some
/// of the call's output reached the process or its job.
async fn exchange(
    pipes: &PipeClient,
    request: &Request,
    events: &mut mpsc::UnboundedReceiver<CallEvent>,
    input: &mut Option<Input>,
    output: &mut CommandOutput<'_>,
    transport: &CallTransport<'_>,
    delivered: &AtomicBool,
) -> Result<Part, ResidentError> {
    let errors = output.errors();
    let media = output.media();
    // A medium the call handed its job is kept before the standard output
    // the call wrote after it reaches the job: the output waits its turn.
    let turn = tokio::sync::Mutex::new(());
    let &CallTransport {
        id,
        output: connection,
        stop,
    } = transport;
    let (stdin_sender, stdin_receiver) = oneshot::channel();
    let (stdout_sender, stdout_receiver) = oneshot::channel();
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
                CallEvent::Medium(facts, reference) => {
                    let _turn = turn.lock().await;
                    let kept = match medium_bytes(pipes, stop, &reference).await {
                        Ok(bytes) => media.medium(facts, bytes).await.map_err(|error| io::Error::other(error.to_string())),
                        Err(error) => Err(error),
                    };
                    report(connection, &reference, &kept)?;
                    kept.map_err(ResidentError::failed)?;
                    delivered.store(true, Ordering::Relaxed);
                }
                CallEvent::Lost(reason) => {
                    return Err(ResidentError::failed(io::Error::other(reason)));
                }
                CallEvent::Resume(reason) => return Ok(Part::Resume(reason)),
                CallEvent::Resumable if request.tells => errors.resumable(id.to_owned()).await?,
                CallEvent::Resumable => {}
                CallEvent::Exit(code) => {
                    if let Some((stdin, stdout)) = pipe_senders.take() {
                        if code == 0 {
                            return Err(ResidentError::failed(RpcError::SuccessBeforePipes));
                        }
                        stdin.send(None).map_err(|_| ResidentError::Cancelled)?;
                        stdout.send(None).map_err(|_| ResidentError::Cancelled)?;
                    }
                    return Ok(Part::Exit(code));
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
        download(pipes, connection, stop, output, &reference, &turn, delivered).await
    };
    let input = send_input(pipes, stdin_receiver, input, transport);
    // The exit waits for the whole stdout; a session's end leaves the
    // stdout of its pipe unread, the next session's pipe carrying it.
    let complete = async {
        tokio::pin!(control, download);
        let mut downloaded = false;
        loop {
            tokio::select! {
                part = &mut control => match part? {
                    Part::Exit(code) => {
                        if !downloaded {
                            (&mut download).await?;
                        }
                        return Ok(Part::Exit(code));
                    }
                    part @ Part::Resume(_) => return Ok(part),
                },
                result = &mut download, if !downloaded => {
                    result?;
                    downloaded = true;
                }
            }
        }
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
    turn: &tokio::sync::Mutex<()>,
    delivered: &AtomicBool,
) -> Result<(), ResidentError> {
    let mut unwritten = None;
    let result = async {
        let mut stream = pipes.resumed(reference, stop.clone()).await?;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            let _turn = turn.lock().await;
            if let Err(error) = output.stdout(chunk).await {
                let reported = io::Error::other(error.to_string());
                unwritten = Some(error);
                return Err(reported);
            }
            delivered.store(true, Ordering::Relaxed);
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

/// The bytes of a medium the pipe `reference` carries, at most
/// [`MAX_MEDIUM_BYTES`]: a medium is held whole in memory.
async fn medium_bytes(pipes: &PipeClient, stop: &CancellationToken, reference: &PipeRef) -> io::Result<Bytes> {
    let mut bytes = Vec::new();
    let mut stream = pipes.resumed(reference, stop.clone()).await?;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if (bytes.len() + chunk.len()) as u64 > MAX_MEDIUM_BYTES {
            return Err(io::Error::other(format!("a medium is over {MAX_MEDIUM_BYTES} bytes")));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(Bytes::from(bytes))
}

/// Sends the call's input to the pipe `reference` names, once: a call
/// whose input is a pipe is never resumed.
async fn send_input(
    pipes: &PipeClient,
    reference: oneshot::Receiver<Option<PipeRef>>,
    input: &mut Option<Input>,
    transport: &CallTransport<'_>,
) -> Result<(), ResidentError> {
    let &CallTransport {
        output: connection,
        stop,
        ..
    } = transport;
    if let Some(reference) = reference.await.map_err(|_| ResidentError::Cancelled)? {
        let input = input.take().ok_or_else(|| ResidentError::failed(RpcError::DuplicatePipes))?;
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
        error: result.as_ref().err().map(|error| demi_runner_process::pipes::causes(error)),
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
    #[error("RPC success arrived before pipe descriptors")]
    SuccessBeforePipes,
}
