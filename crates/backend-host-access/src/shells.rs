//! Each agent node's Host and shell environments
//! (`crates/backend-host-access/src/access.rs`,
//! `crates/host-interface/src/rpc.rs`).
//!
//! A node's Host is the conversation's current primary Host, which the host
//! access admits and lets go at once. A node's environment on it is
//! backend-remote-host's:
//!
//! - Each of its jobs runs inside the conversation's host access and is refused
//!   once the conversation's Host is another.
//! - Each job's command context names the conversation and the node.
//! - While the environment lives, the node's commands answer its jobs' rpc
//!   calls.

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
    CommandCatalog, CommandKeeper, ContextSource, EnvironmentOptions, HostAccess, JobWaiting, KeptStart, RemoteHost,
    RemoteShellEnvironment, Withdrawal, edited_file, encode_output,
};
use demi_backend_runners::command_context::command_context;
use demi_backend_runners::files::text_of;
use demi_backend_runners::host_key::device_of;
use demi_backend_runners::router::CommandRegistration;
use demi_command_protocol::{CommandCaller, EDIT_FILE_BYTES};
use demi_host_interface::{
    CommandSet,
    CommandMedium, CommandStatus, Ending, ExecRequest, Host, HostError, HostErrorKind, HostKey,
    PageView, Seen, ShellEnvironment, ShellError, TakenUp, Unreachable, UnsentStart, WholeOutput,
};
use demi_runner_protocol::wire::JobFileChange;
use demi_shared_types::{
    BlobRef, Clock, CommandEnd, CommandId, EditCopies, EditedFile, NodeId, PixelSize, model_media_type_for,
};
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use futures_util::future::LocalBoxFuture;
use tokio_util::sync::CancellationToken;

