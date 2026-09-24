//! The rpc relay (`commands.md` § Handle an rpc call): a job's `rpc_call`
//! becomes a handler call in the backend. The relay checks that the call
//! comes from a live job on this connection, mints the call's standard
//! output pipe (and its standard input's, when the process has one), and
//! serves the handler's port: output through the pipe, errors as
//! `rpc_output`, live input on demand, and storage through the connection's
//! policy. The call exits after its standard output drained, with the
//! handler's code, 130 when the runner cancelled it, or 1 with the first
//! cause that stopped it.

use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

use bytes::Bytes;
use demi_core::B64Bytes;
use demi_gates::SerialGate;
use demi_runner_protocol::wire::{Inbound, STDIN_CHUNK_BYTES, WireBytes};
use demi_shell::{
    PortError, PortRequest, PortResponse, PortTransport, RelayedPipes, RpcInvocation, RpcPort,
};
use futures_util::future::LocalBoxFuture;
use serde_json::{Map, Value};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::{
    Link,
    link::JobOrigin,
    pipes::{Pipe, PipeError, PipeReader, PipeWriter},
};

/// An `rpc_call` as the runner sent it.
pub(crate) struct RpcCall {
    pub(crate) job_id: String,
    pub(crate) call_id: String,
    pub(crate) root: String,
    pub(crate) path: Vec<String>,
    pub(crate) argv: Vec<String>,
    pub(crate) args: Map<String, Value>,
    pub(crate) json: bool,
    pub(crate) cwd: String,
    pub(crate) env: BTreeMap<String, String>,
    pub(crate) stdin: bool,
}

/// Why a call stopped before its handler finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Stop {
    /// The runner cancelled it.
    Cancelled,
    /// Something else ended it: its job, a pipe, the connection.
    Ended(String),
}

impl Stop {
    fn text(&self) -> &str {
        match self {
            Self::Cancelled => "command cancelled",
            Self::Ended(reason) => reason,
        }
    }
}

/// One call in flight on a connection.
pub(crate) struct CallEntry {
    pub(crate) job_id: String,
    stopped: CancellationToken,
    /// The first cause that stopped the call.
    cause: RefCell<Option<Stop>>,
    live: RefCell<Live>,
    pipes: RefCell<Vec<Pipe>>,
}

/// The call's live input: one chunk requested at a time.
enum Live {
    Idle,
    Waiting(oneshot::Sender<Option<Bytes>>),
    Closed,
}

impl CallEntry {
    fn new(job_id: String) -> Rc<Self> {
        Rc::new(Self {
            job_id,
            stopped: CancellationToken::new(),
            cause: RefCell::new(None),
            live: RefCell::new(Live::Idle),
            pipes: RefCell::new(Vec::new()),
        })
    }

    /// Stops the call; a cause after the first changes nothing. A relayed
    /// pipe that failed already is the earlier cause, even when its failure
    /// has not reached the call yet.
    pub(crate) fn stop(&self, cause: Stop) {
        if self.cause.borrow().is_some() {
            return;
        }
        let failed = self
            .pipes
            .borrow()
            .iter()
            .find_map(Pipe::failure)
            .map(|failure| Stop::Ended(failure.to_string()));
        let cause = failed.unwrap_or(cause);
        let reason = cause.text().to_owned();
        *self.cause.borrow_mut() = Some(cause);
        self.stopped.cancel();
        self.end_live_input();
        for pipe in self.pipes.borrow().iter() {
            pipe.fail(&reason);
        }
    }

    fn cause(&self) -> Option<Stop> {
        self.cause.borrow().clone()
    }

    /// A chunk of live input the runner sent. One nobody asked for, or one
    /// over the chunk limit, stops the call.
    pub(crate) fn live_input(&self, bytes: Bytes) {
        let phase = std::mem::replace(&mut *self.live.borrow_mut(), Live::Idle);
        match phase {
            Live::Waiting(next) if bytes.len() <= STDIN_CHUNK_BYTES => {
                // A reader that went away took nothing.
                let _ = next.send(Some(bytes));
            }
            Live::Closed => *self.live.borrow_mut() = Live::Closed,
            Live::Idle | Live::Waiting(_) => {
                self.stop(Stop::Ended(
                    "Unrequested or oversized RPC stdin chunk".into(),
                ));
            }
        }
    }

    /// The live input ended: the job's stdin closed, or the call stopped.
    pub(crate) fn end_live_input(&self) {
        let phase = std::mem::replace(&mut *self.live.borrow_mut(), Live::Closed);
        if let Live::Waiting(next) = phase {
            // A reader that went away hears nothing.
            let _ = next.send(None);
        }
    }

