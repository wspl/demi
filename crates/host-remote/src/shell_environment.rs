//! `RemoteShellEnvironment`: the shell behind the `shell_*` tools on a Host
//! reached through its runner (`runner.md` § Shell jobs). Every exec is one
//! job; the record holds the model's view of its output, the head while it
//! runs and the tail at its end, and the pages' view, which also holds the
//! newest output the runner sends while a page watches and the job is
//! therefore followed (`runtime.md` § Live output). The whole output stays in
//! the files the runner wrote. The directory a script ends in carries into
//! the shell's next exec; nothing else of the shell's state does.

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap},
    rc::Rc,
    time::Duration,
};

use bytes::Bytes;
use demi_command_service::protocol::{CommandContext, EditKind as JobEditKind};
use demi_core::{CommandId, EditKind, EditedFile, KeptEdit, ShellId, StreamKind};
use demi_runner_protocol::{
    manifest::ManifestError,
    wire::{self, JobFileChange},
};
use demi_shell::{
    CommandRecord, CommandSet, CommandStatus, DEFAULT_BINARY_LIMIT_BYTES,
    DEFAULT_OUTPUT_LIMIT_BYTES, EditedFiles, Ending, ExecRequest, Host, HostError, HostKey,
    JobCaller, PageFeed, PageView, ProcessEnd, ShellEnvironment, ShellError, ShellTarget,
    SpawnErrorKind, final_stdout_boundary,
};
use futures_util::future::LocalBoxFuture;
use tokio::sync::watch;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::{
    CommandCatalog, JobEnd, JobOutput, JobStart, RemoteHost, RemoteJob, manifest::CommandSelection,
};

/// How long an abort waits for the runner to report the job's end after
/// each signal.
const ABORT_GRACE: Duration = Duration::from_secs(5);

/// The product's host access around one job: it holds what the Host needs,
/// such as the conversation's file gate and a Cloud's admission, through the
/// job's start, output and edit publication, and refuses when the
/// conversation's Host is no longer `host`.
pub trait HostAccess {
    fn run_job<'a>(
        &'a self,
        host: &'a HostKey,
        cancel: CancellationToken,
        job: LocalBoxFuture<'a, ()>,
    ) -> LocalBoxFuture<'a, Result<(), HostError>>;
}

/// Publishes a job's edits (`edit-tracking.md`) before its command reads as
/// ended.
pub trait RetainEdits {
    fn retain<'a>(
        &'a self,
        command: &'a CommandId,
        files: &'a [JobFileChange],
    ) -> LocalBoxFuture<'a, Result<Vec<EditedFile>, String>>;
}

/// Builds each job's command context (`native-runtime.md` § Command
/// context).
pub type ContextSource = Rc<dyn Fn() -> LocalBoxFuture<'static, Result<CommandContext, HostError>>>;

/// What an environment runs with.
pub struct EnvironmentOptions {
    pub host: RemoteHost,
    /// The commands its jobs may run; none for jobs without declared
    /// commands.
    pub commands: Option<CommandSelection>,
    pub context: ContextSource,
    /// Where the pages' views of its commands go, and whether a page
    /// watches, which its jobs follow.
    pub feed: Rc<dyn PageFeed>,
    pub access: Option<Rc<dyn HostAccess>>,
    pub retain: Option<Rc<dyn RetainEdits>>,
    /// The variables every shell starts with, above the device's own.
    pub initial_env: BTreeMap<String, String>,
    /// The budget of one status view's new output, per stream.
    pub output_limit: usize,
    /// The most of a binary final stdout a status carries.
    pub binary_limit: usize,
}

impl EnvironmentOptions {
    pub fn new(host: RemoteHost, context: ContextSource, feed: Rc<dyn PageFeed>) -> Self {
        Self {
            host,
            commands: None,
            context,
            feed,
            access: None,
            retain: None,
            initial_env: BTreeMap::new(),
            output_limit: DEFAULT_OUTPUT_LIMIT_BYTES,
            binary_limit: DEFAULT_BINARY_LIMIT_BYTES,
        }
    }
}

