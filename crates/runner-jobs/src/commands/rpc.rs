//! Application callbacks (`commands.md`): a declared command the backend
//! implements runs as a call over the job's connection, with its input and
//! output on pipes. The connection's owner routes the call's inbound events
//! to it; the call itself sends its requests.

use crate::{
    commands::command_output::CommandOutput,
    commands::contexts::ExecutionContext,
    connection::{ConnectionHandle, Live, UNREACHABLE},
};
use bytes::{Bytes, BytesMut};
use demi_command_declarations::Parsed;
use demi_command_sdk::{Input, ServiceError};
use demi_runner_process::pipes::PipeClient;
use demi_runner_protocol::wire::{self, PipeRef};
use futures_util::StreamExt;
use std::{
    collections::{BTreeMap, VecDeque},
    io,
    sync::Arc,
    time::Duration,
};
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
    /// The handler returned a medium (`commands.md` § Return media).
    Medium(RpcMedium),
    Pull,
    Exit(u8),
}

/// A medium an `rpc` handler returned: it goes after the first `after`
/// bytes of the call's stdout, and its `size` bytes flow through `pipe`.
pub struct RpcMedium {
    pub after: u64,
    pub size: u64,
    pub pipe: PipeRef,
}

struct CallTransport<'a> {
    id: &'a str,
    output: &'a mpsc::Sender<wire::Frame>,
    stop: &'a CancellationToken,
}

