//! Jobs and raw processes (`runner.md` § Command lifetime): the connection
//! owns its job table and routes each job's input, signals, following and end
//! to it. Every job keeps its output within `JOB_KEPT_BYTES` and sends
//! bounded views of it (`runner.md` § Pipes and output).

use crate::job_directories::{JobDirectories, JobDirectory};
use crate::job_media::{JobMedia, MEDIA_DIRECTORY};
use crate::kept_output::KeptOutput;
use crate::{
    commands::{
        contexts::{self, ContextPaths, ExecutionContext, Installation},
        dispatch::Dispatcher,
    },
    connection::ConnectionHandle,
};
use bytes::Bytes;
use demi_command_protocol::CommandContext;
use demi_runner_command_packages::ServiceHandle;
use demi_runner_process::{
    job_shell::{JobCommands, JobShell, JobStart, ShellJob},
    pipes::{PipeClient, report_pipe},
    process::{ChildProcess, OutputChunk, ProcessExit, ProcessInput, SpawnOptions},
};
use demi_runner_protocol::wire::{self, OutputStream, Signal};
use futures_util::{FutureExt, StreamExt};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    path::PathBuf,
    sync::Arc,
};
use tokio::{
    sync::{mpsc, watch},
    task::JoinSet,
    time::Instant,
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

/// A job's or raw process's id, which the two kinds do not share.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum WorkId {
    Job(String),
    Spawn(String),
}

/// Live stdin chunks a task has not read yet; past them the task is
/// cancelled rather than holding up the connection.
const INPUT_QUEUE: usize = 64;
/// Signals a task has not taken yet.
const SIGNAL_QUEUE: usize = 16;
/// How long a task's pipe transfers may run on after its process ended.
const PIPE_GRACE: std::time::Duration = std::time::Duration::from_secs(30);

/// A connection's jobs and raw processes. The connection owns the table,
/// registers each task before its setup runs so that its input and signals
/// find it, and learns of each end through `finished`.
pub struct JobTable {
    entries: HashMap<WorkId, Entry>,
    running: JoinSet<WorkId>,
    config: Arc<JobConfig>,
    closed: CancellationToken,
}

/// What every task of one connection shares.
pub struct JobConfig {
    pub output: mpsc::Sender<wire::Frame>,
    /// Where each job gets its own directory.
    pub directories: Arc<JobDirectories>,
    pub pipes: PipeClient,
    /// Runs the jobs' scripts.
    pub shell: Arc<dyn JobShell>,
    /// For jobs whose manifest declares commands; absent where nothing makes
    /// execution contexts live.
    pub commands: Option<Commands>,
}

/// What a job needs to make its execution context and run declared commands.
pub struct Commands {
    pub dispatcher: Arc<Dispatcher>,
    pub connection: ConnectionHandle,
    pub installation: watch::Receiver<Installation>,
    pub paths: ContextPaths,
    pub services: ServiceHandle,
    /// The local endpoint command clients reach.
    pub endpoint: String,
    /// The installation directory, `DEMI_HOME`.
    pub home: String,
}

struct Entry {
    input: mpsc::Sender<TaskInput>,
    signals: mpsc::Sender<Signal>,
    /// Whether the backend follows the job's output beyond each stream's
    /// first `JOB_VIEW_BYTES`.
    follow: watch::Sender<bool>,
    cancel: CancellationToken,
}

enum TaskInput {
    Bytes(Bytes),
    End,
}

/// What reaches a task from the backend while it runs.
struct Controls {
    input: mpsc::Receiver<TaskInput>,
    signals: mpsc::Receiver<Signal>,
    following: watch::Receiver<bool>,
}

pub enum TaskCommand {
    Shell {
        script: String,
        /// Where the job starts when its cwd no longer exists: the
        /// conversation's working directory (`runner.md` § Shell jobs).
        workspace: PathBuf,
        stdin: Option<wire::PipeRef>,
        stdout: Option<wire::PipeRef>,
        /// The manifest and command context of a job with declared commands.
        commands: Option<(String, CommandContext)>,
    },
    Process {
        command: String,
        args: Vec<String>,
        process_group: bool,
    },
}

pub struct TaskSpec {
    pub id: String,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub command: TaskCommand,
}

