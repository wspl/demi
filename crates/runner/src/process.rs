use std::{
    collections::BTreeMap,
    io,
    path::PathBuf,
    process::{ExitStatus, Stdio},
    time::Duration,
};

use bytes::Bytes;
use demi_command_service::descriptors::{self, Backoff};
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use futures_util::StreamExt;
use tokio::{
    io::{AsyncRead, AsyncWriteExt},
    process::Command,
    sync::mpsc,
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

/// The most one read of a process's output takes.
const OUTPUT_CHUNK_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone)]
pub struct SpawnOptions {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub process_group: bool,
}

#[derive(Debug, Clone)]
pub struct SpawnFailure {
    pub kind: SpawnErrorKind,
    pub message: String,
}

pub use crate::connection::wire::{OutputStream, Signal};
use crate::connection::wire::SpawnErrorKind;

pub struct OutputChunk {
    pub stream: OutputStream,
    pub bytes: Bytes,
}

#[derive(Debug, Clone)]
pub struct ProcessExit {
    pub code: Option<i32>,
    pub signal: Option<String>,
    pub error: Option<String>,
}

pub enum ProcessInput {
    Bytes(Bytes),
    End,
}

/// Handles never own child processes implicitly: their owner task reaps on every
/// completion path, and dropping a handle cancels that owner.
pub struct ChildProcess {
    pub input: mpsc::Sender<ProcessInput>,
    pub output: mpsc::Receiver<OutputChunk>,
    signals: mpsc::Sender<Signal>,
    /// Ends with the process's exit, once it is reaped and its pipes drained.
    owner: Option<tokio::task::JoinHandle<ProcessExit>>,
    exited: Option<ProcessExit>,
    cancel: CancellationToken,
    pub pid: u32,
}

impl ChildProcess {
    pub async fn spawn(options: SpawnOptions) -> Result<Self, SpawnFailure> {
        let mut command = Command::new(&options.command);
        command
            .args(&options.args)
            .current_dir(&options.cwd)
            .env_clear()
            .envs(&options.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut command = CommandWrap::from(command);
        command.wrap(KillOnDrop);
        if options.process_group {
            #[cfg(unix)]
            command.wrap(process_wrap::tokio::ProcessGroup::leader());
            #[cfg(windows)]
            command.wrap(process_wrap::tokio::JobObject);
        }
        let mut child = match start(|| command.spawn()).await {
            Ok(child) => child,
            Err(error) => return Err(classify_failure(error, &options).await),
        };
        let pid = child.id().expect("newly spawned process has an ID");
        let mut stdin = child.stdin().take().expect("piped stdin");
        let stdout = child.stdout().take().expect("piped stdout");
        let stderr = child.stderr().take().expect("piped stderr");
        let (input, mut input_rx) = mpsc::channel(4);
        let (output_tx, output) = mpsc::channel(4);
        let (signals, mut signal_rx) = mpsc::channel::<Signal>(4);
        let cancel = CancellationToken::new();
        let owner_cancel = cancel.clone();
        let owner = tokio::spawn(async move {
            let io_cancel = CancellationToken::new();
            let stdin_cancel = CancellationToken::new();
            let mut readers = JoinSet::new();
            readers.spawn(pump(
                stdout,
                OutputStream::Stdout,
                output_tx.clone(),
                io_cancel.clone(),
            ));
            readers.spawn(pump(
                stderr,
                OutputStream::Stderr,
                output_tx,
                io_cancel.clone(),
            ));
            let writer_cancel = stdin_cancel.clone();
            let writer = tokio::spawn(async move {
                tokio::select! {
                    _ = writer_cancel.cancelled() => Ok(()),
                    result = async {
                        while let Some(input) = input_rx.recv().await {
                            match input {
                                ProcessInput::Bytes(bytes) => stdin.write_all(&bytes).await?,
                                ProcessInput::End => break,
                            }
                        }
                        stdin.shutdown().await
                    } => result,
                }
            });
            let mut failure = None;
            let status = loop {
                tokio::select! {
                    biased;
                    _ = owner_cancel.cancelled() => {
                        if let Err(error) = kill(child.as_mut()) {
                            failure = Some(error.to_string());
                        }
                        break child.wait().await;
                    }
                    signal = signal_rx.recv(), if !signal_rx.is_closed() => {
                        if let Some(signal) = signal && let Err(error) = send_signal(child.as_mut(), signal) {
                            failure = Some(error.to_string());
                        }
                    }
                    status = child.wait() => break status,
                    reader = readers.join_next(), if !readers.is_empty() => {
                        match reader {
                            Some(Ok(Err(error))) => {
                                failure = Some(error.to_string());
                                owner_cancel.cancel();
                            }
                            Some(Err(error)) => {
                                failure = Some(error.to_string());
                                owner_cancel.cancel();
                            }
                            _ => {},
                        }
                    }
                }
            };
            stdin_cancel.cancel();
            match writer.await {
                Ok(Err(error)) if error.kind() != io::ErrorKind::BrokenPipe => {
                    failure = Some(error.to_string());
                }
                Err(error) => {
                    failure = Some(error.to_string());
                }
                // The child may close stdin before finishing; this is not a process failure.
                _ => {}
            }
            if options.process_group {
                // The leader's exit also ends this invocation's descendants.
                if let Err(error) = kill(child.as_mut()) {
                    failure = Some(error.to_string());
                }
            }
            while !readers.is_empty() {
                tokio::select! {
                    biased;
                    _ = owner_cancel.cancelled(), if !io_cancel.is_cancelled() => {
                        // The leader may have exited while a descendant still owns its pipes.
                        if let Err(error) = kill(child.as_mut()) {
                            failure = Some(error.to_string());
                        }
                        io_cancel.cancel();
                    }
                    reader = readers.join_next() => match reader {
                        Some(Ok(Err(error))) => { failure = Some(error.to_string()); }
                        Some(Err(error)) => { failure = Some(error.to_string()); }
                        _ => {},
                    }
                }
            }
            match status {
                Ok(status) => ProcessExit {
                    code: status.code(),
                    signal: exit_signal(status),
                    error: failure,
                },
                Err(error) => ProcessExit {
                    code: None,
                    signal: None,
                    error: Some(error.to_string()),
                },
            }
        });
        Ok(Self {
            input,
            output,
            signals,
            owner: Some(owner),
            exited: None,
            cancel,
            pid,
        })
    }

    pub async fn signal(&self, signal: Signal) -> io::Result<()> {
        self.signals
            .send(signal)
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "process has exited"))
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// The process's exit, once it is reaped and its pipes drained.
    pub async fn wait(&mut self) -> ProcessExit {
        if let Some(owner) = self.owner.take() {
            self.exited = Some(owner.await.unwrap_or_else(|_| ProcessExit {
                code: None,
                signal: None,
                error: Some("the process's owner panicked".into()),
            }));
        }
        self.exited.clone().expect("the owner left the exit")
    }
}

