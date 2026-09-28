//! `RemoteShellEnvironment`: the shell behind the `shell_*` tools on a Host
//! reached through its runner (`runner.md` § Shell jobs). Every exec is one
//! job; the record holds the model's view of its output, the head while it
//! runs and the whole output once it ended, and the pages' view, which also
//! holds the newest output the runner sends while a page watches and the job
//! is therefore followed (`runtime.md` § Live output). At the job's end the
//! product's keeper stores the whole output, and the runner lets the job's
//! directory go (`runtime.md` § The whole output). The directory a script
//! ends in carries into the shell's next exec; nothing else of the shell's
//! state does.

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use bytes::Bytes;
use demi_command_service::protocol::{CommandContext, EditKind as JobEditKind};
use demi_core::{CommandId, EditCopies, EditKind, EditSegment, EditedFile, Sequence, ShellId, StreamKind};
use demi_runner_protocol::{
    manifest::ManifestError,
    wire::{self, JobFileChange},
};
use demi_shell::{
    BinaryOutput, CommandRecord, CommandSet, CommandStatus, DEFAULT_BINARY_LIMIT_BYTES,
    DEFAULT_OUTPUT_LIMIT_BYTES, EditedFiles, Ending, ExecRequest, Host, HostError, HostKey,
    JobCaller, Missing, Numbers, OutputRecord, PageFeed, PageView, ProcessEnd, Seen, ShellEnvironment,
    ShellError, ShellTarget, SpawnErrorKind, Streams, WholeOutput, binary_line,
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

/// Keeps what a command leaves when it ends, before its command reads as
/// ended: the copies of its edits (`edit-tracking.md` § Edit copies), and
/// its whole output (`runtime.md` § The whole output). A failure to keep the
/// output is the keeper's to record.
pub trait CommandKeeper {
    /// The list the command's view shows of `files`: every file, each
    /// segment with its copies when they were stored.
    fn retain<'a>(
        &'a self,
        command: &'a CommandId,
        files: &'a [JobFileChange],
    ) -> LocalBoxFuture<'a, Vec<EditedFile>>;

    fn keep_output<'a>(
        &'a self,
        command: &'a CommandId,
        output: &'a WholeOutput,
    ) -> LocalBoxFuture<'a, ()>;
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
    /// Where its commands' and shells' numbers come from: the
    /// conversation's sequences.
    pub numbers: Rc<dyn Numbers>,
    pub access: Option<Rc<dyn HostAccess>>,
    pub keeper: Option<Rc<dyn CommandKeeper>>,
    /// The variables every shell starts with, above the device's own.
    pub initial_env: BTreeMap<String, String>,
    /// The budget of one status view's new output, per stream.
    pub output_limit: usize,
    /// The most of a binary final stdout a status carries.
    pub binary_limit: usize,
}

