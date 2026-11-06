//! The rpc relay (`crates/device-interface/src/rpc.rs`): a job's `rpc_call`
//! becomes a handler call in the backend.
//!
//! The relay:
//!
//! - Checks that the call comes from a live job on this connection.
//! - Mints the call's standard output pipe, and its standard input's when the
//!   process has one.
//! - Serves the handler's port: output through the pipe, errors as
//!   `rpc_output`.
//!
//! The call exits after its standard output drained, with the handler's code,
//! 130 when the runner cancelled it, or 1 with the first cause that stopped it.
//!
//! A call belongs to its device's session, not to the connection it started
//! on ([`crate::session::DeviceSession`]): its handler goes on when the
//! connection drops, the messages it sends wait for the next one, and only the
//! end of the session, of its job or of the device stops it. A resumable call
//! that its session's end stops tells the runner no end: the runner sends it
//! again to the next session.

use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
};

use bytes::Bytes;
use demi_device_interface::{
    PortError, PortRequest, PortResponse, PortTransport, RelayedPipes, RpcError, RpcInvocation,
    RpcPort,
};
use demi_command_protocol::MediumFacts;
use demi_runner_protocol::wire::{Inbound, STDIN_CHUNK_BYTES, WireBytes};
use demi_shared_gates::SerialGate;
use demi_shared_types::B64Bytes;
use futures_util::future::LocalBoxFuture;
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use crate::{
    DeviceJobs, Link, LinkPolicy,
    link::JobOrigin,
    pipes::{Pipe, PipeError, PipeReader, PipeWriter, Pipes},
    session::DeviceSession,
};

/// What a call needs of its device: the connection it started on is not it,
/// since the call outlives the connection.
#[derive(Clone)]
pub(crate) struct CallEnv {
    pub(crate) device: String,
    pub(crate) policy: Rc<dyn LinkPolicy>,
    pub(crate) pipes: Pipes,
    pub(crate) jobs: DeviceJobs,
    pub(crate) session: DeviceSession,
}

impl CallEnv {
    fn device(&self) -> &str {
        &self.device
    }

    fn policy(&self) -> &Rc<dyn LinkPolicy> {
        &self.policy
    }

    fn pipes(&self) -> &Pipes {
        &self.pipes
    }

    fn device_jobs(&self) -> &DeviceJobs {
        &self.jobs
    }

    /// Queues `message` for the runner, behind the call messages before it.
    fn send(&self, message: &Inbound) -> Result<(), demi_device_interface::DeviceError> {
        self.session.send(message)
    }
}

/// An `rpc_call` as the runner sent it.
pub(crate) struct RpcCall {
    pub(crate) job_id: String,
    pub(crate) call_id: String,
    pub(crate) path: Vec<String>,
    pub(crate) argv: Vec<String>,
    pub(crate) args: Map<String, Value>,
    pub(crate) json: bool,
    pub(crate) cwd: String,
    pub(crate) env: BTreeMap<String, String>,
    pub(crate) stdin: bool,
    /// Sent again after the session it ran in ended.
    pub(crate) resumed: bool,
}

/// Why a call stopped before its handler finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Stop {
    /// The runner cancelled it.
    Cancelled,
    /// Something else ended it: its job, a pipe, its session.
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

/// One call in flight on a device.
pub(crate) struct CallEntry {
    pub(crate) job_id: String,
    stopped: CancellationToken,
    /// What the handler's port watches once the call runs: a child of its
    /// job's stop, so that a Stop of the command stops the handler at once,
    /// before the runner hears of it, and cancelled too when the call stops.
    handler: RefCell<Option<CancellationToken>>,
    /// The first cause that stopped the call.
    cause: RefCell<Option<Stop>>,
    pipes: RefCell<Option<CallPipes>>,
    /// The backend told the runner the call is resumable.
    resumable: Cell<bool>,
    /// The call stopped with a session that the runner sends it again
    /// after, when it is resumable.
    returns: Cell<bool>,
}

/// The pipes a call relays, once minted: its standard streams, and the
/// media it hands its job while one goes, which its stop fails as it fails
/// the others.
struct CallPipes {
    stdout: Pipe,
    stdin: Option<Pipe>,
    media: Vec<Pipe>,
}

impl CallPipes {
    fn iter(&self) -> impl Iterator<Item = &Pipe> {
        std::iter::once(&self.stdout).chain(&self.stdin).chain(&self.media)
    }
}