impl Drop for ChildProcess {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// Sends one of the process's output streams to its owner, a chunk per read.
/// Cancellation stops a read and a send alike, since the owner may have
/// stopped taking chunks.
async fn pump<T: AsyncRead + Unpin>(
    input: T,
    stream: OutputStream,
    output: mpsc::Sender<OutputChunk>,
    cancel: CancellationToken,
) -> io::Result<()> {
    let chunks = tokio_util::io::ReaderStream::with_capacity(input, OUTPUT_CHUNK_BYTES);
    tokio::pin!(chunks);
    loop {
        let bytes = tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            bytes = chunks.next() => match bytes {
                Some(bytes) => bytes?,
                None => return Ok(()),
            },
        };
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            sent = output.send(OutputChunk { stream, bytes }) => {
                if sent.is_err() {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "process output consumer closed",
                    ));
                }
            }
        }
    }
}

/// Starts a process with `attempt`, which tries to spawn it once. Every process
/// the runner starts, a service, a raw process, a job's external command or a
/// utility's child program, starts this way, so none fails for a condition
/// that passes (`runner.md` § Load; `StartRetry` says which).
pub(crate) async fn start<T>(mut attempt: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let mut retry = StartRetry::default();
    loop {
        let error = match attempt() {
            Ok(started) => return Ok(started),
            Err(error) => error,
        };
        let Some(pause) = retry.pause(&error) else {
            return Err(error);
        };
        tokio::time::sleep(pause).await;
    }
}

/// `start` on a thread that may block, such as the shell's hooks.
pub(crate) fn start_blocking<T>(mut attempt: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let mut retry = StartRetry::default();
    loop {
        let error = match attempt() {
            Ok(started) => return Ok(started),
            Err(error) => error,
        };
        let Some(pause) = retry.pause(&error) else {
            return Err(error);
        };
        std::thread::sleep(pause);
    }
}

/// How long a start waits, in all, for its program to stop being busy.
const BUSY_WAIT: Duration = Duration::from_secs(1);

