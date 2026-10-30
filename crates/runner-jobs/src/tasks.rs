//! Jobs and raw processes: the registration owns the job table, which outlives
//! each connection, and routes each job's input, signals, following and end to
//! it. A raw process ends with the connection that started it.
//!
//! # Output views
//!
//! A job keeps its output in its directory (`kept_output`) and sends the
//! backend bounded views of it in `job_output` messages:
//!
//! - the first `JOB_VIEW_BYTES` of each stream;
//! - then, while the backend follows the job, its newest bytes at most every
//!   `JOB_LIVE_INTERVAL`;
//! - otherwise only how far a stream grew, every `JOB_GROWTH_INTERVAL`.
//!
//! A job's last output leaves before `job_exit`.
//!
//! What goes beyond the first view never slows the job: its message is queued
//! without waiting, at most once per interval.
//!
//! When a stream went beyond what the backend received, the backend reads the
//! rest from the kept output. When that read fails, it keeps the first bytes, a
//! line counting the bytes left out and the newest bytes, so the end of a
//! build's log, where it says what failed, still shows.
//!
//! # Input
//!
//! A job or process that does not read its live stdin holds it until it reads
//! it, stops or ends: the queue has no length limit, routing a message never
//! waits for work that does not read, and nothing is cancelled for not
//! reading.

