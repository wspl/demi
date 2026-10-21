//! The changes that end a conversation's target: a move (a target switch),
//! an archive and a detach.
//!
//! An archive and a detach run while the conversation is held for them
//! (`hold_for_transition`):
//!
//! - The shard reserves the idle tree.
//! - The file transfers, user streams and the one-shot calls admitted like them
//!   end.
//! - The file gate is reserved.
//!
//! Each sends the conversation release to the devices it leaves, then commits,
//! whether the devices took the release or not
//! (`crates/backend-user-shard/src/lifecycle/conversations.rs`). The shard's
//! `transition` is the one entry every change of a conversation goes through.
//!
//! A move (`docs/decisions/0027-a-move-applies-at-once.md`) stops and waits
//! for nothing, in order:
//!
//! ```text
//! 1. check the destination: one question to its Host for a directory
//! 2. end transfers, streams and the one-shot calls like them; await them
//! 3. commit the target and attachment changes against the expected selection
//! 4. advance the execution-context revision; each agent's next request names
//!    the new Host and directory
//! 5. once nothing of the conversation runs on the device it left, reserve
//!    the conversation's gate of that device, which every admission there
//!    holds a lease of, and send the release, unless the conversation is back
//!    on it
//! ```
//!
//! Commands already running finish where they started and report as usual;
//! every command started after the commit runs on the new target. Concurrent
//! moves cannot both replace the same expected selection: the loser is refused
//! as conflicting, so work is never dispatched against an ambiguous target.
//!
//! Turn admission uses the tree's lifecycle boundary, not the root's displayed
//! phase, which would miss a running child.
//!
//! A move away from an offline device fails the commands that wait for it,
//! which were never sent, and the agents go on on the new Host. The commands
//! still running there let go of the conversation's file gate, which nothing
//! would release while the device is away; their runner keeps them until it is
//! back.
//!
//! The device a move leaves stays attached, its commands starting in the
//! conversation's directory there, and the incoming device leaves the
//! attachment set. A move of directory on the same device adds no duplicate.
//! Files do not move, and a path keeps referring to its original device.
//!
//! If the database commit of an archive fails after the release, the old
//! selection stays and the conversation browser starts fresh at its next open:
//! the state of its tabs cannot be rolled back.

use demi_backend_database::conversation_index::{
    ChangeOutcome, ConversationRecord, RecordChange, SwitchEnds, SwitchWrite, TargetSwitch,
};
use demi_backend_database::workspaces::WorkspaceRecord;
use demi_backend_database::{Selection, StorageError, Unselectable};
use demi_shared_gates::Reservation;
use demi_web_api_protocol::conversations::ConversationTarget;
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
    #[error("A conversation with running work cannot be archived or restored")]
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
    /// The destination is on a device whose runner never reported its
    /// home, so whether it is a project's directory is unknown
    /// (`crates/backend-database/src/selection.rs`).
    #[error("The device has not connected yet; connect it, then move the conversation there")]
    DeviceHomeUnknown,
    /// The destination's device is not connected, so its directory cannot
    /// be resolved (`crates/backend-database/src/selection.rs`).
    #[error("{0}")]
    DeviceOffline(String),
    /// The destination names no directory on its Host.
    #[error("{0}")]
    NotADirectory(String),
    /// The destination's Host failed to resolve its directory.
    #[error("{0}")]
    HostFailed(String),
    /// The move's caller went away before its commit started.
    #[error("The move was cancelled; the conversation did not move")]
    Cancelled,
    /// Another switch changed the target first.
    #[error("Another target change came first")]
    Conflict,
    /// The device to attach is the conversation's primary Host.
    #[error("That device is the conversation's primary host")]
    HostIsPrimary,
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
            Self::DeviceHomeUnknown => (ErrorCode::DeviceHomeUnknown, 409),
            Self::DeviceOffline(_) => (ErrorCode::DeviceOffline, 409),
            Self::NotADirectory(_) => (ErrorCode::FsNotFound, 404),
            Self::HostFailed(_) => (ErrorCode::HostOperationFailed, 502),
            Self::Conflict => (ErrorCode::TargetConflict, 409),
            Self::HostIsPrimary => (ErrorCode::HostIsPrimary, 409),
            Self::Runtime(_) | Self::Storage(_) | Self::Cancelled => (ErrorCode::OperationFailed, 500),
        }
    }
}

impl From<Unselectable> for ChangeRefusal {
    fn from(reason: Unselectable) -> Self {
        match reason {
            Unselectable::WorkspaceNotFound => Self::WorkspaceNotFound,
            Unselectable::DeviceNotFound => Self::DeviceNotFound,
            Unselectable::HomeUnknown => Self::DeviceHomeUnknown,
        }
    }
}

