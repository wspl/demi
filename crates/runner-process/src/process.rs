use std::{
    collections::BTreeMap,
    io,
    path::PathBuf,
    process::{ExitStatus, Stdio},
    time::Duration,
};

use bytes::Bytes;
use demi_command_sdk::descriptors::{self, Backoff};
use futures_util::StreamExt;
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
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
    /// Pipes the process inherits besides its standard streams: at each
    /// descriptor, 3 or higher, a pipe that holds the bytes and then ends.
    pub descriptors: Vec<(u32, Bytes)>,
}

/// What a process the runner starts gets besides its command: the umask and
/// resource limits its job's `umask` and `ulimit` set (`crates/runner-shell/src/process_builtins.rs`). Nothing set, it gets the runner's own,
/// and the open-file limits the runner was started with (`crates/command-sdk/src/descriptors.rs`).
#[derive(Debug, Clone, Default)]
pub struct ChildAttributes {
    /// The file mode creation mask; absent, the process inherits the runner's.
    pub umask: Option<u32>,
    /// Each resource at most once, with its soft and hard limit; a resource
    /// not listed keeps the runner's.
    #[cfg(unix)]
    pub limits: Vec<(rlimit::Resource, u64, u64)>,
}

#[derive(Debug, Clone)]
pub struct SpawnFailure {
    pub kind: SpawnErrorKind,
    pub message: String,
}

use demi_runner_protocol::wire::{OutputStream, Signal, SpawnErrorKind};

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
        let mut command = wrap(command, options.process_group, &ChildAttributes::default());
        let fillings = inherit(command.command_mut().as_std_mut(), &options.descriptors)?;
        let mut child = match start(|| command.spawn()).await {
            Ok(child) => child,
            Err(error) => return Err(classify_failure(error, &options).await),
        };
        // The child holds the pipes' read ends now; the runner's copies go
        // with the command.
        drop(command);
        fill(fillings);
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

/// A pipe's write end and the bytes it is to carry to the process.
#[cfg(unix)]
type Filling = (std::io::PipeWriter, Bytes);
#[cfg(not(unix))]
type Filling = ();

/// Gives `command` a pipe at each of `descriptors`, and answers the write
/// ends with what each must carry. The write ends close on exec, so the
/// process holds only the read ends.
#[cfg(unix)]
fn inherit(
    command: &mut std::process::Command,
    descriptors: &[(u32, Bytes)],
) -> Result<Vec<Filling>, SpawnFailure> {
    use command_fds::{CommandFdExt, FdMapping};
    let failed = |message: String| SpawnFailure {
        kind: SpawnErrorKind::Other,
        message,
    };
    if descriptors.is_empty() {
        return Ok(Vec::new());
    }
    let mut mappings = Vec::new();
    let mut fillings = Vec::new();
    for (fd, bytes) in descriptors {
        let child_fd = i32::try_from(*fd)
            .ok()
            .filter(|fd| *fd > 2)
            .ok_or_else(|| failed(format!("descriptor {fd} is not one a process inherits")))?;
        let (reader, writer) = std::io::pipe()
            .map_err(|error| failed(format!("the pipe of descriptor {fd}: {error}")))?;
        mappings.push(FdMapping {
            parent_fd: reader.into(),
            child_fd,
        });
        fillings.push((writer, bytes.clone()));
    }
    command
        .fd_mappings(mappings)
        .map_err(|error| failed(error.to_string()))?;
    Ok(fillings)
}

#[cfg(not(unix))]
fn inherit(
    _command: &mut std::process::Command,
    descriptors: &[(u32, Bytes)],
) -> Result<Vec<Filling>, SpawnFailure> {
    if descriptors.is_empty() {
        return Ok(Vec::new());
    }
    Err(SpawnFailure {
        kind: SpawnErrorKind::Other,
        message: "a process inherits no further descriptors on Windows".into(),
    })
}

/// Writes each pipe's bytes and closes it, off the runtime's threads, since
/// a pipe holds only so much until the process reads it. A process that
/// ends without reading its pipe makes the write fail, which concerns
/// nobody: what it does without its bytes is its own.
#[cfg(unix)]
fn fill(fillings: Vec<Filling>) {
    use std::io::Write as _;
    for (mut writer, bytes) in fillings {
        tokio::task::spawn_blocking(move || {
            let _unread = writer.write_all(&bytes);
        });
    }
}

#[cfg(not(unix))]
fn fill(_fillings: Vec<Filling>) {}

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

