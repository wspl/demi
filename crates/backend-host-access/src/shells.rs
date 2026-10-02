//! Each agent node's Host and shell environments (`sessions-and-targets.md`
//! § Host operations, `commands.md` § Handle an rpc call). A node's Host is
//! the conversation's current main Host, which the host access admits and
//! lets go at once. A node's environment on it is backend-remote-host's: each of its
//! jobs runs inside the conversation's host access and is refused once the
//! conversation's Host is another, each job's command context names the
//! conversation and the node, and while the environment lives the node's
//! commands answer its jobs' rpc calls.

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
use demi_command_protocol::CommandCaller;
use demi_host_interface::{
    CommandStatus, ExecRequest, Host, HostError, HostErrorKind, HostFs, HostKey, PageView,
    ShellEnvironment, ShellError, WholeOutput,
};
use demi_runner_protocol::wire::JobFileChange;
use demi_shared_types::{Clock, CommandId, EditCopies, EditedFile, ShellId};
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use futures_util::future::LocalBoxFuture;
use tokio_util::sync::CancellationToken;

use crate::access::{HostAccessError, Refusal};
use crate::blobs::ConversationBlobs;
use crate::{HostShard, conversation_of};

impl dyn HostShard + '_ {
    /// A node's Host: the conversation's current main Host. The host access
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
            self.install_directories(device, admitted).await?;
            job.await;
            Ok(())
        })
        .await?
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

    /// Stores an edit segment's two sides as blobs: the Host's copy before
    /// it, empty for a segment that created the file, and its copy after
    /// it. Both must be text the change view can show.
    async fn store_copies(
        &self,
        original: Option<&str>,
        modified: &str,
    ) -> Result<EditCopies, String> {
        let before = match original {
            Some(path) => self.text_copy(path).await?,
            None => String::new(),
        };
        let after = self.text_copy(modified).await?;
        let put = async |text: String| {
            self.blobs
                .put(Bytes::from(text))
                .await
                .map_err(|error| error.to_string())
        };
        Ok(EditCopies {
            original: put(before).await?,
            modified: put(after).await?,
        })
    }

    /// The Host's copy at `path`, when it is text within the edit limits;
    /// the runner already keeps no other.
    async fn text_copy(&self, path: &str) -> Result<String, String> {
        let bytes = HostFs::read_file(&*self.host, path)
            .await
            .map_err(|error| error.to_string())?;
        text_of(bytes).map_err(|refusal| refusal.to_string())
    }
}

impl CommandKeeper for Keeper {
    fn retain<'a>(
        &'a self,
        command: &'a CommandId,
        files: &'a [JobFileChange],
    ) -> LocalBoxFuture<'a, Vec<EditedFile>> {
        Box::pin(async move {
            let mut retained = Vec::with_capacity(files.len());
            for file in files {
                let mut copies = Vec::with_capacity(file.edits.len());
                for segment in &file.edits {
                    let Some(modified) = &segment.modified else {
                        copies.push(None);
                        continue;
                    };
                    let stored = self
                        .store_copies(segment.original.as_deref(), modified)
                        .await;
                    if let Err(error) = &stored {
                        // The file's record stays in the list without this
                        // segment's copies.
                        tracing::warn!(conversation = %self.conversation, %command, path = %file.path, "an edit's copies were not stored: {error}");
                    }
                    copies.push(stored.ok());
                }
                retained.push(edited_file(file, |segment| copies[segment].take()));
            }
            retained
        })
    }

    fn keep_output<'a>(
        &'a self,
        command: &'a CommandId,
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
                output: stored,
            };
            let blobs = ConversationBlobs(self.blobs.clone());
            let recorded = self
                .db
                .call(move |connection| command_outputs::insert(connection, &blobs, &[row]))
                .await;
            // The command ends all the same; `demi shell output` then finds
            // no record of it.
            match recorded {
                Ok(Ok(())) => {}
                Ok(Err(refused)) => {
                    tracing::warn!(conversation = %self.conversation, %command, "a command's output was not recorded: {refused}");
                }
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