impl CallEntry {
    fn new(job_id: String) -> Rc<Self> {
        Rc::new(Self {
            job_id,
            stopped: CancellationToken::new(),
            handler: RefCell::new(None),
            cause: RefCell::new(None),
            pipes: RefCell::new(None),
            resumable: Cell::new(false),
            returns: Cell::new(false),
        })
    }

    /// Says the call's session ended, and whether a resumable call comes
    /// back to the next.
    pub(crate) fn lose(&self, returns: bool) {
        self.returns.set(returns);
    }

    /// Whether the call, stopped, comes back: it was resumable and its
    /// session ended, so the runner sends it again, and its end here tells
    /// the runner nothing.
    fn comes_back(&self) -> bool {
        self.resumable.get() && self.returns.get()
    }

    /// Ends the call's finite standard input as finished, read or unread:
    /// the handler took what it wanted, so the process's upload is answered
    /// as a whole one rather than failed, and its stdout goes on to its end
    /// (`crates/device-interface/src/rpc.rs`).
    fn finish_input(&self) {
        let pipes = self.pipes.borrow();
        let Some(stdin) = pipes.as_ref().and_then(|pipes| pipes.stdin.as_ref()) else {
            return;
        };
        match stdin.reader() {
            // A reader in this process that lets go drains the pipe.
            Ok(reader) => drop(reader),
            // A reader already taken drained the pipe as it let go, a pipe
            // that is over stays as it ended, and one handed to a device is
            // that device's to end.
            Err(PipeError::Taken(_) | PipeError::Settled | PipeError::AlreadyFixed(_)) => {}
        }
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
            .flat_map(CallPipes::iter)
            .find_map(Pipe::failure)
            .map(|failure| Stop::Ended(failure.reason().to_owned()));
        let cause = failed.unwrap_or(cause);
        let reason = cause.text().to_owned();
        *self.cause.borrow_mut() = Some(cause);
        self.stopped.cancel();
        if let Some(handler) = &*self.handler.borrow() {
            handler.cancel();
        }
        for pipe in self.pipes.borrow().iter().flat_map(CallPipes::iter) {
            pipe.fail(&reason);
        }
    }

    fn cause(&self) -> Option<Stop> {
        self.cause.borrow().clone()
    }
}