use crate::access::{DeviceWait, HostAccessError, Refusal, ShowWait, Waits};
use crate::attachment_commands::{Numbering, Unnumbered, commit};
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

    /// A node's Host for taking up the commands its agents ran there, which
    /// admits nothing: the conversation's primary Host as its record says, with
    /// no lease of its file gate, no wait for its device to return and no
    /// wake of its Cloud. The commands taken up wait for the runner's hello
    /// to say whether they went on, ended or were lost
    /// (`crates/backend-remote-host/src/device_jobs.rs`).
    pub async fn adopting_host(&self, id: &ConversationId) -> Result<Rc<RemoteHost>, HostError> {
        let record = self.owned_conversation(id).await?;
        let target = self.resolve_target(&record).await.map_err(|error| HostError::failed(None, error.to_string()))?;
        let Some(device) = target.device() else {
            return Err(HostError::new(
                HostErrorKind::Unavailable,
                "the conversation's Cloud was never made",
            ));
        };
        let device_record = self
            .control()
            .device(device.clone())
            .await
            .map_err(|error| HostError::failed(None, error.to_string()))?;
        let admission = match device_record {
            Some(record) if record.kind == demi_web_api_protocol::devices::DeviceKind::Managed => self
                .cloud_shard()
                .operation_admission(&record)
                .await
                .map_err(|error| HostError::failed(None, error.to_string()))?,
            Some(_) => demi_backend_remote_host::Admission::Free,
            // A revoked device: the commands taken up on it are lost.
            None => {
                self.devices().removed(&device);
                demi_backend_remote_host::Admission::Free
            }
        };
        Ok(Rc::new(self.devices().adopting_host(
            device,
            id,
            target.path().to_owned(),
            admission,
        )))
    }

    /// Runs `job`, a shell job of the conversation's agent started on `host`,
    /// inside the conversation's host access.
    ///
    /// The job holds what the Host needs until it ended, and it is refused once
    /// `host` is not the Host the conversation reaches on that device, as after
    /// a target switch. While the conversation's primary Host is a paired
    /// device without a live runner, the job waits for it as `wait` says
    /// (`crates/backend-host-access/src/access.rs`).
    pub async fn run_job(
        &self,
        id: &ConversationId,
        host: &HostKey,
        cancel: &CancellationToken,
        wait: DeviceWait<'_>,
        withdrawal: Rc<Withdrawal>,
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
            .admit_host(id, Some(&device), Waits::job(cancel, wait))
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
            .hold_job(device.clone(), Some(admitted), withdrawal.clone());
        // A detach that committed since the admission, or does while the job
        // installs its directories, ends it here, before it starts.
        self.reaches(id, &device, &withdrawal).await?;
        self.install_directories(&device, &admitted_host).await?;
        self.reaches(id, &device, &withdrawal).await?;
        job.await;
        Ok(())
    }

    /// Registers a job taken up again after a restart, which runs on the
    /// device of `host`, with its conversation until the answer is dropped,
    /// so a detach of the device stops it. A device the conversation no
    /// longer reaches stops it at once.
    pub async fn hold_adopted(
        &self,
        id: &ConversationId,
        host: &HostKey,
        withdrawal: Rc<Withdrawal>,
    ) -> Option<Box<dyn std::any::Any>> {
        let device = device_of(host).and_then(|device| DeviceId::try_from(device).ok())?;
        let held = self
            .conversations()
            .slot(id)
            .hold_job(device.clone(), None, withdrawal.clone());
        // A failure to say is no reason to stop it: the withdrawal holds
        // the reason when it was made.
        let _ = self.reaches(id, &device, &withdrawal).await;
        Some(Box::new(held))
    }

    /// Whether `device` is still the conversation's primary Host or one
    /// attached to it. When it is not, the job's `withdrawal` is made, and
    /// the answer is the error that ends the job without starting it. A
    /// withdrawal made already ends it the same way.
    pub(crate) async fn reaches(
        &self,
        id: &ConversationId,
        device: &DeviceId,
        withdrawal: &Withdrawal,
    ) -> Result<(), HostError> {
        if let Some(reason) = withdrawal.reason() {
            return Err(HostError::new(HostErrorKind::Unavailable, reason));
        }
        // A conversation that cannot be read is no reason to end the job:
        // what refuses it says so.
        let Ok(hosts) = self.conversation_hosts(id).await else {
            return Ok(());
        };
        if hosts.iter().any(|host| host.device == *device) {
            return Ok(());
        }
        let name = match self.control().device(device.clone()).await {
            Ok(Some(record)) => record.name,
            _ => device.to_string(),
        };
        let reason = crate::transition::detached_reason(&name);
        withdrawal.withdraw(&reason);
        Err(HostError::new(HostErrorKind::Unavailable, reason))
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
                shard: self.shard.clone(),
                conversation: conversation.clone(),
                node: scope.node.clone(),
                device,
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
                registration,
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
    fn hold_adopted<'a>(
        &'a self,
        host: &'a HostKey,
        withdrawal: Rc<Withdrawal>,
    ) -> LocalBoxFuture<'a, Option<Box<dyn std::any::Any>>> {
        Box::pin(async move {
            let shard = self.shard.upgrade()?;
            shard.hold_adopted(&self.conversation, host, withdrawal).await
        })
    }

    fn run_job<'a>(
        &'a self,
        host: &'a HostKey,
        cancel: CancellationToken,
        waiting: JobWaiting<'a>,
        withdrawal: Rc<Withdrawal>,
        job: LocalBoxFuture<'a, ()>,
    ) -> LocalBoxFuture<'a, Result<(), HostError>> {
        Box::pin(async move {
            let shard = self
                .shard
                .upgrade()
                .ok_or_else(|| HostError::offline("the backend is shutting down"))?;
            let shown = Shown(waiting);
            let wait = DeviceWait { shown: &shown };
            shard.run_job(&self.conversation, host, &cancel, wait, withdrawal, job).await
        })
    }
}

/// How a job's environment shows it waiting for its device.
struct Shown<'a>(JobWaiting<'a>);

impl ShowWait for Shown<'_> {
    fn shown(&self, host: Option<String>) -> LocalBoxFuture<'_, ()> {
        (self.0.shown)(host)
    }
}