/// Makes each node's environments against the catalog fixed at startup.
pub struct RemoteShellEnvironmentFactory {
    catalog: CommandCatalog,
}

impl RemoteShellEnvironmentFactory {
    pub fn new(catalog: CommandCatalog) -> Self {
        Self { catalog }
    }

    /// An environment whose jobs run `commands` on `options.host`.
    pub fn create(
        &self,
        commands: &CommandSet,
        mut options: EnvironmentOptions,
    ) -> Result<RemoteShellEnvironment, ManifestError> {
        options.commands = Some(self.catalog.select(commands)?);
        Ok(RemoteShellEnvironment::new(options))
    }
}

/// A node's shells on a Host behind a runner.
#[derive(Clone)]
pub struct RemoteShellEnvironment(Rc<Environment>);

struct Environment {
    options: EnvironmentOptions,
    state: RefCell<State>,
    /// The jobs' tasks.
    tasks: TaskTracker,
}

#[derive(Default)]
struct State {
    shells: HashMap<ShellId, Shell>,
    default_shell: Option<ShellId>,
    records: HashMap<CommandId, Rc<RefCell<CommandRecord>>>,
    running: HashMap<CommandId, Rc<Running>>,
}

struct Shell {
    cwd: String,
    env: BTreeMap<String, String>,
    foreground: Option<CommandId>,
}

/// A command that has not ended.
struct Running {
    /// Stops it: an abort, or the caller's cancellation.
    stop: CancellationToken,
    /// Its job, once started.
    job: RefCell<Option<RemoteJob>>,
    /// Set by an abort: the end that follows is the stop, not the
    /// command's own.
    aborted: Cell<bool>,
    settled: watch::Sender<bool>,
}

impl Running {
    async fn settled(&self) {
        let mut settled = self.settled.subscribe();
        // The sender lives as long as this `Running`.
        let _ = settled.wait_for(|settled| *settled).await;
    }

    /// Waits for the command to settle, at most `within`; true when it did.
    async fn settled_within(&self, within: Duration) -> bool {
        tokio::time::timeout(within, self.settled()).await.is_ok()
    }
}

impl RemoteShellEnvironment {
    pub fn new(options: EnvironmentOptions) -> Self {
        Self(Rc::new(Environment {
            options,
            state: RefCell::default(),
            tasks: TaskTracker::new(),
        }))
    }

    /// Picks the exec's shell, reserves it, and starts the command. The
    /// reservation happens before anything awaits, so two execs never share
    /// a shell.
    fn start(
        &self,
        request: ExecRequest,
        cancel: CancellationToken,
    ) -> Result<(CommandId, Rc<Running>), ShellError> {
        let mut state = self.0.state.borrow_mut();
        let shell_id = match request.shell {
            ShellTarget::Existing(id) => {
                if !state.shells.contains_key(&id) {
                    return Err(ShellError::UnknownShell(id));
                }
                id
            }
            ShellTarget::Ephemeral { cwd } => self.create_shell(&mut state, cwd),
            ShellTarget::Default => match state.default_shell.clone() {
                Some(id) if state.shells[&id].foreground.is_some() => {
                    self.create_shell(&mut state, None)
                }
                Some(id) => id,
                None => {
                    let id = self.create_shell(&mut state, None);
                    state.default_shell = Some(id.clone());
                    id
                }
            },
        };
        if let Some(command) = &state.shells[&shell_id].foreground {
            return Err(ShellError::ShellBusy {
                shell: shell_id,
                command: command.clone(),
            });
        }
        let command = new_id::<CommandId>();
        let record = Rc::new(RefCell::new(CommandRecord::new(
            shell_id.clone(),
            command.clone(),
            request.tool_use_id,
        )));
        let running = Rc::new(Running {
            stop: cancel.child_token(),
            job: RefCell::new(None),
            aborted: Cell::new(false),
            settled: watch::Sender::new(false),
        });
        state
            .shells
            .get_mut(&shell_id)
            .expect("the shell was just found")
            .foreground = Some(command.clone());
        state.records.insert(command.clone(), record.clone());
        state.running.insert(command.clone(), running.clone());
        drop(state);
        // The pages learn of the command before its first output.
        self.report(&record);
        let environment = self.clone();
        let task_running = running.clone();
        let task_command = command.clone();
        self.0.tasks.spawn_local(async move {
            environment
                .run(
                    shell_id,
                    task_command,
                    request.script,
                    request.caller,
                    task_running,
                    record,
                )
                .await;
        });
        Ok((command, running))
    }