impl JobTable {
    pub fn new(config: JobConfig) -> Self {
        Self {
            entries: HashMap::new(),
            running: JoinSet::new(),
            config: Arc::new(config),
            closed: CancellationToken::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn job_count(&self) -> usize {
        self.entries
            .keys()
            .filter(|id| matches!(id, WorkId::Job(_)))
            .count()
    }

    /// Registers the task at once, so the input and signals that follow find
    /// it, and runs its setup and work in a task of its own.
    pub fn start(&mut self, spec: TaskSpec) -> io::Result<()> {
        if self.closed.is_cancelled() {
            return Err(io::Error::other("task table is closed"));
        }
        if !spec.cwd.is_absolute() {
            return Err(io::Error::other("task cwd must be absolute"));
        }
        let key = match &spec.command {
            TaskCommand::Shell { .. } => WorkId::Job(spec.id.clone()),
            TaskCommand::Process { .. } => WorkId::Spawn(spec.id.clone()),
        };
        if self.entries.contains_key(&key) {
            return Err(io::Error::other("duplicate live task id"));
        }
        let (input, receiver) = mpsc::channel(INPUT_QUEUE);
        let (signals, signal_receiver) = mpsc::channel(SIGNAL_QUEUE);
        let (follow, following) = watch::channel(false);
        let cancel = CancellationToken::new();
        self.entries.insert(
            key.clone(),
            Entry {
                input,
                signals,
                follow,
                cancel: cancel.clone(),
            },
        );
        let config = self.config.clone();
        let closed = self.closed.clone();
        self.running.spawn(async move {
            let controls = Controls {
                input: receiver,
                signals: signal_receiver,
                following,
            };
            let run = config.run(spec, controls, cancel, closed.clone());
            let outcome = std::panic::AssertUnwindSafe(run).catch_unwind().await;
            let terminal = match outcome {
                Ok(Ok(terminal)) => Ok(terminal),
                Ok(Err(error)) => failure_exit(&key, error.to_string()),
                Err(_) => failure_exit(&key, "task owner panicked".into()),
            };
            match terminal {
                Ok(message) => {
                    // A disconnected backend no longer receives task results.
                    tokio::select! {
                        _ = closed.cancelled() => {},
                        _ = config.output.send(message) => {},
                    }
                }
                Err(error) => tracing::warn!("task terminal encoding failed: {error}"),
            }
            key
        });
        Ok(())
    }

    /// Live stdin is bounded; bulk streams use the independently flowing HTTP pipe.
    pub fn input(&self, id: &WorkId, bytes: Bytes) -> io::Result<()> {
        if bytes.len() > wire::STDIN_CHUNK_BYTES {
            return Err(io::Error::other("live stdin chunk exceeds 64 KiB"));
        }
        self.send(id, TaskInput::Bytes(bytes))
    }

    pub fn end_input(&self, id: &WorkId) -> io::Result<()> {
        self.send(id, TaskInput::End)
    }

    /// Starts or stops following a job's output beyond each stream's first
    /// `JOB_VIEW_BYTES`; a job that ended is not followed any more.
    pub fn follow(&self, id: &WorkId, follow: bool) {
        if let Some(entry) = self.entries.get(id) {
            entry.follow.send_replace(follow);
        }
    }

    pub fn signal(&self, id: &WorkId, signal: Signal) -> io::Result<()> {
        let Some(entry) = self.entries.get(id) else {
            return Ok(());
        };
        if signal == Signal::Kill {
            entry.cancel.cancel();
            return Ok(());
        }
        entry
            .signals
            .try_send(signal)
            .map_err(|error| io::Error::other(format!("signal queue unavailable: {error}")))
    }

    fn send(&self, id: &WorkId, input: TaskInput) -> io::Result<()> {
        let Some(entry) = self.entries.get(id) else {
            return Ok(());
        };
        match entry.input.try_send(input) {
            Ok(()) | Err(mpsc::error::TrySendError::Closed(_)) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                // Never block connection processing behind a task that is not reading.
                entry.cancel.cancel();
                Err(io::Error::other("live stdin buffer is full"))
            }
        }
    }

    /// The next task that ended, after it sent its result; its entry is gone.
    /// `None` while no task runs.
    pub async fn finished(&mut self) -> Option<WorkId> {
        let key = self
            .running
            .join_next()
            .await?
            .expect("task owners catch their panics");
        self.entries.remove(&key);
        Some(key)
    }

    /// Cancels every task and waits for each to end; results are no longer sent.
    pub async fn close(&mut self) {
        self.closed.cancel();
        for entry in self.entries.values() {
            entry.cancel.cancel();
        }
        while self.finished().await.is_some() {}
    }
}

impl Drop for JobTable {
    fn drop(&mut self) {
        // The tasks still stop and reap what they started; the join is `close`'s.
        self.closed.cancel();
        for entry in self.entries.values() {
            entry.cancel.cancel();
        }
    }
}

impl JobConfig {
    /// Makes the job's execution context live once its manifest is installed,
    /// and adds what its commands need to `env`. The context lasts as long
    /// as the job holds it: its alias directory goes with it.
    async fn context(
        &self,
        job_id: &str,
        manifest_hash: &str,
        command: CommandContext,
        edits: demi_command_protocol::EditContext,
        media: Arc<JobMedia>,
        env: &mut BTreeMap<String, String>,
    ) -> io::Result<(Arc<ExecutionContext>, JobCommands)> {
        let commands = self
            .commands
            .as_ref()
            .ok_or_else(|| io::Error::other("shell command dispatcher is unavailable"))?;
        let mut installation = commands.installation.clone();
        let installed = installation
            .wait_for(|installation| !matches!(installation, Installation::Installing))
            .await
            .map_err(|_| io::Error::other("host connection closed"))?
            .clone();
        let manifest = match installed {
            Installation::Ready(manifest) if manifest.hash == manifest_hash => manifest,
            _ => return Err(io::Error::other("job manifest is not installed")),
        };
        let context = Arc::new(
            ExecutionContext::create(
                job_id.to_owned(),
                command,
                manifest,
                edits,
                media,
                commands.connection.clone(),
                &commands.paths,
            )
            .await?,
        );
        let leases = contexts::leases(&context.manifest, &commands.services).await;
        commands
            .connection
            .register_context(context.clone(), leases)
            .await?;
        let path = env.get("PATH").cloned();
        env.extend(context.environment(&commands.endpoint, &commands.home, path.as_deref())?);
        let declared = JobCommands {
            context: context.id.clone(),
            roots: context.manifest.roots.keys().cloned().collect(),
            handler: commands.dispatcher.clone(),
        };
        Ok((context, declared))
    }

