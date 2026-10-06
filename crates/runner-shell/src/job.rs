//! Resident shell job with bounded input/output and an owned completion task.

use crate::{
    ShellRuntime,
    interpreter::{ShellOptions, execute},
    scope::Scope,
};
use bytes::Bytes;
use demi_runner_process::{
    job_shell::ShellJob,
    process::{OutputChunk, ProcessExit, ProcessInput},
    stdio::{JOB_OUTPUT_ENV, LIVE_INPUT_ENV, reference},
};
use demi_runner_protocol::wire::{OutputStream, Signal};
use futures_util::future::BoxFuture;
use std::{
    collections::BTreeMap,
    fs::File,
    io,
    path::PathBuf,
    sync::{Arc, OnceLock},
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub struct Job {
    pub input: mpsc::Sender<ProcessInput>,
    pub output: mpsc::Receiver<OutputChunk>,
    /// Ends with the job's exit and last working directory, once everything
    /// the job ran has finished.
    owner: Option<tokio::task::JoinHandle<(ProcessExit, Option<String>)>>,
    exited: Option<(ProcessExit, Option<String>)>,
    cancel: CancellationToken,
    requested_signal: Arc<OnceLock<String>>,
}

/// The most one read of a job's output takes.
const OUTPUT_CHUNK_BYTES: usize = 64 * 1024;

/// A job's three pipes: the ends the interpreter uses and the runner's.
struct Pipes {
    stdin: File,
    input_writer: File,
    output_reader: File,
    stdout: File,
    error_reader: File,
    stderr: File,
    /// Kept open until every interpreter task finishes.
    input_reference: File,
    /// A copy of the job's stdout pipe, which `DEMI_JOB_OUTPUT` names
    /// (`runner.md` § Where a command's stdout goes); kept open as long.
    output_reference: File,
}

impl Pipes {
    /// Out of open files, this waits for one on a shell thread until the
    /// job is cancelled (`runner.md` § Load).
    async fn open(shell: &ShellRuntime, scope: &Scope) -> io::Result<Self> {
        let scope = scope.clone();
        shell
            .spawn_blocking(move || {
                let (stdin, input_writer) = scope.descriptors(pipe)?;
                let (output_reader, stdout) = scope.descriptors(pipe)?;
                let (error_reader, stderr) = scope.descriptors(pipe)?;
                let input_reference = scope.duplicate(&stdin)?;
                let output_reference = scope.duplicate(&stdout)?;
                Ok(Self {
                    stdin,
                    input_writer,
                    output_reader,
                    stdout,
                    error_reader,
                    stderr,
                    input_reference,
                    output_reference,
                })
            })
            .await
            .map_err(io::Error::other)?
    }
}

impl Job {
    /// Starts `script`. `live` says whether the job's stdin is its live
    /// terminal, and `output` whether its stdout is the job's output, which
    /// it is unless the backend relays it elsewhere.
    pub async fn start(
        script: String,
        cwd: PathBuf,
        workspace: PathBuf,
        mut env: BTreeMap<String, String>,
        live: bool,
        output: bool,
        mut scope: Scope,
        shell: &ShellRuntime,
    ) -> io::Result<Self> {
        let Pipes {
            stdin,
            input_writer,
            output_reader,
            stdout,
            error_reader,
            stderr,
            input_reference,
            output_reference,
        } = Pipes::open(shell, &scope).await?;
        env.remove(LIVE_INPUT_ENV);
        if live {
            env.insert(LIVE_INPUT_ENV.into(), reference(&input_reference)?);
        }
        env.remove(JOB_OUTPUT_ENV);
        if output {
            env.insert(JOB_OUTPUT_ENV.into(), reference(&output_reference)?);
        }
        let (input, receiver) = mpsc::channel(4);
        let (sender, output) = mpsc::channel(4);
        let cancel = scope.cancellation.clone();
        let owner_cancel = cancel.clone();
        let requested_signal = Arc::new(OnceLock::<String>::new());
        let owner_signal = requested_signal.clone();
        scope.cancellation = cancel.child_token();
        let input_cancel = cancel.child_token();
        let writer = feed(shell, input_writer, receiver, input_cancel.clone());
        let readers = [
            pump(
                shell,
                output_reader,
                OutputStream::Stdout,
                sender.clone(),
                cancel.clone(),
            ),
            pump(
                shell,
                error_reader,
                OutputStream::Stderr,
                sender,
                cancel.clone(),
            ),
        ];
        let worker = shell.spawn_blocking(move || {
            let _input_reference = input_reference;
            let _output_reference = output_reference;
            let runtime = tokio::runtime::Handle::current();
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                runtime.block_on(execute(
                    &script,
                    ShellOptions {
                        scope: scope.clone(),
                        login: true,
                        cwd,
                        workspace,
                        env,
                        stdin,
                        stdout,
                        stderr,
                    },
                ))
            }));
            // Preserve successful process-substitution output until its producer
            // or consumer finishes. Failed execution cancels remaining work.
            if !matches!(outcome, Ok(Ok(_))) {
                scope.cancellation.cancel();
            }
            runtime.block_on(scope.finish());
            outcome
        });
        let owner = tokio::spawn(async move {
            let outcome = worker.await;
            input_cancel.cancel();
            let mut error = match writer.await {
                Ok(Err(error))
                    if !matches!(
                        error.kind(),
                        io::ErrorKind::BrokenPipe | io::ErrorKind::Interrupted
                    ) && !error
                        .get_ref()
                        .is_some_and(|error| error.is::<crate::scope::Cancelled>())
                        && !owner_cancel.is_cancelled() =>
                {
                    Some(error.to_string())
                }
                Err(error) => Some(error.to_string()),
                _ => None,
            };
            for reader in readers {
                match reader.await {
                    Ok(Err(failure)) if !owner_cancel.is_cancelled() => {
                        error = Some(failure.to_string())
                    }
                    Err(failure) => error = Some(failure.to_string()),
                    _ => {}
                }
            }
            let (code, cwd) = match outcome {
                Ok(Ok(Ok(result))) => (
                    Some(i32::from(result.code)),
                    Some(result.cwd.to_string_lossy().into_owned()),
                ),
                Ok(Ok(Err(failure))) => {
                    error = Some(failure.to_string());
                    (None, None)
                }
                Ok(Err(_)) => {
                    error = Some("shell worker panicked".into());
                    (None, None)
                }
                Err(failure) => {
                    error = Some(failure.to_string());
                    (None, None)
                }
            };
            // A cancelled job reports the signal that asked for its end, or
            // `SIGKILL`, and no exit code: bash's own is the interruption's
            // (`runner.md` § Cancellation and completion).
            let exit = if owner_cancel.is_cancelled() {
                ProcessExit {
                    code: None,
                    signal: Some(
                        owner_signal
                            .get()
                            .cloned()
                            .unwrap_or_else(|| "SIGKILL".into()),
                    ),
                    error: None,
                }
            } else {
                ProcessExit {
                    code,
                    signal: None,
                    error,
                }
            };
            (exit, cwd)
        });
        Ok(Self {
            input,
            output,
            owner: Some(owner),
            exited: None,
            cancel,
            requested_signal,
        })
    }
}