/// The soft and hard open-file limits the runner was started with, once it
/// raised its own; every process it starts gets them back.
#[cfg(unix)]
static STARTED_WITH: std::sync::OnceLock<(u64, u64)> = std::sync::OnceLock::new();

/// Keeps the soft and hard open-file limits the runner was started with,
/// once it raised its soft limit from `soft` to `raised` (`crates/command-sdk/src/descriptors.rs`), so that every process it starts gets them back. Without a raise
/// the processes inherit the runner's own, which are the same.
#[cfg(unix)]
pub fn keep_started_open_file_limit(soft: u64, hard: u64, raised: u64) {
    if raised > soft {
        STARTED_WITH.get_or_init(|| (soft, hard));
    }
}

/// The limits a process the runner starts gets for `resource` unless its
/// job's `ulimit` set others: for open files the ones the runner was started
/// with, otherwise the runner's own, which the process inherits
/// (`crates/command-sdk/src/descriptors.rs`).
#[cfg(unix)]
pub fn child_limit(resource: rlimit::Resource) -> io::Result<(u64, u64)> {
    match STARTED_WITH.get() {
        Some(&limits) if resource == rlimit::Resource::NOFILE => Ok(limits),
        _ => resource.get(),
    }
}

/// The runner's own file mode creation mask, which a process it starts
/// inherits unless its job's `umask` set another. Nothing in the runner
/// changes it (`crates/runner-shell/src/process_builtins.rs`), so it is read
/// once: from `/proc` on Linux, elsewhere by setting a strict mask and
/// restoring the old one, which the runner does as it starts, before any job
/// runs.
#[cfg(unix)]
pub fn umask() -> u32 {
    static UMASK: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *UMASK.get_or_init(|| {
        #[cfg(target_os = "linux")]
        if let Some(umask) = procfs::process::Process::myself()
            .and_then(|process| process.status())
            .ok()
            .and_then(|status| status.umask)
        {
            return umask;
        }
        // A file created meanwhile gets the stricter mask, never a looser one.
        let strict = rustix::fs::Mode::from_raw_mode(0o077);
        let umask = rustix::process::umask(strict);
        rustix::process::umask(umask);
        #[allow(
            clippy::useless_conversion,
            reason = "mode_t is u16 on macOS, u32 on Linux"
        )]
        u32::from(umask.as_raw_mode())
    })
}

/// Windows has no mask (`crates/runner-shell/src/process_builtins.rs`): a
/// job's shell shows `022`, the mask a Unix runner most often has, until
/// its `umask` sets another, and nothing applies either.
#[cfg(windows)]
pub fn umask() -> u32 {
    0o022
}

/// Gives a process the runner starts its attributes before it runs its
/// program: the open-file limits the runner was started with, the ones it
/// would have from a terminal (`crates/command-sdk/src/descriptors.rs`), and what its job's
/// `umask` and `ulimit` set (`crates/runner-shell/src/process_builtins.rs`).
/// A program that uses `select()`, or that closes every descriptor up to its
/// limit, misbehaves with the runner's raised limit. Each call adds a hook,
/// so a command needs one call.
#[cfg(unix)]
pub fn set_attributes(command: &mut std::process::Command, attributes: &ChildAttributes) {
    use std::os::unix::process::CommandExt;
    let mut limits = attributes.limits.clone();
    if let Some(&(soft, hard)) = STARTED_WITH.get()
        && !limits
            .iter()
            .any(|&(resource, ..)| resource == rlimit::Resource::NOFILE)
    {
        limits.push((rlimit::Resource::NOFILE, soft, hard));
    }
    // A umask has nine bits, which every platform's mode type holds.
    let umask = attributes
        .umask
        .map(|umask| rustix::fs::Mode::from_raw_mode(umask as rustix::fs::RawMode));
    if limits.is_empty() && umask.is_none() {
        return;
    }
    // SAFETY: the hook runs in the child between fork and exec. It makes only
    // `umask` and `setrlimit` calls, which are async-signal-safe, and
    // allocates nothing unless one of them fails.
    unsafe {
        command.pre_exec(move || {
            if let Some(umask) = umask {
                rustix::process::umask(umask);
            }
            for &(resource, soft, hard) in &limits {
                rlimit::setrlimit(resource, soft, hard)?;
            }
            Ok(())
        });
    }
}

