//! A resident shell job with bounded input and output and an owned completion
//! task.
//!
//! Completion means the job has released its local work and IO, not merely that
//! its foreground script returned.
//!
//! # Stopping a job
//!
//! A job is stopped in two steps, as a program in a terminal is:
//!
//! 1. `TERM` stops its shell work at its next step and sends `SIGTERM` to every
//!    process group the job started, so a dev server can close its connections
//!    and remove its socket.
//! 2. `KILL` follows `STOP_GRACE` later for what remains.
//!
//! A stop that went straight to `SIGKILL` left a dev server's backend running
//! in a group of its own, holding its port.
//!
//! A program can leave the job's groups (`setsid`, as a daemon does), so each
//! signal also goes to:
//!
//! - every process that carries the job's `DEMI_JOB_ID` in its environment, as
//!   GitHub's runner finds a job's processes by `RUNNER_TRACKING_ID`;
//! - on macOS, which shows the environment only of programs that are not
//!   Apple's, the descendants of every process the signal reaches.
//!
//! A process that cleared its environment, or an Apple program orphaned by a
//! double fork, escapes. The job ends once every process of those groups and
//! every process a signal reached has exited.
//!
//! # Work inside the runner
//!
//! Work inside the runner (an embedded utility, an interpreter unit) cannot be
//! killed. It stops at its next check of the job's cancellation, which each
//! utility makes at least once per entry or block it reads, so even a quiet
//! `find /` stops.
//!
//! A thread can still sit in a system call only a signal would interrupt, such
//! as an `open()` the kernel holds while macOS asks the user for access to
//! another app's data. So once `KILL` has gone, the job ends `KILLED_WORK`
//! later whatever in-process work is left, reporting `SIGKILL`, and the Host
//! log names the work it left behind.
//!
//! A cancellation request alone does not establish that execution stopped: the
//! backend counts a command as stopped only when the runner says its job ended.

use crate::{
    ShellRuntime,
    interpreter::{ShellOptions, execute},
    scope::Scope,
};
use bytes::Bytes;
#[cfg(unix)]
use demi_runner_process::process::{JOB_ID_ENV, Stray};
use demi_runner_process::{
    job_shell::ShellJob,
    process::{OutputChunk, ProcessExit, ProcessInput},
    stdio::{LIVE_INPUT_ENV, reference},
};
#[cfg(unix)]
use tokio_util::task::AbortOnDropHandle;
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
    /// Takes each signal that stops the job to its process groups and its
    /// processes outside them ([`stop_strays`]); none for a job without an
    /// id, whose groups take the signals at once.
    #[cfg(unix)]
    strays: Option<mpsc::UnboundedSender<i32>>,
}

/// The most one read of a job's output takes.
const OUTPUT_CHUNK_BYTES: usize = 64 * 1024;

/// How long a killed job waits for its in-process work before it ends
/// without it.
const KILLED_WORK: std::time::Duration = std::time::Duration::from_secs(1);

/// A job's three pipes: the ends the interpreter uses and the runner's.
struct Pipes {
    stdin: File,
    input_writer: File,
    output_reader: File,
    stdout: File,
    error_reader: File,
    stderr: File,
    /// A copy of the job's stdin pipe, which `DEMI_LIVE_INPUT` names
    /// (`crates/runner-process/src/stdio.rs`); kept open until every
    /// interpreter task finishes.
    input_reference: File,
}

