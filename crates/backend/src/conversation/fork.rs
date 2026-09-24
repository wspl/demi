//! Conversation Fork on the backend (`conversation-fork.md` § Backend
//! creation and retries): a new conversation holding the source's history
//! through one of its completed assistant texts. The creation reserves its
//! destination's id in the control store, the agent prepares the seed, the
//! kept edits of its retained shell calls are copied into the destination's
//! namespace, the agent commits the destination's root in the destination's
//! own database, and a control transaction then publishes the destination. Requests for one
//! destination run one at a time, a retry of the same attempt finds its
//! destination, and startup publishes a destination whose root committed
//! before its publication.

use demi_agent::ForkError;
use demi_core::{BlockId, ModelSelection};
use demi_web_api::conversations::ConversationTarget;
use demi_web_api::ids::ConversationId;

use super::root_of;
use crate::shard::Shard;
use crate::storage::StorageError;
use crate::storage::control::ControlService;
use crate::storage::conversation_index::ConversationRecord;
use crate::storage::conversations::ConversationStores;
use crate::storage::forks::{ForkMetadata, ForkOperation};
use crate::storage::tree;

/// A Fork's destination, and whether this request created it.
pub(crate) struct Forked {
    pub(crate) record: ConversationRecord,
    pub(crate) created: bool,
    /// The model selection the destination inherited.
    pub(crate) model: ModelSelection,
}

