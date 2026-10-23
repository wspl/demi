//! One resident process (`crates/runner-command-packages/src/lib.rs`).
//!
//! The runner starts the executable, speaks HTTP/2 to it over its standard
//! input and output, relays its numbers and messages streams, drains its
//! standard error into the Host log, and reaps it. One owner task holds the
//! process from its start to its end and returns how it ended, with the end of
//! its standard error.

use std::{
    collections::BTreeMap, future::Future, path::Path, process::Stdio, sync::Arc, time::Duration,
};

use demi_command_protocol::{PackageDescriptor, ResidentInfo};
use demi_command_sdk::{ArtifactsAsk, Client, MessagesAsk, NumbersAsk, RequestStream, ResidentError};
use process_wrap::tokio::ChildWrapper;
use tokio::{
    io::AsyncReadExt,
    process::{ChildStderr, Command},
    sync::oneshot,
    task::JoinHandle,
};
use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};

use demi_runner_process::{
    lines::LineSplitter,
    process::{ChildAttributes, start, wait_leader, wrap},
    tail::TailBuffer,
};
use demi_runner_protocol::wire;

use crate::invocations::ResidentArtifacts;
use crate::{ExitReason, ResidentBackend, RuntimeError, ResidentExit};

/// How long a new resident has to finish the handshake and answer its catalog.
const START_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a resident has to shut down when asked (`crates/runner-command-packages/src/lib.rs`).
const STOP_TIMEOUT: Duration = Duration::from_secs(6);
/// A resident whose connection closed usually is exiting: its exit status is
/// awaited this long before the runner stops it as broken.
const EXIT_GRACE: Duration = Duration::from_millis(250);
/// Standard error is still read this long after the process ends, so a
/// descendant that keeps the pipe open cannot hold up the resident's end.
const STDERR_GRACE: Duration = Duration::from_millis(250);

pub struct ResidentProcess {
    client: Client,
    info: ResidentInfo,
    pid: u32,
    owner: JoinHandle<Ended>,
    /// Answers the resident's numbers, artifacts and messages streams until the
    /// resident ends them.
    _streams: AbortOnDropHandle<()>,
}

/// How a resident process ended.
pub struct Ended {
    /// Why it ended on its own; `None` when it was asked to stop.
    pub reason: Option<ExitReason>,
    /// The end of its standard error.
    pub stderr: String,
}