/// Starts relaying a call. A second call under one id breaks the protocol
/// and ends the connection.
///
/// The call's task belongs to the device: the connection that started it may
/// end, and the task goes on. Once the call stops it has `END_GRACE` to
/// finish, such as a handler committing its change, and what still runs then
/// is aborted.
pub(crate) fn start(link: &Link, call: RpcCall) {
    let entry = CallEntry::new(call.job_id.clone());
    let relaying = link.call_env();
    if !relaying.session.add_call(call.call_id.clone(), entry.clone()) {
        link.disconnect(&format!("duplicate rpc call {}", call.call_id));
        return;
    }
    let command = call.path.join(" ");
    let name = command.clone();
    let (entry_for_abort, call_id_for_abort, command_for_abort) =
        (entry.clone(), call.call_id.clone(), command.clone());
    let (session, stopped) = (relaying.session.clone(), entry.stopped.clone());
    let task = async move {
        let call_id = call.call_id.clone();
        let job_id = call.job_id.clone();
        let began = tokio::time::Instant::now();
        let origin = tokio::select! {
            origin = origin_of(&relaying, &call.job_id) => origin,
            () = entry.stopped.cancelled() => Err(entry
                .cause()
                .map_or_else(|| "rpc call ended".to_owned(), |cause| cause.text().to_owned())),
        };
        let no_origin = origin.is_err();
        let ran = match origin {
            Ok(origin) => run(&relaying, call, origin, &entry).await,
            Err(error) => Err(Ended::Failed(error)),
        };
        let (outcome, reason, exit) = match (&ran, entry.cause()) {
            (Ok(code), _) => ("ok", None, Some(*code)),
            (Err(Ended::Usage(_)), _) => ("refused", Some("usage"), None),
            (Err(Ended::Failed(_)), Some(Stop::Cancelled)) => ("cancelled", Some("cancelled_by_runner"), None),
            (Err(Ended::Failed(_)), Some(Stop::Ended(text))) => (
                "cancelled",
                Some(if text.contains("disconnected") { "runner_disconnected" } else { "ended" }),
                None,
            ),
            (Err(Ended::Failed(_)), None) if no_origin => ("refused", Some("no_origin"), None),
            (Err(Ended::Failed(_)), None) => ("failed", Some("handler_failed"), None),
        };
        let cause = entry.cause();
        tracing::info!(
            event = "rpc_call.ended",
            device_id = relaying.device(),
            job_id = job_id.as_str(),
            call = call_id.as_str(),
            command = command.as_str(),
            outcome,
            reason,
            cause = cause.as_ref().map(Stop::text),
            exit,
            ms = began.elapsed().as_millis() as u64,
            "a runner's call of a demi command ended"
        );
        if entry.comes_back() {
            relaying.session.remove_call(&call_id);
            entry.stop(Stop::Ended("rpc call ended".into()));
            return;
        }
        let exit_code = match ran {
            Ok(code) => code,
            Err(ended) => {
                // A usage error is clap's text whole; a runtime error is
                // GNU's line, the command's path first (`crates/device-interface/src/rpc.rs`).
                let (text, code) = match ended {
                    Ended::Usage(text) => (format!("{text}\n"), 2),
                    Ended::Failed(message) => (
                        format!("{command}: {message}\n"),
                        match entry.cause() {
                            Some(Stop::Cancelled) => 130,
                            _ => 1,
                        },
                    ),
                };
                let error = Inbound::RpcOutput {
                    call_id: call_id.clone(),
                    bytes: WireBytes(text.into_bytes()),
                };
                if let Err(error) = relaying.send(&error) {
                    tracing::warn!(event = "link.send_failed", device_id = relaying.device(), what = "rpc_error", call = %call_id, %error, "a message to a runner was not sent");
                }
                code
            }
        };
        relaying.session.remove_call(&call_id);
        entry.finish_input();
        entry.stop(Stop::Ended("rpc call ended".into()));
        let exit = Inbound::RpcExit {
            call_id: call_id.clone(),
            exit_code,
        };
        if let Err(error) = relaying.send(&exit) {
            tracing::warn!(event = "link.send_failed", device_id = relaying.device(), what = "exit", call = %call_id, %error, "a message to a runner was not sent");
        }
    };
    // A handler aborted after its grace never answered: the call ends here
    // with the reason it stopped for, so its caller does not wait for ever.
    let aborted = {
        let (relaying, entry, call_id) = (link.call_env(), entry_for_abort, call_id_for_abort);
        let command = command_for_abort;
        move || {
            if relaying.session.remove_call(&call_id).is_none() || entry.comes_back() {
                return;
            }
            entry.finish_input();
            let reason = entry
                .cause()
                .map_or_else(|| "rpc call ended".to_owned(), |cause| cause.text().to_owned());
            let text = Inbound::RpcOutput {
                call_id: call_id.clone(),
                bytes: WireBytes(format!("{command}: {reason}\n").into_bytes()),
            };
            let exit = Inbound::RpcExit {
                call_id: call_id.clone(),
                exit_code: match entry.cause() {
                    Some(Stop::Cancelled) => 130,
                    _ => 1,
                },
            };
            for message in [text, exit] {
                if let Err(error) = relaying.send(&message) {
                    tracing::warn!(event = "link.send_failed", device_id = relaying.device(), what = "aborted_call", call = %call_id, %error, "a message to a runner was not sent");
                }
            }
        }
    };
    session.spawn_call(name, stopped, task, aborted);
}

/// Why a call ended without its handler's exit code.
enum Ended {
    /// Its arguments do not fit its command: the usage error's whole text.
    Usage(String),
    /// What stopped it, as a runtime error's object and reason.
    Failed(String),
}

impl From<String> for Ended {
    fn from(message: String) -> Self {
        Self::Failed(message)
    }
}

impl From<&str> for Ended {
    fn from(message: &str) -> Self {
        Self::Failed(message.to_owned())
    }
}

/// Whose job `job` is: a job no agent took up yet, as a restarted backend
/// parks one, is taken up first (`crates/agent-session/src/reports.rs`).
async fn origin_of(link: &CallEnv, job: &str) -> Result<Rc<JobOrigin>, String> {
    let unknown = || "rpc requires a live job dispatched to this device".to_owned();
    // What a job left running after its end calls the job's commands as the
    // job did, while the runner keeps its execution context.
    let Some(mut claimed) = link.device_jobs().claimed(job) else {
        return link.device_jobs().detached(job).ok_or_else(unknown);
    };
    if !*claimed.borrow_and_update() {
        link.policy().take_up_job(job.to_owned()).await?;
        claimed.wait_for(|claimed| *claimed).await.map_err(|_| unknown())?;
    }
    link.device_jobs()
        .with(job, |job| job.origin.clone())
        .flatten()
        .ok_or_else(unknown)
}