/// A command the runner starts: it gets its `attributes` and the open-file
/// limit the runner was started with, is killed when its handle is dropped
/// and, with `group`, leads a process group of its own (a job object on
/// Windows).
pub fn wrap(command: Command, group: bool, attributes: &ChildAttributes) -> CommandWrap {
    let mut command = CommandWrap::from(command);
    #[cfg(unix)]
    set_attributes(command.command_mut().as_std_mut(), attributes);
    #[cfg(windows)]
    let _no_attributes_on_windows = attributes;
    command.wrap(KillOnDrop);
    if group {
        #[cfg(unix)]
        command.wrap(process_wrap::tokio::ProcessGroup::leader());
        #[cfg(windows)]
        command.wrap(process_wrap::tokio::JobObject);
    }
    command
}

/// Starts a process with `attempt`, which tries to spawn it once. Every process
/// the runner starts, a service, a raw process, a job's external command or a
/// utility's child program, starts this way, so none fails for a condition
/// that passes (`crates/command-sdk/src/descriptors.rs`; `StartRetry` says which).
pub async fn start<T>(mut attempt: impl FnMut() -> io::Result<T>) -> io::Result<T> {
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
pub fn start_blocking<T>(mut attempt: impl FnMut() -> io::Result<T>) -> io::Result<T> {
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
///   (`demi_command_sdk::descriptors`).
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

/// The processor time a process used, with that of the children it waited
/// for, as wait4(2) reports it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProcessorTime {
    pub user: Duration,
    pub system: Duration,
}

/// Waits for `child` to end and reaps it, with the processor time it used,
/// which the system keeps only until the reaping (`crates/runner-shell/src/process_builtins.rs`). Dropped while it waits, it has reaped nothing.
/// A time it cannot read counts as none: it only shortens what `time`
/// reports, while the wait itself goes on.
pub async fn wait_timed(child: &mut dyn ChildWrapper) -> io::Result<(ExitStatus, ProcessorTime)> {
    #[cfg(unix)]
    let time = match child.id() {
        Some(pid) => ended_wait(pid).await.unwrap_or_else(|error| {
            tracing::warn!("reading the processor time of process {pid} failed: {error}");
            ProcessorTime::default()
        }),
        // Reaped already, its time went with it.
        None => ProcessorTime::default(),
    };
    // The process stays readable while a handle to it is open, after the
    // reaping closes the child's own.
    #[cfg(windows)]
    let handle = child
        .process_handle()
        .and_then(|handle| handle.try_clone_to_owned().ok());
    let status = child.wait().await?;
    #[cfg(windows)]
    let time = handle
        .and_then(|handle| handle_time(std::os::windows::io::AsHandle::as_handle(&handle)).ok())
        .unwrap_or_default();
    Ok((status, time))
}

/// Waits for the runner's child `pid` to end, without reaping it, and reads
/// its processor time.
#[cfg(unix)]
async fn ended_wait(pid: u32) -> io::Result<ProcessorTime> {
    use tokio::signal::unix::{SignalKind, signal};
    // Made before the first look, so that an end between the two still wakes
    // the wait.
    let mut ended = signal(SignalKind::child())?;
    loop {
        if let Some(time) = ended_time(pid, false)? {
            return Ok(time);
        }
        if ended.recv().await.is_none() {
            return Err(io::Error::other("the runtime stopped delivering signals"));
        }
    }
}

/// The processor time of the runner's child `pid` once it has ended and
/// before it is reaped, as [`wait_timed`] reads it; `None` while it runs on.
/// With `block`, it waits for the end. A child reaped already counts as
/// having used none.
#[cfg(unix)]
pub fn ended_time(pid: u32, block: bool) -> io::Result<Option<ProcessorTime>> {
    use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};
    let Some(id) = i32::try_from(pid).ok().and_then(Pid::from_raw) else {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    };
    let mut options = WaitIdOptions::EXITED | WaitIdOptions::NOWAIT;
    if !block {
        options |= WaitIdOptions::NOHANG;
    }
    loop {
        return match waitid(WaitId::Pid(id), options) {
            Ok(None) => Ok(None),
            Ok(Some(_)) => ended_child_time(pid).map(Some),
            Err(rustix::io::Errno::INTR) => continue,
            Err(rustix::io::Errno::CHILD) => Ok(Some(ProcessorTime::default())),
            Err(error) => Err(error.into()),
        };
    }
}