    async fn run(
        self: &Arc<Self>,
        spec: TaskSpec,
        controls: Controls,
        cancel: CancellationToken,
        closed: CancellationToken,
    ) -> io::Result<wire::Frame> {
        let Controls {
            mut input,
            mut signals,
            mut following,
        } = controls;
        let id = spec.id;
        let mut env = spec.env;
        let mut job = None;
        // A job's execution context, held until the job has ended.
        let mut execution = None;
        let (mut child, stdin, stdout) = match spec.command {
            TaskCommand::Process {
                command,
                args,
                process_group,
            } => {
                let child = ChildProcess::spawn(SpawnOptions {
                    command,
                    args,
                    process_group,
                    cwd: spec.cwd,
                    env,
                })
                .await;
                let child = match child {
                    Ok(child) => child,
                    Err(error) => {
                        return wire::encode(&wire::Outbound::SpawnExit {
                            spawn_id: id,
                            exit_code: None,
                            signal: None,
                            spawn_error: Some(wire::SpawnError {
                                kind: error.kind,
                                detail: Some(error.message),
                            }),
                        })
                        .map_err(io::Error::other);
                    }
                };
                (Execution::process(child), None, None)
            }
            TaskCommand::Shell {
                script,
                workspace,
                stdin,
                stdout,
                commands,
            } => {
                let JobDirectory {
                    path,
                    scratch,
                    output,
                    running,
                } = self.directories.create(&id, &cancel).await?;
                env.insert(
                    "TMPDIR".into(),
                    scratch.path().to_string_lossy().into_owned(),
                );
                env.insert("TEMP".into(), scratch.path().to_string_lossy().into_owned());
                env.insert("DEMI_JOB_ID".into(), id.clone());
                let edit_context = demi_command_protocol::EditContext {
                    directory: path.join("changes").to_string_lossy().into_owned(),
                    lock: self
                        .directories
                        .root()
                        .join("edits.lock")
                        .to_string_lossy()
                        .into_owned(),
                };
                let recording = edit_context.clone();
                let recorder = tokio::task::spawn_blocking(move || {
                    demi_command_sdk::edits::Recorder::new(recording)
                })
                .await
                .map_err(io::Error::other)?;
                let recorder = match recorder {
                    Ok(recorder) => Some(recorder),
                    Err(error) => {
                        tracing::warn!("edit recording failed: {error}");
                        None
                    }
                };
                job = Some((Logs::new(output), scratch, recorder.clone(), running));
                let commands = match commands {
                    Some((manifest_hash, command)) => {
                        let media = Arc::new(JobMedia::new(
                            id.clone(),
                            path.join(MEDIA_DIRECTORY),
                            self.output.clone(),
                        ));
                        let setup = self.context(
                            &id,
                            &manifest_hash,
                            command,
                            edit_context,
                            media,
                            &mut env,
                        );
                        // A job killed while it waits for its manifest never starts.
                        let (context, declared) = tokio::select! {
                            _ = cancel.cancelled() => {
                                return Err(io::Error::other("the job was cancelled before it started"));
                            }
                            context = setup => context?,
                        };
                        execution = Some(context);
                        Some(declared)
                    }
                    None => None,
                };
                let child = self
                    .shell
                    .start(JobStart {
                        script,
                        cwd: spec.cwd,
                        workspace,
                        env,
                        live: stdin.is_none(),
                        output: stdout.is_none(),
                        cancellation: cancel.child_token(),
                        commands,
                        edits: recorder,
                    })
                    .await?;
                (Execution::shell(child), stdin, stdout)
            }
        };
        let kind = if job.is_some() {
            WorkId::Job(id.clone())
        } else {
            WorkId::Spawn(id.clone())
        };
        let io_cancel = cancel.child_token();
        let _io_guard = io_cancel.clone().drop_guard();
        let input_cancel = io_cancel.child_token();
        let pipe_tasks = TaskTracker::new();
        if let Some(reference) = stdin {
            let pipes = self.pipes.clone();
            let input = child.input.clone();
            let output = self.output.clone();
            let cancel = input_cancel.clone();
            let shutdown = closed.clone();
            pipe_tasks.spawn(async move {
                let result = async {
                    let mut body = pipes.get(&reference.url, cancel.clone()).await?;
                    while let Some(bytes) = body.next().await {
                        let bytes = bytes?;
                        tokio::select! {
                            _ = cancel.cancelled() => return Err(io::Error::new(io::ErrorKind::Interrupted, "stdin pipe cancelled")),
                            sent = input.send(ProcessInput::Bytes(bytes)) => sent.map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "task stdin closed"))?,
                        }
                    }
                    Ok(())
                }.await;
                // EOF also releases a reader when the HTTP source was refused.
                let ended = tokio::select! {
                    _ = cancel.cancelled() => Ok(()),
                    sent = input.send(ProcessInput::End) => sent.map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "task stdin closed")),
                };
                let result = result.and(ended);
                report_pipe(&output, reference.id, result, &shutdown).await;
            });
        }
        let stdout_pipe = stdout.map(|reference| {
            let (sender, mut receiver) = mpsc::channel::<Bytes>(4);
            let pipes = self.pipes.clone();
            let output = self.output.clone();
            let cancel = io_cancel.clone();
            let shutdown = closed.clone();
            pipe_tasks.spawn(async move {
                let stream = futures_util::stream::poll_fn(move |cx| {
                    receiver.poll_recv(cx).map(|bytes| bytes.map(Ok))
                });
                let result = pipes.put(&reference.url, stream, &cancel).await;
                report_pipe(&output, reference.id, result, &shutdown).await;
            });
            sender
        });
        let mut failure = None;
        let mut pending_input = None;
        // Whether the backend follows the job; a raw process has no views
        // to follow.
        let mut followed = false;
        let mut follow_open = job.is_some();
        let streamed = async {
        loop {
            let due = job.as_ref().and_then(|(logs, ..)| logs.due(followed));
            tokio::select! {
                biased;
                _ = cancel.cancelled(), if !child.is_cancelled() => child.cancel(),
                signal = signals.recv(), if !signals.is_closed() => {
                    if let Some(signal) = signal
                        && let Err(error) = child.signal(signal).await {
                        failure = Some(error.to_string());
                        child.cancel();
                    }
                }
                command = input.recv(), if pending_input.is_none() && !input.is_closed() => match command {
                    Some(TaskInput::Bytes(bytes)) => pending_input = Some(ProcessInput::Bytes(bytes)),
                    Some(TaskInput::End) => {
                        pending_input = Some(ProcessInput::End);
                        input.close();
                    }
                    None => {},
                },
                permit = child.input.reserve(), if pending_input.is_some() => match permit {
                    Ok(permit) => {
                        permit.send(pending_input.take().expect("pending input"));
                    }
                    Err(_) => {
                        // A program may close stdin before it exits.
                        pending_input = None;
                        input.close();
                    }
                },
                // Before the output: a job that prints without a pause
                // still sends what is due beyond its views.
                () = tokio::time::sleep_until(due.unwrap_or_else(Instant::now)), if due.is_some() => {
                    if let Some((logs, ..)) = job.as_mut() {
                        logs.send_beyond(&id, followed, Due::Now, &self.output)?;
                    }
                }
                changed = following.changed(), if follow_open => match changed {
                    Ok(()) => {
                        followed = *following.borrow_and_update();
                        // Following starts with each stream's newest bytes.
                        if followed && let Some((logs, ..)) = job.as_mut() {
                            logs.send_beyond(&id, followed, Due::Waiting, &self.output)?;
                        }
                    }
                    // The table let go of the job: nobody follows it.
                    Err(_) => {
                        followed = false;
                        follow_open = false;
                    }
                },
                chunk = child.output.recv() => {
                    let Some(chunk) = chunk else {
                        break;
                    };
                    let message = match job.as_mut() {
                        Some((logs, ..)) => {
                            let (offset, head) = logs.write(chunk.stream, &chunk.bytes).await?;
                            (!head.is_empty()).then(|| wire::encode(&wire::Outbound::JobOutput {
                                job_id: id.clone(),
                                stream: chunk.stream,
                                offset,
                                bytes: wire::WireBytes(head.to_vec()),
                            }))
                        }
                        None => Some(wire::encode(&wire::Outbound::SpawnOutput {
                            spawn_id: id.clone(),
                            stream: chunk.stream,
                            bytes: wire::WireBytes(chunk.bytes.to_vec()),
                        })),
                    };
                    if let Some(message) = message {
                        let message = message.map_err(io::Error::other)?;
                        tokio::select! {
                            _ = cancel.cancelled() => child.cancel(),
                            _ = self.output.send(message) => {},
                        }
                    }
                    if let (OutputStream::Stdout, Some(pipe)) = (chunk.stream, stdout_pipe.as_ref()) {
                        tokio::select! {
                            _ = cancel.cancelled() => child.cancel(),
                            // The upload task reports a closed consumer separately.
                            _ = pipe.send(chunk.bytes) => {},
                        }
                    }
                }
            }
        }
        Ok::<_, io::Error>(())
        }.await;
        if let Err(error) = streamed {
            failure = Some(error.to_string());
            child.cancel();
        }
        // A job's last output leaves before its exit: while followed, what
        // the backend does not hold; otherwise each stream's newest bytes.
        if let Some((logs, ..)) = job.as_mut() {
            logs.send_last(&id, followed, &self.output, &closed).await?;
        }
        let (exit, cwd) = child.wait().await;
        drop(execution);
        drop(stdout_pipe);
        input_cancel.cancel();
        // Input may remain live after a command that does not read it exits.
        // Cancel reads, but let a completed stdout upload receive its confirmation.
        pipe_tasks.close();
        if cancel.is_cancelled() {
            io_cancel.cancel();
        }
        // A pipe transfer cannot outlive its process indefinitely.
        let finished = tokio::select! {
            _ = pipe_tasks.wait() => true,
            _ = cancel.cancelled() => false,
            _ = tokio::time::sleep(PIPE_GRACE) => false,
        };
        if !finished {
            io_cancel.cancel();
            pipe_tasks.wait().await;
        }
        let error = failure.or(exit.error);
        // `signal` names only the signal that ended the process. A failure of
        // the runner's own that left no status is the work's spawn error;
        // beside a status it is only logged, since the status says how the
        // work ended.
        let spawn_error = match error {
            Some(reason) if exit.code.is_none() && exit.signal.is_none() => {
                Some(runner_failure(reason))
            }
            Some(reason) => {
                tracing::warn!("{kind:?} ended with a failure of the runner's: {reason}");
                None
            }
            None => None,
        };
        match job {
            // The job's directory stays running until its exit is built, and
            // then lasts until the backend releases it.
            Some((logs, scratch, recorder, _running)) => {
                let output = logs.lengths();
                let report = tokio::task::spawn_blocking(move || {
                    scratch.close()?;
                    Ok::<_, io::Error>(crate::edit_report::finish(recorder.as_ref()))
                })
                .await
                .map_err(io::Error::other)??;
                wire::encode(&wire::Outbound::JobExit {
                    job_id: id,
                    exit_code: exit.code,
                    signal: exit.signal,
                    spawn_error,
                    cwd,
                    output: Some(output),
                    files: report.files,
                    files_truncated: report.files_truncated,
                    presented: report.presented,
                })
                .map_err(io::Error::other)
            }
            None => wire::encode(&wire::Outbound::SpawnExit {
                spawn_id: id,
                exit_code: exit.code,
                signal: exit.signal,
                spawn_error,
            })
            .map_err(io::Error::other),
        }
    }
}

