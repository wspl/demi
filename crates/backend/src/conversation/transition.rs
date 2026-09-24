//! Changes of a conversation (`web-api.md` § Sidebar mutations, read state
//! and page synchronization; `sessions-and-targets.md` § Switch the main
//! target, § Lifecycle access). Every change goes through one entry,
//! `Shard::transition`. A rename, a pin or a model change waits while a
//! transition holds the conversation, and is then applied as if it arrived
//! afterwards. An archive, a restore, a target switch and a detach are
//! transitions: each holds the conversation, with its idle tree reserved, its
//! file transfers, user streams and the one-shot calls admitted like them
//! ended, and its file gate reserved; sends the conversation release to the
//! devices it leaves; and commits, advancing the execution-context revision
//! when the context changed. A transition waits for no other work: a
//! conversation that is busy refuses it.

use demi_gates::{Purpose, Reservation};
use demi_web_api::conversations::{ConversationPatch, ConversationTarget, ConversationUpdate, FieldResult, PatchField};
use demi_web_api::devices::DeviceKind;
use demi_web_api::error::ErrorCode;
use demi_web_api::ids::ConversationId;

use super::root_of;
use super::target::TargetSwitch;
use super::transfer::TransfersClosed;
use crate::shard::Shard;
use crate::storage::StorageError;
use crate::storage::conversation_index::{
    ChangeOutcome, ConversationChange, ConversationModel, ConversationRecord, RecordChange, SwitchEnds,
};

/// Why a change was not applied.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ChangeRefusal {
    #[error("No such conversation")]
    NotFound,
    #[error("Restore the conversation before changing it")]
    Archived,
    /// Work of the tree is running, or another transition or a Host
    /// operation holds the conversation.
    #[error("A conversation with running work cannot be archived, restored or moved")]
    TurnInFlight,
    #[error("No such provider")]
    ProviderNotFound,
    #[error("No such workspace")]
    WorkspaceNotFound,
    /// The destination names a device the user did not pair.
    #[error("No such device")]
    DeviceNotFound,
    /// Another switch changed the target first.
    #[error("Another target change came first")]
    Conflict,
    /// A device the transition leaves did not take the conversation
    /// release.
    #[error("The conversation release failed: {0}")]
    Release(String),
    /// The device to attach is the conversation's main Host.
    #[error("That device is the conversation's main host")]
    HostIsMain,
    /// The device to rename is not attached.
    #[error("No such attached host")]
    NotAttached,
    #[error("Another attached host has that name")]
    NameTaken,
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
            Self::WorkspaceNotFound => (ErrorCode::WorkspaceNotFound, 404),
            Self::DeviceNotFound => (ErrorCode::DeviceNotFound, 404),
            Self::Conflict => (ErrorCode::TargetConflict, 409),
            Self::HostIsMain => (ErrorCode::HostIsMain, 409),
            Self::NotAttached => (ErrorCode::HostNotAttached, 404),
            Self::NameTaken => (ErrorCode::NameTaken, 409),
            Self::Release(_) | Self::Storage(_) => (ErrorCode::OperationFailed, 500),
        }
    }
}

/// Whether a change of the record is a transition, which holds the
/// conversation: an archive, a restore and a detach. A target switch is one
/// too.
fn change_is_transition(change: &RecordChange) -> bool {
    matches!(change, RecordChange::Archived(_) | RecordChange::Detach(_))
}

/// A conversation held for a transition. Its fields drop in order: the file
/// gate's reservation, then the transfers open again, then the tree's
/// reservation.
struct Hold {
    _files: Reservation,
    _transfers: TransfersClosed,
    _tree: Option<Reservation>,
}