/// Keeps what each job's commands leave when they end, before they read as
/// ended, as blobs of the conversation owner's:
///
/// - The copies of their edits (`crates/agent-store/src/media.rs`), read from
///   the job's Host inside its host access.
/// - Their whole outputs, each with its record in the conversation's database
///   (`crates/backend-database/src/command_outputs.rs`).
struct Keeper {
    /// The shard, through which a stored attachment reaches the pages.
    shard: Weak<dyn HostShard>,
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
    /// Records `command`, which runs as the job `job`, and while `unsent` waits
    /// to start.
    ///
    /// It records the job first, by its device, so a runner's hello finds its
    /// conversation, then the command in the conversation's database
    /// (`crates/backend-database/src/command_outputs.rs`). A command recorded
    /// as waiting is recorded again as sent once its job starts.
    async fn record(&self, command: &CommandId, job: &str, start: &KeptStart, unsent: Option<&UnsentStart>) {
        let indexed = self
            .control
            .record_running_job(job.to_owned(), self.device.clone(), self.conversation.clone(), unsent.is_none())
            .await;
        if let Err(error) = indexed {
            // The command runs all the same; a backend that starts again
            // does not take it up.
            tracing::warn!(event = "job.record_failed", conversation = %self.conversation, %command, kind = "job", error = &error as &dyn std::error::Error, "a running job was not recorded");
            return;
        }
        let running = RunningCommand {
            command: command.clone(),
            node: self.node.clone(),
            device: self.device.clone(),
            job: job.to_owned(),
            tool_use_id: start.caller.tool_use_id.clone(),
            turn: start.caller.turn.clone(),
            title: start.title.clone(),
            script: start.script.clone(),
            service: start.service,
            started: self.clock.now(),
            places: Default::default(),
            unsent: unsent.cloned(),
        };
        let recorded = self
            .db
            .call(move |connection| running::record(connection, &running))
            .await;
        if let Err(error) = recorded {
            tracing::warn!(event = "job.record_failed", conversation = %self.conversation, %command, kind = "command", error = &error as &dyn std::error::Error, "a running command was not recorded");
        }
    }

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

    /// Stores `medium`, the original bytes its job viewed, as the agent's
    /// attachment, the one that has its bytes already or the next, and
    /// answers its id or why it is none; none for a medium whose bytes the
    /// backend does not have.
    async fn attach_medium(&self, command: &CommandId, medium: &CommandMedium) -> Option<Result<String, String>> {
        let bytes = medium.bytes.as_ref().ok()?;
        let stored = self.store_medium(medium, bytes).await;
        match &stored {
            Ok(number) => {
                tracing::info!(event = "medium_attachment.stored", conversation = %self.conversation, %command, medium = medium.number, attachment = %number, "a medium a command viewed is an attachment");
            }
            Err(error) => {
                tracing::warn!(event = "medium_attachment.store_failed", conversation = %self.conversation, %command, medium = medium.number, %error, "a medium a command viewed was not stored as an attachment");
            }
        }
        Some(stored)
    }

    /// Stores `bytes`, `medium`'s, as an attachment and answers its id.
    async fn store_medium(&self, medium: &CommandMedium, bytes: &Bytes) -> Result<String, String> {
        let shard = self.shard.upgrade().ok_or("the backend is stopping")?;
        let blob = self.blobs.put(bytes.clone()).await.map_err(|error| error.to_string())?;
        let attachment = Unnumbered {
            name: medium_name(medium),
            media_type: medium.media_type.clone(),
            size: u64::try_from(bytes.len()).expect("a medium's size fits u64"),
            pixels: medium
                .width
                .zip(medium.height)
                .map(|(width, height)| PixelSize { width, height }),
            blob,
            node: Some(self.node.clone()),
            source: None,
        };
        let announce = {
            let shard = shard.this();
            let conversation = self.conversation.clone();
            move || shard.summary_changed(&conversation)
        };
        let row = commit(shard.tasks(), self.db.clone(), attachment, Numbering::SameBytes, announce).await?;
        Ok(row.number.to_string())
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
/// one request (`crates/agent-store/src/media.rs`): each copy's text, when it
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
    /// conversation's (`crates/backend-database/src/command_outputs.rs`).
    fn started<'a>(&'a self, command: &'a CommandId, job: &'a str, start: &'a KeptStart) -> LocalBoxFuture<'a, ()> {
        Box::pin(self.record(command, job, start, None))
    }

    fn waiting<'a>(
        &'a self,
        command: &'a CommandId,
        job: &'a str,
        start: &'a KeptStart,
        unsent: &'a UnsentStart,
    ) -> LocalBoxFuture<'a, ()> {
        Box::pin(self.record(command, job, start, Some(unsent)))
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
                            tracing::warn!(event = "edit_copies.store_failed", conversation = %self.conversation, %command, path = %file.path, %error, "an edit's copies were not stored");
                        }
                        copies.ok()
                    })
                    .collect();
                retained.push(edited_file(file, |segment| copies[segment].clone()));
            }
            retained
        })
    }

    fn attach<'a>(
        &'a self,
        command: &'a CommandId,
        media: &'a [CommandMedium],
    ) -> LocalBoxFuture<'a, Vec<Option<Result<String, String>>>> {
        Box::pin(async move {
            let mut attachments = Vec::with_capacity(media.len());
            for medium in media {
                attachments.push(self.attach_medium(command, medium).await);
            }
            attachments
        })
    }

    fn stored_blob<'a>(&'a self, blob: &'a BlobRef) -> LocalBoxFuture<'a, Option<Bytes>> {
        Box::pin(async move {
            match self.blobs.get(blob).await {
                Ok(bytes) => bytes,
                // The medium is read from the Host instead.
                Err(error) => {
                    tracing::warn!(event = "blob.read_failed", conversation = %self.conversation, %blob, %error, "a blob was not read");
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
                tracing::warn!(event = "command_place.record_failed", conversation = %self.conversation, %command, error = &error as &dyn std::error::Error, "a place in a command's output was not recorded");
            }
        })
    }

    fn keep_output<'a>(
        &'a self,
        command: &'a CommandId,
        title: &'a str,
        end: CommandEnd,
        output: &'a WholeOutput,
        media: &'a [CommandMedium],
    ) -> LocalBoxFuture<'a, ()> {
        Box::pin(async move {
            let ended = self.clock.now();
            let stored = match self.put(output).await {
                Ok(blob) => OutputRow::Stored {
                    blob,
                    missing: output.missing().cloned(),
                },
                Err(reason) => {
                    tracing::warn!(event = "command_output.store_failed", conversation = %self.conversation, %command, %reason, "a command's output was not stored");
                    OutputRow::NotStored(reason)
                }
            };
            let row = CommandOutput {
                command: command.clone(),
                title: title.to_owned(),
                ended,
                end,
                output: stored,
                media: media.to_vec(),
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
                        tracing::warn!(event = "job_record.remove_failed", conversation = %self.conversation, %command, error = &error as &dyn std::error::Error, "a running job's record was not removed");
                    }
                }
                Ok(None) => {}
                // The command ends all the same; `demi shell output` then
                // finds no record of it.
                Err(error) => {
                    tracing::warn!(event = "command_output.record_failed", conversation = %self.conversation, %command, error = &error as &dyn std::error::Error, "a command's output was not recorded");
                }
            }
        })
    }
}

