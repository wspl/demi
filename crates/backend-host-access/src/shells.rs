//! Each agent node's Host and shell environments (`sessions-and-targets.md`
//! § Host operations, `commands.md` § Handle an rpc call). A node's Host is
//! the conversation's current primary Host, which the host access admits and
//! lets go at once. A node's environment on it is backend-remote-host's: each of its
//! jobs runs inside the conversation's host access and is refused once the
//! conversation's Host is another, each job's command context names the
//! conversation and the node, and while the environment lives the node's
//! commands answer its jobs' rpc calls.

use std::collections::{BTreeSet, HashMap};
use std::rc::{Rc, Weak};
use std::sync::Arc;

use bytes::Bytes;
use demi_agent_tools::{EnvironmentScope, ShellEnvironmentFactory};
use demi_backend_blobs::blobs::UserBlobs;
use demi_backend_database::command_outputs::{CommandOutput, OutputRow};
use demi_backend_database::control::ControlService;
use demi_backend_database::conversations::ConversationDb;
use demi_backend_database::running::{self, RunningCommand};
use demi_backend_remote_host::{
    CommandCatalog, CommandKeeper, ContextSource, EnvironmentOptions, HostAccess, RemoteHost,
    RemoteShellEnvironment, edited_file, encode_output,
};
use demi_backend_runners::command_context::command_context;
use demi_backend_runners::files::text_of;
use demi_backend_runners::host_key::device_of;
use demi_backend_runners::router::CommandRegistration;
use demi_command_protocol::{CommandCaller, EDIT_FILE_BYTES};
use demi_host_interface::{
    CommandMedium, CommandStatus, Ending, ExecRequest, Host, HostError, HostErrorKind, HostKey,
    PageView, Seen, ShellEnvironment, ShellError, TakenUp, Unreachable, WholeOutput,
};
use demi_runner_protocol::wire::JobFileChange;
use demi_shared_types::{BlobRef, Clock, CommandEnd, CommandId, EditCopies, EditedFile, NodeId};
use demi_web_api_protocol::devices::DeviceKind;
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use futures_util::future::LocalBoxFuture;
use tokio_util::sync::CancellationToken;

use crate::access::{ConversationHost, HostAccessError, Refusal, Waits};
use crate::{HostShard, conversation_of};

impl dyn HostShard + '_ {
    /// A node's Host: the conversation's current primary Host. The host access
    /// admits it and lets it go at once; its later operations take only a
    /// Cloud's per-operation admission.
    pub async fn conversation_host(
        &self,
        id: &ConversationId,
    ) -> Result<Rc<RemoteHost>, HostError> {
        let host = self
            .with_host(id, None, &CancellationToken::new(), async |host| {
                host.host.clone()
            })
            .await?;
        Ok(Rc::new(host))
    }

    /// Runs `job`, a shell job the conversation's agent started on `host`,
    /// inside the conversation's host access: it holds what the Host needs
    /// until the job ended, and it is refused once `host` is not the Host
    /// the conversation reaches on that device, as after a target switch.
    pub async fn run_job(
        &self,
        id: &ConversationId,
        host: &HostKey,
        cancel: &CancellationToken,
        job: LocalBoxFuture<'_, ()>,
    ) -> Result<(), HostError> {
        let device = device_of(host)
            .and_then(|device| DeviceId::try_from(device).ok())
            .ok_or_else(|| {
                HostError::new(
                    HostErrorKind::Protocol,
                    "the job's Host is no conversation's",
                )
            })?;
        let admitted = self
            .admit_host(id, Some(&device), Waits::request(cancel))
            .await?;
        if admitted.host.host.key() != *host {
            return Err(HostError::new(
                HostErrorKind::Unavailable,
                "the conversation's Host changed; the command did not run",
            ));
        }
        let admitted_host = admitted.host.clone();
        // The admission is the job's until it ends, unless a move away from
        // its offline Host leaves the job to that Host's runner.
        let _held = self
            .conversations()
            .slot(id)
            .hold_job(device.clone(), admitted);
        // Dropped once the job ends, or when it is stopped.
        let _ended = self.begin_job(id, &device, &admitted_host).await?;
        job.await;
        Ok(())
    }

    /// Readies `host`, admitted for a job of the conversation on `device`,
    /// before the job starts: installs the user's Host directories there
    /// (`plugins.md` § Host directories). The answer tells the shard that
    /// the job ended when it drops, so the job holds it inside its
    /// admission until it ended, however it ends.
    pub(crate) async fn begin_job<'a>(
        &'a self,
        id: &'a ConversationId,
        device: &DeviceId,
        host: &ConversationHost,
    ) -> Result<JobEnd<'a>, HostError> {
        self.install_directories(device, host).await?;
        Ok(JobEnd {
            shard: self,
            conversation: id,
        })
    }
}

