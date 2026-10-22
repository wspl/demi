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
//! 5. The agent commits the destination's root in the destination's own
//!    database.
//! 6. A control transaction publishes the destination.
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
        // history, did: its replies show them still. Another agent's stay that
        // agent's, which no reply here shows.
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
            self.mark(Part::Workspaces);
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

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;
    use std::sync::Arc;

    use demi_agent_store::{
        AgentTreeStore as _, CheckpointState, CheckpointUpdate, NodeRecord, testing::test_model,
    };
    use demi_shared_types::{SessionPhase, Timestamp};

    use super::*;
    use demi_backend_blobs::blobs::{BlobStores, UserBlobs};
    use demi_backend_blobs::store as objects;
    use demi_backend_database::accounts::TokenHash;
    use demi_backend_database::control::testing;
    use demi_backend_database::conversation_index::{AttachedHostRecord, RecordChange};
    use demi_backend_database::tree::SqliteTreeStore;
    use demi_runner_protocol::wire::RunnerPlatform;

    fn conversation(id: &str) -> ConversationId {
        ConversationId::try_from(id).unwrap()
    }

    /// A destination root as a Fork's initialization commits it, with no
    /// history, so no media.
    async fn commit_root(
        stores: &ConversationStores,
        blobs: &UserBlobs,
        id: &ConversationId,
        at: Timestamp,
    ) {
        let state = CheckpointState {
            phase: SessionPhase::Idle,
            queue: Vec::new(),
            agent_inputs: Vec::new(),
            reports: Vec::new(),
            next_turn_reports: Vec::new(),
            watches: Vec::new(),
            model: test_model(),
            edits: Vec::new(),
            account: None,
            turn_speed: None,
        };
        let initial = CheckpointUpdate {
            state,
            changed_blocks: Vec::new(),
            block_count: 0,
            moved_from: None,
            version: false,
        };
        let blobs = Arc::new(blobs.clone());
        SqliteTreeStore::new(
            stores.db(id),
            blobs,
            std::rc::Rc::new(|_: &demi_shared_types::NodeId, _: Option<demi_backend_database::search::TranscriptChange>| {}),
        )
        .create_node(NodeRecord::root(root_of(id), at), initial)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn startup_publishes_a_fork_whose_root_committed_and_keeps_an_uncommitted_one_hidden() {
        let data = tempfile::tempdir().unwrap();
        let control = ControlService::open(
            &data.path().join("control.sqlite"),
            Arc::new(demi_shared_types::SystemClock),
            std::sync::Arc::new(demi_backend_database::providers::NoSealedConfigs),
        )
        .await
        .unwrap();
        // New databases, which no migration reads blobs for.
        struct NoBlobs;
        impl demi_backend_database::conversations::ConversationBlobs for NoBlobs {
            fn read(
                &self,
                _: &ConversationId,
                _: &demi_shared_types::BlobRef,
            ) -> Result<Option<bytes::Bytes>, String> {
                Ok(None)
            }
        }
        let stores = ConversationStores::open(
            data.path().join("conversations"),
            NonZeroUsize::new(4).unwrap(),
            Arc::new(NoBlobs),
        )
        .await
        .unwrap();
        let master = testing::master(&control).await.id;
        let source = conversation("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b");
        control
            .create_conversation(master.clone(), source.clone(), demi_backend_database::conversation_index::ConversationStart::default())
            .await
            .unwrap();
        let mut devices = Vec::new();
        for (name, token) in [("laptop", "one"), ("ci", "two")] {
            let device = control
                .create_device(
                    master.clone(),
                    name.into(),
                    RunnerPlatform::Linux,
                    TokenHash::of(token),
                )
                .await
                .unwrap();
            let host = AttachedHostRecord {
                device: device.id.clone(),
                cwd: Some(format!("/{name}")),
            };
            control
                .change_conversation(source.clone(), RecordChange::Attach(host))
                .await
                .unwrap();
            devices.push(device.id);
        }
        let attached = control.attached_hosts(source.clone()).await.unwrap();
        let created_at: Timestamp = "2026-09-24T08:00:00Z".parse().unwrap();
        let operation = |id: &ConversationId| ForkOperation {
            id: id.clone(),
            owner: master.clone(),
            source: source.clone(),
            block: BlockId::try_from("text-1").unwrap(),
            metadata: ForkMetadata {
                title: Some("Notes (Fork)".into()),
                target: ConversationTarget::Cloud {
                    path: Some(format!("/home/demi/sessions/{source}")),
                },
                model: Some(test_model()),
                workflow: demi_shared_types::WorkflowChoice::Default,
                created_at,
                attached_hosts: attached.iter().map(|host| host.record()).collect(),
                context_version: 5,
            },
        };
        let committed = conversation("7d1c2e3f-4a5b-4c1e-9d2b-0b6f7f3e8f3a");
        let uncommitted = conversation("5a4b3c2d-1e0f-4a1b-8c2d-3e4f5a6b7c8d");
        for id in [&committed, &uncommitted] {
            control
                .reserve_fork(operation(id))
                .await
                .unwrap()
                .expect("the id is free");
        }
        // One root commits, and the backend stops before it publishes the
        // destination; a device is revoked meanwhile.
        let blobs = BlobStores::new(
            objects::open(data.path(), &objects::Storage::Local)
                .await
                .unwrap()
                .store,
            std::sync::Arc::new(demi_shared_types::SystemClock),
        )
        .for_user(&master);
        commit_root(&stores, &blobs, &committed, created_at).await;
        control.delete_device(devices[1].clone()).await.unwrap();

        // A second start finds nothing more to publish.
        for _ in 0..2 {
            recover_forks(&control, &stores).await.unwrap();
        }

        let published = control
            .conversation(committed.clone())
            .await
            .unwrap()
            .expect("published");
        assert_eq!(
            (
                published.title.as_deref(),
                &published.target,
                published.pinned,
                published.archived
            ),
            (
                Some("Notes (Fork)"),
                &Selection::Cloud {
                    path: Some(format!("/home/demi/sessions/{source}"))
                },
                false,
                false
            )
        );
        assert_eq!(published.model, Some(test_model()));
        assert_eq!(published.created_at, created_at);
        // It starts at its source's revision at the Fork.
        assert_eq!(published.context_version, 5);
        // The revoked device is left out; the other keeps its name and cwd.
        let kept: Vec<_> = attached
            .into_iter()
            .filter(|host| host.device == devices[0])
            .collect();
        assert_eq!(
            control.attached_hosts(committed.clone()).await.unwrap(),
            kept
        );
        let listed: Vec<ConversationId> = control
            .conversations(master.clone(), false)
            .await
            .unwrap()
            .into_iter()
            .map(|record| record.id)
            .collect();
        assert_eq!(
            listed,
            [committed.clone(), source],
            "the destination first in the sidebar"
        );
        assert!(
            control
                .conversation(uncommitted.clone())
                .await
                .unwrap()
                .is_none()
        );
        let pending: Vec<ConversationId> = control
            .pending_forks()
            .await
            .unwrap()
            .into_iter()
            .map(|operation| operation.id)
            .collect();
        assert_eq!(pending, [uncommitted]);
        control.close().await.unwrap();
    }
}