/// Streaming handles are independent from the execution owner so input and output
/// can be selected concurrently without borrowing the owner's control state.
struct Execution {
    input: mpsc::Sender<ProcessInput>,
    output: mpsc::Receiver<OutputChunk>,
    owner: ExecutionOwner,
}

enum ExecutionOwner {
    Process(ChildProcess),
    Shell(Box<dyn ShellJob>),
}

impl Execution {
    fn process(mut child: ChildProcess) -> Self {
        Self {
            input: child.input.clone(),
            output: std::mem::replace(&mut child.output, mpsc::channel(1).1),
            owner: ExecutionOwner::Process(child),
        }
    }
    fn shell(mut child: Box<dyn ShellJob>) -> Self {
        Self {
            input: child.input().clone(),
            output: std::mem::replace(child.output(), mpsc::channel(1).1),
            owner: ExecutionOwner::Shell(child),
        }
    }
    fn cancel(&self) {
        match &self.owner {
            ExecutionOwner::Process(child) => child.cancel(),
            ExecutionOwner::Shell(child) => child.cancel(),
        }
    }
    fn is_cancelled(&self) -> bool {
        match &self.owner {
            ExecutionOwner::Process(child) => child.is_cancelled(),
            ExecutionOwner::Shell(child) => child.is_cancelled(),
        }
    }
    async fn signal(&self, signal: Signal) -> io::Result<()> {
        match &self.owner {
            ExecutionOwner::Process(child) => child.signal(signal).await,
            ExecutionOwner::Shell(child) => child.signal(signal),
        }
    }
    async fn wait(&mut self) -> (ProcessExit, Option<String>) {
        match &mut self.owner {
            ExecutionOwner::Process(child) => (child.wait().await, None),
            ExecutionOwner::Shell(child) => child.wait().await,
        }
    }
}