    fn create_shell(&self, state: &mut State, cwd: Option<String>) -> ShellId {
        let id = new_id::<ShellId>();
        state.shells.insert(
            id.clone(),
            Shell {
                cwd: cwd.unwrap_or_else(|| self.0.options.host.default_cwd().to_owned()),
                env: self.0.options.initial_env.clone(),
                foreground: None,
            },
        );
        id
    }

    /// Tells the pages that `record`'s view changed.
    fn report(&self, record: &Rc<RefCell<CommandRecord>>) {
        self.0.options.feed.changed(record);
    }

    /// Runs one command to its end, inside the product's host access when
    /// there is one.
    async fn run(
        &self,
        shell: ShellId,
        command: CommandId,
        script: String,
        caller: JobCaller,
        running: Rc<Running>,
        record: Rc<RefCell<CommandRecord>>,
    ) {
        let failure: RefCell<Option<String>> = RefCell::new(None);
        let job = async {
            if let Err(error) = self
                .execute(&shell, &command, script, caller, &running, &record)
                .await
            {
                *failure.borrow_mut() = Some(error);
            }
        };
        let access = match &self.0.options.access {
            Some(access) => {
                let key = self.0.options.host.key();
                access
                    .run_job(&key, running.stop.clone(), Box::pin(job))
                    .await
                    .map_err(|error| error.message)
            }
            None => {
                job.await;
                Ok(())
            }
        };
        let failure = access.err().or_else(|| failure.into_inner());
        if let Some(message) = failure {
            let ended = if running.stop.is_cancelled() {
                record.borrow_mut().mark_aborted()
            } else if record.borrow().is_running() {
                let mut record = record.borrow_mut();
                let stdout = record.text(StreamKind::Stdout).to_owned();
                let reason = format!("{message}\n");
                let stderr = format!("{}{reason}", record.text(StreamKind::Stderr));
                record.settle(Ending::Exited(127), stdout, stderr, None, &reason)
            } else {
                false
            };
            if ended {
                self.report(&record);
            }
        }
        let mut state = self.0.state.borrow_mut();
        state.running.remove(&command);
        if let Some(shell) = state.shells.get_mut(&shell)
            && shell.foreground.as_ref() == Some(&command)
        {
            shell.foreground = None;
        }
        drop(state);
        running.settled.send_replace(true);
    }