impl EnvironmentOptions {
    pub fn new(host: RemoteHost, context: ContextSource, feed: Rc<dyn PageFeed>, numbers: Rc<dyn Numbers>) -> Self {
        Self {
            host,
            commands: None,
            context,
            feed,
            numbers,
            access: None,
            keeper: None,
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
    /// A shell number taken for a shell not made yet.
    spare_shell: Option<ShellId>,
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
    /// The output the backend received within the streams' views, in the
    /// order it came: the whole output when neither stream went beyond.
    received: RefCell<Vec<OutputRecord>>,
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

    /// Starts the command: it takes the conversation's next command number,
    /// then picks the exec's shell and reserves it in one step that nothing
    /// awaits in, so two execs never share a shell. A new shell takes the
    /// conversation's next shell number, fetched first when none is spare.
    /// An exec refused before its number is taken takes none.
    async fn start(
        &self,
        request: ExecRequest,
        cancel: CancellationToken,
    ) -> Result<(CommandId, Rc<Running>), ShellError> {
        if let ShellTarget::Existing(id) = &request.shell {
            check_free(&self.0.state.borrow(), id)?;
        }
        let command = numbered::<CommandId>(self.0.options.numbers.next(Sequence::Command).await?);
        let shell_id = loop {
            if let Some(shell) = self.reserve(&request.shell, &command)? {
                break shell;
            }
            let spare = numbered::<ShellId>(self.0.options.numbers.next(Sequence::Shell).await?);
            self.0.state.borrow_mut().spare_shell = Some(spare);
        };
        let record = Rc::new(RefCell::new(CommandRecord::new(
            shell_id.clone(),
            command.clone(),
            request.tool_use_id,
        )));
        let running = Rc::new(Running {
            stop: cancel.child_token(),
            job: RefCell::new(None),
            received: RefCell::new(Vec::new()),
            aborted: Cell::new(false),
            settled: watch::Sender::new(false),
        });
        {
            let mut state = self.0.state.borrow_mut();
            state.records.insert(command.clone(), record.clone());
            state.running.insert(command.clone(), running.clone());
        }
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

    /// Picks `target`'s shell and makes `command` its foreground; none when
    /// the pick needs a new shell and no shell number is spare. The shell
    /// it picks runs nothing: an existing one is checked again, since the
    /// state may have changed while the command's number was fetched.
    fn reserve(&self, target: &ShellTarget, command: &CommandId) -> Result<Option<ShellId>, ShellError> {
        let mut state = self.0.state.borrow_mut();
        let shell_id = match target {
            ShellTarget::Existing(id) => {
                check_free(&state, id)?;
                id.clone()
            }
            ShellTarget::Ephemeral { cwd } => match self.create_shell(&mut state, cwd.clone()) {
                Some(id) => id,
                None => return Ok(None),
            },
            ShellTarget::Default => match state.default_shell.clone() {
                Some(id) if state.shells[&id].foreground.is_none() => id,
                busy => {
                    let Some(id) = self.create_shell(&mut state, None) else {
                        return Ok(None);
                    };
                    if busy.is_none() {
                        state.default_shell = Some(id.clone());
                    }
                    id
                }
            },
        };
        state
            .shells
            .get_mut(&shell_id)
            .expect("the shell was just found")
            .foreground = Some(command.clone());
        Ok(Some(shell_id))
    }

    /// Makes a shell with the spare shell number; none when none is spare.
    fn create_shell(&self, state: &mut State, cwd: Option<String>) -> Option<ShellId> {
        let id = state.spare_shell.take()?;
        state.shells.insert(
            id.clone(),
            Shell {
                cwd: cwd.unwrap_or_else(|| self.0.options.host.default_cwd().to_owned()),
                env: self.0.options.initial_env.clone(),
                foreground: None,
            },
        );
        Some(id)
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
        if let Some(reason) = failure
            && record.borrow().is_running()
        {
            let mut output = running.received.take();
            let (ending, page) = if running.stop.is_cancelled() {
                (Ending::Aborted, String::new())
            } else {
                (Ending::Exited(127), push_reason(&mut output, &reason))
            };
            let output = WholeOutput::new(output, None);
            self.end(&command, &record, ending, output, None, &page, None)
                .await;
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
                    if stream.receive(record, chunk, &running.received) {
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
        self.finish(shell, command, running, record, &job, end, streams)
            .await;
        Ok(())
    }

    /// Settles the record from the job's end: its edits, its directory, and
    /// its whole output, which the backend received or reads from the Host
    /// now; the runner then lets the job's directory go.
    async fn finish(
        &self,
        shell: &ShellId,
        command: &CommandId,
        running: &Running,
        record: &Rc<RefCell<CommandRecord>>,
        job: &RemoteJob,
        end: JobEnd,
        streams: [Received; 2],
    ) {
        if !end.files.is_empty() {
            let files = match &self.0.options.keeper {
                Some(keeper) => keeper.retain(command, &end.files).await,
                None => end.files.iter().map(unkept).collect(),
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
        let mut received = running.received.take();
        let exit_code = match &end.status {
            // A job the runner could not run names why; any other kind is
            // bash's own.
            ProcessEnd::NotStarted(error) => {
                let reason = match (&error.kind, &error.detail) {
                    (SpawnErrorKind::Other, Some(detail)) => detail.clone(),
                    _ => format!("bash: {}", error.kind),
                };
                let page = push_reason(&mut received, &reason);
                let output = WholeOutput::new(received, None);
                let ending = Ending::Exited(127);
                return self
                    .end(command, record, ending, output, None, &page, Some(job))
                    .await;
            }
            ProcessEnd::Lost(reason) => {
                // What the Host held beyond what the backend received went
                // with the connection.
                let missing = Some(Missing {
                    bytes: streams.iter().map(Received::unreceived).sum(),
                    reason: "lost with the Host's connection".into(),
                })
                .filter(|missing| missing.bytes > 0);
                let mut page = push_reason(&mut received, reason);
                if let Some(missing) = &missing {
                    page.push_str(&missing.line());
                    page.push('\n');
                }
                let output = WholeOutput::new(received, missing);
                let ending = Ending::Exited(127);
                return self
                    .end(command, record, ending, output, None, &page, Some(job))
                    .await;
            }
            ProcessEnd::Exited(code) => *code,
            ProcessEnd::Signalled(signal) if matches!(signal.as_str(), "SIGTERM" | "SIGKILL") => {
                130
            }
            ProcessEnd::Signalled(_) => 128,
        };
        let lengths = end.output.unwrap_or(wire::OutputLengths {
            stdout_bytes: streams[0].received(),
            stderr_bytes: streams[1].received(),
        });
        let unreceived = lengths.stdout_bytes.saturating_sub(streams[0].received())
            + lengths.stderr_bytes.saturating_sub(streams[1].received());
        // What the end adds to the pages' view: the whole output when the
        // view lacks some of it, which shows its end anew; otherwise only a
        // line that stands for a binary stdout.
        let (output, mut page, read) = if unreceived == 0 {
            (WholeOutput::new(received, None), String::new(), false)
        } else {
            match job.read_output().await {
                Ok(output) => (output, String::new(), true),
                Err(error) => {
                    tracing::warn!(%command, "could not read the command's kept output: {error}");
                    let missing = Missing {
                        bytes: unreceived,
                        reason: format!("not read from the Host: {}", error.message),
                    };
                    let page = format!("{}\n", missing.line());
                    (WholeOutput::new(received, Some(missing)), page, false)
                }
            }
        };
        let binary = output.binary_stdout(lengths.stdout_bytes, self.0.options.binary_limit);
        let binary_length = binary.as_ref().map(|binary| binary.info.total_bytes);
        if read {
            page = output
                .text(Streams::Both, binary_length, Seen::default())
                .display();
        } else if let Some(length) = binary_length {
            page.insert_str(0, &format!("{}\n", binary_line(length)));
        }
        let ending = if running.aborted.get() {
            Ending::Aborted
        } else {
            Ending::Exited(exit_code)
        };
        {
            let mut record = record.borrow_mut();
            record.grew(StreamKind::Stdout, lengths.stdout_bytes);
            record.grew(StreamKind::Stderr, lengths.stderr_bytes);
        }
        self.end(command, record, ending, output, binary, &page, Some(job))
            .await;
    }

    /// Ends the command with its whole output: the keeper stores it, the
    /// runner lets the job's directory go, since the backend now has what it
    /// needs of the job, and the record settles, the pages' view with
    /// `page` added.
    async fn end(
        &self,
        command: &CommandId,
        record: &Rc<RefCell<CommandRecord>>,
        ending: Ending,
        output: WholeOutput,
        binary_stdout: Option<BinaryOutput>,
        page: &str,
        job: Option<&RemoteJob>,
    ) {
        if let Some(keeper) = &self.0.options.keeper {
            keeper.keep_output(command, &output).await;
        }
        if let Some(job) = job {
            job.release().await;
        }
        let ended = record
            .borrow_mut()
            .settle(ending, Arc::new(output), binary_stdout, page);
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
            let (command, running) = self.start(request, cancel).await?;
            running.settled_within(window).await;
            self.view(&command)
        })
    }

    fn status(&self, command: &CommandId) -> Result<CommandStatus, ShellError> {
        self.view(command)
    }

    fn read_output<'a>(
        &'a self,
        command: &'a CommandId,
    ) -> LocalBoxFuture<'a, Result<WholeOutput, ShellError>> {
        Box::pin(async move {
            let record = self.record(command)?;
            let running = self.0.state.borrow().running.get(command).cloned();
            let Some(running) = running.filter(|_| record.borrow().is_running()) else {
                return Err(ShellError::NotRunning(command.clone()));
            };
            let job = running
                .job
                .borrow()
                .clone()
                .ok_or_else(|| ShellError::Starting(command.clone()))?;
            Ok(job.read_output().await?)
        })
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
            // A command that ran keeps its output when its job ends, and
            // its Host lets the job's directory go then.
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

/// Refuses `id` when the environment has no such shell or it runs a
/// command.
fn check_free(state: &State, id: &ShellId) -> Result<(), ShellError> {
    let shell = state.shells.get(id).ok_or_else(|| ShellError::UnknownShell(id.clone()))?;
    match &shell.foreground {
        Some(command) => Err(ShellError::ShellBusy {
            shell: id.clone(),
            command: command.clone(),
        }),
        None => Ok(()),
    }
}

/// The identity the model knows a command or a shell by: its number.
fn numbered<T: TryFrom<String>>(number: u64) -> T
where
    T::Error: std::fmt::Debug,
{
    T::try_from(number.to_string()).expect("a number is a nonempty identity")
}

fn stream_index(stream: StreamKind) -> usize {
    match stream {
        StreamKind::Stdout => 0,
        StreamKind::Stderr => 1,
    }
}

/// A changed file whose copies were not stored.
fn unkept(file: &JobFileChange) -> EditedFile {
    edited_file(file, |_| None)
}

/// The record a view lists for a file a job changed: its line counts and,
/// for each edit segment, the copies `copies` answers for its index.
pub fn edited_file(file: &JobFileChange, mut copies: impl FnMut(usize) -> Option<EditCopies>) -> EditedFile {
    EditedFile {
        path: file.path.clone(),
        kind: match file.kind {
            JobEditKind::Added => EditKind::Added,
            JobEditKind::Modified => EditKind::Modified,
        },
        added: u32::try_from(file.added).unwrap_or(u32::MAX),
        removed: u32::try_from(file.removed).unwrap_or(u32::MAX),
        edits: (0..file.edits.len())
            .map(|segment| EditSegment {
                copies: copies(segment),
            })
            .collect(),
    }
}

/// Adds why a command ended without running to its end to its output: a
/// line on stderr, after the output's last line. Returns the text it added.
fn push_reason(output: &mut Vec<OutputRecord>, reason: &str) -> String {
    let last = output.iter().rev().find_map(|record| match record {
        OutputRecord::Output(_, bytes) => bytes.last().copied(),
        OutputRecord::LeftOut(_) => None,
    });
    let separator = if last.is_none_or(|byte| byte == b'\n') {
        ""
    } else {
        "\n"
    };
    let text = format!("{separator}{reason}\n");
    output.push(OutputRecord::Output(
        StreamKind::Stderr,
        Bytes::from(text.clone()),
    ));
    text
}

/// What the backend received of one stream of a job (`runner.md` § Pipes
/// and output).
#[derive(Default)]
struct Received {
    /// How many of the stream's first `JOB_VIEW_BYTES` it received: the
    /// stream's part of the output it holds.
    head: u64,
    /// The stream's length on the Host, as its messages told it.
    host: u64,
    /// Where the pages' view of the stream's next bytes starts.
    next: u64,
    text: Utf8Stream,
}

impl Received {
    /// Adds one message of the job's output: a stream's first bytes to both
    /// views of `record` and to the `received` output, its newest bytes
    /// beyond them to the pages' view, and a message without bytes as
    /// growth alone. True when the pages' view changed.
    fn receive(
        &mut self,
        record: &RefCell<CommandRecord>,
        chunk: JobOutput,
        received: &RefCell<Vec<OutputRecord>>,
    ) -> bool {
        let end = chunk.offset + chunk.bytes.len() as u64;
        self.host = self.host.max(end);
        if chunk.offset < wire::JOB_VIEW_BYTES as u64 {
            self.head = end;
            self.next = end;
            let text = self.text.decode(&chunk.bytes);
            if !chunk.bytes.is_empty() {
                received
                    .borrow_mut()
                    .push(OutputRecord::Output(chunk.stream, chunk.bytes));
            }
            return record.borrow_mut().append_output(chunk.stream, &text);
        }
        record.borrow_mut().grew(chunk.stream, end);
        if chunk.bytes.is_empty() {
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
        self.next = end;
        record.borrow_mut().append_page_output(&text)
    }

    /// The bytes of the stream in the output the backend holds.
    fn received(&self) -> u64 {
        self.head
    }

    /// The bytes of the stream the Host held beyond those, as far as its
    /// messages told.
    fn unreceived(&self) -> u64 {
        self.host.saturating_sub(self.head)
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
    use super::Utf8Stream;

    #[test]
    fn a_character_split_across_chunks_decodes_once_whole() {
        let mut stream = Utf8Stream::default();
        let bytes = "aé€😀".as_bytes();
        let decoded: String = bytes.iter().map(|byte| stream.decode(&[*byte])).collect();
        assert_eq!(decoded, "aé€😀");
        assert_eq!(stream.decode(b"\xff!"), "\u{fffd}!");
    }
}