impl Pipes {
    /// Out of open files, this waits for one on a shell thread until the
    /// job is cancelled (`crates/command-sdk/src/descriptors.rs`).
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
        #[cfg(unix)]
        let (strays, stray_stop) = match env.get(JOB_ID_ENV) {
            Some(id) => {
                let (signals, received) = mpsc::unbounded_channel();
                let stop = tokio::spawn(stop_strays(
                    scope.clone(),
                    id.clone(),
                    received,
                    cancel.clone(),
                ));
                (Some(signals), Some(AbortOnDropHandle::new(stop)))
            }
            None => (None, None),
        };
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
            // Work in the runner's process cannot be killed: a thread can sit
            // in a system call no cancellation reaches, such as an `open()`
            // the kernel holds while macOS asks the user for access. Once
            // killed, the job ends a second later without what is left,
            // which ends by itself at its next check.
            let killed = async {
                owner_cancel.cancelled().await;
                tokio::time::sleep(KILLED_WORK).await;
            };
            let outcome = tokio::select! {
                outcome = worker => Some(outcome),
                () = killed => None,
            };
            let left = outcome.is_none().then(|| {
                let utilities = owner_scope.work.running_utilities();
                let what = if utilities.is_empty() {
                    "its shell work".to_owned()
                } else {
                    utilities.join(", ")
                };
                format!("{what} had not ended 1 s after KILL and was left to end at its next check")
            });
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
            // A killed job ends once what it killed outside its groups has
            // ended; a job that ended otherwise leaves them alone, and the
            // handle's drop ends their watch.
            #[cfg(unix)]
            if let Some(stray_stop) = stray_stop
                && owner_cancel.is_cancelled()
                && let Err(failure) = stray_stop.await
            {
                error = Some(failure.to_string());
            }
            let code = match outcome {
                Some(Ok(Ok(Ok(result)))) => Some(i32::from(result.code)),
                Some(Ok(Ok(Err(failure)))) => {
                    error = Some(failure.to_string());
                    None
                }
                Some(Ok(Err(_))) => {
                    error = Some("shell worker panicked".into());
                    None
                }
                Some(Err(failure)) => {
                    error = Some(failure.to_string());
                    None
                }
                None => None,
            };
            // A stopped job reports the signal that ended it, `KILL` once it
            // was killed, and no exit code: bash's own is the interruption's
            //. What a killed job
            // left running goes to the Host log beside it.
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
                    error: left,
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
            #[cfg(unix)]
            strays,
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
    /// it the first way: its
    /// shell work ends at its next step, every process group it started
    /// takes the signal, and the job ends once every process of them has.
    /// Its processes that left its groups take each signal too.
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
                #[cfg(unix)]
                if let Some(strays) = &self.strays {
                    // The task ends only with the job, which still runs.
                    let _ = strays.send(number(signal));
                    return Ok(());
                }
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

/// Takes the signals that stop job `job` to its process groups and to its
/// processes outside them, as a daemon that calls `setsid` is.
///
/// For each signal it finds those processes while the tree is whole, then
/// signals the groups, through `scope`, and each of them.
///
/// Once `killed` is cancelled, which kills the groups, it kills them too, with
/// those the last signal reached that still run, and ends when they have ended.
#[cfg(unix)]
async fn stop_strays(
    scope: Scope,
    job: String,
    mut signals: mpsc::UnboundedReceiver<i32>,
    killed: CancellationToken,
) {
    let find = |known: Vec<Stray>| {
        let work = scope.work.clone();
        let job = job.clone();
        async move {
            tokio::task::spawn_blocking(move || work.strays(&job, &known))
                .await
                .unwrap_or_else(|failure| {
                    tracing::warn!("finding the job's processes outside its groups failed: {failure}");
                    Vec::new()
                })
        }
    };
    let mut known = Vec::new();
    loop {
        tokio::select! {
            biased;
            () = killed.cancelled() => break,
            signal = signals.recv() => {
                // The job is gone, which cancels `killed`.
                let Some(signal) = signal else {
                    killed.cancelled().await;
                    break;
                };
                let strays = find(known).await;
                if let Err(failure) = scope.signal(signal, false) {
                    tracing::warn!("signalling the job's process groups failed: {failure}");
                }
                known = reach(strays, signal);
            }
        }
    }
    let strays = find(known).await;
    demi_runner_process::process::strays_ended(reach(strays, number(Signal::Kill))).await;
}

/// Sends `signal` to each of `strays`; answers those it was sent to.
#[cfg(unix)]
fn reach(strays: Vec<Stray>, signal: i32) -> Vec<Stray> {
    strays
        .into_iter()
        .filter(|stray| match demi_runner_process::process::signal_process(stray.pid, signal) {
            Ok(()) => true,
            Err(failure) => {
                tracing::warn!("signalling process {}, outside the job's groups, failed: {failure}", stray.pid);
                false
            }
        })
        .collect()
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
    drains: Option<mpsc::UnboundedReceiver<oneshot::Sender<()>>>,
) -> tokio::task::JoinHandle<io::Result<()>> {
    let cancelled = || io::Error::new(io::ErrorKind::Interrupted, "job output cancelled");
    let closed = || io::Error::new(io::ErrorKind::BrokenPipe, "job output consumer closed");
    #[cfg(unix)]
    {
        use tokio::io::AsyncReadExt;
        let _ = shell;
        // Only here does it change: it is dropped once the drains end.
        let mut drains = drains;
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