    /// Starts the job and follows it to its end; an error is why it never
    /// ran.
    async fn execute(
        &self,
        shell: &ShellId,
        command: &CommandId,
        script: String,
        caller: JobCaller,
        running: &Running,
        record: &Rc<RefCell<CommandRecord>>,
    ) -> Result<(), String> {
        let context = (self.0.options.context)()
            .await
            .map_err(|error| error.message)?;
        if running.stop.is_cancelled() {
            return Err("Shell command aborted".into());
        }
        let (cwd, env) = {
            let state = self.0.state.borrow();
            let shell = &state.shells[shell];
            let mut env = shell.env.clone();
            env.insert("PWD".into(), shell.cwd.clone());
            (shell.cwd.clone(), env)
        };
        let job = self
            .0
            .options
            .host
            .start_job(JobStart {
                script,
                cwd,
                env,
                context,
                caller: Some(caller),
                commands: self.0.options.commands.clone(),
                stdin: None,
                stdout: None,
            })
            .await
            .map_err(|error| error.message)?;
        *running.job.borrow_mut() = Some(job.clone());
        let mut streams = [Received::default(), Received::default()];
        // The job is followed while a page watches; it starts unfollowed.
        let mut watching = self.0.options.feed.watching();
        let mut watch_open = true;
        let mut followed = false;
        let stopped = running.stop.cancelled();
        tokio::pin!(stopped);
        let mut signalled = false;
        loop {
            let follow = watch_open && *watching.borrow_and_update();
            if follow != followed {
                followed = follow;
                if let Err(error) = job.follow(followed).await {
                    // The job's end or the connection's loss says what
                    // happened; a following that could not change is
                    // diagnostic.
                    tracing::debug!(%command, "could not change the job's following: {error}");
                }
            }
            tokio::select! {
                chunk = job.next_output() => {
                    let Some(chunk) = chunk else {
                        break;
                    };
                    let stream = &mut streams[stream_index(chunk.stream)];
                    if stream.receive(record, chunk) {
                        self.report(record);
                    }
                }
                changed = watching.changed(), if watch_open => {
                    // A feed that is gone has no page.
                    watch_open = changed.is_ok();
                }
                () = &mut stopped, if !signalled => {
                    signalled = true;
                    if let Err(error) = job.kill(wire::Signal::Terminate).await {
                        // The job's end or the connection's loss says what
                        // happened; a signal that could not go is diagnostic.
                        tracing::debug!(%command, "could not interrupt the job: {error}");
                    }
                }
            }
        }
        let end = job.end().await;
        self.finish(shell, command, running, record, end, streams)
            .await;
        Ok(())
    }

    /// Settles the record from the job's end: its edits, its directory, its
    /// streams as the model sees them, and what the end adds to the pages'
    /// view.
    async fn finish(
        &self,
        shell: &ShellId,
        command: &CommandId,
        running: &Running,
        record: &Rc<RefCell<CommandRecord>>,
        end: JobEnd,
        streams: [Received; 2],
    ) {
        if !end.files.is_empty() {
            let unavailable = || end.files.iter().map(unkept).collect::<Vec<_>>();
            let files = match &self.0.options.retain {
                Some(retain) => match retain.retain(command, &end.files).await {
                    Ok(files) => files,
                    Err(error) => {
                        // History publication is best effort; it must not
                        // replace the exit status.
                        tracing::warn!(%command, "could not retain the command's edits: {error}");
                        unavailable()
                    }
                },
                None => unavailable(),
            };
            record.borrow_mut().set_files(EditedFiles {
                files,
                truncated: end.files_truncated,
            });
        }
        if let Some(cwd) = end.cwd
            && let Some(shell) = self.0.state.borrow_mut().shells.get_mut(shell)
        {
            shell.cwd = cwd;
        }
        let [stdout_received, stderr_received] = streams;
        let exit_code = match &end.status {
            ProcessEnd::NotStarted(error) => {
                // A job the runner could not run names why; any other kind is
                // bash's own.
                let reason = match (&error.kind, &error.detail) {
                    (SpawnErrorKind::Other, Some(detail)) => detail.clone(),
                    _ => format!("bash: {}", error.kind),
                };
                return self.never_ran(
                    record,
                    &stdout_received.head,
                    &stderr_received.head,
                    &reason,
                );
            }
            ProcessEnd::Lost(reason) => {
                return self.never_ran(
                    record,
                    &stdout_received.head,
                    &stderr_received.head,
                    reason,
                );
            }
            ProcessEnd::Exited(code) => *code,
            ProcessEnd::Signalled(signal) if matches!(signal.as_str(), "SIGTERM" | "SIGKILL") => {
                130
            }
            ProcessEnd::Signalled(_) => 128,
        };
        let output = end.output;
        let stdout = stream_text(
            &stdout_received.head,
            output.as_ref().map(|output| {
                (
                    output.stdout_bytes,
                    &output.stdout_tail.0,
                    output.stdout_path.as_str(),
                )
            }),
        );
        let stderr = stream_text(
            &stderr_received.head,
            output.as_ref().map(|output| {
                (
                    output.stderr_bytes,
                    &output.stderr_tail.0,
                    output.stderr_path.as_str(),
                )
            }),
        );
        // A binary final stream is on the target whole: read back within the
        // binary limit, so the tools can look at it.
        let mut stdout_text = stdout.text;
        let mut binary = None;
        if let Some(output) = &output
            && stdout.binary
            && output.stdout_bytes <= self.0.options.binary_limit as u64
            && let Ok(bytes) = self
                .0
                .options
                .host
                .fs()
                .read_file(&output.stdout_path)
                .await
        {
            let (text, bytes) = final_stdout_boundary(
                bytes,
                self.0.options.binary_limit,
                Some(&output.stdout_path),
            );
            stdout_text = text;
            binary = bytes;
        }
        let ending = if running.aborted.get() {
            Ending::Aborted
        } else {
            Ending::Exited(exit_code)
        };
        // The pages' view gets the end of each stream it had not received,
        // or a binary stdout's description.
        let retained = |stream: StreamKind| {
            output.as_ref().map(|output| match stream {
                StreamKind::Stdout => (output.stdout_bytes, output.stdout_tail.0.as_slice()),
                StreamKind::Stderr => (output.stderr_bytes, output.stderr_tail.0.as_slice()),
            })
        };
        let mut page_end = match &binary {
            Some(_) => stdout_text.clone(),
            None => stdout_received.rest(StreamKind::Stdout, retained(StreamKind::Stdout)),
        };
        page_end.push_str(&stderr_received.rest(StreamKind::Stderr, retained(StreamKind::Stderr)));
        let ended = {
            let mut record = record.borrow_mut();
            if let Some(output) = &output {
                // The runner names where its tee wrote: the output files are
                // the target's.
                if let Some((directory, _)) = output.stdout_path.rsplit_once('/') {
                    record.set_output_dir(directory.to_owned());
                }
            }
            let ended = record.settle(ending, stdout_text, stderr.text, binary, &page_end);
            if let Some(output) = &output {
                record.set_host_bytes(output.stdout_bytes, output.stderr_bytes);
            }
            ended
        };
        if ended {
            self.report(record);
        }
    }

