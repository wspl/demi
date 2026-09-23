//! Resident shell job with bounded input/output and an owned completion task.

use super::{ShellOptions, ShellRuntime, scope::Scope};
use crate::process::{OutputChunk, OutputStream, ProcessExit, ProcessInput};
use bytes::Bytes;
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
}

impl Pipes {
    /// Out of open files, this waits for one on a shell thread
    /// (`runner.md` § Load).
    async fn open(shell: &ShellRuntime) -> io::Result<Self> {
        shell
            .spawn_blocking(|| {
                let (stdin, input_writer) = pipe()?;
                let (output_reader, stdout) = pipe()?;
                let (error_reader, stderr) = pipe()?;
                let input_reference =
                    demi_command_service::descriptors::retry_blocking(|| stdin.try_clone())?;
                Ok(Self {
                    stdin,
                    input_writer,
                    output_reader,
                    stdout,
                    error_reader,
                    stderr,
                    input_reference,
                })
            })
            .await
            .map_err(io::Error::other)?
    }
}

impl Job {
    pub async fn start(
        script: String,
        cwd: PathBuf,
        mut env: BTreeMap<String, String>,
        live: bool,
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
        } = Pipes::open(shell).await?;
        env.remove(crate::stdio::LIVE_INPUT_ENV);
        if live {
            env.insert(
                crate::stdio::LIVE_INPUT_ENV.into(),
                crate::stdio::live_reference(&input_reference)?,
            );
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
            pump(shell, error_reader, OutputStream::Stderr, sender, cancel.clone()),
        ];
        let worker = shell.spawn_blocking(move || {
            let _input_reference = input_reference;
            let runtime = tokio::runtime::Handle::current();
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                runtime.block_on(super::execute(
                    &script,
                    ShellOptions {
                        scope: scope.clone(),
                        login: true,
                        cwd,
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
                        .is_some_and(|error| error.is::<super::scope::Cancelled>())
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
            (
                ProcessExit {
                    code,
                    signal: owner_cancel.is_cancelled().then(|| {
                        owner_signal
                            .get()
                            .cloned()
                            .unwrap_or_else(|| "SIGKILL".into())
                    }),
                    error: if owner_cancel.is_cancelled() {
                        None
                    } else {
                        error
                    },
                },
                cwd,
            )
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

    pub fn cancel(&self) {
        self.cancel.cancel();
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
    pub async fn signal(&self, signal: &str) -> io::Result<()> {
        match signal {
            "SIGINT" | "SIGTERM" | "SIGKILL" | "SIGHUP" | "SIGQUIT" => {
                if !self.cancel.is_cancelled() {
                    self.requested_signal.get_or_init(|| signal.into());
                }
                self.cancel();
                Ok(())
            }
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsupported shell job signal",
            )),
        }
    }
    /// The job's exit, once everything it ran has finished.
    pub async fn wait(&mut self) -> (ProcessExit, Option<String>) {
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
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel();
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
            let mut buffer = vec![0; 64 * 1024];
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
            let mut buffer = vec![0; 64 * 1024];
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

pub(super) fn pipe() -> io::Result<(File, File)> {
    // Out of open files, the job waits for one (`runner.md` § Load).
    let (reader, writer) = demi_command_service::descriptors::retry_blocking(std::io::pipe)?;
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