/// An ended, unreaped child's processor time, from its `/proc` entry, which
/// counts its own and its waited children's in clock ticks.
#[cfg(target_os = "linux")]
fn ended_child_time(pid: u32) -> io::Result<ProcessorTime> {
    let stat = procfs::process::Process::new(pid as i32)
        .and_then(|process| process.stat())
        .map_err(io::Error::other)?;
    let ticks = procfs::ticks_per_second();
    let duration = |own: u64, children: i64| {
        let total = own.saturating_add(u64::try_from(children).unwrap_or(0));
        Duration::from_nanos(total.saturating_mul(1_000_000_000) / ticks)
    };
    Ok(ProcessorTime {
        user: duration(stat.utime, stat.cutime),
        system: duration(stat.stime, stat.cstime),
    })
}

/// An ended, unreaped child's processor time, which the system keeps with its
/// waited children's until the reaping.
#[cfg(target_vendor = "apple")]
fn ended_child_time(pid: u32) -> io::Result<ProcessorTime> {
    let pid = i32::try_from(pid).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let mut info = std::mem::MaybeUninit::<libc::rusage_info_v2>::zeroed();
    // SAFETY: the buffer holds a `rusage_info_v2`, the flavour asked for;
    // `rusage_info_t` is the C API's untyped buffer pointer.
    let result = unsafe {
        libc::proc_pid_rusage(
            pid,
            libc::RUSAGE_INFO_V2,
            info.as_mut_ptr().cast::<libc::rusage_info_t>(),
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the call succeeded, so it filled the buffer.
    let info = unsafe { info.assume_init() };
    // The times are in Mach time units, which are nanoseconds on Intel and
    // not on Apple silicon.
    let mut timebase = mach2::mach_time::mach_timebase_info_data_t::default();
    // SAFETY: the call fills the structure it is given.
    let result = unsafe { mach2::mach_time::mach_timebase_info(&mut timebase) };
    if result != mach2::kern_return::KERN_SUCCESS || timebase.denom == 0 {
        return Err(io::Error::other("the Mach timebase is unreadable"));
    }
    let duration = |own: u64, children: u64| {
        let ticks = u128::from(own) + u128::from(children);
        let nanos = ticks * u128::from(timebase.numer) / u128::from(timebase.denom);
        Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
    };
    Ok(ProcessorTime {
        user: duration(info.ri_user_time, info.ri_child_user_time),
        system: duration(info.ri_system_time, info.ri_child_system_time),
    })
}

/// The processor time of the process `handle` holds.
#[cfg(windows)]
pub fn handle_time(handle: std::os::windows::io::BorrowedHandle<'_>) -> io::Result<ProcessorTime> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{Foundation::FILETIME, System::Threading::GetProcessTimes};
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut creation, mut exit, mut kernel, mut user) = (zero, zero, zero, zero);
    // SAFETY: the handle is open for the call, and each time is a FILETIME
    // the call fills.
    let result = unsafe {
        GetProcessTimes(handle.as_raw_handle(), &mut creation, &mut exit, &mut kernel, &mut user)
    };
    if result == 0 {
        return Err(io::Error::last_os_error());
    }
    // A FILETIME counts 100-nanosecond intervals.
    let duration = |time: FILETIME| {
        let intervals = (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
        Duration::from_nanos(intervals.saturating_mul(100))
    };
    Ok(ProcessorTime {
        user: duration(user),
        system: duration(kernel),
    })
}

pub fn kill(child: &mut dyn ChildWrapper) -> io::Result<()> {
    match child.start_kill() {
        #[cfg(unix)]
        Err(error) if gone(&error) => Ok(()),
        result => result,
    }
}

/// Sends signal number `signal` to `target` as kill(2) takes it: a process
/// ID, or the negative of a process group's ID; signal 0 asks only whether
/// it exists. A number no signal has is `EINVAL`, and a target no process
/// has `ESRCH`, as are 0 and -1, which kill(2) takes for the caller's group
/// and every process.
#[cfg(unix)]
pub fn signal_target(target: i32, signal: i32) -> io::Result<()> {
    use rustix::process::{self as system, Pid};
    let pid = Pid::from_raw(target.saturating_abs())
        .filter(|_| target != -1)
        .ok_or_else(|| io::Error::from(rustix::io::Errno::SRCH))?;
    let named = || {
        system::Signal::from_named_raw(signal).ok_or_else(|| io::Error::from(rustix::io::Errno::INVAL))
    };
    let sent = match (target < 0, signal) {
        (false, 0) => system::test_kill_process(pid),
        (true, 0) => system::test_kill_process_group(pid),
        (false, _) => system::kill_process(pid, named()?),
        (true, _) => system::kill_process_group(pid, named()?),
    };
    sent.map_err(io::Error::from)
}

/// Windows has no signals (`crates/runner-shell/src/process_builtins.rs`):
/// signal 0 asks whether the process `target` runs, and any other ends it
/// with `TerminateProcess` and the exit status 128 plus the signal, the
/// status a shell gives a process that signal ended on Unix. A negative
/// target, a process group on Unix, is `InvalidInput`, as Windows has no
/// process groups; a target no process has is `NotFound`, worded as
/// kill(2)'s `ESRCH`.
#[cfg(windows)]
pub fn signal_target(target: i32, signal: i32) -> io::Result<()> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::{
        Foundation::{ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, STILL_ACTIVE},
        System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            PROCESS_TERMINATE, TerminateProcess,
        },
    };
    let no_such_process = || io::Error::new(io::ErrorKind::NotFound, "No such process");
    let Ok(pid) = u32::try_from(target) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Windows has no process groups",
        ));
    };
    let access = if signal == 0 {
        PROCESS_QUERY_LIMITED_INFORMATION
    } else {
        PROCESS_TERMINATE
    };
    // SAFETY: OpenProcess takes no pointers.
    let handle = unsafe { OpenProcess(access, 0, pid) };
    if handle.is_null() {
        let error = io::Error::last_os_error();
        return Err(match error.raw_os_error().map(|code| code as u32) {
            Some(ERROR_INVALID_PARAMETER) => no_such_process(),
            Some(ERROR_ACCESS_DENIED) => {
                io::Error::new(io::ErrorKind::PermissionDenied, "Operation not permitted")
            }
            _ => error,
        });
    }
    // SAFETY: the handle OpenProcess just returned is open and nothing else
    // owns it; `process` closes it when it drops.
    let process = unsafe { OwnedHandle::from_raw_handle(handle) };
    if signal == 0 {
        let mut code = 0;
        // SAFETY: `process` is a live handle with query access, and `code`
        // outlives the call.
        if unsafe { GetExitCodeProcess(process.as_raw_handle(), &mut code) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // A process that ended but whose handle another holds still opens.
        return if code == STILL_ACTIVE as u32 {
            Ok(())
        } else {
            Err(no_such_process())
        };
    }
    let status = 128 + signal.unsigned_abs();
    // SAFETY: `process` is a live handle with terminate access.
    if unsafe { TerminateProcess(process.as_raw_handle(), status) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Sends signal number `signal` to the process group `group`, as a stop
/// signals each group a job started; a group that is gone or only exiting
/// takes none (`gone`).
#[cfg(unix)]
pub fn signal_group(group: i32, signal: i32) -> io::Result<()> {
    match signal_target(-group, signal) {
        Err(error) if gone(&error) => Ok(()),
        result => result,
    }
}

/// Sends signal number `signal` to the process `pid` alone; a process that
/// is gone takes none.
#[cfg(unix)]
pub fn signal_process(pid: i32, signal: i32) -> io::Result<()> {
    match signal_target(pid, signal) {
        Err(error) if error.raw_os_error() == Some(rustix::io::Errno::SRCH.raw_os_error()) => Ok(()),
        result => result,
    }
}

/// The variable that names a job in the environment of every process the job
/// starts, by which a stop finds the processes that left the job's groups
/// (`crates/runner-shell/src/job.rs`).
pub const JOB_ID_ENV: &str = "DEMI_JOB_ID";

/// A process a stop of its job reaches by its own id, outside the job's
/// process groups, with when it started, so that a later signal never
/// reaches another process that took the id after it ended.
#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stray {
    pub pid: i32,
    started: u64,
}

/// The processes a stop of job `job` signals one by one (`crates/runner-shell/src/job.rs`): of the runner's user, the runner aside,
/// and outside the job's process groups, whose ids `grouped` holds, each
/// one that carries the job's id in its environment, each of `known` that
/// still runs, and every descendant, by parent process id, of those and of
/// the groups' members. sysinfo reads each environment, from
/// `/proc/<pid>/environ` on Linux and with `KERN_PROCARGS2` on macOS, which
/// shows none for Apple's own programs: such a program that called `setsid`
/// is found as its parent's descendant while the parent runs. Reading the
/// whole process table takes milliseconds, so it runs off the runtime's
/// threads, and before the groups take the signal, while the tree is whole.
#[cfg(unix)]
pub fn job_strays(job: &str, grouped: &std::collections::HashSet<i32>, known: &[Stray]) -> Vec<Stray> {
    use std::collections::{HashMap, HashSet};
    use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, UpdateKind};
    let marker = std::ffi::OsString::from(format!("{JOB_ID_ENV}={job}"));
    let user = rustix::process::getuid().as_raw();
    let runner = std::process::id();
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_user(UpdateKind::Always)
            .with_environ(UpdateKind::Always),
    );
    // The user's live processes, each with whether it is in one of the
    // job's groups.
    let mine: HashMap<Pid, (&sysinfo::Process, bool)> = system
        .processes()
        .iter()
        .filter(|(pid, process)| {
            pid.as_u32() != runner
                && process.status() != ProcessStatus::Zombie
                && process.user_id().is_some_and(|id| **id == user)
        })
        .map(|(pid, process)| {
            let grouped = i32::try_from(pid.as_u32())
                .ok()
                .and_then(process_group)
                .is_some_and(|group| grouped.contains(&group));
            (*pid, (process, grouped))
        })
        .collect();
    let mut children: HashMap<Pid, Vec<Pid>> = HashMap::new();
    for (pid, (process, _)) in &mine {
        if let Some(parent) = process.parent() {
            children.entry(parent).or_default().push(*pid);
        }
    }
    let mut reached: HashSet<Pid> = mine
        .iter()
        .filter(|(pid, (process, grouped))| {
            *grouped
                || process.environ().contains(&marker)
                || known.iter().any(|stray| {
                    i64::from(stray.pid) == i64::from(pid.as_u32()) && stray.started == process.start_time()
                })
        })
        .map(|(pid, _)| *pid)
        .collect();
    let mut unvisited: Vec<Pid> = reached.iter().copied().collect();
    while let Some(pid) = unvisited.pop() {
        for child in children.get(&pid).into_iter().flatten() {
            if reached.insert(*child) {
                unvisited.push(*child);
            }
        }
    }
    reached
        .into_iter()
        .filter_map(|pid| {
            let (process, grouped) = mine.get(&pid)?;
            (!grouped).then_some(())?;
            Some(Stray {
                pid: i32::try_from(pid.as_u32()).ok()?,
                started: process.start_time(),
            })
        })
        .collect()
}