    /// Asks the runner for the next chunk of live input.
    async fn next_live_input(
        &self,
        link: &Link,
        call_id: &str,
    ) -> Result<Option<Bytes>, PortError> {
        let received = {
            let mut live = self.live.borrow_mut();
            match &*live {
                Live::Closed => return Ok(None),
                Live::Waiting(_) => {
                    return Err(PortError::Ended("live input is already being read".into()));
                }
                Live::Idle => {}
            }
            let (next, received) = oneshot::channel();
            *live = Live::Waiting(next);
            received
        };
        link.send(&Inbound::RpcStdinPull {
            call_id: call_id.into(),
        })
        .await
        .map_err(|error| PortError::Ended(error.to_string()))?;
        // A dropped sender is an input that ended.
        Ok(received.await.unwrap_or(None))
    }
}

/// Starts relaying a call. A second call under one id breaks the protocol
/// and ends the connection.
pub(crate) fn start(link: &Link, call: RpcCall) {
    let entry = CallEntry::new(call.job_id.clone());
    let origin = link.with_state(|state| {
        state
            .add_call(call.call_id.clone(), entry.clone())
            .then(|| state.job_origin(&call.job_id))
    });
    let Some(origin) = origin else {
        link.disconnect(&format!("duplicate rpc call {}", call.call_id));
        return;
    };
    let relaying = link.clone();
    link.spawn(async move {
        let call_id = call.call_id.clone();
        let root = call.root.clone();
        let exit_code = match run(&relaying, call, origin, &entry).await {
            Ok(code) => code,
            Err(message) => {
                let error = Inbound::RpcOutput {
                    call_id: call_id.clone(),
                    bytes: WireBytes(format!("{root}: {message}\n").into_bytes()),
                };
                if let Err(error) = relaying.send(&error).await {
                    tracing::debug!(call = %call_id, "rpc error not sent: {error}");
                }
                match entry.cause() {
                    Some(Stop::Cancelled) => 130,
                    _ => 1,
                }
            }
        };
        relaying.with_state(|state| state.remove_call(&call_id));
        entry.stop(Stop::Ended("rpc call ended".into()));
        let exit = Inbound::RpcExit {
            call_id: call_id.clone(),
            exit_code,
        };
        if let Err(error) = relaying.send(&exit).await {
            tracing::debug!(call = %call_id, "rpc exit not sent: {error}");
        }
    });
}

/// Runs the call to its exit code, or the text of what stopped it.
async fn run(
    link: &Link,
    call: RpcCall,
    origin: Option<Rc<JobOrigin>>,
    entry: &Rc<CallEntry>,
) -> Result<u8, String> {
    let origin = origin.ok_or("rpc requires a live job dispatched to this device")?;
    link.policy().admit_call(&origin)?;
    let stdout = link.pipes().to_device(link.device());
    stdout.hold_source().map_err(|error| error.to_string())?;
    let stdin = call.stdin.then(|| link.pipes().from_device(link.device()));
    entry
        .pipes
        .borrow_mut()
        .extend([Some(stdout.clone()), stdin.clone()].into_iter().flatten());
    if let Some(cause) = entry.cause() {
        return Err(cause.text().into());
    }
    let pipes = Inbound::RpcPipes {
        call_id: call.call_id.clone(),
        stdin: stdin.as_ref().map(Pipe::wire_ref),
        stdout: stdout.wire_ref(),
    };
    link.send(&pipes).await.map_err(|error| error.to_string())?;
    let invocation = RpcInvocation {
        path: call.path,
        argv: call.argv,
        args: call.args,
        json: call.json,
        cwd: call.cwd,
        env: call.env,
        context: origin.context.clone(),
        stdin: call.stdin,
        pipes: Some(RelayedPipes {
            stdin: stdin.as_ref().map(|pipe| pipe.id().to_owned()),
            stdout: stdout.id().to_owned(),
        }),
    };
    let port = Rc::new(RelayPort {
        link: link.clone(),
        call_id: call.call_id.clone(),
        entry: entry.clone(),
        origin: origin.clone(),
        stdout: RefCell::new(Stdout::Held(stdout.clone())),
        stdout_turn: SerialGate::new(),
        stdin: RefCell::new(stdin.clone().map(Stdin::Unread)),
        stdin_turn: SerialGate::new(),
    });
    let handler = link.policy().dispatch(
        origin,
        invocation,
        RpcPort::new(port.clone(), entry.stopped.clone()),
    );
    // A relayed pipe that fails stops the call; the handler is awaited to
    // its end all the same, since it may be committing.
    let broken = {
        let stdout = stdout.failed();
        let stdin = stdin.as_ref().map(Pipe::failed);
        async move {
            match stdin {
                Some(stdin) => tokio::select! {
                    failure = stdout => failure,
                    failure = stdin => failure,
                },
                None => stdout.await,
            }
        }
    };
    tokio::pin!(handler, broken);
    let mut watching = true;
    let result = loop {
        tokio::select! {
            result = &mut handler => break result,
            failure = &mut broken, if watching => {
                watching = false;
                entry.stop(Stop::Ended(failure.to_string()));
            }
        }
    };
    if let Some(cause) = entry.cause() {
        return Err(cause.text().into());
    }
    let code = result.map_err(|error| error.to_string())?;
    port.finish_stdout();
    // The calling process has read everything before the call exits.
    tokio::select! {
        biased;
        () = entry.stopped.cancelled() => Err(entry.cause().map_or_else(String::new, |cause| cause.text().into())),
        drained = stdout.done() => drained.map(|()| code).map_err(|failure| failure.to_string()),
    }
}