/// The exit of work the runner could not run, or failed before its end was
/// known: no status, and the reason as its spawn error.
pub fn failure_exit(work: &WorkId, reason: String) -> Result<wire::Frame, wire::WireError> {
    let spawn_error = Some(runner_failure(reason));
    match work {
        WorkId::Job(id) => wire::encode(&wire::Outbound::JobExit {
            job_id: id.clone(),
            exit_code: None,
            signal: None,
            spawn_error,
            cwd: None,
            output: None,
            files: Vec::new(),
            files_truncated: false,
            presented: Vec::new(),
        }),
        WorkId::Spawn(id) => wire::encode(&wire::Outbound::SpawnExit {
            spawn_id: id.clone(),
            exit_code: None,
            signal: None,
            spawn_error,
        }),
    }
}

/// The spawn error of work the runner failed itself, in its words.
fn runner_failure(reason: String) -> wire::SpawnError {
    wire::SpawnError {
        kind: wire::SpawnErrorKind::Other,
        detail: Some(reason),
    }
}

/// How many of a stream's last bytes a job keeps for its messages beyond the
/// stream's first `JOB_VIEW_BYTES`: the most either kind of message carries.
const TAIL_BYTES: usize = if wire::JOB_LIVE_BYTES > wire::JOB_VIEW_BYTES {
    wire::JOB_LIVE_BYTES
} else {
    wire::JOB_VIEW_BYTES
};

