//! Conversation Fork on the backend: a new conversation holding the source's
//! history through one of its completed assistant texts.
//!
//! # Steps
//!
//! 1. The creation reserves its destination's id in the control store.
//! 2. The agent prepares the seed.
//! 3. The kept edits of its retained shell calls are copied into the
//!    destination's namespace.
//! 4. The outputs of its commands and the conversation's attachments are copied
//!    into the destination's database.
//! 5. The plugins that keep something of a conversation are told of the
//!    fork, as its copies are made: what they keep of the source is the
//!    destination's too.
//! 6. The agent commits the destination's root in the destination's own
//!    database.
//! 7. A control transaction publishes the destination.
//!
//! A committed root is a destination whose plugins were told. A fork holds
//! the user's lifecycle gate from reading its source until its root
//! commits, so a deletion of the source reaches the plugins either before
//! the fork finds its source or after they learned of the fork, never
//! between.
//!
//! Requests for one destination run one at a time, a retry of the same attempt
//! finds its destination, and startup publishes a destination whose root
//! committed before its publication.
//!
//! # Publication
//!
//! The control metadata and the conversation's history live in different SQLite
//! files, so no transaction spans them. The reservation makes the attempt
//! durable and the public conversation row is the publication boundary.
//!
//! - Listing and opening show the destination only after its root checkpoint is
//!   committed.
//! - A failure never publishes an empty or partial conversation.
//! - A retry after a lost confirmation must create no second conversation and
//!   recapture no newer data over a committed destination.
//!
//! # What a Fork copies
//!
//! A Fork copies history and configuration, not live state. The destination
//! starts idle, with no queue, pending steer, edit receipt, child, running
//! command, permission grant or request (the user decides for the new
//! conversation again).
//!
//! Attached hosts are copied as configuration, but a device revoked before
//! publication is left out, as its revocation detached it from the source.
//!
//! The destination's execution-context revision starts where the source's
//! stood, so the context blocks its copied transcript holds are older than any
//! announced later, its first switch included.

use demi_backend_database::Selection;
use demi_agent_session::ForkError;
use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;
use demi_backend_database::conversation_index::{AttachedHost, ConversationRecord};
use demi_backend_database::conversations::ConversationStores;
use demi_backend_database::forks::{ForkMetadata, ForkOperation};
use demi_backend_database::tree;
use demi_backend_database::{command_outputs, conversation_attachments, sequences};
use demi_backend_page_sync::Part;
use demi_shared_types::BlockId;
use demi_web_api_protocol::conversations::ConversationTarget;
use demi_web_api_protocol::ids::ConversationId;

use crate::shard::Shard;
use demi_backend_host_access::root_of;

/// A Fork's destination, and whether this request created it.
pub struct Forked {
    pub record: ConversationRecord,
    pub created: bool,
}

/// Why a Fork was refused.
#[derive(Debug, thiserror::Error)]
pub enum ForkRefusal {
    #[error("No such conversation")]
    SourceNotFound,
    #[error("The Fork's id belongs to another creation attempt")]
    Conflict,
    #[error("Conversation id is unavailable")]
    Unavailable,
    /// The text is not a completed assistant text of the source's history.
    #[error("{0}")]
    Target(String),
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// A plugin that keeps something of a conversation did not learn of
    /// the fork.
    #[error("{0}")]
    Plugin(String),
    /// The agent could not store the destination's root.
    #[error("{0}")]
    Failed(String),
}

/// The suffix every Fork appends to its source's title.
const TITLE_SUFFIX: &str = " (Fork)";