/// When a failed process start is tried again.
///
/// - Out of open files, always: another descriptor will close
///   (`demi_command_service::descriptors`).
/// - While its program is busy (`ETXTBSY`), for at most `BUSY_WAIT`. Linux
///   refuses to run a file that any process holds open for writing, and the
///   runner causes that itself. It writes files that it then runs, such as a
///   service executable it has just copied into its cache or a script a job
///   has just written, while other threads start processes. Starting a
///   process forks the runner, and the child holds a copy of every descriptor
///   the runner had open, the one writing that file too, until it runs its
///   own program. Close-on-exec closes the copy only then, and Linux has no
///   close-on-fork, so nothing the writer does can prevent the copy. A start
///   of the file in that moment fails although the runner has closed it. The
///   failure is inherent to fork and exec from threads on Linux, and it is
///   bounded: every child the runner forks runs its program at once (std's
///   and process-wrap's spawns; nothing in the runner forks otherwise), so the
///   copy lasts from a fork to its exec. A program still busy after
///   `BUSY_WAIT` is open for writing elsewhere, and the start fails with it.
#[derive(Default)]
struct StartRetry {
    backoff: Backoff,
    /// The pauses taken so far for a busy program.
    busy: Duration,
}

impl StartRetry {
    /// The pause before the next attempt, or `None` when `error` ends the
    /// start.
    fn pause(&mut self, error: &io::Error) -> Option<Duration> {
        if descriptors::exhausted(error) {
            return Some(self.backoff.pause());
        }
        if error.kind() != io::ErrorKind::ExecutableFileBusy || self.busy >= BUSY_WAIT {
            return None;
        }
        let pause = self.backoff.pause();
        self.busy += pause;
        Some(pause)
    }
}

pub(crate) fn kill(child: &mut dyn ChildWrapper) -> io::Result<()> {
    match child.start_kill() {
        #[cfg(unix)]
        Err(error) if gone(&error) => Ok(()),
        result => result,
    }
}

/// Whether signalling a child's process group failed only because the group
/// is already ending: ESRCH when it is gone, and on macOS EPERM, which XNU's
/// `killpg` returns while only exiting members remain. Its exit is still
/// reaped and reported.
#[cfg(unix)]
fn gone(error: &io::Error) -> bool {
    let code = error.raw_os_error();
    code == Some(rustix::io::Errno::SRCH.raw_os_error())
        || (cfg!(target_os = "macos") && code == Some(rustix::io::Errno::PERM.raw_os_error()))
}

fn send_signal(child: &mut dyn ChildWrapper, signal: Signal) -> io::Result<()> {
    #[cfg(unix)]
    {
        match child.signal(number(signal).as_raw()) {
            Err(error) if gone(&error) => Ok(()),
            result => result,
        }
    }
    #[cfg(windows)]
    {
        match signal {
            Signal::Terminate | Signal::Kill | Signal::Interrupt => kill(child),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsupported Windows process signal",
            )),
        }
    }
}

#[cfg(unix)]
fn number(signal: Signal) -> rustix::process::Signal {
    use rustix::process::Signal as Number;
    match signal {
        Signal::Terminate => Number::TERM,
        Signal::Kill => Number::KILL,
        Signal::Interrupt => Number::INT,
        Signal::Hangup => Number::HUP,
        Signal::Quit => Number::QUIT,
        Signal::User1 => Number::USR1,
        Signal::User2 => Number::USR2,
        Signal::Stop => Number::STOP,
        Signal::Continue => Number::CONT,
    }
}

/// The name of the signal that ended a process: one of the set requests
/// name, `SIGPIPE`, or its number.
fn exit_signal(status: ExitStatus) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        const SIGNALS: [Signal; 9] = [
            Signal::Terminate,
            Signal::Kill,
            Signal::Interrupt,
            Signal::Hangup,
            Signal::Quit,
            Signal::User1,
            Signal::User2,
            Signal::Stop,
            Signal::Continue,
        ];
        status.signal().map(|raw| {
            match SIGNALS.into_iter().find(|signal| number(*signal).as_raw() == raw) {
                Some(signal) => signal.to_string(),
                None if raw == rustix::process::Signal::PIPE.as_raw() => "SIGPIPE".into(),
                None => format!("SIG{raw}"),
            }
        })
    }
    #[cfg(windows)]
    {
        let _ = status;
        None
    }
}

async fn classify_failure(error: io::Error, options: &SpawnOptions) -> SpawnFailure {
    let kind = if !tokio::fs::metadata(&options.cwd)
        .await
        .is_ok_and(|metadata| metadata.is_dir())
    {
        SpawnErrorKind::CwdUnusable
    } else {
        match error.kind() {
            io::ErrorKind::NotFound => SpawnErrorKind::ExecutableNotFound,
            io::ErrorKind::PermissionDenied => SpawnErrorKind::PermissionDenied,
            io::ErrorKind::IsADirectory => SpawnErrorKind::IsDirectory,
            _ => SpawnErrorKind::Other,
        }
    };
    SpawnFailure {
        kind,
        message: error.to_string(),
    }
}