impl Shard {
    /// Applies `change` to the user's conversation `id`.
    pub(crate) async fn transition(&self, id: &ConversationId, change: ConversationChange) -> Result<(), ChangeRefusal> {
        let services = self.services();
        let record = services.control.conversation(id.clone()).await?;
        let Some(record) = record.filter(|record| record.owner == *self.user()) else {
            return Err(ChangeRefusal::NotFound);
        };
        // An archived conversation takes nothing but its restore; the index
        // transaction checks again when it applies the change.
        if record.archived && !matches!(change, ConversationChange::Record(RecordChange::Archived(_))) {
            return Err(ChangeRefusal::Archived);
        }
        match &change {
            ConversationChange::Record(RecordChange::Model(Some(model))) => {
                if services.vault.visible(self.user(), &model.provider).await?.is_none() {
                    return Err(ChangeRefusal::ProviderNotFound);
                }
            }
            // The conversation's own target is no change.
            ConversationChange::Target(to) if record.target == *to => return Ok(()),
            ConversationChange::Target(to) => self.check_destination(&record, to).await?,
            _ => {}
        }
        let change = match change {
            ConversationChange::Record(change) if !change_is_transition(&change) => {
                // A field update waits while a transition holds the
                // conversation, and applies as if it arrived afterwards.
                let _admitted = self.conversations().slot(&record.id).files.enter(Purpose::Demand).await;
                if let RecordChange::Attach(host) = &change {
                    let target = self.resolve_target(&record).await?;
                    if target.device() == Some(&host.device) {
                        return Err(ChangeRefusal::HostIsMain);
                    }
                }
                return self.commit(&record.id, change).await;
            }
            change => change,
        };
        let hold = self.hold(&record.id).await?;
        let committed = match change {
            ConversationChange::Target(to) => self.switch_target(&record, to).await,
            ConversationChange::Record(RecordChange::Archived(true)) => {
                self.release_everywhere(&record).await?;
                let committed = self.commit(&record.id, RecordChange::Archived(true)).await;
                // An archived conversation's title request ends.
                if committed.is_ok() {
                    self.titles().abort(&record.id);
                }
                committed
            }
            ConversationChange::Record(RecordChange::Detach(device)) => {
                let attached = services.control.attached_hosts(record.id.clone()).await?;
                if attached.iter().any(|host| host.device == device) {
                    self.release_on(&record.id, &device)
                        .await
                        .map_err(|error| ChangeRefusal::Release(error.to_string()))?;
                }
                self.commit(&record.id, RecordChange::Detach(device)).await
            }
            ConversationChange::Record(change) => self.commit(&record.id, change).await,
        };
        drop(hold);
        committed
    }

    /// Holds the conversation for a transition (§ Switch the main target,
    /// steps 2 to 4): its idle tree reserved, its transfers and streams
    /// ended and their release awaited, and its file gate reserved. A busy
    /// tree or file gate refuses.
    async fn hold(&self, id: &ConversationId) -> Result<Hold, ChangeRefusal> {
        let tree = self.reserve_idle_tree(id)?;
        let slot = self.conversations().slot(id);
        let transfers = slot.transfers.close().await;
        let files = slot.files.try_reserve().ok_or(ChangeRefusal::TurnInFlight)?;
        Ok(Hold {
            _files: files,
            _transfers: transfers,
            _tree: tree,
        })
    }

    /// Commits a change of the conversation's record.
    async fn commit(&self, id: &ConversationId, change: RecordChange) -> Result<(), ChangeRefusal> {
        match self.services().control.change_conversation(id.clone(), change).await? {
            ChangeOutcome::Applied => Ok(()),
            ChangeOutcome::Missing => Err(ChangeRefusal::NotFound),
            ChangeOutcome::Archived => Err(ChangeRefusal::Archived),
            ChangeOutcome::NotAttached => Err(ChangeRefusal::NotAttached),
            ChangeOutcome::NameTaken => Err(ChangeRefusal::NameTaken),
        }
    }

    /// Whether the user has what `to` names: a workspace of theirs, or a
    /// device they paired.
    async fn check_destination(&self, record: &ConversationRecord, to: &ConversationTarget) -> Result<(), ChangeRefusal> {
        let control = &self.services().control;
        match to {
            ConversationTarget::Workspace { workspace_id } => {
                let workspace = control.workspace(workspace_id.clone()).await?;
                if !workspace.is_some_and(|workspace| workspace.user == record.owner) {
                    return Err(ChangeRefusal::WorkspaceNotFound);
                }
            }
            ConversationTarget::Device { device_id, .. } => {
                let device = control.device(device_id.clone()).await?;
                if !device.is_some_and(|device| device.user == record.owner && device.kind == DeviceKind::User) {
                    return Err(ChangeRefusal::DeviceNotFound);
                }
            }
            ConversationTarget::Cloud { .. } => {}
        }
        Ok(())
    }

    /// The held switch's steps 4 to 6 (§ Switch the main target): the device
    /// it leaves hears the conversation release and stays attached where it
    /// was left, the device it reaches is main alone, and the commit is
    /// against the target the switch started from.
    async fn switch_target(&self, expected: &ConversationRecord, to: ConversationTarget) -> Result<(), ChangeRefusal> {
        let control = &self.services().control;
        let current = control.conversation(expected.id.clone()).await?.ok_or(ChangeRefusal::NotFound)?;
        if current.archived {
            return Err(ChangeRefusal::Archived);
        }
        let from = self.resolve_target(expected).await?;
        let reaching = ConversationRecord {
            target: to.clone(),
            ..expected.clone()
        };
        let destination = self.resolve_target(&reaching).await?;
        let departed = from.device().cloned();
        if let Some(device) = &departed
            && Some(device) != destination.device()
        {
            self.release_on(&expected.id, device)
                .await
                .map_err(|error| ChangeRefusal::Release(error.to_string()))?;
        }
        let ends = SwitchEnds {
            departed: departed.map(|device| (device, from.path().to_owned())),
            arriving: destination.device().cloned(),
        };
        let switch = TargetSwitch { from, to: destination };
        let won = control
            .switch_conversation_target(expected.id.clone(), expected.target.clone(), to, switch, ends)
            .await?;
        if won { Ok(()) } else { Err(ChangeRefusal::Conflict) }
    }