    /// A job that never ran its script: exit 127, and why, on stderr.
    fn never_ran(
        &self,
        record: &Rc<RefCell<CommandRecord>>,
        stdout: &[u8],
        stderr: &[u8],
        reason: &str,
    ) {
        let stdout = String::from_utf8_lossy(stdout).into_owned();
        let reason = format!("{reason}\n");
        let stderr = format!("{}{reason}", String::from_utf8_lossy(stderr));
        let ended = record
            .borrow_mut()
            .settle(Ending::Exited(127), stdout, stderr, None, &reason);
        if ended {
            self.report(record);
        }
    }

    fn record(&self, command: &CommandId) -> Result<Rc<RefCell<CommandRecord>>, ShellError> {
        self.0
            .state
            .borrow()
            .records
            .get(command)
            .cloned()
            .ok_or_else(|| ShellError::UnknownCommand(command.clone()))
    }

    /// The model's status of `command`.
    fn view(&self, command: &CommandId) -> Result<CommandStatus, ShellError> {
        let record = self.record(command)?;
        let hint = self
            .0
            .state
            .borrow()
            .running
            .get(command)
            .and_then(|running| {
                running
                    .job
                    .borrow()
                    .as_ref()
                    .and_then(RemoteJob::running_hint)
            });
        let mut record = record.borrow_mut();
        let hint = if record.is_running() { hint } else { None };
        Ok(record.status(self.0.options.output_limit, hint))
    }

