//! Changes of a conversation's record (`web-api.md` § Sidebar mutations,
//! read state and page synchronization; `sessions-and-targets.md` § Switch
//! the main target). Every change goes through one entry, `Shard::transition`:
//! an archive or a restore is a transition, which needs the conversation's
//! tree idle and waits for no work; a rename, a pin or a model change is
//! applied once no transition holds the conversation.

use demi_web_api::conversations::{ConversationPatch, ConversationTarget, ConversationUpdate, FieldResult, PatchField};
use demi_web_api::error::ErrorCode;
use demi_web_api::ids::ConversationId;

use super::root_of;
use crate::shard::Shard;
use crate::storage::StorageError;
use crate::storage::conversation_index::{ChangeOutcome, ConversationChange, ConversationModel};

/// Why a change was not applied.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ChangeRefusal {
    #[error("No such conversation")]
    NotFound,
    #[error("Restore the conversation before changing it")]
    Archived,
    /// Work of the tree is running, or another transition holds the
    /// conversation.
    #[error("A conversation with running work cannot be archived or restored")]
    TurnInFlight,
    #[error("No such provider")]
    ProviderNotFound,
    #[error(transparent)]
    Storage(#[from] StorageError),
}

impl ChangeRefusal {
    /// The code and HTTP status the refusal answers with.
    pub(crate) fn code(&self) -> (ErrorCode, u16) {
        match self {
            Self::NotFound => (ErrorCode::ConversationNotFound, 404),
            Self::Archived => (ErrorCode::ConversationArchived, 409),
            Self::TurnInFlight => (ErrorCode::TurnInFlight, 409),
            Self::ProviderNotFound => (ErrorCode::ProviderNotFound, 404),
            Self::Storage(_) => (ErrorCode::OperationFailed, 500),
        }
    }
}

impl ConversationChange {
    /// Whether the change is a transition, which needs the tree idle.
    fn is_transition(&self) -> bool {
        matches!(self, Self::Archived(_))
    }
}

impl Shard {
    /// Applies `change` to the user's conversation `id`: a transition
    /// reserves the conversation's idle tree, so that no action starts until
    /// the change is committed, and refuses running work; the change itself
    /// is one index transaction. Interim: the conversation's host access
    /// completes this with its steps, which end the conversation's file
    /// transfers, user streams and one-shot calls, reserve its file gate (a
    /// field update enters it, waiting while a transition holds it), send
    /// the conversation release and advance the context revision.
    pub(crate) async fn transition(&self, id: &ConversationId, change: ConversationChange) -> Result<(), ChangeRefusal> {
        let services = self.services();
        let record = services.control.conversation(id.clone()).await?;
        let Some(record) = record.filter(|record| record.owner == *self.user()) else {
            return Err(ChangeRefusal::NotFound);
        };
        // An archived conversation refuses as archived before anything else;
        // the index transaction checks it again when it applies the change.
        if record.archived && !change.is_transition() {
            return Err(ChangeRefusal::Archived);
        }
        if let ConversationChange::Model(Some(model)) = &change
            && services.vault.visible(self.user(), &model.provider).await?.is_none()
        {
            return Err(ChangeRefusal::ProviderNotFound);
        }
        // The tree's admission, held until the change commits.
        let _idle = if change.is_transition() {
            self.reserve_idle_tree(id)?
        } else {
            None
        };
        let archives = change == ConversationChange::Archived(true);
        match services.control.change_conversation(id.clone(), change).await? {
            ChangeOutcome::Applied => {
                // An archived conversation's title request ends.
                if archives {
                    self.titles().abort(&record.id);
                }
                Ok(())
            }
            ChangeOutcome::Missing => Err(ChangeRefusal::NotFound),
            ChangeOutcome::Archived => Err(ChangeRefusal::Archived),
        }
    }