/// A node's environment, and the registration that answers its jobs' rpc
/// calls with the node's commands while it lives.
struct Registered {
    environment: RemoteShellEnvironment,
    registration: CommandRegistration,
}

impl ShellEnvironment for Registered {
    /// Starts the command; a job whose Host is the conversation's offline
    /// device waits for it in its admission (`crates/backend-host-access/src/access.rs`).
    fn start(
        &self,
        request: ExecRequest,
        cancel: CancellationToken,
    ) -> LocalBoxFuture<'_, Result<CommandId, ShellError>> {
        self.environment.start(request, cancel)
    }

    fn ended<'a>(&'a self, command: &'a CommandId) -> LocalBoxFuture<'a, Result<Ending, ShellError>> {
        self.environment.ended(command)
    }

    fn status(&self, command: &CommandId) -> Result<CommandStatus, ShellError> {
        self.environment.status(command)
    }

    fn title(&self, command: &CommandId) -> Option<String> {
        self.environment.title(command)
    }

    fn quiet(&self, command: &CommandId) -> Result<std::time::Duration, ShellError> {
        self.environment.quiet(command)
    }

    fn outliving(&self, command: &CommandId) -> Vec<String> {
        self.environment.outliving(command)
    }

    fn awaits_user<'a>(
        &'a self,
        command: &'a CommandId,
    ) -> LocalBoxFuture<'a, Option<tokio::sync::watch::Receiver<bool>>> {
        self.environment.awaits_user(command)
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

    fn stop_waiting(&self) -> LocalBoxFuture<'_, ()> {
        self.environment.stop_waiting()
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

    /// The jobs' manifest, then the node's registration, which every
    /// environment of the node shares and each replaces alike.
    fn replace_commands(&self, commands: &Rc<CommandSet>) -> Result<(), HostError> {
        self.environment.replace_commands(commands)?;
        if let Some(selection) = self.environment.selection() {
            self.registration.replace(commands.clone(), selection);
        }
        Ok(())
    }
}

/// The name an attachment of `medium` goes by: its file's, or, for one viewed
/// from stdin, its kind and number with its type's extension, such as
/// `image-1.png`.
fn medium_name(medium: &CommandMedium) -> String {
    if let Some(name) = &medium.name {
        return name.clone();
    }
    match model_media_type_for(&medium.media_type) {
        Some(entry) => format!("{}-{}.{}", entry.kind.name(), medium.number, entry.extension),
        None => format!("medium-{}", medium.number),
    }
}