    /// Stops a running command: asks it to end, and ends it when it does not.
    async fn stop_command(&self, command: &CommandId) -> Result<(), ShellError> {
        let record = self.record(command)?;
        let running = self.0.state.borrow().running.get(command).cloned();
        let Some(running) = running.filter(|_| record.borrow().is_running()) else {
            return Ok(());
        };
        running.aborted.set(true);
        // The job's task asks the job to end.
        running.stop.cancel();
        if !running.settled_within(ABORT_GRACE).await {
            let job = running.job.borrow().clone();
            if let Some(job) = job
                && let Err(error) = job.kill(wire::Signal::Kill).await
            {
                tracing::debug!(%command, "could not kill the job: {error}");
            }
            if !running.settled_within(ABORT_GRACE).await {
                let ended = record.borrow_mut().mark_aborted();
                if ended {
                    self.report(&record);
                }
            }
        }
        Ok(())
    }

    async fn dispose(&self, shell: &ShellId) -> bool {
        let foreground = match self.0.state.borrow().shells.get(shell) {
            None => return false,
            Some(shell) => shell.foreground.clone(),
        };
        if let Some(command) = foreground
            && let Err(error) = self.stop_command(&command).await
        {
            tracing::debug!(%command, "could not abort the shell's command: {error}");
        }
        let mut state = self.0.state.borrow_mut();
        state.shells.remove(shell);
        if state.default_shell.as_ref() == Some(shell) {
            state.default_shell = None;
        }
        true
    }
}

impl ShellEnvironment for RemoteShellEnvironment {
    fn exec(
        &self,
        request: ExecRequest,
        cancel: CancellationToken,
    ) -> LocalBoxFuture<'_, Result<CommandStatus, ShellError>> {
        Box::pin(async move {
            let window = request.window.duration();
            let (command, running) = self.start(request, cancel)?;
            running.settled_within(window).await;
            self.view(&command)
        })
    }

    fn status(&self, command: &CommandId) -> Result<CommandStatus, ShellError> {
        self.view(command)
    }

    fn write<'a>(
        &'a self,
        command: &'a CommandId,
        stdin: Bytes,
    ) -> LocalBoxFuture<'a, Result<(), ShellError>> {
        Box::pin(async move {
            let record = self.record(command)?;
            let running = self.0.state.borrow().running.get(command).cloned();
            let Some(running) = running.filter(|_| record.borrow().is_running()) else {
                return Err(ShellError::NotRunning(command.clone()));
            };
            if stdin.is_empty() {
                return Err(ShellError::EmptyStdin);
            }
            let job = running
                .job
                .borrow()
                .clone()
                .ok_or_else(|| ShellError::Starting(command.clone()))?;
            job.write_stdin(stdin).await?;
            Ok(())
        })
    }

    fn abort<'a>(&'a self, command: &'a CommandId) -> LocalBoxFuture<'a, Result<(), ShellError>> {
        Box::pin(self.stop_command(command))
    }

    fn page_views(&self) -> Vec<PageView> {
        self.0
            .state
            .borrow()
            .records
            .values()
            .map(|record| record.borrow().page_view())
            .collect()
    }

    fn release_command<'a>(&'a self, command: &'a CommandId) -> LocalBoxFuture<'a, bool> {
        Box::pin(async move {
            let Ok(record) = self.record(command) else {
                return false;
            };
            let running = record.borrow().is_running();
            if running && let Err(error) = self.stop_command(command).await {
                tracing::debug!(%command, "could not abort the released command: {error}");
            }
            // The output files are the runner's; they stay on the target
            // with the rest of the command's history.
            self.0.state.borrow_mut().records.remove(command);
            true
        })
    }

    fn dispose_shell<'a>(&'a self, shell: &'a ShellId) -> LocalBoxFuture<'a, bool> {
        Box::pin(self.dispose(shell))
    }

    fn dispose_all(&self) -> LocalBoxFuture<'_, ()> {
        Box::pin(async move {
            let shells: Vec<_> = self.0.state.borrow().shells.keys().cloned().collect();
            for shell in shells {
                self.dispose(&shell).await;
            }
            let running: Vec<_> = self.0.state.borrow().running.values().cloned().collect();
            for running in running {
                running.settled().await;
            }
        })
    }

    fn owns_shell(&self, shell: &ShellId) -> bool {
        self.0.state.borrow().shells.contains_key(shell)
    }

    fn owns_command(&self, command: &CommandId) -> bool {
        self.0.state.borrow().records.contains_key(command)
    }
}