/// One stream of a job: its length and last bytes, and what the backend
/// was sent beyond its first `JOB_VIEW_BYTES`.
struct Log {
    stream: OutputStream,
    /// The stream's length.
    bytes: u64,
    /// Its last `TAIL_BYTES` bytes.
    tail: Vec<u8>,
    /// Where the bytes the backend holds end: the first `JOB_VIEW_BYTES`,
    /// then each followed message's end.
    held: u64,
    /// The length the last message beyond the first `JOB_VIEW_BYTES`
    /// reported.
    reported: u64,
    /// When that message went, or found the connection's queue full.
    reported_at: Option<Instant>,
}

/// Which of a job's waiting messages beyond its views go.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Due {
    /// Those whose interval has passed.
    Now,
    /// Every one that waits.
    Waiting,
}

impl Log {
    fn new(stream: OutputStream) -> Self {
        Self {
            stream,
            bytes: 0,
            tail: Vec::new(),
            held: 0,
            reported: 0,
            reported_at: None,
        }
    }

    /// Counts `bytes`, and returns where they start in the stream and their
    /// part within its first `JOB_VIEW_BYTES`, which the backend always
    /// receives.
    fn write(&mut self, bytes: &Bytes) -> (u64, Bytes) {
        let offset = self.bytes;
        let remaining = wire::JOB_VIEW_BYTES.saturating_sub(self.bytes as usize);
        self.bytes += bytes.len() as u64;
        if bytes.len() >= TAIL_BYTES {
            self.tail.clear();
            self.tail
                .extend_from_slice(&bytes[bytes.len() - TAIL_BYTES..]);
        } else {
            let remove = (self.tail.len() + bytes.len()).saturating_sub(TAIL_BYTES);
            self.tail.drain(..remove);
            self.tail.extend_from_slice(bytes);
        }
        let head = bytes.slice(..remaining.min(bytes.len()));
        self.held += head.len() as u64;
        (offset, head)
    }