impl ShellJob for Job {
    fn input(&self) -> &mpsc::Sender<ProcessInput> {
        &self.input
    }

    fn output(&mut self) -> &mut mpsc::Receiver<OutputChunk> {
        &mut self.output
    }

    fn cancel(&self) {
        self.cancel.cancel();
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    fn signal(&self, signal: Signal) -> io::Result<()> {
        match signal {
            Signal::Interrupt
            | Signal::Terminate
            | Signal::Kill
            | Signal::Hangup
            | Signal::Quit => {
                if !self.cancel.is_cancelled() {
                    self.requested_signal.get_or_init(|| signal.to_string());
                }
                self.cancel();
                Ok(())
            }
            Signal::User1 | Signal::User2 | Signal::Stop | Signal::Continue => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsupported shell job signal",
            )),
        }
    }

    fn wait(&mut self) -> BoxFuture<'_, (ProcessExit, Option<String>)> {
        Box::pin(async move {
            if let Some(owner) = self.owner.take() {
                self.exited = Some(owner.await.unwrap_or_else(|_| {
                    (
                        ProcessExit {
                            code: None,
                            signal: None,
                            error: Some("the shell job's owner panicked".into()),
                        },
                        None,
                    )
                }));
            }
            self.exited.clone().expect("the owner left the exit")
        })
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// Writes the job's input into its stdin pipe. On Unix the runner's end is
/// asynchronous and takes no thread; on Windows it takes one of the shell
/// pool's.
fn feed(
    shell: &ShellRuntime,
    file: File,
    mut receiver: mpsc::Receiver<ProcessInput>,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<io::Result<()>> {
    #[cfg(unix)]
    {
        use tokio::io::AsyncWriteExt;
        let _ = shell;
        tokio::spawn(async move {
            let mut pipe = tokio::net::unix::pipe::Sender::from_file(file)?;
            loop {
                let item = tokio::select! {
                    _ = cancel.cancelled() => return Ok(()),
                    item = receiver.recv() => item,
                };
                let Some(ProcessInput::Bytes(bytes)) = item else {
                    return Ok(());
                };
                tokio::select! {
                    _ = cancel.cancelled() => return Ok(()),
                    written = pipe.write_all(&bytes) => written?,
                }
            }
        })
    }
    #[cfg(windows)]
    {
        shell.spawn_blocking(move || -> io::Result<()> {
            let scope = Scope::new(cancel, None);
            let runtime = tokio::runtime::Handle::current();
            loop {
                let item = runtime.block_on(async {
                    tokio::select! {
                        _ = scope.cancellation.cancelled() => None,
                        item = receiver.recv() => item,
                    }
                });
                match item {
                    Some(ProcessInput::Bytes(mut bytes)) => {
                        while !bytes.is_empty() {
                            let count = scope.write(&file, &bytes)?;
                            if count == 0 {
                                return Err(io::ErrorKind::WriteZero.into());
                            }
                            bytes = bytes.slice(count..);
                        }
                    }
                    Some(ProcessInput::End) | None => return Ok(()),
                }
            }
        })
    }
}

/// Reads one of the job's output pipes into chunks for the task. On Unix the
/// runner's end is asynchronous and takes no thread; on Windows it takes one
/// of the shell pool's.
fn pump(
    shell: &ShellRuntime,
    file: File,
    stream: OutputStream,
    sender: mpsc::Sender<OutputChunk>,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<io::Result<()>> {
    let cancelled = || io::Error::new(io::ErrorKind::Interrupted, "job output cancelled");
    let closed = || io::Error::new(io::ErrorKind::BrokenPipe, "job output consumer closed");
    #[cfg(unix)]
    {
        use tokio::io::AsyncReadExt;
        let _ = shell;
        tokio::spawn(async move {
            let mut pipe = tokio::net::unix::pipe::Receiver::from_file(file)?;
            let mut buffer = vec![0; OUTPUT_CHUNK_BYTES];
            loop {
                let count = tokio::select! {
                    _ = cancel.cancelled() => return Err(cancelled()),
                    count = pipe.read(&mut buffer) => count?,
                };
                if count == 0 {
                    return Ok(());
                }
                let chunk = OutputChunk {
                    stream,
                    bytes: Bytes::copy_from_slice(&buffer[..count]),
                };
                tokio::select! {
                    _ = cancel.cancelled() => return Err(cancelled()),
                    sent = sender.send(chunk) => sent.map_err(|_| closed())?,
                }
            }
        })
    }
    #[cfg(windows)]
    {
        shell.spawn_blocking(move || {
            let scope = Scope::new(cancel.clone(), None);
            let runtime = tokio::runtime::Handle::current();
            let mut buffer = vec![0; OUTPUT_CHUNK_BYTES];
            let result = (|| {
                loop {
                    let count = scope.read(&file, &mut buffer)?;
                    if count == 0 {
                        return Ok(());
                    }
                    let chunk = OutputChunk {
                        stream,
                        bytes: Bytes::copy_from_slice(&buffer[..count]),
                    };
                    runtime.block_on(async {
                        tokio::select! {
                            _ = cancel.cancelled() => Err(cancelled()),
                            result = sender.send(chunk) => result.map_err(|_| closed()),
                        }
                    })?;
                }
            })();
            runtime.block_on(scope.finish());
            result
        })
    }
}

/// A pipe as two files; a caller waits out a lack of open files with
/// `Scope::descriptors`.
pub(crate) fn pipe() -> io::Result<(File, File)> {
    let (reader, writer) = std::io::pipe()?;
    #[cfg(unix)]
    {
        use std::os::fd::OwnedFd;
        Ok((
            File::from(OwnedFd::from(reader)),
            File::from(OwnedFd::from(writer)),
        ))
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::OwnedHandle;
        Ok((
            File::from(OwnedHandle::from(reader)),
            File::from(OwnedHandle::from(writer)),
        ))
    }
}