    /// The conversation release on every Host the conversation reaches, for
    /// an archive. Every device is asked; the first that failed fails the
    /// archive.
    async fn release_everywhere(&self, record: &ConversationRecord) -> Result<(), ChangeRefusal> {
        let target = self.resolve_target(record).await?;
        let hosts = self.reachable_hosts(record, &target).await?;
        let mut failure = None;
        for host in hosts {
            if let Err(error) = self.release_on(&record.id, &host.device).await {
                failure.get_or_insert_with(|| ChangeRefusal::Release(error.to_string()));
            }
        }
        failure.map_or(Ok(()), Err)
    }

    /// Reserves the conversation's live tree while it does nothing by itself
    /// (`runtime.md` § Actions): no action runs or waits, no child is live
    /// and no wakeup is scheduled. A conversation without a live tree runs
    /// nothing; one whose tree works refuses the transition.
    fn reserve_idle_tree(&self, id: &ConversationId) -> Result<Option<Reservation>, ChangeRefusal> {
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
            changes.push((PatchField::Archived, RecordChange::Archived(archived).into()));
        }
        if let Some(title) = patch.title {
            changes.push((PatchField::Title, RecordChange::Title(title.into_string()).into()));
        }
        if let Some(pinned) = patch.pinned {
            changes.push((PatchField::Pinned, RecordChange::Pinned(pinned).into()));
        }
        if let Some(model) = patch.model {
            let model = model.map(|choice| ConversationModel {
                provider: choice.provider_id,
                model: choice.model_id,
            });
            changes.push((PatchField::Model, RecordChange::Model(model).into()));
        }
        if let Some(target) = patch.target {
            changes.push((PatchField::Target, ConversationChange::Target(target)));
        }
        let mut results = Vec::new();
        for (field, change) in changes {
            let result = match self.transition(id, change).await {
                Ok(()) => FieldResult::Applied { field },
                Err(refusal) => failed(field, &refusal),
            };
            results.push(result);
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

impl From<RecordChange> for ConversationChange {
    fn from(change: RecordChange) -> Self {
        Self::Record(change)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use demi_web_api::ids::UserId;

    use super::*;
    use crate::auth::sessions::TokenHash;
    use crate::backend::Services;
    use crate::shard::{ShardPlacement, ShardPool};
    use crate::storage::control::testing;
    use crate::storage::conversation_index::Creation;

    const ID: &str = "0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01";

    #[tokio::test(flavor = "local")]
    async fn a_transition_waits_for_no_work_and_a_field_update_waits_for_a_transition() {
        let data = tempfile::tempdir().unwrap();
        let services = Services::start_for_tests(data.path()).await;
        let control = services.control.clone();
        let owner: UserId = testing::master(&control).await.id;
        let id = ConversationId::try_from(ID).unwrap();
        assert!(matches!(
            control.create_conversation(owner.clone(), id.clone()).await.unwrap(),
            Creation::Created(_)
        ));
        let laptop = control
            .create_device(owner.clone(), "laptop".into(), "linux".into(), TokenHash::of("laptop"))
            .await
            .unwrap()
            .id;
        let pool = ShardPool::start(ShardPlacement::Inline, services).await.unwrap();
        let refusals = pool
            .shards()
            .of(&owner)
            .call(move |shard, _| async move {
                let slot = shard.conversations().slot(&id);
                let to = ConversationTarget::Device {
                    device_id: laptop.clone(),
                    path: "/work".into(),
                };
                // A Host operation in flight: a transition refuses rather
                // than wait for it.
                let operation = slot.files.enter(Purpose::Demand).await;
                let mut refusals = Vec::new();
                for change in [
                    ConversationChange::Target(to.clone()),
                    RecordChange::Archived(true).into(),
                    RecordChange::Detach(laptop.clone()).into(),
                ] {
                    refusals.push(shard.transition(&id, change).await.map_err(|refusal| refusal.code().0));
                }
                drop(operation);
                // A transition holds the conversation: a rename waits for it,
                // then applies.
                let held = slot.files.try_reserve().unwrap();
                let renaming = {
                    let shard = shard.clone();
                    let id = id.clone();
                    tokio::task::spawn_local(async move {
                        shard.transition(&id, RecordChange::Title("Renamed".into()).into()).await
                    })
                };
                tokio::time::sleep(Duration::from_millis(50)).await;
                let waited = !renaming.is_finished();
                drop(held);
                renaming.await.unwrap().unwrap();
                let record = shard.owned_conversation(&id).await.unwrap();
                (refusals, waited, record.title)
            })
            .await
            .unwrap();
        let (refusals, waited, title) = refusals;
        assert_eq!(refusals, [Err(ErrorCode::TurnInFlight); 3]);
        assert!(waited);
        assert_eq!(title, "Renamed");
        pool.close().await;
    }
}
