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
use demi_backend_database::command_outputs::{self, CommandOutput, OutputRow};
use demi_backend_database::conversations::ConversationDb;
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
    CommandMedium, CommandStatus, ExecRequest, Host, HostError, HostErrorKind, HostKey,
    MediumKept, PageView, ShellEnvironment, ShellError, StoredMedium, WholeOutput,
};
use demi_runner_protocol::wire::JobFileChange;
use demi_shared_types::{BlobRef, Clock, CommandId, EditCopies, EditedFile, ShellId};
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use futures_util::future::LocalBoxFuture;
use tokio_util::sync::CancellationToken;

use crate::access::{ConversationHost, HostAccessError, Refusal};
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
        let device = &device;
        self.with_host(id, Some(device), cancel, async move |admitted| {
            if admitted.host.key() != *host {
                return Err(HostError::new(
                    HostErrorKind::Unavailable,
                    "the conversation's Host changed; the command did not run",
                ));
            }
            // Dropped once the job ends, or when it is stopped.
            let _ended = self.begin_job(id, device, admitted).await?;
            job.await;
            Ok(())
        })
        .await?
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
            options.keeper = Some(Rc::new(Keeper {
                conversation: conversation.clone(),
                host: host.clone(),
                db: shard.conversation_db(&conversation),
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
    host: Rc<RemoteHost>,
    db: ConversationDb,
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

impl Keeper {
    /// Stores each of a command's media the backend has as a blob, before
    /// its row names it (`storage.md` § Command outputs).
    async fn store_media(&self, command: &CommandId, media: &[CommandMedium]) -> Vec<StoredMedium> {
        let mut stored = Vec::with_capacity(media.len());
        for medium in media {
            let kept = match &medium.bytes {
                Ok(bytes) => match self.blobs.put(bytes.clone()).await {
                    Ok(blob) => MediumKept::Stored { blob },
                    Err(error) => {
                        tracing::warn!(conversation = %self.conversation, %command, number = medium.number, "a command's medium was not stored: {error}");
                        MediumKept::Missing {
                            reason: format!("not stored: {error}"),
                        }
                    }
                },
                Err(reason) => MediumKept::Missing {
                    reason: reason.clone(),
                },
            };
            stored.push(StoredMedium {
                number: medium.number,
                media_type: medium.media_type.clone(),
                size: medium.size,
                kept,
            });
        }
        stored
    }
}

impl CommandKeeper for Keeper {
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

    fn keep_output<'a>(
        &'a self,
        command: &'a CommandId,
        output: &'a WholeOutput,
        media: &'a [CommandMedium],
    ) -> LocalBoxFuture<'a, ()> {
        Box::pin(async move {
            let ended = self.clock.now();
            let stored = match self.put(output).await {
                Ok(blob) => OutputRow::Stored {
                    blob,
                    missing: output.missing().cloned(),
                    media: self.store_media(command, media).await,
                },
                Err(reason) => {
                    tracing::warn!(conversation = %self.conversation, %command, "a command's output was not stored: {reason}");
                    OutputRow::NotStored(reason)
                }
            };
            let row = CommandOutput {
                command: command.clone(),
                ended,
                output: stored,
            };
            let recorded = self
                .db
                .call(move |connection| command_outputs::insert(connection, &[row]))
                .await;
            // The command ends all the same; `demi shell output` then finds
            // no record of it.
            if let Err(error) = recorded {
                tracing::warn!(conversation = %self.conversation, %command, error = &error as &dyn std::error::Error, "a command's output was not recorded");
            }
        })
    }
}

/// A node's environment, and the registration that answers its jobs' rpc
/// calls with the node's commands while it lives.
struct Registered {
    environment: RemoteShellEnvironment,
    _registration: CommandRegistration,
}

impl ShellEnvironment for Registered {
    fn exec(
        &self,
        request: ExecRequest,
        cancel: CancellationToken,
    ) -> LocalBoxFuture<'_, Result<CommandStatus, ShellError>> {
        self.environment.exec(request, cancel)
    }

    fn status(&self, command: &CommandId) -> Result<CommandStatus, ShellError> {
        self.environment.status(command)
    }

    fn read_output<'a>(
        &'a self,
        command: &'a CommandId,
    ) -> LocalBoxFuture<'a, Result<WholeOutput, ShellError>> {
        self.environment.read_output(command)
    }

    fn read_medium<'a>(
        &'a self,
        command: &'a CommandId,
        number: u32,
    ) -> LocalBoxFuture<'a, Result<Bytes, ShellError>> {
        self.environment.read_medium(command, number)
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

    fn dispose_shell<'a>(&'a self, shell: &'a ShellId) -> LocalBoxFuture<'a, bool> {
        self.environment.dispose_shell(shell)
    }

    fn dispose_all(&self) -> LocalBoxFuture<'_, ()> {
        self.environment.dispose_all()
    }

    fn owns_shell(&self, shell: &ShellId) -> bool {
        self.environment.owns_shell(shell)
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