/// Runs the call to its exit code, or what stopped it.
async fn run(
    link: &CallEnv,
    call: RpcCall,
    origin: Rc<JobOrigin>,
    entry: &Rc<CallEntry>,
) -> Result<u8, Ended> {
    link.policy().admit_call(&origin).await?;
    let stdout = link.pipes().call_stdout(link.device());
    stdout.hold_source().map_err(|error| error.to_string())?;
    let stdin = call.stdin.then(|| link.pipes().call_stdin(link.device()));
    *entry.pipes.borrow_mut() = Some(CallPipes {
        stdout: stdout.clone(),
        stdin: stdin.clone(),
        media: Vec::new(),
    });
    if let Some(cause) = entry.cause() {
        return Err(cause.text().into());
    }
    let pipes = Inbound::RpcPipes {
        call_id: call.call_id.clone(),
        stdin: stdin.as_ref().map(Pipe::wire_ref),
        stdout: stdout.call_output_ref(),
    };
    link.send(&pipes).map_err(|error| error.to_string())?;
    let invocation = RpcInvocation {
        call_id: call.call_id.clone(),
        resumed: call.resumed,
        path: call.path,
        argv: call.argv,
        args: call.args,
        json: call.json,
        // A job's calls come over the connection of the device it runs on,
        // whose state holds the job's record.
        device: link.device().to_owned(),
        cwd: call.cwd,
        env: call.env,
        context: origin.context.clone(),
        caller: origin.caller.clone(),
        stdin: call.stdin,
        pipes: Some(RelayedPipes {
            stdin: stdin.as_ref().map(|pipe| pipe.id().to_owned()),
            stdout: stdout.id().to_owned(),
        }),
        // The dispatch resolves the call's device.
        brought: None,
    };
    let port = Rc::new(RelayPort {
        link: link.clone(),
        call_id: call.call_id.clone(),
        entry: entry.clone(),
        stdout: RefCell::new(Stdout::Held(stdout.clone())),
        stdout_turn: SerialGate::new(),
        stdin: RefCell::new(stdin.clone().map(Stdin::Unread)),
        stdin_turn: SerialGate::new(),
    });
    // The job is claimed by now, so its stop is the stop of the command
    // that runs it.
    let cancel = link
        .device_jobs()
        .with(&entry.job_id, |job| job.stop.child_token())
        // A job the device no longer holds has ended: the call stops with
        // its own end.
        .unwrap_or_default();
    if entry.stopped.is_cancelled() {
        cancel.cancel();
    }
    *entry.handler.borrow_mut() = Some(cancel.clone());
    let handler = link.policy().dispatch(origin, invocation, RpcPort::new(port.clone(), cancel));
    // A relayed pipe that fails stops the call; the handler is awaited to
    // its end all the same, since it may be committing, until the
    // connection's end gives up on it after its grace (`link::END_GRACE`).
    // A resumable call whose output has not begun outlives its pipe, which
    // its runner's going away fails: the end of the session stops it, and
    // the runner sends it again.
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
                if entry.resumable.get() && port.untouched() {
                    tracing::info!(event = "rpc_call.pipe_lost", call = %port.call_id, reason = failure.reason(), "a resumable call's pipe ended; the call waits for its session");
                } else {
                    entry.stop(Stop::Ended(failure.reason().to_owned()));
                }
            }
        }
    };
    if let Some(cause) = entry.cause() {
        return Err(cause.text().into());
    }
    port.finish_stdout();
    // The calling process has read everything before the call exits, a
    // failed call's output too: the process was handed the stdout pipe, and
    // one that comes for it after the handler failed at once, as a call the
    // permission check refuses does, finds it ended rather than gone.
    let drained = tokio::select! {
        biased;
        () = entry.stopped.cancelled() => Err(entry.cause().map_or_else(String::new, |cause| cause.text().into())),
        drained = stdout.done() => drained.map_err(|failure| failure.reason().to_owned()),
    };
    let code = result.map_err(|error| match error {
        RpcError::Usage(text) => Ended::Usage(text),
        error => Ended::Failed(error.to_string()),
    })?;
    Ok(drained.map(|()| code)?)
}