/// The process group of the process `pid`; none once it is gone.
#[cfg(unix)]
fn process_group(pid: i32) -> Option<i32> {
    let pid = rustix::process::Pid::from_raw(pid)?;
    rustix::process::getpgid(Some(pid))
        .ok()
        .map(|group| group.as_raw_nonzero().get())
}

/// Ends once none of `strays` runs: each is gone, a zombie that only waits
/// for its parent, or its id another process's, as a stop waits for the
/// processes it killed outside the job's groups (`crates/runner-shell/src/job.rs`). They are no children of the runner's, so no wait tells
/// when they end: this asks the system every 25 ms, as [`group_ended`] does.
#[cfg(unix)]
pub async fn strays_ended(mut strays: Vec<Stray>) {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System};
    const POLL: Duration = Duration::from_millis(25);
    let pid = |stray: &Stray| Pid::from_u32(stray.pid.unsigned_abs());
    let mut system = System::new();
    while !strays.is_empty() {
        let pids: Vec<Pid> = strays.iter().map(pid).collect();
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&pids),
            true,
            ProcessRefreshKind::nothing(),
        );
        strays.retain(|stray| {
            system.process(pid(stray)).is_some_and(|process| {
                process.status() != ProcessStatus::Zombie && process.start_time() == stray.started
            })
        });
        if !strays.is_empty() {
            tokio::time::sleep(POLL).await;
        }
    }
}

/// Ends once no process of the group `group` is left, as a stop waits for
/// every process of a group it signalled (`crates/runner-shell/src/job.rs`). No call tells when a group empties: this asks the system
/// every 25 ms, as a utility waits for its child.
#[cfg(unix)]
pub async fn group_ended(group: i32) {
    const POLL: Duration = Duration::from_millis(25);
    let Some(group) = rustix::process::Pid::from_raw(group) else {
        return;
    };
    // Any failure, ESRCH or macOS's EPERM for a group of exiting members
    // (`gone`), means no process of it runs on.
    while rustix::process::test_kill_process_group(group).is_ok() {
        tokio::time::sleep(POLL).await;
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

/// The system's signal a request names.
#[cfg(unix)]
pub fn number(signal: Signal) -> rustix::process::Signal {
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
            match SIGNALS
                .into_iter()
                .find(|signal| number(*signal).as_raw() == raw)
            {
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