/// Tells the shard that a job ended, however it ends.
pub(crate) struct JobEnd<'a> {
    shard: &'a (dyn HostShard + 'a),
    conversation: &'a ConversationId,
}

impl Drop for JobEnd<'_> {
    fn drop(&mut self) {
        self.shard.job_ended(self.conversation);
    }
}

impl From<HostAccessError> for HostError {
    fn from(error: HostAccessError) -> Self {
        match error {
            HostAccessError::Host(error) => error,
            HostAccessError::Cancelled => {
                HostError::new(HostErrorKind::Interrupted, error.to_string())
            }
            HostAccessError::Storage(_)
            | HostAccessError::Objects(_)
            | HostAccessError::Store(_) => HostError::failed(None, error.to_string()),
            HostAccessError::Missing
            | HostAccessError::Cloud(_)
            | HostAccessError::Refused(
                Refusal::Archived
                | Refusal::NotAttached
                | Refusal::Busy
                | Refusal::Stopped
                | Refusal::DeviceGone,
            ) => HostError::new(HostErrorKind::Unavailable, error.to_string()),
        }
    }
}

/// Makes each node's shell environments.
pub struct ShardShellEnvironments {
    /// Weak: the shard owns the agent server that holds this factory.
    shard: Weak<dyn HostShard>,
    /// The command packages a node's commands bind to.
    catalog: CommandCatalog,
}

impl ShardShellEnvironments {
    pub fn new(shard: Weak<dyn HostShard>, catalog: CommandCatalog) -> Self {
        Self { shard, catalog }
    }
}

impl ShellEnvironmentFactory<RemoteHost> for ShardShellEnvironments {
    fn create<'a>(
        &'a self,
        scope: EnvironmentScope<'a>,
        host: Rc<RemoteHost>,
    ) -> LocalBoxFuture<'a, Result<Rc<dyn ShellEnvironment>, HostError>> {
        Box::pin(async move {
            let shard = self
                .shard
                .upgrade()
                .ok_or_else(|| HostError::offline("the backend is shutting down"))?;
            let conversation = conversation_of(scope.root);
            let context: ContextSource = {
                let shard = self.shard.clone();
                let conversation = conversation.clone();
                let caller = CommandCaller::agent(scope.agent);
                Rc::new(move || {
                    let shard = shard.clone();
                    let conversation = conversation.clone();
                    let caller = caller.clone();
                    Box::pin(async move {
                        let shard = shard
                            .upgrade()
                            .ok_or_else(|| HostError::offline("the backend is shutting down"))?;
                        command_context(shard.control(), shard.user(), &conversation, caller)
                            .await
                            .map_err(|error| HostError::failed(None, error.to_string()))
                    })
                })
            };
            let mut options = EnvironmentOptions::new(
                (*host).clone(),
                context,
                scope.feed.clone(),
                scope.numbers.clone(),
            );
            options.access = Some(Rc::new(JobAccess {
                shard: self.shard.clone(),
                conversation: conversation.clone(),
            }));
            let device = device_of(&host.key())
                .and_then(|device| DeviceId::try_from(device).ok())
                .ok_or_else(|| {
                    HostError::new(HostErrorKind::Protocol, "the node's Host is no device's")
                })?;
            options.keeper = Some(Rc::new(Keeper {
                conversation: conversation.clone(),
                node: scope.node.clone(),
                device: device.clone(),
                host: host.clone(),
                db: shard.conversation_db(&conversation),
                control: shard.control().clone(),
                blobs: shard.blobs(),
                clock: shard.clock().clone(),
            }));
            let selection = self
                .catalog
                .select(scope.commands)
                .map_err(|error| HostError::new(HostErrorKind::Protocol, error.to_string()))?;
            options.commands = Some(selection.clone());
            let environment = RemoteShellEnvironment::new(options);
            let registration = shard.commands().register(
                scope.node.as_str(),
                &conversation,
                scope.commands.clone(),
                selection,
            );
            Ok(Rc::new(Registered {
                environment,
                host,
                offline: Offline {
                    shard: self.shard.clone(),
                    device,
                },
                _registration: registration,
            }) as Rc<dyn ShellEnvironment>)
        })
    }
}

