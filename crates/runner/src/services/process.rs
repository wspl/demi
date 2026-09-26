//! One resident service process (`native-runtime.md` § Invoke and retire a
//! service): the runner starts the executable, speaks HTTP/2 to it over its
//! standard input and output, drains its standard error into the Host log,
//! and reaps it. One owner task holds the process from its start to its end
//! and returns how it ended, with the end of its standard error.

use std::{collections::BTreeMap, future::Future, path::Path, process::Stdio, time::Duration};

use demi_command_service::{
    Client, ServiceError,
    protocol::{PackageDescriptor, ServiceInfo},
};
use process_wrap::tokio::ChildWrapper;
use tokio::{
    io::AsyncReadExt,
    process::{ChildStderr, Command},
    sync::oneshot,
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

use super::{ExitReason, RuntimeError, ServiceExit};
use crate::{
    connection::wire,
    host_log::LineSplitter,
    tail::TailBuffer,
};

/// How long a new service has to finish the handshake and answer its catalog.
const START_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a service has to shut down when asked (`native-runtime.md`
/// § Invoke and retire a service).
const STOP_TIMEOUT: Duration = Duration::from_secs(6);
/// A service whose connection closed usually is exiting: its exit status is
/// awaited this long before the runner stops it as broken.
const EXIT_GRACE: Duration = Duration::from_millis(250);
/// Standard error is still read this long after the process ends, so a
/// descendant that keeps the pipe open cannot hold up the service's end.
const STDERR_GRACE: Duration = Duration::from_millis(250);

pub struct ResidentService {
    client: Client,
    info: ServiceInfo,
    pid: u32,
    owner: JoinHandle<Ended>,
}

/// How a service process ended.
pub struct Ended {
    /// Why it ended on its own; `None` when it was asked to stop.
    pub reason: Option<ExitReason>,
    /// The end of its standard error.
    pub stderr: String,
}

impl ResidentService {
    /// Starts `executable` and checks that it serves `descriptor`.
    /// Cancelling `stop` asks the service to shut down, also while it starts.
    pub async fn start(
        executable: &Path,
        descriptor: &PackageDescriptor,
        cwd: &Path,
        env: &BTreeMap<String, String>,
        stop: CancellationToken,
    ) -> Result<Self, RuntimeError> {
        let mut command = Command::new(executable);
        command
            .arg("--command-service")
            .current_dir(cwd)
            .env_clear()
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let attributes = brush_core::execution_host::ChildAttributes::default();
        let mut command = crate::process::wrap(command, true, &attributes);
        // A start that waits (`crate::process::start`) ends with `stop`.
        let mut child = tokio::select! {
            biased;
            _ = stop.cancelled() => return Err(RuntimeError::Cancelled),
            child = crate::process::start(|| command.spawn()) => child?,
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
            Stderr::new(stderr, format!("service:{}", descriptor.id)),
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
            Ok::<_, ServiceError>(Some((client, info)))
        };
        let started = tokio::time::timeout(START_TIMEOUT, started).await;
        if let Ok(Ok(Some((client, info)))) = &started
            && descriptor.serves(info)
        {
            return Ok(Self {
                client: client.clone(),
                info: info.clone(),
                pid,
                owner,
            });
        }
        owner_stop.cancel();
        let ended = owner
            .await
            .expect("the service owner does not panic");
        if stop.is_cancelled() {
            return Err(RuntimeError::Cancelled);
        }
        let reason = match (started, ended.reason) {
            (_, Some(reason)) => reason,
            (Ok(Ok(Some(_))), None) => return Err(RuntimeError::CatalogMismatch),
            (Ok(Err(error)), None) => ExitReason::Protocol(error.to_string()),
            (Ok(Ok(None)) | Err(_), None) => ExitReason::Deadline("start"),
        };
        Err(ServiceExit {
            service: descriptor.id.clone(),
            reason,
            stderr: ended.stderr,
        }
        .into())
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn info(&self) -> &ServiceInfo {
        &self.info
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Ends when the process does, on its own or after `stop`.
    pub async fn ended(self) -> Ended {
        self.owner.await.expect("the service owner does not panic")
    }
}

/// The owner of one service process: it drives the HTTP/2 connection, drains
/// standard error, and on `stop` asks the service to shut down, killing it
/// after the deadline. Whichever way the service ends, the owner reaps it.
async fn own<Connect, Connection, Failure>(
    mut child: Box<dyn ChildWrapper>,
    connect: Connect,
    mut stderr: Stderr,
    stop: CancellationToken,
    connected: oneshot::Sender<Result<Client, ServiceError>>,
) -> Ended
where
    Connect: Future<Output = Result<(Client, Connection), ServiceError>>,
    Connection: Future<Output = Result<(), Failure>> + Send + 'static,
    Failure: std::fmt::Display + Send + 'static,
{
    tokio::pin!(connect);
    let (client, connection) = loop {
        tokio::select! {
            _ = stop.cancelled() => return finish(child, stderr, None).await,
            status = child.wait() => {
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
            status = child.wait() => break Some(exited(status)),
            result = &mut connection => {
                let reason = match result {
                    Ok(Ok(())) => "it closed its connection".to_owned(),
                    Ok(Err(error)) => error.to_string(),
                    Err(error) => error.to_string(),
                };
                // Its own exit status tells more than the broken connection.
                break match tokio::time::timeout(EXIT_GRACE, child.wait()).await {
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

/// Asks the service to shut down and waits for its exit, still driving the
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
                    "{} did not shut down within {} seconds and was killed",
                    stderr.source,
                    STOP_TIMEOUT.as_secs()
                );
                return;
            }
            _ = child.wait() => return,
            // A service may close its connection before it answers.
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
            tracing::warn!("{}: {error}", stderr.source);
            true
        }
    };
    if running && let Err(error) = Box::into_pin(child.kill()).await {
        tracing::warn!("{} could not be killed: {error}", stderr.source);
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

/// A service's standard error, which goes two ways (`native-runtime.md`
/// § Invocation protocol): each line to the Host's log as it arrives, and
/// the end kept to accompany the service's failure.
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
            tail: TailBuffer::new(wire::SERVICE_STDERR_CHARS),
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
                    tracing::info!(source = self.source.as_str(), "{line}");
                }
                self.tail.push(&buffer[..count]);
            }
            Err(error) => {
                tracing::warn!("{} standard error: {error}", self.source);
                self.close();
            }
        }
    }

    fn close(&mut self) {
        self.pipe = None;
        if let Some(line) = std::mem::take(&mut self.lines).finish() {
            tracing::info!(source = self.source.as_str(), "{line}");
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
