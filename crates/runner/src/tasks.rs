//! Process and shell-job ownership, full output logs and bounded wire views.

use crate::connection::wire;
use crate::{
    pipes::PipeClient,
    process::{ChildProcess, OutputStream, ProcessInput, SpawnOptions},
};
use bytes::Bytes;
use futures_util::{FutureExt, StreamExt};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::{
    io::AsyncWriteExt,
    sync::{Notify, mpsc},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum TaskKind {
    Job,
    Spawn,
}

#[derive(Clone)]
pub struct TaskTable {
    shared: Arc<Shared>,
    tasks: TaskTracker,
    output: mpsc::Sender<wire::Frame>,
    dispatcher: Option<Arc<crate::commands::dispatch::Dispatcher>>,
    output_dir: PathBuf,
    pipes: PipeClient,
}

struct Shared {
    entries: Mutex<HashMap<String, Entry>>,
    changed: Notify,
    closed: CancellationToken,
}

struct Entry {
    kind: TaskKind,
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
    pub lifetime: Option<Box<dyn Send>>,
}

impl TaskTable {
    pub fn new(
        output: mpsc::Sender<wire::Frame>,
        dispatcher: Option<Arc<crate::commands::dispatch::Dispatcher>>,
        output_dir: PathBuf,
        pipes: PipeClient,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                entries: Mutex::new(HashMap::new()),
                changed: Notify::new(),
                closed: CancellationToken::new(),
            }),
            tasks: TaskTracker::new(),
            output,
            dispatcher,
            output_dir,
            pipes,
        }
    }

    pub fn count(&self) -> usize {
        self.shared.entries.lock().unwrap().len()
    }

    pub fn start(&self, mut spec: TaskSpec) -> io::Result<()> {
        if self.shared.closed.is_cancelled() {
            return Err(io::Error::other("task table is closed"));
        }
        if !spec.cwd.is_absolute() {
            return Err(io::Error::other("task cwd must be absolute"));
        }
        let kind = match &spec.command {
            TaskCommand::Shell { .. } => TaskKind::Job,
            TaskCommand::Process { .. } => TaskKind::Spawn,
        };
        let key = task_key(kind, &spec.id);
        let (input, receiver) = mpsc::channel(64);
        let (signals, signal_receiver) = mpsc::channel(16);
        let cancel = CancellationToken::new();
        let mut entries = self.shared.entries.lock().unwrap();
        if self.shared.closed.is_cancelled() {
            return Err(io::Error::other("task table is closed"));
        }
        if entries.contains_key(&key) {
            return Err(io::Error::other("duplicate live task id"));
        }
        entries.insert(
            key.clone(),
            Entry {
                kind,
                input,
                signals,
                cancel: cancel.clone(),
            },
        );
        let table = self.clone();
        self.tasks.spawn(async move {
            let _registration = Registration {
                shared: table.shared.clone(),
                key,
            };
            let lifetime = spec.lifetime.take();
            let id = spec.id.clone();
            let run = table.run(spec, receiver, signal_receiver, cancel);
            let outcome = std::panic::AssertUnwindSafe(run).catch_unwind().await;
            drop(lifetime);
            let terminal = match outcome {
                Ok(Ok(terminal)) => Ok(terminal),
                Ok(Err(error)) => failure(kind, id, error.to_string()),
                Err(_) => failure(kind, id, "task owner panicked".into()),
            };
            match terminal {
                Ok(message) => {
                    // A disconnected backend no longer receives task results.
                    tokio::select! {
                        _ = table.shared.closed.cancelled() => {},
                        _ = table.output.send(message) => {},
                    }
                }
                Err(error) => {
                    tracing::warn!("task terminal encoding failed: {error}")
                }
            }
        });
        drop(entries);
        Ok(())
    }

    /// Live stdin is bounded; bulk streams use the independently flowing HTTP pipe.
    pub fn input(&self, kind: TaskKind, id: &str, bytes: Bytes) -> io::Result<()> {
        // docs/execution/runner.md § Pipes and output: the native stdin chunk limit.
        if bytes.len() > 64 * 1024 {
            return Err(io::Error::other("live stdin chunk exceeds 64 KiB"));
        }
        self.send(kind, id, TaskInput::Bytes(bytes))
    }

    pub fn end_input(&self, kind: TaskKind, id: &str) -> io::Result<()> {
        self.send(kind, id, TaskInput::End)
    }

    pub fn signal(&self, kind: TaskKind, id: &str, signal: String) -> io::Result<()> {
        if signal == "SIGKILL" {
            if let Some(entry) = self.shared.entries.lock().unwrap().get(&task_key(kind, id)) {
                entry.cancel.cancel();
            }
            return Ok(());
        }
        let entries = self.shared.entries.lock().unwrap();
        let Some(entry) = entries.get(&task_key(kind, id)) else {
            return Ok(());
        };
        entry
            .signals
            .try_send(signal)
            .map_err(|error| io::Error::other(format!("signal queue unavailable: {error}")))
    }

    fn send(&self, kind: TaskKind, id: &str, input: TaskInput) -> io::Result<()> {
        let entries = self.shared.entries.lock().unwrap();
        let Some(entry) = entries.get(&task_key(kind, id)) else {
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

    pub async fn wait_idle(&self) {
        loop {
            let changed = self.shared.changed.notified();
            if self.count() == 0 {
                return;
            }
            changed.await;
        }
    }

    pub fn cancel(&self) {
        self.shared.closed.cancel();
        for entry in self.shared.entries.lock().unwrap().values() {
            entry.cancel.cancel();
        }
        self.tasks.close();
    }

    pub async fn close(&self) {
        self.cancel();
        self.tasks.wait().await;
    }

    pub fn job_count(&self) -> usize {
        self.shared
            .entries
            .lock()
            .unwrap()
            .values()
            .filter(|entry| entry.kind == TaskKind::Job)
            .count()
    }

    async fn run(
        &self,
        spec: TaskSpec,
        mut input: mpsc::Receiver<TaskInput>,
        mut signals: mpsc::Receiver<String>,
        cancel: CancellationToken,
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
            } => {
                // Wire IDs are not paths. Each job receives a runner-owned directory.
                tokio::fs::create_dir_all(&self.output_dir).await?;
                let directory = tempfile::Builder::new()
                    .prefix("job-")
                    .tempdir_in(&self.output_dir)?;
                let path = directory.keep();
                crate::fs::chmod(&path, 0o700).await?;
                let scratch = tempfile::Builder::new()
                    .prefix(".work-")
                    .tempdir_in(&path)?;
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
                let recorder =
                    match demi_command_service::edits::Recorder::new(edit_context.clone()) {
                        Ok(recorder) => Some(recorder),
                        Err(error) => {
                            tracing::warn!("edit recording failed: {error}");
                            None
                        }
                    };
                let logs = Logs::new(path, &cancel).await?;
                job = Some((logs, scratch, recorder.clone()));
                let commands = match (
                    &self.dispatcher,
                    env.get(crate::commands::command_client::CONTEXT_ENV),
                ) {
                    (Some(dispatcher), Some(id)) => Some(crate::shell::scope::CommandContext {
                        dispatcher: dispatcher.clone(),
                        execution: dispatcher.contexts.get(id)?,
                    }),
                    (_, None) => None,
                    (None, Some(_)) => {
                        return Err(io::Error::other("shell command dispatcher is unavailable"));
                    }
                };
                if let Some(commands) = &commands {
                    commands
                        .execution
                        .edits
                        .set(edit_context)
                        .map_err(|_| io::Error::other("job recording context was already set"))?;
                }
                let mut scope = crate::shell::scope::Scope::new(cancel.child_token(), commands);
                scope.edits = recorder;
                let child =
                    crate::shell::job::Job::start(script, spec.cwd, env, stdin.is_none(), scope)?;
                (Execution::shell(child), stdin, stdout)
            }
        };
        let kind = if job.is_some() {
            TaskKind::Job
        } else {
            TaskKind::Spawn
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
            let shutdown = self.shared.closed.clone();
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
            let shutdown = self.shared.closed.clone();
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
                            TaskKind::Job => wire::encode(&wire::Outbound::JobOutput {
                                job_id: id.clone(),
                                stream: chunk.stream,
                                bytes: wire::WireBytes(bytes.to_vec()),
                            }),
                            TaskKind::Spawn => wire::encode(&wire::Outbound::SpawnOutput {
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
                scratch.close()?;
                let (files, files_truncated) = tokio::task::spawn_blocking(move || {
                    crate::shell::edit_report::finish(recorder.as_ref())
                })
                .await
                .map_err(io::Error::other)?;
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

struct Registration {
    shared: Arc<Shared>,
    key: String,
}
impl Drop for Registration {
    fn drop(&mut self) {
        self.shared.entries.lock().unwrap().remove(&self.key);
        self.shared.changed.notify_waiters();
    }
}

fn task_key(kind: TaskKind, id: &str) -> String {
    format!(
        "{}:{id}",
        if kind == TaskKind::Job {
            "job"
        } else {
            "spawn"
        }
    )
}

fn failure(kind: TaskKind, id: String, error: String) -> Result<wire::Frame, wire::WireError> {
    match kind {
        TaskKind::Job => wire::encode(&wire::Outbound::JobExit {
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
        TaskKind::Spawn => wire::encode(&wire::Outbound::SpawnExit {
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