/// The conversation's host access around each job of a node's environment.
struct JobAccess {
    shard: Weak<dyn HostShard>,
    conversation: ConversationId,
}

impl HostAccess for JobAccess {
    fn run_job<'a>(
        &'a self,
        host: &'a HostKey,
        cancel: CancellationToken,
        job: LocalBoxFuture<'a, ()>,
    ) -> LocalBoxFuture<'a, Result<(), HostError>> {
        Box::pin(async move {
            let shard = self
                .shard
                .upgrade()
                .ok_or_else(|| HostError::offline("the backend is shutting down"))?;
            shard.run_job(&self.conversation, host, &cancel, job).await
        })
    }
}

/// Keeps what each job's commands leave when they end, before they read as
/// ended, as blobs of the conversation owner's: the copies of their edits
/// (`edit-tracking.md` § Edit copies), read from the job's Host inside its
/// host access, and their whole outputs, each with its record in the
/// conversation's database (`storage.md` § Command outputs).
struct Keeper {
    conversation: ConversationId,
    /// The node whose commands these are.
    node: NodeId,
    /// The device of the node's Host.
    device: DeviceId,
    host: Rc<RemoteHost>,
    db: ConversationDb,
    control: ControlService,
    blobs: UserBlobs,
    clock: Arc<dyn Clock>,
}

impl Keeper {
    /// Stores `output` as a blob, in the kept output's records.
    async fn put(&self, output: &WholeOutput) -> Result<demi_shared_types::BlobRef, String> {
        let output = output.clone();
        // Encoding 16 MiB would hold the shard's thread.
        let encoded = tokio::task::spawn_blocking(move || encode_output(&output))
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
        self.blobs
            .put(Bytes::from(encoded))
            .await
            .map_err(|error| error.to_string())
    }

    /// Stores `text` as a blob.
    async fn put_text(&self, text: String) -> Result<BlobRef, String> {
        self.blobs
            .put(Bytes::from(text))
            .await
            .map_err(|error| error.to_string())
    }
}

/// The Host's copies the edit segments of `files` name, each read once with
/// one request (`edit-tracking.md` § Edit copies): each copy's text, when it
/// is text within the edit limits, or why it is not stored. The runner
/// keeps no copy that is not.
async fn read_copies(
    host: &RemoteHost,
    files: &[JobFileChange],
) -> HashMap<String, Result<String, String>> {
    let paths: Vec<String> = files
        .iter()
        .flat_map(|file| &file.edits)
        .filter(|segment| segment.modified.is_some())
        .flat_map(|segment| [segment.original.clone(), segment.modified.clone()])
        .flatten()
        .collect::<BTreeSet<String>>()
        .into_iter()
        .collect();
    if paths.is_empty() {
        return HashMap::new();
    }
    let limit = u64::try_from(EDIT_FILE_BYTES).expect("the edit limit fits u64");
    match host.read_files(&paths, limit).await {
        Ok(read) => paths
            .into_iter()
            .zip(read)
            .map(|(path, bytes)| {
                let text = bytes
                    .map_err(|error| error.to_string())
                    .and_then(|bytes| text_of(Vec::from(bytes)).map_err(|refusal| refusal.to_string()));
                (path, text)
            })
            .collect(),
        Err(error) => paths
            .into_iter()
            .map(|path| (path, Err(error.to_string())))
            .collect(),
    }
}

