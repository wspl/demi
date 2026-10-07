//! The transitions that end a conversation's target
//! (`sessions-and-targets.md` § Switch the primary target, § Lifecycle
//! access): a target switch, an archive and a detach. Each runs while the
//! conversation is held for it (`hold_for_transition`): the idle tree
//! reserved by the shard, the file transfers, user streams and the one-shot
//! calls admitted like them ended, and the file gate reserved. It sends the
//! conversation release to the devices it leaves, then commits, whether the
//! devices took the release or not (`resource-lifecycle.md` § A release that
//! fails). The shard's `transition` is the one entry every change of a
//! conversation goes through.

use demi_backend_database::StorageError;
use demi_backend_database::conversation_index::{
    ChangeOutcome, ConversationRecord, RecordChange, SwitchEnds, TargetSwitch,
};
use demi_shared_gates::Reservation;
use demi_web_api_protocol::conversations::ConversationTarget;
use demi_web_api_protocol::devices::DeviceKind;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::{ConversationId, DeviceId, UserId};

use crate::HostShard;
use crate::transfer::TransfersClosed;

/// Why a change was not applied.
#[derive(Debug, thiserror::Error)]
pub enum ChangeRefusal {
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
    /// The entry's catalog does not list the model.
    #[error("The provider does not list that model")]
    ModelNotFound,
    #[error(transparent)]
    SettingUnavailable(#[from] demi_shared_types::UnavailableSetting),
    /// An effort or a tier for a conversation without a model.
    #[error("Choose a model for the conversation first")]
    ModelNotSelected,
    /// The live tree could not get a runtime for the new model.
    #[error("The model cannot run: {0}")]
    Runtime(String),
    #[error("No such workspace")]
    WorkspaceNotFound,
    /// The destination names a device the user did not pair.
    #[error("No such device")]
    DeviceNotFound,
    /// Another switch changed the target first.
    #[error("Another target change came first")]
    Conflict,
    /// The device to attach is the conversation's primary Host.
    #[error("That device is the conversation's primary host")]
    HostIsPrimary,
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
    pub fn code(&self) -> (ErrorCode, u16) {
        match self {
            Self::NotFound => (ErrorCode::ConversationNotFound, 404),
            Self::Archived => (ErrorCode::ConversationArchived, 409),
            Self::TurnInFlight => (ErrorCode::TurnInFlight, 409),
            Self::ProviderNotFound => (ErrorCode::ProviderNotFound, 404),
            Self::ModelNotFound => (ErrorCode::ModelNotFound, 404),
            Self::SettingUnavailable(_) => (ErrorCode::SettingUnavailable, 409),
            Self::ModelNotSelected => (ErrorCode::ModelNotSelected, 409),
            Self::WorkspaceNotFound => (ErrorCode::WorkspaceNotFound, 404),
            Self::DeviceNotFound => (ErrorCode::DeviceNotFound, 404),
            Self::Conflict => (ErrorCode::TargetConflict, 409),
            Self::HostIsPrimary => (ErrorCode::HostIsPrimary, 409),
            Self::NotAttached => (ErrorCode::HostNotAttached, 404),
            Self::NameTaken => (ErrorCode::NameTaken, 409),
            Self::Runtime(_) | Self::Storage(_) => (ErrorCode::OperationFailed, 500),
        }
    }
}

/// A conversation held for a transition. Its fields drop in order: the file
/// gate's reservation, then the transfers open again, then the tree's
/// reservation.
pub struct TransitionHold {
    _files: Reservation,
    _transfers: TransfersClosed,
    _tree: Option<Reservation>,
}

impl dyn HostShard + '_ {
    /// Holds the conversation for a transition (§ Switch the primary target,
    /// steps 2 to 4) once the shard reserved its idle tree, `tree`: its
    /// transfers and streams ended and their release awaited, and its file
    /// gate reserved. A busy file gate refuses.
    pub async fn hold_for_transition(
        &self,
        id: &ConversationId,
        tree: Option<Reservation>,
    ) -> Result<TransitionHold, ChangeRefusal> {
        let slot = self.conversations().slot(id);
        let transfers = slot.transfers.close().await;
        let files = slot
            .file_gate()
            .try_reserve()
            .ok_or(ChangeRefusal::TurnInFlight)?;
        Ok(TransitionHold {
            _files: files,
            _transfers: transfers,
            _tree: tree,
        })
    }

