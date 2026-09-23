//! Jobs and raw processes (`runner.md` § Command lifetime): the connection
//! owns its job table and routes each job's input, signals and end to it.
//! Every job keeps full output logs and sends bounded views.

use crate::connection::wire;
use crate::{
    commands::{
        contexts::{self, ContextPaths, ExecutionContext, Installation},
        dispatch::Dispatcher,
    },
    connection::ConnectionHandle,
    pipes::PipeClient,
    process::{ChildProcess, OutputStream, ProcessInput, SpawnOptions},
    services::ServiceHandle,
    shell::ShellRuntime,
};
use bytes::Bytes;
use demi_command_service::protocol::CommandContext;
use futures_util::{FutureExt, StreamExt};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    path::PathBuf,
    sync::Arc,
};
use tokio::{
    io::AsyncWriteExt,
    sync::{mpsc, watch},
    task::JoinSet,
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
    pub output_dir: PathBuf,
    pub pipes: PipeClient,
    pub shell: ShellRuntime,
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
    signals: mpsc::Sender<String>,
    cancel: CancellationToken,
}

enum TaskInput {
    Bytes(Bytes),
    End,
}

pub enum TaskCommand {
    Shell {
        script: String,
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
        let cancel = CancellationToken::new();
        self.entries.insert(
            key.clone(),
            Entry {
                input,
                signals,
                cancel: cancel.clone(),
            },
        );
        let config = self.config.clone();
        let closed = self.closed.clone();
        self.running.spawn(async move {
            let id = spec.id.clone();
            let run = config.run(spec, receiver, signal_receiver, cancel, closed.clone());
            let outcome = std::panic::AssertUnwindSafe(run).catch_unwind().await;
            let terminal = match outcome {
                Ok(Ok(terminal)) => Ok(terminal),
                Ok(Err(error)) => failure(&key, id, error.to_string()),
                Err(_) => failure(&key, id, "task owner panicked".into()),
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
        // docs/execution/runner.md § Pipes and output: the native stdin chunk limit.
        if bytes.len() > 64 * 1024 {
            return Err(io::Error::other("live stdin chunk exceeds 64 KiB"));
        }
        self.send(id, TaskInput::Bytes(bytes))
    }

    pub fn end_input(&self, id: &WorkId) -> io::Result<()> {
        self.send(id, TaskInput::End)
    }

    pub fn signal(&self, id: &WorkId, signal: String) -> io::Result<()> {
        let Some(entry) = self.entries.get(id) else {
            return Ok(());
        };
        if signal == "SIGKILL" {
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
    /// and adds what its commands need to `env`.
    async fn context(
        &self,
        job_id: &str,
        manifest_hash: &str,
        command: CommandContext,
        edits: demi_command_service::protocol::EditContext,
        env: &mut BTreeMap<String, String>,
    ) -> io::Result<crate::shell::scope::CommandContext> {
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
        Ok(crate::shell::scope::CommandContext {
            dispatcher: commands.dispatcher.clone(),
            execution: context,
        })
    }

    async fn run(
        self: &Arc<Self>,
        spec: TaskSpec,
        mut input: mpsc::Receiver<TaskInput>,
        mut signals: mpsc::Receiver<String>,
        cancel: CancellationToken,
        closed: CancellationToken,
    ) -> io::Result<wire::Frame> {
        let id = spec.id;
        let mut env = spec.env;
        let mut job = None;
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
                stdin,
                stdout,
                commands,
            } => {
                let JobDirectory { path, scratch } =
                    JobDirectory::create(self.output_dir.clone()).await?;
                env.insert(
                    "TMPDIR".into(),
                    scratch.path().to_string_lossy().into_owned(),
                );
                env.insert("TEMP".into(), scratch.path().to_string_lossy().into_owned());
                env.insert("DEMI_JOB_ID".into(), id.clone());
                let edit_context = demi_command_service::protocol::EditContext {
                    directory: path.join("changes").to_string_lossy().into_owned(),
                    lock: self
                        .output_dir
                        .join("edits.lock")
                        .to_string_lossy()
                        .into_owned(),
                };
                let recording = edit_context.clone();
                let recorder = tokio::task::spawn_blocking(move || {
                    demi_command_service::edits::Recorder::new(recording)
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
                let logs = Logs::new(path, &cancel).await?;
                job = Some((logs, scratch, recorder.clone()));
                let commands = match commands {
                    Some((manifest_hash, command)) => {
                        let setup = self.context(&id, &manifest_hash, command, edit_context, &mut env);
                        // A job killed while it waits for its manifest never starts.
                        let context = tokio::select! {
                            _ = cancel.cancelled() => {
                                return Err(io::Error::other("the job was cancelled before it started"));
                            }
                            context = setup => context?,
                        };
                        Some(context)
                    }
                    None => None,
                };
                let mut scope = crate::shell::scope::Scope::new(cancel.child_token(), commands);
                scope.edits = recorder;
                let child = crate::shell::job::Job::start(
                    script,
                    spec.cwd,
                    env,
                    stdin.is_none(),
                    scope,
                    &self.shell,
                )
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
        let streamed = async {
        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled(), if !child.is_cancelled() => child.cancel(),
                signal = signals.recv(), if !signals.is_closed() => {
                    if let Some(signal) = signal
                        && let Err(error) = child.signal(&signal).await {
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
                chunk = child.output.recv() => {
                    let Some(chunk) = chunk else {
                        break;
                    };
                    let bytes = if let Some((logs, _, _)) = job.as_mut() {
                        logs.write(chunk.stream, &chunk.bytes).await?
                    } else {
                        chunk.bytes.clone()
                    };
                    if !bytes.is_empty() {
                        let message = match kind {
                            WorkId::Job(_) => wire::encode(&wire::Outbound::JobOutput {
                                job_id: id.clone(),
                                stream: chunk.stream,
                                bytes: wire::WireBytes(bytes.to_vec()),
                            }),
                            WorkId::Spawn(_) => wire::encode(&wire::Outbound::SpawnOutput {
                                spawn_id: id.clone(),
                                stream: chunk.stream,
                                bytes: wire::WireBytes(bytes.to_vec()),
                            }),
                        }.map_err(io::Error::other)?;
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
        let (exit, cwd) = child.wait().await;
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
            _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => false,
        };
        if !finished {
            io_cancel.cancel();
            pipe_tasks.wait().await;
        }
        let error = failure.or(exit.error);
        match job {
            Some((logs, scratch, recorder)) => {
                let output = logs.finish().await?;
                let (files, files_truncated) = tokio::task::spawn_blocking(move || {
                    scratch.close()?;
                    Ok::<_, io::Error>(crate::shell::edit_report::finish(recorder.as_ref()))
                })
                .await
                .map_err(io::Error::other)??;
                wire::encode(&wire::Outbound::JobExit {
                    job_id: id,
                    exit_code: exit.code,
                    signal: error.or(exit.signal),
                    spawn_error: None,
                    cwd,
                    output: Some(output),
                    files,
                    files_truncated,
                })
                .map_err(io::Error::other)
            }
            None => wire::encode(&wire::Outbound::SpawnExit {
                spawn_id: id,
                exit_code: exit.code,
                signal: error.or(exit.signal),
                spawn_error: None,
            })
                .map_err(io::Error::other),
        }
    }
}

/// Streaming handles are independent from the execution owner so input and output
/// can be selected concurrently without borrowing the owner's control state.
struct Execution {
    input: mpsc::Sender<ProcessInput>,
    output: mpsc::Receiver<crate::process::OutputChunk>,
    owner: ExecutionOwner,
}

enum ExecutionOwner {
    Process(ChildProcess),
    Shell(crate::shell::job::Job),
}

impl Execution {
    fn process(mut child: ChildProcess) -> Self {
        Self {
            input: child.input.clone(),
            output: std::mem::replace(&mut child.output, mpsc::channel(1).1),
            owner: ExecutionOwner::Process(child),
        }
    }
    fn shell(mut child: crate::shell::job::Job) -> Self {
        Self {
            input: child.input.clone(),
            output: std::mem::replace(&mut child.output, mpsc::channel(1).1),
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
    async fn signal(&self, signal: &str) -> io::Result<()> {
        match &self.owner {
            ExecutionOwner::Process(child) => child.signal(signal).await,
            ExecutionOwner::Shell(child) => child.signal(signal).await,
        }
    }
    async fn wait(&mut self) -> (crate::process::ProcessExit, Option<String>) {
        match &mut self.owner {
            ExecutionOwner::Process(child) => (child.wait().await, None),
            ExecutionOwner::Shell(child) => child.wait().await,
        }
    }
}

fn failure(kind: &WorkId, id: String, error: String) -> Result<wire::Frame, wire::WireError> {
    match kind {
        WorkId::Job(_) => wire::encode(&wire::Outbound::JobExit {
            job_id: id,
            exit_code: None,
            signal: Some(error),
            spawn_error: Some(wire::SpawnError {
                kind: wire::SpawnErrorKind::Other,
                detail: None,
            }),
            cwd: None,
            output: None,
            files: Vec::new(),
            files_truncated: false,
        }),
        WorkId::Spawn(_) => wire::encode(&wire::Outbound::SpawnExit {
            spawn_id: id,
            exit_code: None,
            signal: Some(error),
            spawn_error: Some(wire::SpawnError {
                kind: wire::SpawnErrorKind::Other,
                detail: None,
            }),
        }),
    }
}

/// Reports a pipe end's outcome to the backend; `shutdown` stops reporting
/// without a backend to receive it.
pub(crate) async fn report_pipe(
    output: &mpsc::Sender<wire::Frame>,
    id: String,
    result: io::Result<()>,
    shutdown: &CancellationToken,
) {
    match wire::encode(&wire::Outbound::PipeDone {
        pipe_id: id,
        ok: result.is_ok(),
        error: result.err().map(|error| error.to_string()),
    }) {
        Ok(message) => {
            tokio::select! {
                _ = shutdown.cancelled() => {},
                _ = output.send(message) => {},
            }
        }
        Err(error) => tracing::warn!("pipe result encoding failed: {error}"),
    }
}

/// A job's runner-owned directory for its logs and change records, and the
/// scratch directory its `TMPDIR` names. Wire ids are not paths.
struct JobDirectory {
    path: PathBuf,
    scratch: tempfile::TempDir,
}

impl JobDirectory {
    /// Made in one blocking call, off the control thread.
    async fn create(root: PathBuf) -> io::Result<Self> {
        tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&root)?;
            let mut builder = tempfile::Builder::new();
            builder.prefix("job-");
            // For the owner alone; Windows directories take their access
            // from the parent's ACL.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                builder.permissions(std::fs::Permissions::from_mode(0o700));
            }
            let path = builder.tempdir_in(&root)?.keep();
            let scratch = tempfile::Builder::new().prefix(".work-").tempdir_in(&path)?;
            Ok(Self { path, scratch })
        })
        .await
        .map_err(io::Error::other)?
    }
}

struct Log {
    path: PathBuf,
    file: tokio::fs::File,
    bytes: u64,
    tail: Vec<u8>,
}

impl Log {
    async fn new(path: PathBuf, cancel: &CancellationToken) -> io::Result<Self> {
        // Out of open files, the log waits for one (`runner.md` § Load).
        let file =
            demi_command_service::descriptors::retry(cancel, || tokio::fs::File::create(&path))
                .await?;
        Ok(Self {
            path,
            file,
            bytes: 0,
            tail: Vec::new(),
        })
    }
    async fn write(&mut self, bytes: &Bytes) -> io::Result<Bytes> {
        self.file.write_all(bytes).await?;
        let remaining = wire::JOB_VIEW_BYTES.saturating_sub(self.bytes as usize);
        self.bytes += bytes.len() as u64;
        if bytes.len() >= wire::JOB_VIEW_BYTES {
            self.tail.clear();
            self.tail
                .extend_from_slice(&bytes[bytes.len() - wire::JOB_VIEW_BYTES..]);
        } else {
            let remove = (self.tail.len() + bytes.len()).saturating_sub(wire::JOB_VIEW_BYTES);
            self.tail.drain(..remove);
            self.tail.extend_from_slice(bytes);
        }
        Ok(bytes.slice(..remaining.min(bytes.len())))
    }
}

struct Logs {
    stdout: Log,
    stderr: Log,
}
impl Logs {
    async fn new(path: PathBuf, cancel: &CancellationToken) -> io::Result<Self> {
        Ok(Self {
            stdout: Log::new(path.join("stdout.txt"), cancel).await?,
            stderr: Log::new(path.join("stderr.txt"), cancel).await?,
        })
    }
    async fn write(&mut self, stream: OutputStream, bytes: &Bytes) -> io::Result<Bytes> {
        match stream {
            OutputStream::Stdout => self.stdout.write(bytes).await,
            OutputStream::Stderr => self.stderr.write(bytes).await,
        }
    }
    async fn finish(mut self) -> io::Result<wire::RetainedOutput> {
        self.stdout.file.flush().await?;
        self.stderr.file.flush().await?;
        Ok(wire::RetainedOutput {
            stdout_path: self.stdout.path.to_string_lossy().into_owned(),
            stderr_path: self.stderr.path.to_string_lossy().into_owned(),
            stdout_bytes: self.stdout.bytes,
            stderr_bytes: self.stderr.bytes,
            stdout_tail: wire::WireBytes(self.stdout.tail),
            stderr_tail: wire::WireBytes(self.stderr.tail),
        })
    }
}