impl CommandKeeper for Keeper {
    /// Records the job in the control database first, by its device, so a
    /// runner's hello finds its conversation, then the command in the
    /// conversation's (`storage.md` § Command outputs).
    fn started<'a>(
        &'a self,
        command: &'a CommandId,
        tool_use_id: &'a str,
        job: &'a str,
    ) -> LocalBoxFuture<'a, ()> {
        Box::pin(async move {
            let indexed = self
                .control
                .record_running_job(job.to_owned(), self.device.clone(), self.conversation.clone())
                .await;
            if let Err(error) = indexed {
                // The command runs all the same; a backend that starts again
                // does not take it up.
                tracing::warn!(conversation = %self.conversation, %command, error = &error as &dyn std::error::Error, "a running job was not recorded");
                return;
            }
            let running = RunningCommand {
                command: command.clone(),
                node: self.node.clone(),
                device: self.device.clone(),
                job: job.to_owned(),
                tool_use_id: tool_use_id.to_owned(),
                started: self.clock.now(),
                places: Default::default(),
            };
            let recorded = self
                .db
                .call(move |connection| running::insert(connection, &running))
                .await;
            if let Err(error) = recorded {
                tracing::warn!(conversation = %self.conversation, %command, error = &error as &dyn std::error::Error, "a running command was not recorded");
            }
        })
    }

    fn retain<'a>(
        &'a self,
        command: &'a CommandId,
        files: &'a [JobFileChange],
    ) -> LocalBoxFuture<'a, Vec<EditedFile>> {
        Box::pin(async move {
            // Each copy is stored once, however many segments name it; an
            // empty one stands before a segment that created its file.
            let mut stored: HashMap<Option<String>, Result<BlobRef, String>> = HashMap::new();
            for (path, text) in read_copies(&self.host, files).await {
                let blob = match text {
                    Ok(text) => self.put_text(text).await,
                    Err(error) => Err(error),
                };
                stored.insert(Some(path), blob);
            }
            let created = files
                .iter()
                .flat_map(|file| &file.edits)
                .any(|segment| segment.original.is_none() && segment.modified.is_some());
            if created {
                stored.insert(None, self.put_text(String::new()).await);
            }
            let mut retained = Vec::with_capacity(files.len());
            for file in files {
                let copies: Vec<Option<EditCopies>> = file
                    .edits
                    .iter()
                    .map(|segment| {
                        let modified = segment.modified.clone()?;
                        let blob = |path: Option<String>| {
                            stored
                                .get(&path)
                                .cloned()
                                .expect("every copy a segment names was read")
                        };
                        let copies = blob(segment.original.clone()).and_then(|original| {
                            Ok(EditCopies {
                                original,
                                modified: blob(Some(modified))?,
                            })
                        });
                        if let Err(error) = &copies {
                            // The file's record stays in the list without this
                            // segment's copies.
                            tracing::warn!(conversation = %self.conversation, %command, path = %file.path, "an edit's copies were not stored: {error}");
                        }
                        copies.ok()
                    })
                    .collect();
                retained.push(edited_file(file, |segment| copies[segment].clone()));
            }
            retained
        })
    }

    fn stored_blob<'a>(&'a self, blob: &'a BlobRef) -> LocalBoxFuture<'a, Option<Bytes>> {
        Box::pin(async move {
            match self.blobs.get(blob).await {
                Ok(bytes) => bytes,
                // The medium is read from the Host instead.
                Err(error) => {
                    tracing::debug!(conversation = %self.conversation, %blob, "a blob was not read: {error}");
                    None
                }
            }
        })
    }

    fn looked<'a>(&'a self, command: &'a CommandId, seen: Seen) -> LocalBoxFuture<'a, ()> {
        Box::pin(async move {
            let (wanted, node) = (command.clone(), self.node.clone());
            let recorded = self
                .db
                .call(move |connection| running::set_place(connection, &wanted, &node, seen))
                .await;
            if let Err(error) = recorded {
                // The model keeps its place; a backend that starts again
                // shows it the output from its last recorded place.
                tracing::warn!(conversation = %self.conversation, %command, error = &error as &dyn std::error::Error, "a place in a command's output was not recorded");
            }
        })
    }

    fn keep_output<'a>(
        &'a self,
        command: &'a CommandId,
        end: CommandEnd,
        output: &'a WholeOutput,
    ) -> LocalBoxFuture<'a, ()> {
        Box::pin(async move {
            let ended = self.clock.now();
            let stored = match self.put(output).await {
                Ok(blob) => OutputRow::Stored {
                    blob,
                    missing: output.missing().cloned(),
                },
                Err(reason) => {
                    tracing::warn!(conversation = %self.conversation, %command, "a command's output was not stored: {reason}");
                    OutputRow::NotStored(reason)
                }
            };
            let row = CommandOutput {
                command: command.clone(),
                ended,
                end,
                output: stored,
            };
            let recorded = self
                .db
                .call(move |connection| running::end(connection, row))
                .await;
            match recorded {
                Ok(Some(job)) => {
                    if let Err(error) = self.control.end_running_job(job).await {
                        // A runner's next hello lists no such job, and the
                        // backend finds nothing to take up.
                        tracing::warn!(conversation = %self.conversation, %command, error = &error as &dyn std::error::Error, "a running job's record was not removed");
                    }
                }
                Ok(None) => {}
                // The command ends all the same; `demi shell output` then
                // finds no record of it.
                Err(error) => {
                    tracing::warn!(conversation = %self.conversation, %command, error = &error as &dyn std::error::Error, "a command's output was not recorded");
                }
            }
        })
    }
}