    /// Commits a change of the conversation's record.
    pub async fn commit(
        &self,
        id: &ConversationId,
        change: RecordChange,
    ) -> Result<(), ChangeRefusal> {
        match self
            .control()
            .change_conversation(id.clone(), change)
            .await?
        {
            ChangeOutcome::Applied => Ok(()),
            ChangeOutcome::Missing => Err(ChangeRefusal::NotFound),
            ChangeOutcome::Archived => Err(ChangeRefusal::Archived),
            ChangeOutcome::NotAttached => Err(ChangeRefusal::NotAttached),
            ChangeOutcome::NameTaken => Err(ChangeRefusal::NameTaken),
        }
    }

    /// Whether the user has what `to` names: a workspace of theirs, or a
    /// device they paired.
    pub async fn check_destination(
        &self,
        owner: &UserId,
        to: &ConversationTarget,
    ) -> Result<(), ChangeRefusal> {
        let control = self.control();
        match to {
            ConversationTarget::Workspace { workspace_id } => {
                let workspace = control.workspace(workspace_id.clone()).await?;
                if !workspace.is_some_and(|workspace| workspace.user == *owner) {
                    return Err(ChangeRefusal::WorkspaceNotFound);
                }
            }
            ConversationTarget::Device { device_id, .. } => {
                let device = control.device(device_id.clone()).await?;
                let paired = device.is_some_and(|device| {
                    device.user == *owner && device.kind == DeviceKind::User
                });
                if !paired {
                    return Err(ChangeRefusal::DeviceNotFound);
                }
            }
            ConversationTarget::Cloud { .. } => {}
        }
        Ok(())
    }

    /// The held switch's steps 4 to 6 (§ Switch the primary target): the device
    /// it leaves hears the conversation release and stays attached where it
    /// was left, the device it reaches is primary alone, and the commit is
    /// against the target the switch started from.
    pub async fn switch_target(
        &self,
        expected: &ConversationRecord,
        to: ConversationTarget,
    ) -> Result<(), ChangeRefusal> {
        let control = self.control();
        let current = control
            .conversation(expected.id.clone())
            .await?
            .ok_or(ChangeRefusal::NotFound)?;
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
            self.release_on(expected.id.as_str(), device).await;
        }
        let ends = SwitchEnds {
            departed: departed.map(|device| (device, from.path().to_owned())),
            arriving: destination.device().cloned(),
        };
        let switch = TargetSwitch {
            from,
            to: destination,
        };
        let won = control
            .switch_conversation_target(
                expected.id.clone(),
                expected.target.clone(),
                to,
                switch,
                ends,
            )
            .await?;
        if !won {
            return Err(ChangeRefusal::Conflict);
        }
        Ok(())
    }

    /// The held archive: the conversation release on every Host the
    /// conversation reaches, then the commit.
    pub async fn archive(&self, record: &ConversationRecord) -> Result<(), ChangeRefusal> {
        self.release_everywhere(record).await?;
        self.commit(&record.id, RecordChange::Archived(true)).await
    }

    /// The held detach of `device`: the release on it while it is attached,
    /// then the commit.
    pub async fn detach(
        &self,
        record: &ConversationRecord,
        device: DeviceId,
    ) -> Result<(), ChangeRefusal> {
        let attached = self.control().attached_hosts(record.id.clone()).await?;
        if attached.iter().any(|host| host.device == device) {
            self.release_on(record.id.as_str(), &device).await;
        }
        self.commit(&record.id, RecordChange::Detach(device)).await
    }

    /// The conversation release on every Host the conversation reaches, primary
    /// and attached, for an archive and for its idle watch. Only reading the
    /// Hosts fails it: a Host that did not take the release hears it again
    /// (`release_on`).
    pub async fn release_everywhere(
        &self,
        record: &ConversationRecord,
    ) -> Result<(), StorageError> {
        let devices = self.release_devices(record).await?;
        self.release_on_each(&record.id, &devices).await;
        Ok(())
    }

    /// The devices of every Host the conversation reaches, primary and
    /// attached: where its release goes. A deletion reads them before its
    /// records go, and releases them after (`storage.md` § Deleting a
    /// conversation).
    pub async fn release_devices(
        &self,
        record: &ConversationRecord,
    ) -> Result<Vec<DeviceId>, StorageError> {
        let target = self.resolve_target(record).await?;
        let hosts = self.reachable_hosts(record, &target).await?;
        Ok(hosts.into_iter().map(|host| host.device).collect())
    }

    /// The conversation release on each of `devices`, as an archive sends it
    /// (`release_on`).
    pub async fn release_on_each(&self, conversation: &ConversationId, devices: &[DeviceId]) {
        for device in devices {
            self.release_on(conversation.as_str(), device).await;
        }
    }
}