/// The handler's port over the relay.
struct RelayPort {
    link: Link,
    call_id: String,
    entry: Rc<CallEntry>,
    origin: Rc<JobOrigin>,
    stdout: RefCell<Stdout>,
    /// Writes to standard output take turns.
    stdout_turn: SerialGate,
    stdin: RefCell<Option<Stdin>>,
    stdin_turn: SerialGate,
}

/// The call's standard output as the relay holds it.
enum Stdout {
    /// Nothing written yet: the handler may still hand it to a job
    /// elsewhere.
    Held(Pipe),
    Writing(PipeWriter),
    /// A write is under way.
    Busy,
    /// The handler finished.
    Done,
}

/// The call's finite standard input.
enum Stdin {
    Unread(Pipe),
    Reading(PipeReader),
    Busy,
}

impl RelayPort {
    /// Ends standard output once the handler returned: its end reaches the
    /// calling process unless the handler handed it to another device.
    fn finish_stdout(&self) {
        let stdout = std::mem::replace(&mut *self.stdout.borrow_mut(), Stdout::Done);
        match stdout {
            Stdout::Held(pipe) => match pipe.writer() {
                Ok(writer) => writer.end(),
                // Handed to a device, whose upload ends it.
                Err(PipeError::AlreadyFixed(_)) => {}
                Err(error) => tracing::debug!(call = %self.call_id, "stdout not ended: {error}"),
            },
            Stdout::Writing(writer) => writer.end(),
            Stdout::Busy | Stdout::Done => {}
        }
    }

    async fn write_stdout(&self, bytes: Bytes) -> Result<(), PortError> {
        let _turn = self.stdout_turn.acquire().await;
        let stdout = std::mem::replace(&mut *self.stdout.borrow_mut(), Stdout::Busy);
        let mut writer = match stdout {
            Stdout::Held(pipe) => pipe
                .writer()
                .map_err(|error| PortError::Ended(format!("standard output: {error}")))?,
            Stdout::Writing(writer) => writer,
            Stdout::Busy | Stdout::Done => {
                *self.stdout.borrow_mut() = Stdout::Done;
                return Err(PortError::Ended("the call has ended".into()));
            }
        };
        let written = writer.write(bytes).await;
        *self.stdout.borrow_mut() = Stdout::Writing(writer);
        written.map_err(|failure| PortError::Ended(failure.to_string()))
    }

    async fn read_stdin(&self) -> Result<Option<Bytes>, PortError> {
        let _turn = self.stdin_turn.acquire().await;
        let stdin = self.stdin.borrow_mut().replace(Stdin::Busy);
        let mut reader = match stdin {
            None => {
                *self.stdin.borrow_mut() = None;
                return Ok(None);
            }
            Some(Stdin::Unread(pipe)) => pipe
                .reader()
                .map_err(|error| PortError::Ended(format!("standard input: {error}")))?,
            Some(Stdin::Reading(reader)) => reader,
            Some(Stdin::Busy) => {
                return Err(PortError::Ended("standard input is being read".into()));
            }
        };
        let chunk = reader.next().await;
        *self.stdin.borrow_mut() = Some(Stdin::Reading(reader));
        chunk
            .transpose()
            .map_err(|failure| PortError::Ended(failure.to_string()))
    }
}

impl PortTransport for RelayPort {
    fn request(&self, request: PortRequest) -> LocalBoxFuture<'_, Result<PortResponse, PortError>> {
        Box::pin(async move {
            match request {
                PortRequest::Stdout { bytes } => {
                    self.write_stdout(bytes.into_bytes()).await?;
                    Ok(PortResponse::Written {})
                }
                PortRequest::Stderr { bytes } => {
                    if !bytes.is_empty() {
                        let output = Inbound::RpcOutput {
                            call_id: self.call_id.clone(),
                            bytes: WireBytes(bytes.to_vec()),
                        };
                        self.link
                            .send(&output)
                            .await
                            .map_err(|error| PortError::Ended(error.to_string()))?;
                    }
                    Ok(PortResponse::Written {})
                }
                PortRequest::ReadStdin {} => Ok(PortResponse::Input {
                    bytes: self.read_stdin().await?.map(B64Bytes::new),
                }),
                PortRequest::ReadLiveStdin {} => Ok(PortResponse::Input {
                    bytes: self
                        .entry
                        .next_live_input(&self.link, &self.call_id)
                        .await?
                        .map(B64Bytes::new),
                }),
                PortRequest::Storage { op } => Ok(PortResponse::Storage {
                    reply: self.link.policy().storage(self.origin.clone(), op).await?,
                }),
            }
        })
    }
}