/// Why a Fork was refused.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ForkRefusal {
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
    pub(crate) async fn fork(
        &self,
        source: ConversationId,
        destination: ConversationId,
        block: BlockId,
    ) -> Result<Forked, ForkRefusal> {
        // Ids compare without case, so one destination takes one turn.
        let _turn = self.forks().acquire(destination.as_str().to_ascii_lowercase()).await;
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
            let operation = reserved.ok_or(ForkRefusal::Unavailable)?;
            return Ok(Forked {
                record: existing,
                created: false,
                model: operation.metadata.model,
            });
        }
        if let Some(operation) = &reserved {
            // A retry of an attempt whose root committed before it was
            // published publishes it; one that did not commit starts again.
            let committed = services.conversations.read(&destination, tree::has_root).await?;
            if committed == Some(true) {
                let record = control.publish_fork(destination).await?;
                return Ok(Forked {
                    record,
                    created: false,
                    model: operation.metadata.model.clone(),
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
                // A Cloud target keeps the source's resolved directory.
                let target = match &source.target {
                    ConversationTarget::Cloud { path: None } => ConversationTarget::Cloud {
                        path: Some(self.resolve_target(&source).await?.path().to_owned()),
                    },
                    target => target.clone(),
                };
                let operation = ForkOperation {
                    id: destination.clone(),
                    owner: self.user().clone(),
                    source: source.id.clone(),
                    block,
                    metadata: ForkMetadata {
                        title: format!("{}{TITLE_SUFFIX}", source.title),
                        target,
                        model: seed.state.model.clone(),
                        created_at: services.clock.now(),
                        attached_hosts: control.attached_hosts(source.id.clone()).await?,
                    },
                };
                control
                    .reserve_fork(operation)
                    .await?
                    .ok_or(ForkRefusal::Unavailable)?
            }
        };
        // The edits the retained shell calls kept are the destination's too,
        // before its root commits; a retry copies them again.
        services
            .changes
            .fork(&source.id, &destination, &seed.transcript)
            .await?;
        // A retry creates the destination with the selection its attempt
        // recorded.
        seed.state.model = operation.metadata.model.clone();
        self.agent()
            .initialize_fork(&root_of(&destination), seed)
            .await
            .map_err(refused)?;
        let record = control.publish_fork(destination).await?;
        Ok(Forked {
            record,
            created: true,
            model: operation.metadata.model,
        })
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

/// Publishes the reserved destinations whose root committed before they
/// were published (`backend.md` § Startup and shutdown); the others stay
/// hidden until their attempt is retried. It runs before the backend serves.
pub(crate) async fn recover_forks(
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

    use demi_agent::AgentTreeStore as _;
    use demi_agent::store::{CheckpointState, CheckpointUpdate, NodeRecord};
    use demi_agent::testing::test_model;
    use demi_core::{SessionPhase, Timestamp};

    use super::*;
    use crate::auth::sessions::TokenHash;
    use crate::storage::blobs::{BlobStores, UserBlobs};
    use crate::storage::control::testing;
    use crate::storage::conversation_index::{AttachedHostRecord, ConversationModel, RecordChange};
    use crate::storage::objects;
    use crate::storage::tree::SqliteTreeStore;

    fn conversation(id: &str) -> ConversationId {
        ConversationId::try_from(id).unwrap()
    }

    /// A destination root as a Fork's initialization commits it, with no
    /// history, so no media.
    async fn commit_root(stores: &ConversationStores, blobs: &UserBlobs, id: &ConversationId, at: Timestamp) {
        let state = CheckpointState {
            phase: SessionPhase::Idle,
            queue: Vec::new(),
            agent_inputs: Vec::new(),
            wakeups: Vec::new(),
            cwd: "/work".into(),
            model: test_model(),
            harness: "coding".into(),
            edits: Vec::new(),
        };
        let initial = CheckpointUpdate {
            state,
            command_state: None,
            changed_blocks: Vec::new(),
            block_count: 0,
        };
        SqliteTreeStore::new(stores.db(id), blobs.clone())
            .create_node(NodeRecord::root(root_of(id), at), initial)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn startup_publishes_a_fork_whose_root_committed_and_keeps_an_uncommitted_one_hidden() {
        let data = tempfile::tempdir().unwrap();
        let control = ControlService::open(&data.path().join("control.sqlite"), Arc::new(demi_core::SystemClock))
            .await
            .unwrap();
        let stores = ConversationStores::open(data.path().join("conversations"), NonZeroUsize::new(4).unwrap())
            .await
            .unwrap();
        let master = testing::master(&control).await.id;
        let source = conversation("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b");
        control.create_conversation(master.clone(), source.clone()).await.unwrap();
        let mut devices = Vec::new();
        for (name, token) in [("laptop", "one"), ("ci", "two")] {
            let device = control
                .create_device(master.clone(), name.into(), "linux".into(), TokenHash::of(token))
                .await
                .unwrap();
            let host = AttachedHostRecord {
                device: device.id.clone(),
                name: name.into(),
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
                title: "New conversation (Fork)".into(),
                target: ConversationTarget::Cloud {
                    path: Some(format!("/home/demi/sessions/{source}")),
                },
                model: test_model(),
                created_at,
                attached_hosts: attached.clone(),
            },
        };
        let committed = conversation("7d1c2e3f-4a5b-4c1e-9d2b-0b6f7f3e8f3a");
        let uncommitted = conversation("5a4b3c2d-1e0f-4a1b-8c2d-3e4f5a6b7c8d");
        for id in [&committed, &uncommitted] {
            control.reserve_fork(operation(id)).await.unwrap().expect("the id is free");
        }
        // One root commits, and the backend stops before it publishes the
        // destination; a device is revoked meanwhile.
        let blobs = BlobStores::new(objects::open(data.path(), None).await.unwrap()).for_user(&master);
        commit_root(&stores, &blobs, &committed, created_at).await;
        control.delete_device(devices[1].clone()).await.unwrap();

        // A second start finds nothing more to publish.
        for _ in 0..2 {
            recover_forks(&control, &stores).await.unwrap();
        }

        let published = control.conversation(committed.clone()).await.unwrap().expect("published");
        assert_eq!(
            (published.title.as_str(), &published.target, published.pinned, published.archived),
            ("New conversation (Fork)", &operation(&committed).metadata.target, false, false)
        );
        assert_eq!(
            published.model,
            Some(ConversationModel {
                provider: "stub".try_into().unwrap(),
                model: "test-model".into(),
            })
        );
        assert_eq!(published.created_at, created_at);
        // The revoked device is left out; the other keeps its name and cwd.
        let kept: Vec<AttachedHostRecord> = attached.into_iter().filter(|host| host.device == devices[0]).collect();
        assert_eq!(control.attached_hosts(committed.clone()).await.unwrap(), kept);
        let listed: Vec<ConversationId> = control
            .conversations(master.clone(), false)
            .await
            .unwrap()
            .into_iter()
            .map(|record| record.id)
            .collect();
        assert_eq!(listed, [committed.clone(), source], "the destination first in the sidebar");
        assert!(control.conversation(uncommitted.clone()).await.unwrap().is_none());
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