/// Ends a call that did not complete: the backend is told to cancel it.
struct Pending<'a> {
    id: String,
    connection: &'a Live,
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
                    self.connection.closed.cancel();
                }
            }
            // A dropped cancellation message could leave callback work live.
            // Closing the transport makes backend teardown authoritative.
            _ => self.connection.closed.cancel(),
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
            && self.connection.control.try_send(message).is_err()
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
) -> Result<Option<RunningHint>, ServiceError> {
    let (Some(hint), Some(connection)) = (hint, connection.current()) else {
        return Ok(None);
    };
    let id = uuid::Uuid::new_v4().simple().to_string();
    let set = wire::encode(&wire::Outbound::JobRunningHint {
        job_id: job_id.into(),
        invocation_id: id.clone(),
        hint: Some(hint.into()),
    })
    .map_err(ServiceError::failed)?;
    let clear = wire::encode(&wire::Outbound::JobRunningHint {
        job_id: job_id.into(),
        invocation_id: id,
        hint: None,
    })
    .map_err(ServiceError::failed)?;
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

/// Runs one call on the connection that serves and returns its exit code;
/// while the runner is away, the call waits for its next connection, and
/// fails once the runner stopped waiting (`runner.md` § Command lifetime).
pub async fn invoke(
    pipes: &PipeClient,
    request: Request,
    input: Input,
    output: &mut CommandOutput<'_>,
    cancel: CancellationToken,
) -> Result<u8, ServiceError> {
    let handle = request.context.connection.clone();
    let connection = tokio::select! {
        biased;
        _ = request.context.cancel.cancelled() => return Err(ServiceError::Cancelled),
        _ = cancel.cancelled() => return Err(ServiceError::Cancelled),
        connected = handle.connected() => connected
            .map_err(|_| ServiceError::failed(io::Error::other(UNREACHABLE)))?,
    };
    let id = uuid::Uuid::new_v4().simple().to_string();
    let stop = cancel.child_token();
    let _stop_guard = stop.clone().drop_guard();
    let (events, receiver) = mpsc::channel(EVENTS);
    if !handle
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
    .map_err(ServiceError::failed)?;
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
        // The call ends with the connection it runs on.
        _ = connection.closed.cancelled() => Err(ServiceError::Cancelled),
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
    output: &mut CommandOutput<'_>,
    transport: &CallTransport<'_>,
) -> Result<u8, ServiceError> {
    let errors = output.errors();
    let &CallTransport {
        id,
        output: connection,
        stop,
    } = transport;
    let (stdin_sender, stdin_receiver) = oneshot::channel();
    let (stdout_sender, stdout_receiver) = oneshot::channel();
    let (pull, demanded) = mpsc::channel(1);
    let (media, returned) = mpsc::channel(EVENTS);
    let control = async {
        let mut pipe_senders = Some((stdin_sender, stdout_sender));
        // Dropped with this future: the call returns no media after its exit.
        let media = media;
        while let Some(event) = events.recv().await {
            match event {
                CallEvent::Pipes { stdin, stdout } => {
                    let (stdin_sender, stdout_sender) = pipe_senders
                        .take()
                        .ok_or_else(|| ServiceError::failed(RpcError::DuplicatePipes))?;
                    if stdin.is_some() != request.finite {
                        return Err(ServiceError::failed(RpcError::InputDisagrees));
                    }
                    stdin_sender
                        .send(stdin)
                        .map_err(|_| ServiceError::Cancelled)?;
                    stdout_sender
                        .send(Some(stdout))
                        .map_err(|_| ServiceError::Cancelled)?;
                }
                CallEvent::Stderr(bytes) => errors.stderr(bytes).await?,
                CallEvent::Medium(medium) => {
                    if pipe_senders.is_some() {
                        return Err(ServiceError::failed(RpcError::MediumBeforePipes));
                    }
                    media
                        .send(medium)
                        .await
                        .map_err(|_| ServiceError::Cancelled)?;
                }
                CallEvent::Pull if request.live => pull
                    .try_send(())
                    .map_err(|_| ServiceError::failed(RpcError::OverlappingDemands))?,
                CallEvent::Pull => connection
                    .send(stdin_end(id)?)
                    .await
                    .map_err(|_| ServiceError::Cancelled)?,
                CallEvent::Exit(code) => {
                    if let Some((stdin, stdout)) = pipe_senders.take() {
                        if code == 0 {
                            return Err(ServiceError::failed(RpcError::SuccessBeforePipes));
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
        let call = Download {
            pipes,
            connection,
            stop,
            output,
            pending: VecDeque::new(),
            written: 0,
        };
        call.run(&reference, returned).await
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

/// A call's stdout pipe, as its download sees it.
enum Stdout {
    Opening,
    Open(futures_util::stream::BoxStream<'static, io::Result<Bytes>>),
    Ended,
}

impl Stdout {
    /// The next chunk of an open pipe.
    async fn next(&mut self) -> Option<io::Result<Bytes>> {
        match self {
            Self::Open(stream) => stream.next().await,
            Self::Opening | Self::Ended => None,
        }
    }
}

/// A call's stdout as the calling process receives it: the pipe's bytes,
/// with each medium the handler returned placed after the bytes it followed.
struct Download<'a, 'o> {
    pipes: &'a PipeClient,
    connection: &'a mpsc::Sender<wire::Frame>,
    stop: &'a CancellationToken,
    output: &'a mut CommandOutput<'o>,
    /// The media that wait for their place in stdout, in the order returned.
    pending: VecDeque<RpcMedium>,
    /// The stdout bytes passed on so far.
    written: u64,
}

impl Download<'_, '_> {
    /// Reads the stdout pipe `reference` to its end, and the media that
    /// arrive at `returned` until it closes. A medium is read while the
    /// pipe opens too: the backend opens it only once the handler writes
    /// stdout or ends, and a handler that returns a medium first waits until
    /// the runner has read it.
    async fn run(
        mut self,
        reference: &PipeRef,
        mut returned: mpsc::Receiver<RpcMedium>,
    ) -> Result<(), ServiceError> {
        let pipes = self.pipes;
        let opening = pipes.get(&reference.url, self.stop.clone());
        tokio::pin!(opening);
        let mut stdout = Stdout::Opening;
        let mut media_open = true;
        loop {
            let ended = matches!(stdout, Stdout::Ended);
            self.deliver_due(ended).await?;
            if ended && !media_open {
                return Ok(());
            }
            tokio::select! {
                medium = returned.recv(), if media_open => match medium {
                    Some(medium) => self.pending.push_back(medium),
                    None => media_open = false,
                },
                opened = &mut opening, if matches!(stdout, Stdout::Opening) => match opened {
                    Ok(stream) => stdout = Stdout::Open(stream),
                    Err(error) => return self.failed(reference, error).await,
                },
                chunk = stdout.next(), if matches!(stdout, Stdout::Open(_)) => match chunk {
                    Some(Ok(bytes)) => self.pass(bytes).await?,
                    Some(Err(error)) => return self.failed(reference, error).await,
                    None => {
                        stdout = Stdout::Ended;
                        report(self.connection, reference, &Ok(()), self.stop).await?;
                    }
                },
            }
        }
    }

    /// Reports the pipe `reference`, the call's stdout or a medium's,
    /// failed with `error`, which fails the call.
    async fn failed(&self, reference: &PipeRef, error: io::Error) -> Result<(), ServiceError> {
        let result = Err(error);
        report(self.connection, reference, &result, self.stop).await?;
        result.map_err(ServiceError::failed)
    }

    /// Passes `bytes` on, delivering each medium whose place falls within
    /// them where it falls.
    async fn pass(&mut self, mut bytes: Bytes) -> Result<(), ServiceError> {
        while !bytes.is_empty() {
            let room = self
                .pending
                .front()
                .map_or(u64::MAX, |medium| medium.after.saturating_sub(self.written));
            let count = usize::try_from(room).unwrap_or(usize::MAX).min(bytes.len());
            let part = bytes.split_to(count);
            self.written += part.len() as u64;
            self.output.stdout(part).await?;
            self.deliver_due(false).await?;
        }
        Ok(())
    }

    /// Delivers each waiting medium whose place stdout has reached: every
    /// one once stdout has `ended`.
    async fn deliver_due(&mut self, ended: bool) -> Result<(), ServiceError> {
        while let Some(medium) = self
            .pending
            .pop_front_if(|medium| ended || medium.after <= self.written)
        {
            let bytes = match self.read(&medium).await {
                Ok(bytes) => bytes,
                Err(error) => return self.failed(&medium.pipe, error).await,
            };
            report(self.connection, &medium.pipe, &Ok(()), self.stop).await?;
            self.output.medium(bytes).await?;
        }
        Ok(())
    }

    /// The bytes of `medium`, exactly its size.
    async fn read(&self, medium: &RpcMedium) -> io::Result<Bytes> {
        let mut stream = self.pipes.get(&medium.pipe.url, self.stop.clone()).await?;
        let mut bytes = BytesMut::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if bytes.len() as u64 + chunk.len() as u64 > medium.size {
                return Err(io::Error::other("a medium's bytes exceed its size"));
            }
            bytes.extend_from_slice(&chunk);
        }
        if bytes.len() as u64 != medium.size {
            return Err(io::Error::other("a medium's bytes fall short of its size"));
        }
        Ok(bytes.freeze())
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
            .map_err(ServiceError::failed)?;
            connection
                .send(message)
                .await
                .map_err(|_| ServiceError::Cancelled)?;
        }
        return Ok(());
    }
    if let Some(reference) = reference.await.map_err(|_| ServiceError::Cancelled)? {
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
        let result = pipes.put(&reference.url, body, stop).await;
        report(connection, &reference, &result, stop).await?;
        result.map_err(ServiceError::failed)?;
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
    .map_err(ServiceError::failed)?;
    tokio::select! {
        _ = stop.cancelled() => Err(ServiceError::Cancelled),
        result = output.send(message) => result.map_err(|_| ServiceError::Cancelled),
    }
}
/// A call's relay broke the rpc protocol (`commands.md` § Handle an rpc call).
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
    #[error("an RPC medium arrived before pipe descriptors")]
    MediumBeforePipes,
}

/// The frame that ends a call's standard input.
fn stdin_end(call_id: &str) -> Result<wire::Frame, ServiceError> {
    wire::encode(&wire::Outbound::RpcStdinEnd {
        call_id: call_id.into(),
    })
    .map_err(ServiceError::failed)
}