fn new_id<T: TryFrom<String>>() -> T
where
    T::Error: std::fmt::Debug,
{
    T::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is a nonempty identity")
}

fn stream_index(stream: StreamKind) -> usize {
    match stream {
        StreamKind::Stdout => 0,
        StreamKind::Stderr => 1,
    }
}

/// A changed file whose contents were not kept.
fn unkept(file: &JobFileChange) -> EditedFile {
    edited_file(file, |_| false)
}

/// The record a view lists for a file a job changed: its line counts and,
/// for each edit segment, whether `kept` says its contents were kept.
pub fn edited_file(file: &JobFileChange, mut kept: impl FnMut(usize) -> bool) -> EditedFile {
    EditedFile {
        path: file.path.clone(),
        kind: match file.kind {
            JobEditKind::Added => EditKind::Added,
            JobEditKind::Modified => EditKind::Modified,
        },
        added: u32::try_from(file.added).unwrap_or(u32::MAX),
        removed: u32::try_from(file.removed).unwrap_or(u32::MAX),
        edits: (0..file.edits.len())
            .map(|segment| KeptEdit {
                kept: kept(segment),
            })
            .collect(),
    }
}

/// One stream's text as the model sees it.
struct StreamText {
    text: String,
    binary: bool,
}

/// The head that streamed, and, when the stream outgrew the view, a gap note
/// and the tail the runner read from the output file at the end. `retained`
/// is the stream's length on the target, its tail and its file.
fn stream_text(head: &[u8], retained: Option<(u64, &Vec<u8>, &str)>) -> StreamText {
    let whole = |bytes: &[u8]| StreamText {
        text: String::from_utf8_lossy(bytes).into_owned(),
        binary: std::str::from_utf8(bytes).is_err(),
    };
    let Some((total, tail, path)) = retained else {
        return whole(head);
    };
    let total = usize::try_from(total).unwrap_or(usize::MAX);
    if total <= head.len() {
        return whole(head);
    }
    if head.len() + tail.len() >= total {
        let overlap = head.len() + tail.len() - total;
        return whole(&[head, &tail[overlap..]].concat());
    }
    let hidden = total - head.len() - tail.len();
    let shown = [head, tail.as_slice()].concat();
    StreamText {
        text: format!(
            "{}\n[... {hidden} bytes not shown; the full stream is at {path} ...]\n{}",
            String::from_utf8_lossy(head),
            String::from_utf8_lossy(tail)
        ),
        binary: std::str::from_utf8(&shown).is_err(),
    }
}

/// What the backend received of one stream of a job (`runner.md` § Pipes
/// and output).
#[derive(Default)]
struct Received {
    /// The stream's first bytes, the model's view of it.
    head: Vec<u8>,
    /// Where the stream's next bytes start.
    next: u64,
    text: Utf8Stream,
}

impl Received {
    /// Adds one message of the job's output to `record`: a stream's first
    /// bytes to both views, its newest bytes beyond them to the pages' view,
    /// and a message without bytes as growth alone. True when the pages'
    /// view changed.
    fn receive(&mut self, record: &RefCell<CommandRecord>, chunk: JobOutput) -> bool {
        if chunk.offset < wire::JOB_VIEW_BYTES as u64 {
            self.head.extend_from_slice(&chunk.bytes);
            self.next = chunk.offset + chunk.bytes.len() as u64;
            let text = self.text.decode(&chunk.bytes);
            return record.borrow_mut().append_output(chunk.stream, &text);
        }
        if chunk.bytes.is_empty() {
            record.borrow_mut().grew();
            return false;
        }
        let mut text = String::new();
        // The runner left out what lies between the previous bytes and these.
        let left_out = chunk.offset.saturating_sub(self.next);
        if left_out > 0 {
            text.push_str(&self.text.finish());
            text.push_str(&left_out_note(chunk.stream, left_out));
        }
        text.push_str(&self.text.decode(&chunk.bytes));
        self.next = chunk.offset + chunk.bytes.len() as u64;
        record.borrow_mut().append_page_output(&text)
    }