    /// When the stream's next message beyond its first `JOB_VIEW_BYTES` is
    /// due: while followed, once it has bytes the backend does not hold;
    /// otherwise once it grew past the length last reported.
    fn due(&self, followed: bool) -> Option<Instant> {
        let (waiting, interval) = if followed {
            (self.held < self.bytes, wire::JOB_LIVE_INTERVAL)
        } else {
            (
                self.bytes > wire::JOB_VIEW_BYTES as u64 && self.reported < self.bytes,
                wire::JOB_GROWTH_INTERVAL,
            )
        };
        waiting.then(|| {
            self.reported_at
                .map_or_else(Instant::now, |at| at + interval)
        })
    }

    /// The message beyond the first `JOB_VIEW_BYTES`: while followed, the
    /// newest bytes the backend does not hold, at most `JOB_LIVE_BYTES`;
    /// otherwise the newest `JOB_VIEW_BYTES` beyond the first ones, which end
    /// at the stream's length.
    fn beyond(&self, job: &str, followed: bool) -> Result<wire::Frame, wire::WireError> {
        let offset = if followed {
            self.held
                .max(self.bytes.saturating_sub(wire::JOB_LIVE_BYTES as u64))
        } else {
            (wire::JOB_VIEW_BYTES as u64)
                .max(self.bytes.saturating_sub(wire::JOB_VIEW_BYTES as u64))
        };
        // Past the first `JOB_VIEW_BYTES` the tail is whole, and it holds
        // the newest `TAIL_BYTES`.
        let start = self.bytes - self.tail.len() as u64;
        let from = usize::try_from(offset - start).expect("the newest bytes are in the tail");
        let bytes = self.tail[from..].to_vec();
        wire::encode(&wire::Outbound::JobOutput {
            job_id: job.to_owned(),
            stream: self.stream,
            offset,
            bytes: wire::WireBytes(bytes),
        })
    }