use crate::job_directories::{JobDirectories, JobDirectory, Running, StreamLengths};
use crate::job_media::{Arrival, JobMedia, MEDIA_DIRECTORY};
use crate::kept_output::{KeptOutput, Reopened};
use crate::scratch::{SCRATCH_ENV, Scratch};
use crate::{
    commands::{
        contexts::{self, ContextLease, ContextPaths, ContextTable, ExecutionContext, PinnedManifest},
        dispatch::Dispatcher,
    },
    connection::ConnectionHandle,
};
use bytes::Bytes;
use demi_command_protocol::{CommandCaller, CommandContext, Viewable};
use demi_runner_protocol::manifest::Manifest;
use serde::{Deserialize, Serialize};
use demi_runner_command_packages::ResidentHandle;
use demi_runner_process::{
    job_shell::{JobCommands, JobShell, JobStart, ShellJob},
    pipes::{PipeClient, report_pipe},
    process::{ChildProcess, JOB_ID_ENV, OutputChunk, ProcessExit, ProcessInput, SpawnOptions},
};
use demi_runner_protocol::wire::{self, OutputStream, Signal};
use futures_util::{FutureExt, StreamExt};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    path::PathBuf,
    sync::Arc,
};
use tokio::{
    sync::{Semaphore, mpsc, oneshot, watch},
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

/// What a shell job's directory says of the job, which its start writes, for
/// a runner that takes the job over (`JobTable::reattach`).
#[derive(Serialize, Deserialize)]
struct StartRecord {
    job: String,
    conversation: String,
    edits: demi_command_protocol::EditContext,
    /// A job with declared commands' execution context.
    #[serde(default)]
    context: Option<ContextRecord>,
}

/// An execution context as a [`StartRecord`] keeps it: its id and alias
/// directory, which the job's processes name, and what the backend gave it.
#[derive(Serialize, Deserialize)]
struct ContextRecord {
    id: String,
    command: CommandContext,
    manifest: Manifest,
    #[serde(default)]
    viewable: Option<Viewable>,
    aliases: PathBuf,
}

/// The file of a job's [`StartRecord`] in its directory.
pub const START_RECORD: &str = "start.json";

/// How long a task's pipe transfers may run on after its process ended.
const PIPE_GRACE: std::time::Duration = std::time::Duration::from_secs(30);

/// The runner's jobs and raw processes. The registration owns the table,
/// registers each task before its setup runs so that its input and signals
/// find it, and learns of each end through `finished`.
pub struct JobTable {
    entries: HashMap<WorkId, Entry>,
    /// Every job id the table started, which it never starts again
    /// (`crates/runner-client/src/device_jobs.rs`).
    started: HashSet<String>,
    running: JoinSet<WorkId>,
    config: Arc<JobConfig>,
    closed: CancellationToken,
}

/// What every task shares.
pub struct JobConfig {
    /// Where the tasks' messages go: to the connection that serves, and
    /// nowhere while none does.
    pub output: mpsc::UnboundedSender<wire::Frame>,
    /// Where each job gets its own directory.
    pub directories: Arc<JobDirectories>,
    /// Where each conversation has its scratch directory.
    pub scratch: Scratch,
    pub pipes: PipeClient,
    /// Runs the jobs' scripts.
    pub shell: Arc<dyn JobShell>,
    /// For jobs whose manifest declares commands; absent where nothing makes
    /// execution contexts live.
    pub commands: Option<Commands>,
    /// Set once the runner leaves its jobs to the next (`JobTable::leave`).
    pub leaving: Arc<std::sync::atomic::AtomicBool>,
}

/// What a job needs to make its execution context and run declared commands.
pub struct Commands {
    pub dispatcher: Arc<Dispatcher>,
    /// How its commands reach the backend, whichever connection serves.
    pub connection: ConnectionHandle,
    pub paths: ContextPaths,
    pub residents: ResidentHandle,
    /// The local endpoint command clients reach.
    pub endpoint: String,
    /// The installation directory, `DEMI_HOME`.
    pub home: String,
}

struct Entry {
    input: mpsc::UnboundedSender<TaskInput>,
    signals: mpsc::UnboundedSender<Signal>,
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
    input: mpsc::UnboundedReceiver<TaskInput>,
    signals: mpsc::UnboundedReceiver<Signal>,
    following: watch::Receiver<bool>,
}

fn controls_receivers(
    input: mpsc::UnboundedReceiver<TaskInput>,
    signals: mpsc::UnboundedReceiver<Signal>,
    following: watch::Receiver<bool>,
) -> Controls {
    Controls {
        input,
        signals,
        following,
    }
}

/// A task that runs, as its start or a takeover leaves it to
/// [`JobConfig::carry`].
struct Work {
    id: String,
    life: Life,
    child: Execution,
    stdin: Option<wire::PipeRef>,
    stdout: Option<wire::PipeRef>,
    /// A job's logs, edit recorder and hold on its directory; a raw process
    /// has none.
    job: Option<(Logs, Option<demi_command_sdk::edits::Recorder>, Running)>,
    /// A job's execution context, held until the job has ended.
    execution: Option<Arc<ExecutionContext>>,
    /// The media its commands hand to it, as they arrive.
    arrivals: Option<mpsc::UnboundedReceiver<Arrival>>,
    /// The background tasks that keep a job running once its script has
    /// ended.
    outliving: Option<watch::Receiver<Vec<String>>>,
}

pub enum TaskCommand {
    Shell {
        script: String,
        /// The conversation the job belongs to and the agent that started
        /// it, whose scratch directory `DEMI_SCRATCH` names.
        conversation: String,
        caller: CommandCaller,
        stdin: Option<wire::PipeRef>,
        stdout: Option<wire::PipeRef>,
        /// The manifest, command context and viewable media types of a job
        /// with declared commands.
        commands: Option<(PinnedManifest, CommandContext, Option<Viewable>)>,
    },
    Process {
        command: String,
        args: Vec<String>,
        process_group: bool,
        /// Pipes the process inherits, each at its descriptor with the bytes
        /// it carries.
        descriptors: Vec<(u32, Bytes)>,
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
            started: HashSet::new(),
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

    /// Whether the job `job` was started by this table, running or not: a
    /// job id starts once in a runner's life, so a start the backend sent
    /// again, or the first one arriving after it, runs nothing twice.
    pub fn started(&self, job: &str) -> bool {
        self.started.contains(job)
    }

    /// Whether the job `job` runs.
    pub fn runs(&self, job: &str) -> bool {
        self.entries.contains_key(&WorkId::Job(job.to_owned()))
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
        if let WorkId::Job(job) = &key {
            self.started.insert(job.clone());
        }
        let (input, receiver) = mpsc::unbounded_channel();
        let (signals, signal_receiver) = mpsc::unbounded_channel();
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
        self.spawn(key, controls_receivers(receiver, signal_receiver, following), move |controls| {
            async move { config.run(spec, controls, cancel).await }
        });
        Ok(())
    }

    /// Runs task `key`'s work in a task of its own, and sends its end: a
    /// job's stays with its directory too.
    fn spawn<F, W>(&mut self, key: WorkId, controls: Controls, work: W)
    where
        W: FnOnce(Controls) -> F + Send + 'static,
        F: std::future::Future<Output = io::Result<Terminal>> + Send + 'static,
    {
        let config = self.config.clone();
        self.running.spawn(async move {
            let run = work(controls);
            let outcome = std::panic::AssertUnwindSafe(run).catch_unwind().await;
            let terminal = match outcome {
                Ok(Ok(terminal)) => Ok(terminal),
                Ok(Err(error)) => failure_terminal(&key, error.to_string()),
                Err(_) => failure_terminal(&key, "task owner panicked".into()),
            };
            match terminal {
                Ok(terminal) => {
                    // A job's exit stays with its directory until the
                    // backend releases it, so a connection that lost it
                    // hears it again from the next (`crates/runner/src/connection/kept.rs`).
                    if let WorkId::Job(job) = &key {
                        config.directories.ended(
                            job,
                            terminal.frame.clone(),
                            wire::KeptEnd {
                                exit_code: terminal.exit_code,
                                signal: terminal.signal,
                            },
                        );
                    }
                    // What the job changed of the Host's projects goes
                    // first, so what follows its end finds it (`Backlog::report`).
                    if let (WorkId::Job(_), Some(commands)) = (&key, &config.commands) {
                        commands.connection.backlog().report();
                    }
                    // Without a connection the exit waits with the job's
                    // directory.
                    let _ = config.output.send(terminal.frame);
                }
                Err(error) => tracing::warn!(event = "reply.encode_failed", what = "task_terminal", %error, "a task's end was not encoded"),
            }
            key
        });
    }

    /// Takes over the job a runner before this one started, whose directory
    /// is `path` and whose shell still runs: its execution context goes into
    /// `contexts`, and its directory and kept output into the table's
    /// directories, so the next hello lists it. Answers the job's id; a job
    /// it cannot take over is left as it is.
    pub async fn reattach(&mut self, path: PathBuf, contexts: &mut ContextTable) -> io::Result<String> {
        let bytes = tokio::fs::read(path.join(START_RECORD)).await?;
        let record: StartRecord = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        let id = record.job.clone();
        let key = WorkId::Job(id.clone());
        if self.entries.contains_key(&key) {
            return Err(io::Error::other("duplicate live task id"));
        }
        let (arrival, arrivals) = mpsc::unbounded_channel();
        let mut execution = None;
        if let (Some(context), Some(commands)) = (record.context, &self.config.commands) {
            let media = Arc::new(JobMedia::restore(id.clone(), path.join(MEDIA_DIRECTORY), arrival)?);
            let context = Arc::new(ExecutionContext::restore(
                context.id,
                id.clone(),
                context.command,
                Arc::new(context.manifest),
                record.edits.clone(),
                media,
                context.viewable,
                commands.connection.clone(),
                context.aliases,
            ));
            let leases = contexts::leases(&context.manifest, &commands.residents).await;
            contexts.insert(context.clone(), leases)?;
            execution = Some(context);
        }
        let lease = execution
            .clone()
            .map(|context| Box::new(ContextLease::new(context)) as Box<dyn Send + Sync>);
        let child = match self.config.shell.reattach(&id, lease).await {
            Ok(Some(child)) => child,
            Ok(None) => {
                contexts.remove(&id);
                return Err(io::Error::other("the job's directory records no shell"));
            }
            Err(error) => {
                contexts.remove(&id);
                return Err(error);
            }
        };
        let (directory, reopened) = match self.config.directories.reattach(&id, path, TAIL_BYTES).await {
            Ok(directory) => directory,
            Err(error) => {
                child.cancel();
                contexts.remove(&id);
                return Err(error);
            }
        };
        let JobDirectory {
            path: _,
            output,
            lengths,
            media: announced,
            running,
        } = directory;
        let recording = record.edits.clone();
        let recorder = tokio::task::spawn_blocking(move || demi_command_sdk::edits::Recorder::new(recording))
            .await
            .map_err(io::Error::other)?
            .ok();
        let logs = Logs::reopened(output, lengths, announced, reopened);
        running.outlived_by(child.outliving());
        let outliving = Some(child.outliving());
        let life = Life::reattached(&id, record.conversation, self.config.leaving.clone());
        self.started.insert(id.clone());
        let (input, receiver) = mpsc::unbounded_channel();
        let (signals, signal_receiver) = mpsc::unbounded_channel();
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
        let work = Work {
            id: id.clone(),
            life,
            child: Execution::shell(child),
            stdin: None,
            stdout: None,
            job: Some((logs, recorder, running)),
            execution,
            arrivals: Some(arrivals),
            outliving,
        };
        let config = self.config.clone();
        self.spawn(key, controls_receivers(receiver, signal_receiver, following), move |controls| {
            async move { config.carry(work, controls, cancel).await }
        });
        Ok(id)
    }

    /// Leaves every job to the runner that starts next, which takes them
    /// over: the table lets go of them without stopping one. Raw processes
    /// end with the runner, as they do with a connection.
    pub async fn leave(&mut self) {
        self.config.leaving.store(true, std::sync::atomic::Ordering::SeqCst);
        self.config.shell.leave();
        self.closed.cancel();
        for (id, entry) in &self.entries {
            if matches!(id, WorkId::Spawn(_)) {
                entry.cancel.cancel();
            }
        }
        self.entries.clear();
        self.running.abort_all();
        while self.running.join_next().await.is_some() {}
    }

    /// Live stdin goes in chunks of at most `STDIN_CHUNK_BYTES`; bulk streams
    /// use the independently flowing HTTP pipe.
    pub fn input(&self, id: &WorkId, bytes: Bytes) -> io::Result<()> {
        if bytes.len() > wire::STDIN_CHUNK_BYTES {
            return Err(io::Error::other("live stdin chunk exceeds 64 KiB"));
        }
        self.send(id, TaskInput::Bytes(bytes));
        Ok(())
    }

    pub fn end_input(&self, id: &WorkId) {
        self.send(id, TaskInput::End);
    }

    /// Starts or stops following a job's output beyond each stream's first
    /// `JOB_VIEW_BYTES`; a job that ended is not followed any more.
    pub fn follow(&self, id: &WorkId, follow: bool) {
        if let Some(entry) = self.entries.get(id) {
            entry.follow.send_replace(follow);
        }
    }

    pub fn signal(&self, id: &WorkId, signal: Signal) {
        let Some(entry) = self.entries.get(id) else {
            return;
        };
        if signal == Signal::Kill {
            entry.cancel.cancel();
            return;
        }
        // A task that ended takes no signal.
        let _ = entry.signals.send(signal);
    }

    fn send(&self, id: &WorkId, input: TaskInput) {
        // A task that ended takes no input.
        if let Some(entry) = self.entries.get(id) {
            let _ = entry.input.send(input);
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
    /// Then every job's shell ends, also one that runs on for an ended job.
    pub async fn close(&mut self) {
        self.closed.cancel();
        for entry in self.entries.values() {
            entry.cancel.cancel();
        }
        while self.finished().await.is_some() {}
        self.config.shell.close();
    }

    /// Ends every raw process, which lasts as long as the connection that
    /// started it; the jobs run on.
    pub fn end_spawns(&self) {
        for (id, entry) in &self.entries {
            if matches!(id, WorkId::Spawn(_)) {
                entry.cancel.cancel();
            }
        }
    }

    /// The jobs that run now.
    pub fn running_jobs(&self) -> Vec<String> {
        self.entries
            .keys()
            .filter_map(|id| match id {
                WorkId::Job(job) => Some(job.clone()),
                WorkId::Spawn(_) => None,
            })
            .collect()
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
        manifest: PinnedManifest,
        command: CommandContext,
        edits: demi_command_protocol::EditContext,
        media: Arc<JobMedia>,
        viewable: Option<Viewable>,
        env: &mut BTreeMap<String, String>,
    ) -> io::Result<(Arc<ExecutionContext>, JobCommands)> {
        let commands = self
            .commands
            .as_ref()
            .ok_or_else(|| io::Error::other("shell command dispatcher is unavailable"))?;
        let manifest = manifest.manifest().await?;
        let context = Arc::new(
            ExecutionContext::create(
                job_id.to_owned(),
                command,
                manifest,
                edits,
                media,
                viewable,
                commands.connection.clone(),
                &commands.paths,
            )
            .await?,
        );
        let leases = contexts::leases(&context.manifest, &commands.residents).await;
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
    ) -> io::Result<Terminal> {
        let id = spec.id;
        let mut life = Life::new(
            &id,
            matches!(spec.command, TaskCommand::Shell { .. }),
            match &spec.command {
                TaskCommand::Shell { conversation, .. } => Some(conversation.clone()),
                TaskCommand::Process { .. } => None,
            },
            self.leaving.clone(),
        );
        let mut env = spec.env;
        let mut job = None;
        // A job's execution context, held until the job has ended.
        let mut execution = None;
        // The media its commands hand to it, as they arrive
        // (`crates/plugin-file/command/src/files.rs`).
        let mut arrivals: Option<mpsc::UnboundedReceiver<Arrival>> = None;
        // The background tasks that keep a job running once its script has
        // ended (`crates/agent-tools/src/result.rs`); a raw process has none.
        let mut outliving: Option<watch::Receiver<Vec<String>>> = None;
        let (child, stdin, stdout) = match spec.command {
            TaskCommand::Process {
                command,
                args,
                process_group,
                descriptors,
            } => {
                let child = ChildProcess::spawn(SpawnOptions {
                    command,
                    args,
                    process_group,
                    cwd: spec.cwd,
                    env,
                    descriptors,
                })
                .await;
                let child = match child {
                    Ok(child) => child,
                    Err(error) => {
                        life.ended(None, None, "spawn_failed", Some(spawn_kind(error.kind)), 0);
                        return Terminal::of(wire::encode(&wire::Outbound::SpawnExit {
                            spawn_id: id,
                            exit_code: None,
                            signal: None,
                            spawn_error: Some(wire::SpawnError {
                                kind: error.kind,
                                detail: Some(error.message),
                            }),
                        }), None, None);
                    }
                };
                (Execution::process(child), None, None)
            }
            TaskCommand::Shell {
                script,
                conversation,
                caller,
                stdin,
                stdout,
                commands,
            } => {
                let JobDirectory {
                    path,
                    output,
                    lengths,
                    media: announced,
                    running,
                } = self.directories.create(&id, &cancel).await?;
                env.insert(JOB_ID_ENV.into(), id.clone());
                let scratch = self.scratch.create(&conversation, &caller).await?;
                env.insert(SCRATCH_ENV.into(), scratch.to_string_lossy().into_owned());
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
                let edits_recorded = edit_context.clone();
                let recorder = tokio::task::spawn_blocking(move || {
                    demi_command_sdk::edits::Recorder::new(recording)
                })
                .await
                .map_err(io::Error::other)?;
                let recorder = match recorder {
                    Ok(recorder) => Some(recorder),
                    Err(error) => {
                        tracing::warn!(event = "edit_record.failed", job_id = %id, %error, "a job's edits could not be recorded");
                        None
                    }
                };
                job = Some((Logs::new(output, lengths, announced), recorder.clone(), running));
                let commands = match commands {
                    Some((manifest, command, viewable)) => {
                        let (sender, receiver) = mpsc::unbounded_channel();
                        arrivals = Some(receiver);
                        let media = Arc::new(JobMedia::new(
                            id.clone(),
                            path.join(MEDIA_DIRECTORY),
                            sender,
                        ));
                        let setup = self.context(
                            &id,
                            manifest,
                            command,
                            edit_context,
                            media,
                            viewable,
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
                let record = StartRecord {
                    job: id.clone(),
                    conversation: conversation.clone(),
                    edits: edits_recorded,
                    context: execution.as_ref().map(|context| ContextRecord {
                        id: context.id.clone(),
                        command: context.command.clone(),
                        manifest: Manifest::clone(&context.manifest),
                        viewable: context.viewable.clone(),
                        aliases: context.aliases().to_owned(),
                    }),
                };
                // A runner that starts next takes the job over by what the
                // record says (`JobTable::reattach`).
                let bytes = serde_json::to_vec(&record).map_err(io::Error::other)?;
                tokio::fs::write(path.join(START_RECORD), bytes).await?;
                let child = self
                    .shell
                    .start(JobStart {
                        script,
                        cwd: spec.cwd,
                        env,
                        live: stdin.is_none(),
                        cancellation: cancel.child_token(),
                        commands,
                        edits: recorder,
                        lease: execution.clone().map(|context| {
                            Box::new(ContextLease::new(context)) as Box<dyn Send + Sync>
                        }),
                    })
                    .await?;
                outliving = Some(child.outliving());
                if let Some((_, _, running)) = &job {
                    running.outlived_by(child.outliving());
                }
                (Execution::shell(child), stdin, stdout)
            }
        };
        self.carry(
            Work {
                id,
                life,
                child,
                stdin,
                stdout,
                job,
                execution,
                arrivals,
                outliving,
            },
            controls,
            cancel,
        )
        .await
    }

    /// Follows a job or process that runs, from its start or from a runner
    /// before this one (`JobTable::reattach`), until it ends: its input,
    /// signals, output and media, then its end.
    async fn carry(
        self: &Arc<Self>,
        work: Work,
        controls: Controls,
        cancel: CancellationToken,
    ) -> io::Result<Terminal> {
        let Work {
            id,
            mut life,
            mut child,
            stdin,
            stdout,
            mut job,
            execution,
            mut arrivals,
            mut outliving,
        } = work;
        let Controls {
            mut input,
            mut signals,
            mut following,
        } = controls;
        let io_cancel = cancel.child_token();
        let _io_guard = io_cancel.clone().drop_guard();
        let input_cancel = io_cancel.child_token();
        let pipe_tasks = TaskTracker::new();
        if let Some(reference) = stdin {
            let pipes = self.pipes.clone();
            let input = child.input.clone();
            let output = self.output.clone();
            // The job stops reading when it ends.
            let stopped = input_cancel.clone();
            // The read itself lasts until the backend ends it, within the
            // pipes' grace after the job's end.
            let reading = io_cancel.clone();
            pipe_tasks.spawn(async move {
                // The read, which the job may stop before the backend
                // answers it, and its body once answered.
                let mut getting = std::pin::pin!(pipes.get(&reference, reading));
                let mut body = None;
                // Whether the job stopped reading before the input's end.
                let fed = async {
                    let answered = tokio::select! {
                        biased;
                        _ = stopped.cancelled() => return Ok(true),
                        answered = &mut getting => answered?,
                    };
                    let body = body.insert(answered);
                    loop {
                        let bytes = tokio::select! {
                            biased;
                            _ = stopped.cancelled() => return Ok(true),
                            bytes = body.next() => bytes,
                        };
                        let Some(bytes) = bytes else {
                            return Ok(false);
                        };
                        let bytes = bytes?;
                        tokio::select! {
                            biased;
                            _ = stopped.cancelled() => return Ok(true),
                            // A program that closed its standard input
                            // stopped reading it, as `head -1` does.
                            sent = input.send(ProcessInput::Bytes(bytes)) => if sent.is_err() {
                                return Ok(true);
                            },
                        }
                    }
                }
                .await;
                match fed {
                    // The job stopped reading before the input's end: its
                    // end of the pipe stopped early, which is no failure,
                    // as a local writer whose reader has gone stops writing
                    // without a message. The report drains the pipe while
                    // this end still holds its read, also one the backend
                    // has not answered yet, so the backend never takes the
                    // read's end for a cut response; the backend then ends
                    // the read, and what still arrives goes nowhere.
                    Ok(true) => {
                        report_pipe(&output, reference.id.clone(), Ok(()));
                        // The pipe is over by the report; how its read ends
                        // tells nothing more.
                        let body = match body {
                            Some(body) => Ok(body),
                            None => getting.await,
                        };
                        if let Ok(mut body) = body {
                            while let Some(Ok(_)) = body.next().await {}
                        }
                    }
                    result => {
                        // EOF, also when the pipe failed or the source was
                        // refused, releases a reader; a program that closed
                        // its standard input or a job that ended reads none.
                        tokio::select! {
                            _ = stopped.cancelled() => {}
                            _sent = input.send(ProcessInput::End) => {}
                        }
                        report_pipe(&output, reference.id.clone(), result.map(|_| ()));
                    }
                }
            });
        }
        let stdout_pipe = stdout.map(|reference| {
            // A job's stdout is bulk: the job waits for the pipe's upload rather
            // than the runner holding it without end.
            let (sender, mut receiver) = mpsc::channel::<Bytes>(4);
            let pipes = self.pipes.clone();
            let output = self.output.clone();
            let cancel = io_cancel.clone();
            pipe_tasks.spawn(async move {
                let stream = futures_util::stream::poll_fn(move |cx| {
                    receiver.poll_recv(cx).map(|bytes| bytes.map(Ok))
                });
                // A stream pipe: what the job prints must reach the backend
                // as it prints it, however long it runs (`crates/runner/src/main.rs`).
                let result = pipes.stream(&reference, stream, &cancel).await;
                report_pipe(&output, reference.id, result);
            });
            sender
        });
        let mut failure = None;
        let mut pending_input = None;
        // Whether the backend follows the job; a raw process has no views
        // to follow.
        let mut followed = false;
        let mut follow_open = job.is_some();
        // Whether the job's stdout ends a line, so that a medium's line
        // stands on a line of its own.
        let mut stdout_line_start = true;
        // A raw process's output goes one frame at a time into the
        // connection's queue, so a process that prints faster than the
        // uplink carries is read no faster.
        let paced = Arc::new(Semaphore::new(1));
        // The media that wait for the drain of the job's stdout pipe, and
        // the drain's answer.
        let mut waiting: Vec<Arrival> = Vec::new();
        let mut drained: Option<oneshot::Receiver<()>> = None;
        let streamed = async {
        loop {
            let due = job.as_ref().and_then(|(logs, ..)| logs.due(followed));
            tokio::select! {
                biased;
                _ = cancel.cancelled(), if !child.is_cancelled() => child.cancel(),
                signal = signals.recv(), if !signals.is_closed() => {
                    if let Some(signal) = signal
                        && let Err(error) = child.signal(signal) {
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
                changed = async { outliving.as_mut().expect("the job's tasks are watched").changed().await }, if outliving.is_some() => match changed {
                    Ok(()) => {
                        let tasks = outliving
                            .as_mut()
                            .expect("the job's tasks are watched")
                            .borrow_and_update()
                            .clone();
                        if let Some(message) = outliving_message(&id, &tasks) {
                            let _ = self.output.send(message.map_err(io::Error::other)?);
                        }
                    }
                    // The job's shell let go of its tasks: it is ending.
                    Err(_) => outliving = None,
                },
                // A medium waits for what the job's stdout pipe holds when
                // it arrives, so its line follows output written before it
                // and precedes what the job prints after it.
                arrival = next_arrival(&mut arrivals) => {
                    waiting.push(arrival);
                    if drained.is_none() {
                        drained = Some(child.drain_stdout());
                    }
                }
                // The pipe's bytes are in the output channel now: they go
                // first, then the lines of the media that waited.
                _ = async { drained.as_mut().expect("a drain is asked for").await }, if drained.is_some() => {
                    drained = None;
                    while let Ok(chunk) = child.output.try_recv() {
                        let logs = job.as_mut().map(|(logs, ..)| logs);
                        let (stream, length) = (chunk.stream, chunk.bytes.len());
                        let cancelled = take_chunk(chunk, logs, &id, &self.output, &paced, stdout_pipe.as_ref(), &cancel, &mut stdout_line_start).await?;
                        child.kept(stream, length);
                        if cancelled {
                            child.cancel();
                        }
                    }
                    if let Some((logs, ..)) = job.as_mut() {
                        for arrival in waiting.drain(..) {
                            for message in logs.arrive(&id, arrival, &mut stdout_line_start).await? {
                                let _ = self.output.send(message);
                            }
                        }
                    }
                }
                chunk = child.output.recv() => {
                    let Some(chunk) = chunk else {
                        break;
                    };
                    let logs = job.as_mut().map(|(logs, ..)| logs);
                    let (stream, length) = (chunk.stream, chunk.bytes.len());
                    let cancelled = take_chunk(chunk, logs, &id, &self.output, &paced, stdout_pipe.as_ref(), &cancel, &mut stdout_line_start).await?;
                    child.kept(stream, length);
                    if cancelled {
                        child.cancel();
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
        // The media that arrived as the job ended keep their place before
        // its exit, after everything it printed.
        if let (Some((logs, ..)), Some(receiver)) = (job.as_mut(), arrivals.as_mut()) {
            while let Ok(arrival) = receiver.try_recv() {
                waiting.push(arrival);
            }
            for arrival in waiting.drain(..) {
                for message in logs.arrive(&id, arrival, &mut stdout_line_start).await? {
                    let _ = self.output.send(message);
                }
            }
        }
        // A job's last output leaves before its exit: while followed, what
        // the backend does not hold; otherwise each stream's newest bytes.
        if let Some((logs, ..)) = job.as_mut() {
            logs.send_last(&id, followed, &self.output)?;
        }
        let exit = child.wait().await;
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
                tracing::warn!(
                    event = "task.failed",
                    kind = if job.is_some() { "job" } else { "spawn" },
                    job_id = %id,
                    %reason,
                    "a task ended with a failure of the runner's own"
                );
                None
            }
            None => None,
        };
        let (code, signal) = (exit.code, exit.signal.clone());
        // A failure of the runner's beside a status is only logged, so a spawn
        // error here is one the runner made.
        let cause = if spawn_error.is_some() {
            "runner_failure"
        } else if cancel.is_cancelled() {
            "killed"
        } else if signal.is_some() {
            "stopped"
        } else {
            "exited"
        };
        let spawn_detail = spawn_error.as_ref().map(|error| spawn_kind(error.kind));
        let output_bytes = job.as_ref().map_or(0, |(logs, ..)| {
            let lengths = logs.lengths();
            lengths.stdout_bytes + lengths.stderr_bytes
        });
        life.ended(code, signal.clone(), cause, spawn_detail, output_bytes);
        match job {
            // The job's directory stays running until its exit is built, and
            // then lasts until the backend releases it.
            Some((logs, recorder, _running)) => {
                let output = logs.lengths();
                let named = id.clone();
                let edits = tokio::task::spawn_blocking(move || crate::edit_report::finish(&named, recorder.as_ref()))
                .await
                .map_err(io::Error::other)?;
                Terminal::of(
                    wire::encode(&wire::Outbound::JobExit {
                        job_id: id,
                        exit_code: exit.code,
                        signal: exit.signal,
                        spawn_error,
                        output: Some(output),
                        files: edits.files,
                        path_changes: edits.path_changes,
                        files_truncated: edits.truncated,
                    }),
                    code,
                    signal,
                )
            }
            None => Terminal::of(
                wire::encode(&wire::Outbound::SpawnExit {
                    spawn_id: id,
                    exit_code: exit.code,
                    signal: exit.signal,
                    spawn_error,
                }),
                code,
                signal,
            ),
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
    fn signal(&self, signal: Signal) -> io::Result<()> {
        match &self.owner {
            ExecutionOwner::Process(child) => child.signal(signal),
            ExecutionOwner::Shell(child) => child.signal(signal),
        }
    }
    async fn wait(&mut self) -> ProcessExit {
        match &mut self.owner {
            ExecutionOwner::Process(child) => child.wait().await,
            ExecutionOwner::Shell(child) => child.wait().await,
        }
    }
    /// The runner has kept a chunk of the job's output, which its shell
    /// may let go of; a raw process holds nothing.
    fn kept(&self, stream: OutputStream, bytes: usize) {
        if let ExecutionOwner::Shell(child) = &self.owner {
            child.kept(stream, bytes as u64);
        }
    }
    /// Reads what the stdout pipe holds now into the output channel; a raw
    /// process views no media, so it answers at once.
    fn drain_stdout(&self) -> oneshot::Receiver<()> {
        match &self.owner {
            ExecutionOwner::Shell(child) => child.drain_stdout(),
            ExecutionOwner::Process(_) => {
                let (answer, answered) = oneshot::channel();
                let _ = answer.send(());
                answered
            }
        }
    }
}

/// Records one chunk of output: a job's into its logs, with the part the
/// backend's view takes, and its stdout into the relayed pipe; a raw
/// process's as it is. True when the work was cancelled while a frame
/// waited to go.
async fn take_chunk(
    chunk: OutputChunk,
    logs: Option<&mut Logs>,
    id: &str,
    output: &mpsc::UnboundedSender<wire::Frame>,
    paced: &Arc<Semaphore>,
    pipe: Option<&mpsc::Sender<Bytes>>,
    cancel: &CancellationToken,
    stdout_line_start: &mut bool,
) -> io::Result<bool> {
    if chunk.stream == OutputStream::Stdout
        && let Some(last) = chunk.bytes.last()
    {
        *stdout_line_start = *last == b'\n';
    }
    let message = match logs {
        Some(logs) => {
            let (offset, head) = logs.write(chunk.stream, &chunk.bytes).await?;
            (!head.is_empty()).then(|| {
                wire::encode(&wire::Outbound::JobOutput {
                    job_id: id.to_owned(),
                    stream: chunk.stream,
                    offset,
                    bytes: wire::WireBytes(head.to_vec()),
                })
            })
        }
        None => {
            let permit = tokio::select! {
                _ = cancel.cancelled() => return Ok(true),
                permit = paced.clone().acquire_owned() => permit.expect("the semaphore is never closed"),
            };
            Some(
                wire::encode(&wire::Outbound::SpawnOutput {
                    spawn_id: id.to_owned(),
                    stream: chunk.stream,
                    bytes: wire::WireBytes(chunk.bytes.to_vec()),
                })
                .map(|frame| frame.held_by(permit)),
            )
        }
    };
    if let Some(message) = message {
        let _ = output.send(message.map_err(io::Error::other)?);
    }
    if let (OutputStream::Stdout, Some(pipe)) = (chunk.stream, pipe) {
        tokio::select! {
            _ = cancel.cancelled() => return Ok(true),
            // The upload task reports a closed consumer separately.
            _ = pipe.send(chunk.bytes) => {},
        }
    }
    Ok(false)
}

/// The tasks that outlive a job's script as the wire carries them: at most
/// its number of them, each command line cut to its bound.
pub(crate) fn wire_tasks(tasks: &[String]) -> Vec<String> {
    tasks
        .iter()
        .filter(|line| !line.is_empty())
        .take(wire::JOB_OUTLIVING_TASKS)
        .map(|line| line[..line.floor_char_boundary(wire::JOB_TASK_LINE_BYTES)].to_owned())
        .collect()
}

/// The `job_outliving` that names `tasks`; none while the script runs, when
/// there are none.
fn outliving_message(id: &str, tasks: &[String]) -> Option<Result<wire::Frame, wire::WireError>> {
    let tasks = wire_tasks(tasks);
    (!tasks.is_empty()).then(|| {
        wire::encode(&wire::Outbound::JobOutliving {
            job_id: id.to_owned(),
            tasks,
        })
    })
}

/// The next medium a job's commands hand to it; never, for a job without
/// declared commands or once none can arrive any more.
async fn next_arrival(arrivals: &mut Option<mpsc::UnboundedReceiver<Arrival>>) -> Arrival {
    if let Some(receiver) = arrivals.as_mut()
        && let Some(arrival) = receiver.recv().await
    {
        return arrival;
    }
    *arrivals = None;
    std::future::pending().await
}

/// A task's last message, its exit, with the status it carries.
struct Terminal {
    frame: wire::Frame,
    exit_code: Option<i32>,
    signal: Option<String>,
}

impl Terminal {
    fn of(
        frame: Result<wire::Frame, wire::WireError>,
        exit_code: Option<i32>,
        signal: Option<String>,
    ) -> io::Result<Self> {
        Ok(Self {
            frame: frame.map_err(io::Error::other)?,
            exit_code,
            signal,
        })
    }
}

/// The exit of a task that failed before its end was known.
fn failure_terminal(work: &WorkId, reason: String) -> Result<Terminal, wire::WireError> {
    Ok(Terminal {
        frame: failure_exit(work, reason)?,
        exit_code: None,
        signal: None,
    })
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
            output: None,
            files: Vec::new(),
            path_changes: Vec::new(),
            files_truncated: false,
        }),
        WorkId::Spawn(id) => wire::encode(&wire::Outbound::SpawnExit {
            spawn_id: id.clone(),
            exit_code: None,
            signal: None,
            spawn_error,
        }),
    }
}

/// The word the log gives a spawn error's kind.
fn spawn_kind(kind: wire::SpawnErrorKind) -> &'static str {
    match kind {
        wire::SpawnErrorKind::ExecutableNotFound => "executable_not_found",
        wire::SpawnErrorKind::PermissionDenied => "permission_denied",
        wire::SpawnErrorKind::CwdUnusable => "cwd_unusable",
        wire::SpawnErrorKind::IsDirectory => "is_directory",
        wire::SpawnErrorKind::Other => "other",
    }
}

/// A work's life in the log: `job.started` as it is made, and the one line
/// `job.ended` when it is dropped, whichever way `run` leaves, a failure of
/// the runner's own by `?` included.
struct Life {
    id: String,
    conversation: Option<String>,
    began: Instant,
    end: Option<Ending>,
    /// The runner's: a job it leaves to the next runner has not ended.
    leaving: Arc<std::sync::atomic::AtomicBool>,
}

struct Ending {
    exit: Option<i32>,
    signal: Option<String>,
    cause: &'static str,
    spawn_error: Option<&'static str>,
    output_bytes: u64,
}

impl Life {
    /// The life of a job a runner before this one started, from its
    /// takeover on.
    fn reattached(id: &str, conversation: String, leaving: Arc<std::sync::atomic::AtomicBool>) -> Self {
        tracing::info!(
            event = "job.reattached",
            job_id = id,
            conversation = conversation.as_str(),
            "a job a runner before this one started was taken over"
        );
        Self {
            id: id.to_owned(),
            conversation: Some(conversation),
            began: Instant::now(),
            end: None,
            leaving,
        }
    }

    fn new(
        id: &str,
        shell: bool,
        conversation: Option<String>,
        leaving: Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        tracing::info!(
            event = "job.started",
            job_id = id,
            conversation = conversation.as_deref(),
            shell = if shell { "builtin" } else { "process" },
            "a job started"
        );
        Self {
            id: id.to_owned(),
            conversation,
            began: Instant::now(),
            end: None,
            leaving,
        }
    }

    fn ended(
        &mut self,
        exit: Option<i32>,
        signal: Option<String>,
        cause: &'static str,
        spawn_error: Option<&'static str>,
        output_bytes: u64,
    ) {
        self.end = Some(Ending {
            exit,
            signal,
            cause,
            spawn_error,
            output_bytes,
        });
    }
}

impl Drop for Life {
    fn drop(&mut self) {
        if self.end.is_none() && self.leaving.load(std::sync::atomic::Ordering::SeqCst) {
            tracing::info!(
                event = "job.left",
                job_id = self.id.as_str(),
                conversation = self.conversation.as_deref(),
                ms = self.began.elapsed().as_millis() as u64,
                "a job runs on for the runner that starts next"
            );
            return;
        }
        let end = self.end.take().unwrap_or(Ending {
            exit: None,
            signal: None,
            cause: "runner_failure",
            spawn_error: None,
            output_bytes: 0,
        });
        tracing::info!(
            event = "job.ended",
            job_id = self.id.as_str(),
            conversation = self.conversation.as_deref(),
            exit = end.exit,
            signal = end.signal.as_deref(),
            cause = end.cause,
            spawn_error = end.spawn_error,
            output_bytes = end.output_bytes,
            ms = self.began.elapsed().as_millis() as u64,
            "a job ended"
        );
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
    /// When that message went.
    reported_at: Option<Instant>,
    /// Held by the message beyond the first `JOB_VIEW_BYTES` while it waits
    /// in the connection's queue, so a stream has one waiting at most and the
    /// next one, made when it leaves, holds the newest bytes: a slow uplink
    /// cannot pile previews up.
    waiting: Arc<Semaphore>,
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
            waiting: Arc::new(Semaphore::new(1)),
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
    /// one is `due`.
    fn send_beyond(
        &mut self,
        job: &str,
        followed: bool,
        due: Due,
        output: &mpsc::UnboundedSender<wire::Frame>,
    ) -> io::Result<()> {
        let Some(at) = self.due(followed) else {
            return Ok(());
        };
        if due == Due::Now && at > Instant::now() {
            return Ok(());
        }
        let Ok(permit) = self.waiting.clone().try_acquire_owned() else {
            // The last one has not left yet: the next goes an interval
            // later, with the bytes that arrived by then.
            self.reported_at = Some(Instant::now());
            return Ok(());
        };
        let message = self.beyond(job, followed).map_err(io::Error::other)?;
        // A disconnected backend follows nothing; the job's cancellation
        // ends it.
        if output.send(message.held_by(permit)).is_ok() {
            self.reported(followed);
        }
        Ok(())
    }
}

/// A job's output: what it keeps, and each stream's views.
struct Logs {
    kept: KeptOutput,
    /// Each stream's length, as its directory reports it to the next
    /// connection.
    lengths: Arc<StreamLengths>,
    /// The `job_medium`s sent, which the next connection hears again.
    announced: Arc<std::sync::Mutex<Vec<wire::Frame>>>,
    stdout: Log,
    stderr: Log,
}

impl Logs {
    fn new(
        kept: KeptOutput,
        lengths: Arc<StreamLengths>,
        announced: Arc<std::sync::Mutex<Vec<wire::Frame>>>,
    ) -> Self {
        Self {
            kept,
            lengths,
            announced,
            stdout: Log::new(OutputStream::Stdout),
            stderr: Log::new(OutputStream::Stderr),
        }
    }

    /// The logs of a job a runner before this one kept in `kept`, which this
    /// one writes on: each stream goes on from its length, and the backend
    /// is taken to hold what it was sent, as the next hello's lengths let it
    /// read what it missed (`crates/runner-client/src/device_jobs.rs`).
    fn reopened(
        kept: KeptOutput,
        lengths: Arc<StreamLengths>,
        announced: Arc<std::sync::Mutex<Vec<wire::Frame>>>,
        reopened: Reopened,
    ) -> Self {
        let Reopened {
            lengths: counts,
            tails: [stdout_tail, stderr_tail],
        } = reopened;
        let log = |stream, bytes: u64, tail: Vec<u8>| {
            let mut log = Log::new(stream);
            log.bytes = bytes;
            log.tail = tail;
            log.held = bytes;
            log.reported = bytes;
            log
        };
        let logs = Self {
            kept,
            lengths,
            announced,
            stdout: log(OutputStream::Stdout, counts[0], stdout_tail),
            stderr: log(OutputStream::Stderr, counts[1], stderr_tail),
        };
        logs.lengths.set(logs.lengths());
        logs
    }

    /// Writes a medium's line into the job's stdout, on a line of its own,
    /// answers the command that returned it, and returns the frames that
    /// tell the backend: the line's part within the stream's first
    /// `JOB_VIEW_BYTES`, then the medium's `job_medium`.
    async fn arrive(
        &mut self,
        job: &str,
        arrival: Arrival,
        line_start: &mut bool,
    ) -> io::Result<Vec<wire::Frame>> {
        let separator = if *line_start { "" } else { "\n" };
        let line = Bytes::from(format!("{separator}{}\n", arrival.line));
        *line_start = true;
        let (offset, head) = self.write(OutputStream::Stdout, &line).await?;
        // A command cancelled while it waited needs no answer.
        let _ = arrival.written.send(());
        let mut frames = Vec::new();
        if !head.is_empty() {
            frames.push(
                wire::encode(&wire::Outbound::JobOutput {
                    job_id: job.to_owned(),
                    stream: OutputStream::Stdout,
                    offset,
                    bytes: wire::WireBytes(head.to_vec()),
                })
                .map_err(io::Error::other)?,
            );
        }
        if let Some(medium) = arrival.medium {
            // No section panics while it holds the lock.
            self.announced
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(medium.clone());
            frames.push(medium);
        }
        Ok(frames)
    }

    /// Keeps one read, and returns where it starts in its stream and its
    /// part within the stream's first `JOB_VIEW_BYTES`.
    async fn write(&mut self, stream: OutputStream, bytes: &Bytes) -> io::Result<(u64, Bytes)> {
        self.kept.write(stream, bytes).await?;
        let written = match stream {
            OutputStream::Stdout => self.stdout.write(bytes),
            OutputStream::Stderr => self.stderr.write(bytes),
        };
        self.lengths.set(self.lengths());
        Ok(written)
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
        output: &mpsc::UnboundedSender<wire::Frame>,
    ) -> io::Result<()> {
        self.stdout.send_beyond(job, followed, due, output)?;
        self.stderr.send_beyond(job, followed, due, output)
    }

    /// Sends each stream's last message beyond its first `JOB_VIEW_BYTES`
    /// that waits, before the job's exit goes.
    fn send_last(
        &mut self,
        job: &str,
        followed: bool,
        output: &mpsc::UnboundedSender<wire::Frame>,
    ) -> io::Result<()> {
        for log in [&mut self.stdout, &mut self.stderr] {
            if log.due(followed).is_none() {
                continue;
            }
            let message = log.beyond(job, followed).map_err(io::Error::other)?;
            // A disconnected backend follows nothing; the job's cancellation
            // ends it.
            if output.send(message).is_ok() {
                log.reported(followed);
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