/// The handler's port over the relay.
struct RelayPort {
    link: CallEnv,
    call_id: String,
    entry: Rc<CallEntry>,
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
                Err(error) => tracing::warn!(event = "link.send_failed", device_id = self.link.device(), what = "stdout_end", call = %self.call_id, %error, "a call's standard output was not ended"),
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

    /// Hands the calling job a medium: its facts in a message, its bytes in
    /// a pipe of their own, since bulk bytes never travel in a message
    /// (`crates/runner-protocol/src/wire/mod.rs`). This waits until the
    /// runner has read the pipe, and the runner keeps the medium before the
    /// standard output and the exit the handler writes after it
    /// (`crates/runner-jobs/src/commands/rpc.rs`).
    async fn write_medium(&self, facts: MediumFacts, bytes: Bytes) -> Result<(), PortError> {
        let failed = |error: &dyn std::fmt::Display| PortError::Ended(format!("the medium: {error}"));
        let pipe = self.link.pipes.call_stdout(self.link.device());
        pipe.hold_source().map_err(|error| failed(&error))?;
        if let Some(pipes) = self.entry.pipes.borrow_mut().as_mut() {
            pipes.media.push(pipe.clone());
        }
        let handed = self.hand_medium(&pipe, facts, bytes).await;
        if let Some(pipes) = self.entry.pipes.borrow_mut().as_mut() {
            pipes.media.retain(|kept| kept.id() != pipe.id());
        }
        handed
    }

    async fn hand_medium(&self, pipe: &Pipe, facts: MediumFacts, bytes: Bytes) -> Result<(), PortError> {
        let failed = |error: &dyn std::fmt::Display| PortError::Ended(format!("the medium: {error}"));
        let medium = Inbound::RpcMedium {
            call_id: self.call_id.clone(),
            facts,
            bytes: pipe.call_output_ref(),
        };
        self.link.send(&medium).map_err(|error| failed(&error))?;
        let mut writer = pipe.writer().map_err(|error| failed(&error))?;
        let mut rest = bytes;
        while !rest.is_empty() {
            let chunk = rest.split_to(rest.len().min(STDIN_CHUNK_BYTES));
            writer.write(chunk).await.map_err(|error| failed(&error))?;
        }
        writer.end();
        pipe.done().await.map_err(|error| failed(&error))
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
    fn comes_back(&self) -> bool {
        self.entry.comes_back()
    }

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
                            .map_err(|error| PortError::Ended(error.to_string()))?;
                    }
                    Ok(PortResponse::Written {})
                }
                PortRequest::ReadStdin {} => Ok(PortResponse::Input {
                    bytes: self.read_stdin().await?.map(B64Bytes::new),
                }),
                PortRequest::AwaitsUser { awaits } => {
                    self.await_user(awaits);
                    Ok(PortResponse::Written {})
                }
                PortRequest::Medium { facts, bytes } => {
                    self.write_medium(facts, bytes.into_bytes()).await?;
                    Ok(PortResponse::Written {})
                }
                PortRequest::Resumable {} => {
                    // A call whose output began, or that reads its input,
                    // cannot start over.
                    if self.untouched() {
                        let resumable = Inbound::RpcResumable { call_id: self.call_id.clone() };
                        self.link
                            .send(&resumable)
                            .map_err(|error| PortError::Ended(error.to_string()))?;
                        self.entry.resumable.set(true);
                    }
                    Ok(PortResponse::Written {})
                }
            }
        })
    }
}

impl RelayPort {
    /// Whether the call could start over: its output has not begun and it
    /// has no input.
    fn untouched(&self) -> bool {
        matches!(*self.stdout.borrow(), Stdout::Held(_)) && self.stdin.borrow().is_none()
    }

    /// Says whether the call waits for the user's answer, under a key of the
    /// call's own.
    fn await_user(&self, awaits: bool) {
        let key = format!("call:{}", self.call_id);
        // A job the device no longer holds has ended, and its wait with it.
        let _ = self
            .link
            .device_jobs()
            .with(&self.entry.job_id, |job| job.shared.await_user(key, awaits));
    }
}

impl Drop for RelayPort {
    /// The call ended, however it ended: it waits no longer.
    fn drop(&mut self) {
        self.await_user(false);
    }
}
