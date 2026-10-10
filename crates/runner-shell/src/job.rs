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
    stdio::{LIVE_INPUT_ENV, reference},
};
use demi_runner_protocol::wire::{OutputStream, Signal};
use futures_util::future::BoxFuture;
use std::{
    collections::BTreeMap,
    fs::File,
    io,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

pub struct Job {
    pub input: mpsc::Sender<ProcessInput>,
    pub output: mpsc::Receiver<OutputChunk>,
    /// Ends with the job's exit, once everything the job ran has finished.
    owner: Option<tokio::task::JoinHandle<ProcessExit>>,
    exited: Option<ProcessExit>,
    /// Ends the job and everything it runs.
    cancel: CancellationToken,
    scope: Scope,
    /// The last signal other than `KILL` sent to stop the job, which its
    /// exit reports unless `KILL` ended it.
    stop_signal: Arc<Mutex<Option<Signal>>>,
    /// Asks the stdout pump to read what the pipe holds now, and answers
    /// once it is in [`Job::output`].
    drains: mpsc::UnboundedSender<oneshot::Sender<()>>,
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
    /// A copy of the job's stdin pipe, which `DEMI_LIVE_INPUT` names
    /// (`runner.md` § A job's own input); kept open until every
    /// interpreter task finishes.
    input_reference: File,
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
    /// Starts `script`. `live` says whether the job's stdin is its own
    /// input, which it is unless the backend relays it from another command.
    pub async fn start(
        script: String,
        cwd: PathBuf,
        mut env: BTreeMap<String, String>,
        live: bool,
        scope: Scope,
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
        } = Pipes::open(shell, &scope).await?;
        env.remove(LIVE_INPUT_ENV);
        if live {
            env.insert(LIVE_INPUT_ENV.into(), reference(&input_reference)?);
        }
        let (input, receiver) = mpsc::channel(4);
        let (sender, output) = mpsc::channel(4);
        let cancel = scope.work.cancellation.clone();
        let owner_cancel = cancel.clone();
        let stop_signal = Arc::new(Mutex::new(None::<Signal>));
        let owner_signal = stop_signal.clone();
        let job_scope = scope.clone();
        let owner_scope = scope.clone();
        let input_cancel = cancel.child_token();
        let writer = feed(shell, input_writer, receiver, input_cancel.clone());
        let (drains, drained) = mpsc::unbounded_channel();
        let readers = [
            pump(
                shell,
                output_reader,
                OutputStream::Stdout,
                sender.clone(),
                cancel.clone(),
                Some(drained),
            ),
            pump(
                shell,
                error_reader,
                OutputStream::Stderr,
                sender,
                cancel.clone(),
                None,
            ),
        ];
        let worker = shell.spawn_blocking(move || {
            let _input_reference = input_reference;
            let runtime = tokio::runtime::Handle::current();
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                runtime.block_on(execute(
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
            // or consumer finishes. Failed execution cancels remaining work,
            // unless a stop's signal ended it: what that signalled then ends
            // in its own time.
            if !matches!(outcome, Ok(Ok(_))) && scope.stop.signal().is_none() {
                scope.stop.abort();
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
            let code = match outcome {
                Ok(Ok(Ok(result))) => Some(i32::from(result.code)),
                Ok(Ok(Err(failure))) => {
                    error = Some(failure.to_string());
                    None
                }
                Ok(Err(_)) => {
                    error = Some("shell worker panicked".into());
                    None
                }
                Err(failure) => {
                    error = Some(failure.to_string());
                    None
                }
            };
            // A stopped job reports the signal that ended it, `KILL` once it
            // was killed, and no exit code: bash's own is the interruption's
            // (`runner.md` § Cancellation and completion).
            let stopped_by = if owner_cancel.is_cancelled() {
                Some(Signal::Kill)
            } else {
                owner_scope
                    .stop
                    .signal()
                    .and(*owner_signal.lock().expect("the stop signal is intact"))
            };
            if let Some(signal) = stopped_by {
                ProcessExit {
                    code: None,
                    signal: Some(signal.to_string()),
                    error: None,
                }
            } else {
                ProcessExit {
                    code,
                    signal: None,
                    error,
                }
            }
        });
        Ok(Self {
            input,
            output,
            owner: Some(owner),
            exited: None,
            cancel,
            scope: job_scope,
            stop_signal,
            drains,
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

    fn drain_stdout(&self) -> oneshot::Receiver<()> {
        let (answer, answered) = oneshot::channel();
        // A pump that has ended has nothing left to read: the dropped
        // answer tells the caller so at once.
        let _ = self.drains.send(answer);
        answered
    }

    fn cancel(&self) {
        self.cancel.cancel();
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    fn outliving(&self) -> tokio::sync::watch::Receiver<Vec<String>> {
        self.scope.work.outliving()
    }

    /// `KILL` ends the job at once; the other signals that end a job stop
    /// it the first way (`runner.md` § Cancellation and completion): its
    /// shell work ends at its next step, every process group it started
    /// takes the signal, and the job ends once every process of them has.
    fn signal(&self, signal: Signal) -> io::Result<()> {
        match signal {
            Signal::Kill => {
                self.cancel();
                Ok(())
            }
            Signal::Interrupt | Signal::Terminate | Signal::Hangup | Signal::Quit => {
                if self.cancel.is_cancelled() {
                    return Ok(());
                }
                *self.stop_signal.lock().expect("the stop signal is intact") = Some(signal);
                self.scope.signal(number(signal), false)
            }
            Signal::User1 | Signal::User2 | Signal::Stop | Signal::Continue => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsupported shell job signal",
            )),
        }
    }

    fn wait(&mut self) -> BoxFuture<'_, ProcessExit> {
        Box::pin(async move {
            if let Some(owner) = self.owner.take() {
                self.exited = Some(owner.await.unwrap_or_else(|_| ProcessExit {
                    code: None,
                    signal: None,
                    error: Some("the shell job's owner panicked".into()),
                }));
            }
            self.exited.clone().expect("the owner left the exit")
        })
    }
}

/// The number of a signal that ends a job.
fn number(signal: Signal) -> i32 {
    #[cfg(unix)]
    return demi_runner_process::process::number(signal).as_raw();
    // Windows has no signals; any ends the job's work at once.
    #[cfg(windows)]
    {
        let _ = signal;
        1
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
/// runner's end is asynchronous and takes no thread, and a request on
/// `drains` reads what the pipe holds then, without waiting for more, before
/// it is answered; on Windows a read takes one of the shell pool's threads,
/// and a request is answered at once.
fn pump(
    shell: &ShellRuntime,
    file: File,
    stream: OutputStream,
    sender: mpsc::Sender<OutputChunk>,
    cancel: CancellationToken,
    mut drains: Option<mpsc::UnboundedReceiver<oneshot::Sender<()>>>,
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
            let send = async |count: usize, buffer: &[u8]| {
                let chunk = OutputChunk {
                    stream,
                    bytes: Bytes::copy_from_slice(&buffer[..count]),
                };
                tokio::select! {
                    _ = cancel.cancelled() => Err(cancelled()),
                    sent = sender.send(chunk) => sent.map_err(|_| closed()),
                }
            };
            loop {
                let drain = async {
                    match drains.as_mut() {
                        Some(drains) => drains.recv().await,
                        None => std::future::pending().await,
                    }
                };
                let count = tokio::select! {
                    _ = cancel.cancelled() => return Err(cancelled()),
                    count = pipe.read(&mut buffer) => count?,
                    request = drain => {
                        let Some(answer) = request else {
                            drains = None;
                            continue;
                        };
                        // What the pipe holds now; the writer's later
                        // bytes come after the answer.
                        loop {
                            match pipe.try_read(&mut buffer) {
                                Ok(0) => break,
                                Ok(count) => send(count, &buffer).await?,
                                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                                Err(error) => return Err(error),
                            }
                        }
                        // A caller that stopped waiting needs no answer.
                        let _ = answer.send(());
                        continue;
                    }
                };
                if count == 0 {
                    return Ok(());
                }
                send(count, &buffer).await?;
            }
        })
    }
    #[cfg(windows)]
    {
        // A blocking read cannot be asked to stop short: each request is
        // answered at once.
        if let Some(mut drains) = drains {
            tokio::spawn(async move {
                while let Some(answer) = drains.recv().await {
                    let _ = answer.send(());
                }
            });
        }
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