impl Shard {
    /// Forks the user's conversation `source` after its assistant text
    /// `block` into the conversation `destination`.
    pub async fn fork(
        &self,
        source: ConversationId,
        destination: ConversationId,
        block: BlockId,
    ) -> Result<Forked, ForkRefusal> {
        // Ids compare without case, so one destination takes one turn.
        let _turn = self
            .forks()
            .acquire(destination.as_str().to_ascii_lowercase())
            .await;
        let _lifecycle = self.lifecycle().acquire().await;
        let services = self.services();
        let control = &services.control;
        let source = control
            .conversation(source)
            .await?
            .filter(|record| record.owner == *self.user())
            .ok_or(ForkRefusal::SourceNotFound)?;
        let reserved = control.fork_operation(destination.clone()).await?;
        if let Some(operation) = &reserved
            && !operation.same_attempt(self.user(), &source.id, &block)
        {
            return Err(ForkRefusal::Conflict);
        }
        if let Some(existing) = control.conversation(destination.clone()).await? {
            // A published destination, found again by a retry.
            if reserved.is_none() {
                return Err(ForkRefusal::Unavailable);
            }
            return Ok(Forked {
                record: existing,
                created: false,
            });
        }
        if reserved.is_some() {
            // A retry of an attempt whose root committed before it was
            // published publishes it; one that did not commit starts again.
            let committed = services
                .conversations
                .read(&destination, tree::has_root)
                .await?;
            if committed == Some(true) {
                let (record, made) = control.publish_fork(destination).await?;
                self.published(&record, made.is_some());
                return Ok(Forked {
                    record,
                    created: false,
                });
            }
        }
        let mut seed = self
            .agent()
            .prepare_fork(&root_of(&source.id), &block)
            .await
            .map_err(refused)?;
        let operation = match reserved {
            Some(operation) => operation,
            None => {
                // The destination keeps the source's directory: on the
                // Cloud, the source's own.
                let resolved = self.host_shard().resolve_target(&source).await?;
                let target = match &source.target {
                    Selection::Cloud { path: None } => ConversationTarget::Cloud {
                        path: Some(resolved.path().to_owned()),
                    },
                    selection => selection.shown(&resolved),
                };
                let operation = ForkOperation {
                    id: destination.clone(),
                    owner: self.user().clone(),
                    source: source.id.clone(),
                    block,
                    metadata: ForkMetadata {
                        title: source.title.as_ref().map(|title| format!("{title}{TITLE_SUFFIX}")),
                        target,
                        // The destination inherits the source's model
                        // settings, the selection its record holds.
                        model: source.model.clone(),
                        // And its workflow.
                        workflow: source.workflow.clone(),
                        created_at: services.clock.now(),
                        attached_hosts: control
                            .attached_hosts(source.id.clone())
                            .await?
                            .iter()
                            .map(AttachedHost::record)
                            .collect(),
                        context_version: source.context_version,
                    },
                };
                control
                    .reserve_fork(operation)
                    .await?
                    .ok_or(ForkRefusal::Unavailable)?
            }
        };
        // The destination's blocks reference the same blobs as the source's,
        // media and edit copies alike, so no bytes are copied.
        //
        // The outputs of the commands its history names are its too, which `demi
        // shell output` reads there as in the source; a retry copies them again.
        // The commands of the rows the seed names are read where they are, in the
        // source's database, as the rows are copied there.
        let mut commands = command_outputs::commands_of(&seed.checkpoint.transcript);
        let prefix = seed.prefix.clone();
        let rows = services
            .conversations
            .read(&source.id, move |connection| {
                if let Some(prefix) = &prefix {
                    for command in command_outputs::commands_of_prefix(connection, prefix)? {
                        if !commands.contains(&command) {
                            commands.push(command);
                        }
                    }
                }
                command_outputs::rows(connection, &commands)
            })
            .await?
            .unwrap_or_default();
        if !rows.is_empty() {
            services
                .conversations
                .db(&destination)
                .call(move |connection| command_outputs::insert(connection, &rows))
                .await?;
        }
        // The attachments its messages name are its too, with the blobs shared
        // (`crates/backend-host-access/src/attachment_commands.rs`). They are read
        // before the sequences, so every number copied lies below the next one.
        //
        // The source's root uploaded what the destination's root, which holds its
        // history, did; another agent's uploads stay recorded under that agent.
        let (source_root, destination_root) = (root_of(&source.id), root_of(&destination));
        let attachments: Vec<_> = services
            .conversations
            .read(&source.id, conversation_attachments::all)
            .await?
            .unwrap_or_default()
            .into_iter()
            .map(|mut stored| {
                if stored.node.as_ref() == Some(&source_root) {
                    stored.node = Some(destination_root.clone());
                }
                stored
            })
            .collect();
        if !attachments.is_empty() {
            services
                .conversations
                .db(&destination)
                .call(move |connection| {
                    let transaction = connection.transaction()?;
                    conversation_attachments::insert(&transaction, &attachments)?;
                    transaction.commit()?;
                    Ok(())
                })
                .await?;
        }
        // Its sequences go on from the source's, read after the seed and its
        // records, so a number its history names is never given to
        // something new.
        let numbers = services
            .conversations
            .read(&source.id, sequences::all)
            .await?
            .unwrap_or_default();
        services
            .conversations
            .db(&destination)
            .call(move |connection| sequences::continue_from(connection, &numbers))
            .await?;
        // The destination's root starts with the selection its record will
        // hold, the one the attempt recorded, which a retry repeats; a source
        // without one leaves the seed its own.
        if let Some(model) = &operation.metadata.model {
            seed.checkpoint.state.model = model.clone();
        }
        self.fork_reaches_plugins(&source.id, &destination).await?;
        self.agent()
            .initialize_fork(&root_of(&destination), seed)
            .await
            .map_err(refused)?;
        let (record, made) = control.publish_fork(destination).await?;
        self.published(&record, made.is_some());
        Ok(Forked {
            record,
            created: true,
        })
    }

    /// Shows a Fork's destination, just published, on the user's pages,
    /// with the project its directory made, if it `made` one.
    fn published(&self, record: &ConversationRecord, made: bool) {
        if made {
            self.mark(Part::Projects);
        }
        self.mark(Part::Conversation(record.id.clone()));
        self.mark(Part::ConversationOrder);
    }
}

/// A Fork error of the agent as the refusal the route answers: the text,
/// the source or the seed is not one a Fork starts from, or the store
/// failed.
fn refused(error: ForkError) -> ForkRefusal {
    match error {
        ForkError::Store(message) => ForkRefusal::Failed(message),
        other => ForkRefusal::Target(other.to_string()),
    }
}

/// Publishes the reserved destinations whose root committed before they were
/// published (`crates/backend/src/backend.rs`); the others stay hidden until
/// their attempt is retried. It runs before the backend serves.
pub async fn recover_forks(
    control: &ControlService,
    conversations: &ConversationStores,
) -> Result<(), StorageError> {
    for operation in control.pending_forks().await? {
        let committed = conversations.read(&operation.id, tree::has_root).await?;
        if committed == Some(true) {
            control.publish_fork(operation.id).await?;
        }
    }
    Ok(())
}

impl Shard {
    /// Tells the plugins that keep something of a conversation that `source`
    /// is forked into `destination`, whose history names what the source's
    /// does; telling one again, as a retry does, changes nothing it did.
    pub(crate) async fn fork_reaches_plugins(
        &self,
        source: &ConversationId,
        destination: &ConversationId,
    ) -> Result<(), ForkRefusal> {
        self.plugins().conversation_forked(source, destination).await.map_err(|error| {
            tracing::error!(event = "conversation_fork.plugins_failed", conversation = %destination, %error, "a plugin did not learn of a fork, which fails");
            ForkRefusal::Plugin(error.to_string())
        })
    }
}
