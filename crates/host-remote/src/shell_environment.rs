//! `RemoteShellEnvironment`: the shell behind the `shell_*` tools on a Host
//! reached through its runner (`runner.md` § Shell jobs). Every exec is one
//! job; the record holds the model's view of its output, the head while it
//! runs and the tail at its end, and the whole output stays in the files the
//! runner wrote. The directory a script ends in carries into the shell's next
//! exec; nothing else of the shell's state does.

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
    JobCaller, ProcessEnd, ShellEnvironment, ShellError, ShellTarget, SpawnErrorKind,
    final_stdout_boundary,
};
use futures_util::future::LocalBoxFuture;
use tokio::sync::watch;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::{CommandCatalog, JobEnd, JobStart, RemoteHost, RemoteJob, manifest::CommandSelection};

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
    pub fn new(host: RemoteHost, context: ContextSource) -> Self {
        Self {
            host,
            commands: None,
            context,
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
            let mut record = record.borrow_mut();
            if running.stop.is_cancelled() {
                record.mark_aborted();
            } else if record.is_running() {
                let stdout = record.text(StreamKind::Stdout).to_owned();
                let stderr = format!("{}{message}\n", record.text(StreamKind::Stderr));
                record.settle(Ending::Exited(127), stdout, stderr, None);
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
        record: &RefCell<CommandRecord>,
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
        let mut heads = [Vec::new(), Vec::new()];
        let mut texts = [Utf8Stream::default(), Utf8Stream::default()];
        let stopped = running.stop.cancelled();
        tokio::pin!(stopped);
        let mut signalled = false;
        loop {
            tokio::select! {
                chunk = job.next_output() => {
                    let Some(chunk) = chunk else {
                        break;
                    };
                    let index = stream_index(chunk.stream);
                    heads[index].extend_from_slice(&chunk.bytes);
                    let text = texts[index].decode(&chunk.bytes);
                    record.borrow_mut().append_output(chunk.stream, &text);
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
        self.finish(shell, command, running, record, end, heads)
            .await;
        Ok(())
    }

    /// Settles the record from the job's end: its edits, its directory, and
    /// its streams as the model sees them.
    async fn finish(
        &self,
        shell: &ShellId,
        command: &CommandId,
        running: &Running,
        record: &RefCell<CommandRecord>,
        end: JobEnd,
        heads: [Vec<u8>; 2],
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
        let [stdout_head, stderr_head] = heads;
        let exit_code = match &end.status {
            ProcessEnd::NotStarted(error) => {
                // A job the runner could not run names why; any other kind is
                // bash's own.
                let reason = match (&error.kind, &error.detail) {
                    (SpawnErrorKind::Other, Some(detail)) => detail.clone(),
                    _ => format!("bash: {}", error.kind),
                };
                return self.never_ran(record, &stdout_head, &stderr_head, &reason);
            }
            ProcessEnd::Lost(reason) => {
                return self.never_ran(record, &stdout_head, &stderr_head, reason);
            }
            ProcessEnd::Exited(code) => *code,
            ProcessEnd::Signalled(signal) if matches!(signal.as_str(), "SIGTERM" | "SIGKILL") => {
                130
            }
            ProcessEnd::Signalled(_) => 128,
        };
        let output = end.output;
        let stdout = stream_text(
            &stdout_head,
            output.as_ref().map(|output| {
                (
                    output.stdout_bytes,
                    &output.stdout_tail.0,
                    output.stdout_path.as_str(),
                )
            }),
        );
        let stderr = stream_text(
            &stderr_head,
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
        let mut record = record.borrow_mut();
        if let Some(output) = &output {
            // The runner names where its tee wrote: the output files are the
            // target's.
            if let Some((directory, _)) = output.stdout_path.rsplit_once('/') {
                record.set_output_dir(directory.to_owned());
            }
        }
        record.settle(ending, stdout_text, stderr.text, binary);
        if let Some(output) = &output {
            record.set_host_bytes(output.stdout_bytes, output.stderr_bytes);
        }
    }

    /// A job that never ran its script: exit 127, and why, on stderr.
    fn never_ran(
        &self,
        record: &RefCell<CommandRecord>,
        stdout: &[u8],
        stderr: &[u8],
        reason: &str,
    ) {
        let stdout = String::from_utf8_lossy(stdout).into_owned();
        let stderr = format!("{}{reason}\n", String::from_utf8_lossy(stderr));
        record
            .borrow_mut()
            .settle(Ending::Exited(127), stdout, stderr, None);
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

    async fn abort_command(&self, command: &CommandId) -> Result<CommandStatus, ShellError> {
        let record = self.record(command)?;
        let running = self.0.state.borrow().running.get(command).cloned();
        let Some(running) = running.filter(|_| record.borrow().is_running()) else {
            return self.view(command);
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
                record.borrow_mut().mark_aborted();
            }
        }
        self.view(command)
    }

    async fn dispose(&self, shell: &ShellId) -> bool {
        let foreground = match self.0.state.borrow().shells.get(shell) {
            None => return false,
            Some(shell) => shell.foreground.clone(),
        };
        if let Some(command) = foreground
            && let Err(error) = self.abort_command(&command).await
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
    ) -> LocalBoxFuture<'a, Result<CommandStatus, ShellError>> {
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
            self.view(command)
        })
    }

    fn abort<'a>(
        &'a self,
        command: &'a CommandId,
    ) -> LocalBoxFuture<'a, Result<CommandStatus, ShellError>> {
        Box::pin(self.abort_command(command))
    }

    fn release_command<'a>(&'a self, command: &'a CommandId) -> LocalBoxFuture<'a, bool> {
        Box::pin(async move {
            let Ok(record) = self.record(command) else {
                return false;
            };
            let running = record.borrow().is_running();
            if running && let Err(error) = self.abort_command(command).await {
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
    EditedFile {
        path: file.path.clone(),
        kind: match file.kind {
            JobEditKind::Added => EditKind::Added,
            JobEditKind::Modified => EditKind::Modified,
        },
        added: u32::try_from(file.added).unwrap_or(u32::MAX),
        removed: u32::try_from(file.removed).unwrap_or(u32::MAX),
        edits: file
            .edits
            .iter()
            .map(|_| KeptEdit { kept: false })
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