impl ResidentProcess {
    /// Starts `executable`, checks that it serves `descriptor`, and opens
    /// its numbers and messages streams, whose requests go to `backend`, and its
    /// artifacts stream, which `artifacts` answers. Cancelling `stop` asks
    /// the resident to shut down, also while it starts.
    pub(crate) async fn start(
        executable: &Path,
        descriptor: &PackageDescriptor,
        cwd: &Path,
        env: &BTreeMap<String, String>,
        backend: Arc<dyn ResidentBackend>,
        artifacts: Arc<ResidentArtifacts>,
        stop: CancellationToken,
    ) -> Result<Self, RuntimeError> {
        let mut command = Command::new(executable);
        command
            .arg(demi_command_sdk::COMMAND_RESIDENT)
            .current_dir(cwd)
            .env_clear()
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut command = wrap(command, true, &ChildAttributes::default());
        // A start that waits (`demi_runner_process::process::start`) ends
        // with `stop`.
        let mut child = tokio::select! {
            biased;
            _ = stop.cancelled() => return Err(RuntimeError::Cancelled),
            child = start(|| command.spawn()) => child?,
        };
        let pid = child.id().expect("a new process has an ID");
        let input = child.stdin().take().expect("piped stdin");
        let output = child.stdout().take().expect("piped stdout");
        let stderr = child.stderr().take().expect("piped stderr");
        // Giving up on the start stops the process without cancelling the
        // caller's token.
        let owner_stop = stop.child_token();
        let (connected, handshake) = oneshot::channel();
        let owner = tokio::spawn(own(
            child,
            Client::connect(tokio::io::join(output, input)),
            Stderr::new(stderr, format!("resident:{}", descriptor.id)),
            owner_stop.clone(),
            connected,
        ));
        let started = async {
            let client = match handshake.await {
                Ok(connected) => connected?,
                // The owner ended before the handshake: it says why.
                Err(_) => return Ok(None),
            };
            let info = client.info().await?;
            // The streams open before any call, and only for a resident that
            // serves its descriptor.
            let stream = if descriptor.serves(&info) {
                Some((
                    client.numbers().await?,
                    client.artifacts().await?,
                    client.messages().await?,
                ))
            } else {
                None
            };
            Ok::<_, ResidentError>(Some((client, info, stream)))
        };
        let started = tokio::time::timeout(START_TIMEOUT, started).await;
        if let Ok(Ok(Some((client, info, Some((numbers_stream, artifacts_stream, messages_stream)))))) =
            started
        {
            let resident = descriptor.id.clone();
            let streams = async move {
                tokio::join!(
                    answer_numbers(&resident, numbers_stream, backend.clone()),
                    answer_artifacts(&resident, artifacts_stream, artifacts),
                    answer_messages(&resident, messages_stream, backend),
                );
            };
            return Ok(Self {
                client,
                info,
                pid,
                owner,
                _streams: AbortOnDropHandle::new(tokio::spawn(streams)),
            });
        }
        owner_stop.cancel();
        let ended = owner.await.expect("the resident owner does not panic");
        if stop.is_cancelled() {
            return Err(RuntimeError::Cancelled);
        }
        let reason = match (started, ended.reason) {
            (_, Some(reason)) => reason,
            (Ok(Ok(Some(_))), None) => return Err(RuntimeError::CatalogMismatch),
            (Ok(Err(error)), None) => ExitReason::Protocol(error.to_string()),
            (Ok(Ok(None)) | Err(_), None) => ExitReason::Deadline("start"),
        };
        Err(ResidentExit {
            resident: descriptor.id.clone(),
            reason,
            stderr: ended.stderr,
        }
        .into())
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn info(&self) -> &ResidentInfo {
        &self.info
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Ends when the process does, on its own or after `stop`.
    pub async fn ended(self) -> Ended {
        self.owner.await.expect("the resident owner does not panic")
    }
}

/// Answers the resident's numbers stream from `backend`
/// (`crates/command-protocol/src/numbers.rs`), until the resident ends it.
/// A stream that broke is logged: the resident's draws then fail.
async fn answer_numbers(resident: &str, stream: RequestStream, backend: Arc<dyn ResidentBackend>) {
    let answered = stream
        .answer::<NumbersAsk, _>(|request, _reporter| {
            backend.reserve(request.conversation, request.sequence, request.count)
        })
        .await;
    if let Err(error) = answered {
        tracing::warn!(event = "resident_stream.broke", resident, stream = "numbers", %error, "a resident's stream broke, so its requests then fail");
    }
}

/// Hands the resident's messages to `backend` one after another, in the
/// order the resident wrote them (`crates/command-protocol/src/messages.rs`), and answers each once the connection took it, until the resident
/// ends the stream. A stream that broke is logged: the resident's sends then
/// fail.
async fn answer_messages(resident: &str, stream: RequestStream, backend: Arc<dyn ResidentBackend>) {
    // The answering end takes each message when it reads it, in order, and
    // the one forwarder hands them on in that order; the resident's own
    // bounded queue of requests limits how many are unanswered, which this
    // queue holds.
    type Taken = tokio::sync::oneshot::Sender<Result<(), String>>;
    let (queued, mut queue) =
        tokio::sync::mpsc::unbounded_channel::<(serde_json::Value, String, Taken)>();
    let forward = async {
        while let Some((message, conversation, taken)) = queue.recv().await {
            let told = backend.tell(resident.to_owned(), conversation, message).await;
            // A resident that stopped waiting needs no answer.
            let _left = taken.send(told);
        }
    };
    let answering = stream.answer::<MessagesAsk, _>(|request, _reporter| {
        let (taken, answer) = tokio::sync::oneshot::channel();
        // The forwarder outlives the answering end, which holds the sender.
        let _forwarded = queued.send((request.message, request.conversation, taken));
        async move {
            answer
                .await
                .unwrap_or_else(|_| Err("the runner stopped forwarding".to_owned()))
        }
    });
    let answered = tokio::select! {
        answered = answering => answered,
        () = forward => unreachable!("the forwarder ends only when the answering end does"),
    };
    if let Err(error) = answered {
        tracing::warn!(event = "resident_stream.broke", resident, stream = "messages", %error, "a resident's stream broke, so its requests then fail");
    }
}

/// Answers the resident's artifacts stream from `artifacts`
/// (`crates/command-protocol/src/artifacts.rs`), until the resident ends it.
/// A stream that broke is logged: the resident's requests then fail.
async fn answer_artifacts(resident: &str, stream: RequestStream, artifacts: Arc<ResidentArtifacts>) {
    let answered = stream
        .answer::<ArtifactsAsk, _>(|request, reporter| {
            let artifacts = artifacts.clone();
            async move {
                let ask = request.ask().map_err(|error| error.to_string())?;
                artifacts
                    .answer(ask, &|progress| reporter.report(progress))
                    .await
            }
        })
        .await;
    if let Err(error) = answered {
        tracing::warn!(event = "resident_stream.broke", resident, stream = "artifacts", %error, "a resident's stream broke, so its requests then fail");
    }
}

/// The owner of one resident process: it drives the HTTP/2 connection, drains
/// standard error, and on `stop` asks the resident to shut down, killing it
/// after the deadline. Whichever way the resident ends, the owner reaps it.
async fn own<Connect, Connection, Failure>(
    mut child: Box<dyn ChildWrapper>,
    connect: Connect,
    mut stderr: Stderr,
    stop: CancellationToken,
    connected: oneshot::Sender<Result<Client, ResidentError>>,
) -> Ended
where
    Connect: Future<Output = Result<(Client, Connection), ResidentError>>,
    Connection: Future<Output = Result<(), Failure>> + Send + 'static,
    Failure: std::fmt::Display + Send + 'static,
{
    tokio::pin!(connect);
    let (client, connection) = loop {
        tokio::select! {
            _ = stop.cancelled() => return finish(child, stderr, None).await,
            status = wait_leader(child.as_mut()) => {
                return finish(child, stderr, Some(exited(status))).await;
            }
            () = stderr.read(), if stderr.is_open() => {}
            result = &mut connect => match result {
                Ok(connection) => break connection,
                Err(error) => {
                    let reason = ExitReason::Protocol(error.to_string());
                    let _starting = connected.send(Err(error));
                    return finish(child, stderr, Some(reason)).await;
                }
            },
        }
    };
    // The start may have given up; the owner then sees `stop`.
    let _starting = connected.send(Ok(client.clone()));
    let mut connection = tokio::spawn(connection);
    let reason = loop {
        tokio::select! {
            _ = stop.cancelled() => {
                shut_down(&client, &mut child, &mut stderr, &mut connection).await;
                break None;
            }
            status = wait_leader(child.as_mut()) => break Some(exited(status)),
            result = &mut connection => {
                let reason = match result {
                    Ok(Ok(())) => "it closed its connection".to_owned(),
                    Ok(Err(error)) => error.to_string(),
                    Err(error) => error.to_string(),
                };
                // Its own exit status tells more than the broken connection.
                break match tokio::time::timeout(EXIT_GRACE, wait_leader(child.as_mut())).await {
                    Ok(status) => Some(exited(status)),
                    Err(_) => Some(ExitReason::Protocol(reason)),
                };
            }
            () = stderr.read(), if stderr.is_open() => {}
        }
    };
    connection.abort();
    finish(child, stderr, reason).await
}

/// Asks the resident to shut down and waits for its exit, still driving the
/// connection and draining standard error, for at most `STOP_TIMEOUT`.
async fn shut_down<Failure>(
    client: &Client,
    child: &mut Box<dyn ChildWrapper>,
    stderr: &mut Stderr,
    connection: &mut JoinHandle<Result<(), Failure>>,
) {
    let deadline = tokio::time::sleep(STOP_TIMEOUT);
    tokio::pin!(deadline);
    let request = client.shutdown();
    tokio::pin!(request);
    let mut requested = false;
    let mut connected = true;
    loop {
        tokio::select! {
            _ = &mut deadline => {
                tracing::warn!(
                    event = "resident.killed",
                    resident = stderr.source.as_str(),
                    waited_ms = STOP_TIMEOUT.as_millis() as u64,
                    reason = "stop_timeout",
                    "a resident did not shut down in time and was killed"
                );
                return;
            }
            _ = wait_leader(child.as_mut()) => return,
            // A resident may close its connection before it answers.
            _ = &mut request, if !requested => requested = true,
            _ = &mut *connection, if connected => connected = false,
            () = stderr.read(), if stderr.is_open() => {}
        }
    }
}

/// Kills the process unless it has exited, reaps it, and reads what is left
/// of its standard error.
async fn finish(
    mut child: Box<dyn ChildWrapper>,
    stderr: Stderr,
    reason: Option<ExitReason>,
) -> Ended {
    let running = match child.try_wait() {
        Ok(status) => status.is_none(),
        Err(error) => {
            tracing::warn!(event = "resident.finish_failed", resident = stderr.source.as_str(), %error, "the state of a resident's process could not be read");
            true
        }
    };
    if running && let Err(error) = Box::into_pin(child.kill()).await {
        tracing::warn!(event = "resident.kill_failed", resident = stderr.source.as_str(), %error, "a resident's process could not be killed");
    }
    Ended {
        reason,
        stderr: stderr.finish().await,
    }
}

fn exited(status: std::io::Result<std::process::ExitStatus>) -> ExitReason {
    match status {
        Ok(status) => ExitReason::Exited(status),
        Err(error) => ExitReason::Protocol(format!("its exit status is unknown: {error}")),
    }
}

/// Hands one line of a resident's standard error to the Host's log. A resident
/// writes its `tracing` events as JSON lines (`demi_command_sdk::log_to_stderr`),
/// which the log keeps as they are, in the `resident_line` field that
/// `crates/runner/src/host_log.rs` reads; any other line is kept as text, as a
/// panic's report is.
fn keep(source: &str, line: &str) {
    let parsed = line
        .starts_with('{')
        .then(|| serde_json::from_str::<wire::LogLine>(line).ok())
        .flatten()
        .filter(|parsed| wire::is_event_name(&parsed.event));
    let Some(parsed) = parsed else {
        tracing::info!(source, "{line}");
        return;
    };
    match parsed.level {
        wire::LogLevel::Trace => tracing::trace!(resident_line = line, "{}", parsed.event),
        wire::LogLevel::Debug => tracing::debug!(resident_line = line, "{}", parsed.event),
        wire::LogLevel::Info => tracing::info!(resident_line = line, "{}", parsed.event),
        wire::LogLevel::Warn => tracing::warn!(resident_line = line, "{}", parsed.event),
        wire::LogLevel::Error => tracing::error!(resident_line = line, "{}", parsed.event),
    }
}

/// A resident's standard error, which goes two ways (`crates/command-protocol/src/lib.rs`): each line to the Host's log as it arrives, and
/// the end kept to accompany the resident's failure.
struct Stderr {
    pipe: Option<ChildStderr>,
    source: String,
    lines: LineSplitter,
    tail: TailBuffer,
}

impl Stderr {
    fn new(pipe: ChildStderr, source: String) -> Self {
        Self {
            pipe: Some(pipe),
            source,
            lines: LineSplitter::default(),
            // A byte is at most one UTF-16 unit of the text it becomes.
            tail: TailBuffer::new(wire::RESIDENT_STDERR_CHARS),
        }
    }

    fn is_open(&self) -> bool {
        self.pipe.is_some()
    }

    /// Reads one chunk; the end of the pipe closes it.
    async fn read(&mut self) {
        let Some(pipe) = &mut self.pipe else {
            return std::future::pending().await;
        };
        let mut buffer = [0; 4096];
        match pipe.read(&mut buffer).await {
            Ok(0) => self.close(),
            Ok(count) => {
                for line in self.lines.push(&buffer[..count]) {
                    keep(&self.source, &line);
                }
                self.tail.push(&buffer[..count]);
            }
            Err(error) => {
                tracing::warn!(event = "resident.stderr_read_failed", resident = self.source.as_str(), %error, "a resident's standard error could not be read");
                self.close();
            }
        }
    }

    fn close(&mut self) {
        self.pipe = None;
        if let Some(line) = std::mem::take(&mut self.lines).finish() {
            keep(&self.source, &line);
        }
    }

    /// Reads to the end of the pipe, for at most `STDERR_GRACE`.
    async fn finish(mut self) -> String {
        let deadline = tokio::time::sleep(STDERR_GRACE);
        tokio::pin!(deadline);
        while self.is_open() {
            tokio::select! {
                _ = &mut deadline => self.close(),
                () = self.read() => {}
            }
        }
        self.tail.text()
    }
}