    /// Records that the message beyond the first `JOB_VIEW_BYTES` went.
    fn reported(&mut self, followed: bool) {
        self.reported = self.bytes;
        self.reported_at = Some(Instant::now());
        if followed {
            self.held = self.bytes;
        }
    }

    /// Sends the stream's message beyond its first `JOB_VIEW_BYTES` when
    /// one is `due`, without waiting for room: one that finds the
    /// connection's queue full waits for the stream's next interval.
    fn send_beyond(
        &mut self,
        job: &str,
        followed: bool,
        due: Due,
        output: &mpsc::Sender<wire::Frame>,
    ) -> io::Result<()> {
        let Some(at) = self.due(followed) else {
            return Ok(());
        };
        if due == Due::Now && at > Instant::now() {
            return Ok(());
        }
        let message = self.beyond(job, followed).map_err(io::Error::other)?;
        match output.try_send(message) {
            Ok(()) => self.reported(followed),
            Err(mpsc::error::TrySendError::Full(_)) => self.reported_at = Some(Instant::now()),
            // A disconnected backend follows nothing; the job's cancellation
            // ends it.
            Err(mpsc::error::TrySendError::Closed(_)) => {}
        }
        Ok(())
    }
}

/// A job's output: what it keeps, and each stream's views.
struct Logs {
    kept: KeptOutput,
    stdout: Log,
    stderr: Log,
}

impl Logs {
    fn new(kept: KeptOutput) -> Self {
        Self {
            kept,
            stdout: Log::new(OutputStream::Stdout),
            stderr: Log::new(OutputStream::Stderr),
        }
    }

    /// Keeps one read, and returns where it starts in its stream and its
    /// part within the stream's first `JOB_VIEW_BYTES`.
    async fn write(&mut self, stream: OutputStream, bytes: &Bytes) -> io::Result<(u64, Bytes)> {
        self.kept.write(stream, bytes).await?;
        Ok(match stream {
            OutputStream::Stdout => self.stdout.write(bytes),
            OutputStream::Stderr => self.stderr.write(bytes),
        })
    }

    /// When the next message beyond a stream's first `JOB_VIEW_BYTES` is
    /// due.
    fn due(&self, followed: bool) -> Option<Instant> {
        [self.stdout.due(followed), self.stderr.due(followed)]
            .into_iter()
            .flatten()
            .min()
    }

    /// Sends each stream's message beyond its first `JOB_VIEW_BYTES` that
    /// `due` names.
    fn send_beyond(
        &mut self,
        job: &str,
        followed: bool,
        due: Due,
        output: &mpsc::Sender<wire::Frame>,
    ) -> io::Result<()> {
        self.stdout.send_beyond(job, followed, due, output)?;
        self.stderr.send_beyond(job, followed, due, output)
    }

    /// Sends each stream's last message beyond its first `JOB_VIEW_BYTES`
    /// that waits, waiting for room, before the job's exit goes.
    async fn send_last(
        &mut self,
        job: &str,
        followed: bool,
        output: &mpsc::Sender<wire::Frame>,
        closed: &CancellationToken,
    ) -> io::Result<()> {
        for log in [&mut self.stdout, &mut self.stderr] {
            if log.due(followed).is_none() {
                continue;
            }
            let message = log.beyond(job, followed).map_err(io::Error::other)?;
            tokio::select! {
                // A disconnected backend no longer receives the job's output.
                _ = closed.cancelled() => return Ok(()),
                _ = output.send(message) => log.reported(followed),
            }
        }
        Ok(())
    }

    /// Each stream's length, which the job's exit gives.
    fn lengths(&self) -> wire::OutputLengths {
        wire::OutputLengths {
            stdout_bytes: self.stdout.bytes,
            stderr_bytes: self.stderr.bytes,
        }
    }
}