    /// What the job's end adds to the pages' view of the stream: its bytes
    /// past those received, from `retained`, the stream's length and last
    /// bytes, and the rest of a character the last bytes split.
    fn rest(mut self, stream: StreamKind, retained: Option<(u64, &[u8])>) -> String {
        let mut text = String::new();
        if let Some((length, tail)) = retained
            && length > self.next
        {
            let missing = length - self.next;
            let kept = tail.len() as u64;
            if missing <= kept {
                let from = usize::try_from(kept - missing).expect("within the tail");
                text.push_str(&self.text.decode(&tail[from..]));
            } else {
                text.push_str(&self.text.finish());
                text.push_str(&left_out_note(stream, missing - kept));
                text.push_str(&self.text.decode(tail));
            }
        }
        text.push_str(&self.text.finish());
        text
    }
}

/// The pages' note where the runner left out `bytes` of a stream.
fn left_out_note(stream: StreamKind, bytes: u64) -> String {
    format!("\n[... {bytes} bytes of {stream} not shown ...]\n")
}

/// Decodes one stream's bytes as they arrive: a character split across
/// chunks waits for its rest, and bytes that are not UTF-8 read as U+FFFD,
/// as a lossy decode of the whole stream would. The standard library decodes
/// only whole buffers.
#[derive(Default)]
struct Utf8Stream {
    pending: Vec<u8>,
}

impl Utf8Stream {
    fn decode(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut text = String::new();
        let mut rest = self.pending.as_slice();
        loop {
            match std::str::from_utf8(rest) {
                Ok(valid) => {
                    text.push_str(valid);
                    rest = &[];
                    break;
                }
                Err(error) => {
                    let (valid, after) = rest.split_at(error.valid_up_to());
                    text.push_str(&String::from_utf8_lossy(valid));
                    match error.error_len() {
                        Some(length) => {
                            text.push(char::REPLACEMENT_CHARACTER);
                            rest = &after[length..];
                        }
                        // An incomplete character at the end waits for the
                        // next chunk.
                        None => {
                            rest = after;
                            break;
                        }
                    }
                }
            }
        }
        self.pending = rest.to_vec();
        text
    }

    /// The end of the bytes: a character they split reads as U+FFFD.
    fn finish(&mut self) -> String {
        let rest = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        rest
    }
}

#[cfg(test)]
mod tests {
    use super::{Utf8Stream, stream_text};

    #[test]
    fn a_character_split_across_chunks_decodes_once_whole() {
        let mut stream = Utf8Stream::default();
        let bytes = "aé€😀".as_bytes();
        let decoded: String = bytes.iter().map(|byte| stream.decode(&[*byte])).collect();
        assert_eq!(decoded, "aé€😀");
        assert_eq!(stream.decode(b"\xff!"), "\u{fffd}!");
    }

    #[test]
    fn a_stream_beyond_the_view_is_its_head_a_gap_note_and_its_tail() {
        let head = b"0123".as_slice();
        let tail = b"6789".to_vec();
        let gap = stream_text(head, Some((10, &tail, "/out/stdout.txt")));
        assert_eq!(
            gap.text,
            "0123\n[... 2 bytes not shown; the full stream is at /out/stdout.txt ...]\n6789"
        );
        let overlap = stream_text(head, Some((6, &tail, "/out/stdout.txt")));
        assert_eq!(overlap.text, "012389");
        assert!(!overlap.binary);
        assert_eq!(stream_text(b"\xffx", None).text, "\u{fffd}x");
        assert!(stream_text(b"\xffx", None).binary);
    }
}