/// A node's environment, and the registration that answers its jobs' rpc
/// calls with the node's commands while it lives.
struct Registered {
    environment: RemoteShellEnvironment,
    /// The Host its commands run on.
    host: Rc<RemoteHost>,
    offline: Offline,
    _registration: CommandRegistration,
}

/// What a command started on a paired device without a live runner fails
/// with.
struct Offline {
    shard: Weak<dyn HostShard>,
    device: DeviceId,
}

impl Offline {
    /// The device's offline error, as its record says it now; none for a
    /// Cloud, which the job's admission wakes.
    async fn error(&self) -> Option<HostError> {
        let Some(shard) = self.shard.upgrade() else {
            return Some(HostError::offline("the backend is shutting down"));
        };
        match shard.control().device(self.device.clone()).await {
            Ok(Some(record)) if record.kind == DeviceKind::Managed => None,
            Ok(Some(record)) => Some(shard.offline(&record)),
            Ok(None) => Some(HostError::offline("the device was removed")),
            Err(error) => Some(HostError::offline(format!(
                "the device is offline, and its record could not be read: {error}"
            ))),
        }
    }
}

impl ShellEnvironment for Registered {
    /// Starts nothing on a device without a live runner: the call shows
    /// its offline error, worded for the model (`sessions-and-targets.md`
    /// § Host operations).
    fn start(
        &self,
        request: ExecRequest,
        cancel: CancellationToken,
    ) -> LocalBoxFuture<'_, Result<CommandId, ShellError>> {
        Box::pin(async move {
            if !self.host.online()
                && let Some(error) = self.offline.error().await
            {
                return Err(ShellError::Host(error));
            }
            self.environment.start(request, cancel).await
        })
    }

    fn ended<'a>(&'a self, command: &'a CommandId) -> LocalBoxFuture<'a, Result<Ending, ShellError>> {
        self.environment.ended(command)
    }

    fn status(&self, command: &CommandId) -> Result<CommandStatus, ShellError> {
        self.environment.status(command)
    }

    fn quiet(&self, command: &CommandId) -> Result<std::time::Duration, ShellError> {
        self.environment.quiet(command)
    }

    fn outliving(&self, command: &CommandId) -> Vec<String> {
        self.environment.outliving(command)
    }

    fn unreachable(&self, command: &CommandId) -> Option<Unreachable> {
        self.environment.unreachable(command)
    }

    fn media(&self, command: &CommandId) -> Result<Vec<CommandMedium>, ShellError> {
        self.environment.media(command)
    }

    fn read_output<'a>(
        &'a self,
        command: &'a CommandId,
    ) -> LocalBoxFuture<'a, Result<WholeOutput, ShellError>> {
        self.environment.read_output(command)
    }

    fn write<'a>(
        &'a self,
        command: &'a CommandId,
        stdin: Bytes,
    ) -> LocalBoxFuture<'a, Result<(), ShellError>> {
        self.environment.write(command, stdin)
    }

    fn abort<'a>(&'a self, command: &'a CommandId) -> LocalBoxFuture<'a, Result<(), ShellError>> {
        self.environment.abort(command)
    }

    fn page_views(&self) -> Vec<PageView> {
        self.environment.page_views()
    }

    fn release_command<'a>(&'a self, command: &'a CommandId) -> LocalBoxFuture<'a, bool> {
        self.environment.release_command(command)
    }

    fn adopt(&self, taken: TakenUp) {
        self.environment.adopt(taken);
    }

    fn detach_all(&self) -> LocalBoxFuture<'_, ()> {
        self.environment.detach_all()
    }

    fn dispose_all(&self) -> LocalBoxFuture<'_, ()> {
        self.environment.dispose_all()
    }

    fn owns_command(&self, command: &CommandId) -> bool {
        self.environment.owns_command(command)
    }
}