    /// Reserves the conversation's live tree while it does nothing by itself
    /// (`runtime.md` § Actions): no action runs or waits, no child is live
    /// and no wakeup is scheduled. A conversation without a live tree runs
    /// nothing; one whose tree works refuses the transition.
    fn reserve_idle_tree(&self, id: &ConversationId) -> Result<Option<demi_gates::Reservation>, ChangeRefusal> {
        let Some(tree) = self.agent().tree(&root_of(id)) else {
            return Ok(None);
        };
        // The check and the reservation are one step: no await between them.
        if !tree.is_quiescent() {
            return Err(ChangeRefusal::TurnInFlight);
        }
        tree.admission().try_reserve().map(Some).ok_or(ChangeRefusal::TurnInFlight)
    }
}

impl Shard {
    /// Applies each field of `patch` to the user's conversation `id` on its
    /// own, the archive first, so archiving and renaming together archives
    /// and refuses the rename; a field applied stays applied whatever the
    /// others do. None when the user has no such conversation.
    pub(crate) async fn apply_patch(
        &self,
        id: &ConversationId,
        patch: ConversationPatch,
    ) -> Result<Option<ConversationUpdate>, StorageError> {
        let control = &self.services().control;
        let owned = control.conversation(id.clone()).await?.filter(|record| record.owner == *self.user());
        if owned.is_none() {
            return Ok(None);
        }
        let mut changes = Vec::new();
        if let Some(archived) = patch.archived {
            changes.push((PatchField::Archived, ConversationChange::Archived(archived)));
        }
        if let Some(title) = patch.title {
            changes.push((PatchField::Title, ConversationChange::Title(title.into_string())));
        }
        if let Some(pinned) = patch.pinned {
            changes.push((PatchField::Pinned, ConversationChange::Pinned(pinned)));
        }
        if let Some(model) = patch.model {
            let model = model.map(|choice| ConversationModel {
                provider: choice.provider_id,
                model: choice.model_id,
            });
            changes.push((PatchField::Model, ConversationChange::Model(model)));
        }
        let mut results = Vec::new();
        for (field, change) in changes {
            let result = match self.transition(id, change).await {
                Ok(()) => FieldResult::Applied { field },
                Err(refusal) => failed(field, &refusal),
            };
            results.push(result);
        }
        if let Some(target) = patch.target {
            results.push(self.interim_target_change(id, &target).await?);
        }
        let Some(record) = control.conversation(id.clone()).await? else {
            return Ok(None);
        };
        let conversation = self.conversation_summary(record).await?;
        Ok(Some(ConversationUpdate { conversation, results }))
    }
}

fn failed(field: PatchField, refusal: &ChangeRefusal) -> FieldResult {
    if let ChangeRefusal::Storage(error) = refusal {
        tracing::error!(?field, error = error as &dyn std::error::Error, "a conversation change failed");
    }
    let (code, http_status) = refusal.code();
    FieldResult::Failed {
        field,
        code,
        message: refusal.to_string(),
        http_status,
    }
}

impl Shard {
    /// The result of a patch's `target`: the selection the conversation has
    /// already is no change, and an archived conversation takes none.
    /// Interim: the target switch, a transition of the conversation's host
    /// access, replaces this function, which refuses every other target.
    async fn interim_target_change(
        &self,
        id: &ConversationId,
        target: &ConversationTarget,
    ) -> Result<FieldResult, StorageError> {
        let Some(record) = self.services().control.conversation(id.clone()).await? else {
            return Ok(failed(PatchField::Target, &ChangeRefusal::NotFound));
        };
        if record.archived {
            return Ok(failed(PatchField::Target, &ChangeRefusal::Archived));
        }
        if record.target == *target {
            return Ok(FieldResult::Applied {
                field: PatchField::Target,
            });
        }
        Ok(FieldResult::Failed {
            field: PatchField::Target,
            code: ErrorCode::OperationFailed,
            message: "Target changes are not available on this backend yet".into(),
            http_status: 500,
        })
    }
}