/// A target switch made: where it went from and to, the execution-context
/// revision it made, and the workspace its destination made, if it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Switched {
    pub switch: TargetSwitch,
    pub revision: u64,
    pub made: Option<WorkspaceRecord>,
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
    /// Holds the conversation for an archive or a detach once the shard
    /// reserved its idle tree, `tree`: its transfers and streams ended and
    /// their release awaited, and its file gate reserved. A busy file gate
    /// refuses.
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

    /// Holds the conversation for the detaches an agent asked for, once the
    /// shard reserved its idle tree: as [`hold_for_transition`](Self::hold_for_transition)
    /// does, except that it waits for the Host operations that hold the file
    /// gate to end, since nobody is there to retry a refusal.
    pub async fn hold_when_free(
        &self,
        id: &ConversationId,
        tree: Option<Reservation>,
    ) -> TransitionHold {
        let slot = self.conversations().slot(id);
        let transfers = slot.transfers.close().await;
        let files = slot.file_gate().reserve().await;
        TransitionHold {
            _files: files,
            _transfers: transfers,
            _tree: tree,
        }
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
        }
    }

    /// The selection `to` names for the conversation `record`, as its
    /// switch would make it, without making anything: the conversation's
    /// own selection when the switch would change nothing.
    pub async fn selection_for(
        &self,
        record: &ConversationRecord,
        to: &ConversationTarget,
    ) -> Result<Selection, ChangeRefusal> {
        Ok(self
            .control()
            .selection_for(record.owner.clone(), record.id.clone(), to.clone())
            .await??)
    }

    /// The device a move to `to` reaches; none for the Cloud before its first
    /// use made its device.
    async fn destination_device(
        &self,
        owner: &UserId,
        to: &ConversationTarget,
    ) -> Result<Option<DeviceId>, ChangeRefusal> {
        let control = self.control();
        Ok(match to {
            ConversationTarget::Cloud { .. } => control
                .managed_device(owner.clone())
                .await?
                .map(|device| device.id),
            ConversationTarget::Device { device_id, .. } => Some(device_id.clone()),
            ConversationTarget::Workspace { workspace_id } => Some(
                control
                    .workspace(workspace_id.clone())
                    .await?
                    .filter(|workspace| workspace.user == *owner)
                    .ok_or(ChangeRefusal::WorkspaceNotFound)?
                    .device,
            ),
        })
    }

    /// A move's commit (§ steps 3 and 4), its destination `to` checked:
    ///
    /// - The device it leaves stays attached where it was left.
    /// - The device it reaches is primary alone.
    /// - The commit selects `to`'s directory against the selection the move
    ///   started from (§ Resolve a target).
    ///
    /// Answers where the move went from and to and the execution-context
    /// revision it made, and none when `to` selects what the conversation has.
    /// The release of the device it left is the caller's
    /// ([`release_departed`](Self::release_departed)).
    pub async fn switch_target(
        &self,
        expected: &ConversationRecord,
        to: ConversationTarget,
    ) -> Result<Option<Switched>, ChangeRefusal> {
        let control = self.control();
        let current = control
            .conversation(expected.id.clone())
            .await?
            .ok_or(ChangeRefusal::NotFound)?;
        if current.archived {
            return Err(ChangeRefusal::Archived);
        }
        let from = self.resolve_target(expected).await?;
        let arriving = self.destination_device(&expected.owner, &to).await?;
        let ends = SwitchEnds {
            departed: from.device().cloned().map(|device| (device, from.path().to_owned())),
            arriving,
        };
        match control
            .switch_conversation_target(expected.id.clone(), expected.target.clone(), from, to, ends)
            .await?
        {
            SwitchWrite::Switched { switch, revision, made } => Ok(Some(Switched { switch, revision, made })),
            SwitchWrite::Unchanged => Ok(None),
            SwitchWrite::Conflict => Err(ChangeRefusal::Conflict),
            SwitchWrite::Unselectable(reason) => Err(reason.into()),
        }
    }

    /// The release of `device`, which a move of the conversation `id` left
    /// (§ step 5): once nothing of the conversation runs there, it reserves
    /// the conversation's gate of the device in the same step, so no work
    /// starts there until the release is answered, and sends the release
    /// unless the conversation is back on the device. A release that fails
    /// is logged and fails nothing.
    pub async fn release_departed(&self, id: &ConversationId, device: &DeviceId) {
        let gate = self.conversations().slot(id).host_gate(device);
        loop {
            // Subscribed before the attempt, so a lease that ends after it
            // wakes the wait.
            let mut changed = gate.subscribe();
            if let Some(_released) = gate.try_reserve() {
                if !self.runs_on(id, device).await {
                    self.release_on(id.as_str(), device).await;
                }
                return;
            }
            // Work there for a conversation back on the device is its own.
            if self.runs_on(id, device).await || changed.changed().await.is_err() {
                return;
            }
        }
    }

    /// Whether the conversation `id` runs on `device` as its primary Host;
    /// true for one that is gone, which its deletion released. A record
    /// that cannot be read counts as running there: releasing a device the
    /// conversation is back on would end services its work uses.
    async fn runs_on(&self, id: &ConversationId, device: &DeviceId) -> bool {
        match self.control().conversation(id.clone()).await {
            Ok(Some(record)) => self
                .resolve_target(&record)
                .await
                .map_or(true, |target| target.device() == Some(device)),
            Ok(None) => true,
            Err(error) => {
                tracing::warn!(conversation = %id, "a moved conversation's record cannot be read: {error}");
                true
            }
        }
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
    /// Hosts fails it: a Host that did not take the release is not sent it
    /// again (`release_on`).
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
    /// records go, and releases them after
    /// (`crates/backend-database/src/deletions.rs`).
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