#[cfg(test)]
mod tests {
    use demi_backend_remote_host::testing::{FixtureOptions, RunnerFixture, answered_requests};
    use demi_command_protocol::{EditCopies as CopyPaths, EditKind};
    use tokio::sync::mpsc;

    use super::*;

    // About a tenth of a second: a real runner process reads the copies.
    #[tokio::test(flavor = "local")]
    async fn a_commands_edit_copies_are_read_with_one_request() {
        let (tap, mut replies) = mpsc::channel(1 << 10);
        let fixture = RunnerFixture::start(FixtureOptions {
            tap: Some(tap),
            ..FixtureOptions::default()
        })
        .await;
        let copies = fixture.home_dir().join("copies");
        std::fs::create_dir(&copies).unwrap();
        std::fs::write(copies.join("1"), "one\n").unwrap();
        std::fs::write(copies.join("2"), "two\n").unwrap();
        std::fs::write(copies.join("3"), "three\n").unwrap();
        std::fs::write(copies.join("binary"), b"\x00\xff").unwrap();
        answered_requests(&mut replies);
        let path = |name: &str| Some(format!("{}/copies/{name}", fixture.home()));
        let segment = |original: Option<String>, modified: Option<String>| CopyPaths {
            original,
            modified,
        };
        // Two edits of one file share the copy between them; a created file
        // has no copy before its first; a binary copy is not text.
        let files = [
            JobFileChange {
                path: "notes.txt".into(),
                kind: EditKind::Modified,
                edits: vec![
                    segment(path("1"), path("2")),
                    segment(path("2"), path("3")),
                ],
                added: 2,
                removed: 1,
            },
            JobFileChange {
                path: "new.txt".into(),
                kind: EditKind::Added,
                edits: vec![segment(None, path("3")), segment(path("binary"), path("gone"))],
                added: 1,
                removed: 0,
            },
        ];

        let read = read_copies(&fixture.host(), &files).await;

        assert_eq!(answered_requests(&mut replies), 1, "one request reads every copy");
        let mut texts: Vec<_> = read
            .into_iter()
            .map(|(path, text)| {
                let name = path.rsplit('/').next().unwrap().to_owned();
                (name, text.map_err(|_| ()))
            })
            .collect();
        texts.sort();
        assert_eq!(
            texts,
            [
                ("1".to_owned(), Ok("one\n".to_owned())),
                ("2".to_owned(), Ok("two\n".to_owned())),
                ("3".to_owned(), Ok("three\n".to_owned())),
                ("binary".to_owned(), Err(())),
                ("gone".to_owned(), Err(())),
            ]
        );
        fixture.stop().await;
    }
}
