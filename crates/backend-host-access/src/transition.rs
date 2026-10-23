//! The changes that end a conversation's target: a move (a target switch), an
//! archive and a detach.
//!
//! An archive runs while the conversation is held for it
//! (`hold_for_transition`):
//!
//! - The shard reserves the idle tree.
//! - The file transfers, user streams and the one-shot calls admitted like them
//!   end.
//! - The file gate is reserved.
//!
//! It commits, then tells the connected Hosts it leaves to let go of the
//! conversation and answers: nothing waits for them and nothing is stored, as
//! the Hosts hold what the records say after their next `hello`
//! (`crates/backend-host-access/src/access.rs`). The shard's `transition` is
//! the one entry every change of a conversation goes through.
//!
//! A detach holds nothing and waits for no work and no Host, whatever the
//! conversation does, in order:
//!
//! ```text
//! 1. end the Host's transfers, streams and the one-shot calls like them;
//!    await them
//! 2. commit: the Host leaves the conversation's, and each agent's next
//!    request says so; nothing more when it was not attached
//! 3. ask every job of the conversation on the Host to stop, and answer
//! 4. once the jobs ended, tell the Host to release the conversation, unless
//!    the conversation reaches it again
//! ```
//!
//! A job is stopped as an abort stops it, and its end says the Host was
//! detached. One whose Host cannot be reached at that moment ends as lost with
//! the same reason, since nothing could report the stop. A job that was
//! admitted before the commit and registers after it, or has not started, ends
//! without starting: each registers with the conversation, then checks that
//! its device is still one of its Hosts. The release is sent once no job is
//! left, as every release is
//! (`crates/backend-user-shard/src/lifecycle/conversations.rs`).
//!
//! A move stops and waits for nothing, in order:
//!
//! ```text
//! 1. read the destination as typed, `~` expanded: no question to its Host
//! 2. end transfers, streams and the one-shot calls like them; await them
//! 3. commit the target and attachment changes against the expected selection
//! 4. advance the execution-context revision; each agent's next request names
//!    the new Host and directory
//! 5. once the conversation's commands on the device it left ended, tell it to
//!    release there, unless the conversation is back on it
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
use crate::access::HostAccessError;
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

/// What a command that a detach stopped says of its Host: the reason its end
/// tells and the failure of a call made there.
pub fn detached_reason(host: &str) -> String {
    format!("{host} was detached from this conversation")
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

/// The line that ends a wait of a transition for what holds the conversation.
fn wait_ended(conversation: &ConversationId, what: &'static str, waited: std::time::Instant) {
    tracing::info!(
        event = "host_wait.ended",
        %conversation,
        what,
        waited_ms = u64::try_from(waited.elapsed().as_millis()).unwrap_or(u64::MAX),
        outcome = "ready",
        "a transition waited for a conversation's Host work"
    );
}

impl dyn HostShard + '_ {
    /// Holds the conversation for an archive once the shard
    /// reserved its idle tree, `tree`: its transfers and streams ended and
    /// their release awaited, and its file gate reserved. A busy file gate
    /// refuses.
    pub async fn hold_for_transition(
        &self,
        id: &ConversationId,
        tree: Option<Reservation>,
    ) -> Result<TransitionHold, ChangeRefusal> {
        let slot = self.conversations().slot(id);
        let waited = std::time::Instant::now();
        let transfers = slot.transfers.close().await;
        wait_ended(id, "transfers", waited);
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
            ChangeOutcome::Applied | ChangeOutcome::Unchanged => Ok(()),
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
    /// (§ step 5): sent once the conversation's commands there ended, so the
    /// residents they use end after them, and not sent when the conversation is
    /// back on it or where it runs cannot be known. A release that fails is
    /// logged and fails nothing.
    pub async fn release_departed(&self, id: &ConversationId, device: &DeviceId) {
        self.conversations().slot(id).jobs_ended_on(device).await;
        let primary = match self.control().conversation(id.clone()).await {
            Ok(Some(record)) => match self.resolve_target(&record).await {
                Ok(target) => target.device().cloned(),
                // As for a record that cannot be read: where the conversation
                // runs now is not known.
                Err(error) => {
                    tracing::warn!(event = "conversation_move.target_unresolved", conversation = %id, %error, "a moved conversation's target cannot be resolved");
                    return;
                }
            },
            // A conversation deleted meanwhile was released by its deletion.
            Ok(None) => return,
            // Releasing a device the conversation runs on again would only end
            // its residents early; one that is never released keeps them.
            Err(error) => {
                tracing::warn!(event = "conversation_move.record_unread", conversation = %id, %error, "a moved conversation's record cannot be read");
                return;
            }
        };
        if primary.as_ref() != Some(device) {
            self.tell_release(id, std::slice::from_ref(device), false);
        }
    }

    /// The held archive: one commit, then the Hosts the conversation reaches
    /// are told to let go of it.
    pub async fn archive(&self, record: &ConversationRecord) -> Result<(), ChangeRefusal> {
        let devices = self.release_devices(record).await?;
        self.commit(&record.id, RecordChange::Archived(true)).await?;
        self.tell_release(&record.id, &devices, true);
        Ok(())
    }

    /// The detach of `device`, in the steps of this module's comment:
    /// answers once the Host has left the conversation and its jobs were
    /// asked to stop, and none waits for the Host. Answers whether the
    /// commit removed the Host, which is when the jobs were asked and the
    /// release is due ([`release_detached`](Self::release_detached)): a device
    /// that is not attached detaches as nothing, ending no transfer.
    pub async fn detach(
        &self,
        record: &ConversationRecord,
        device: DeviceId,
    ) -> Result<bool, ChangeRefusal> {
        let attached = self.control().attached_hosts(record.id.clone()).await?;
        let Some(host) = attached.into_iter().find(|host| host.device == device) else {
            return Ok(false);
        };
        let slot = self.conversations().slot(&record.id);
        let transfers = slot.transfers.close_device(&device).await;
        let committed = self
            .control()
            .change_conversation(record.id.clone(), RecordChange::Detach(device.clone()))
            .await;
        drop(transfers);
        match committed? {
            ChangeOutcome::Applied => {}
            // Another detach or a move to the Host came first.
            ChangeOutcome::Unchanged => return Ok(false),
            ChangeOutcome::Missing => return Err(ChangeRefusal::NotFound),
            ChangeOutcome::Archived => return Err(ChangeRefusal::Archived),
        }
        let reason = detached_reason(&host.name);
        slot.withdraw_jobs_on(&device, &reason);
        Ok(true)
    }

    /// The release of `device`, which a detach of the conversation `id`
    /// removed (§ step 4): sent once the conversation's jobs there ended, and
    /// not sent when the conversation reaches the device again by then, a
    /// move to it or a new attach having come first. A release that fails is
    /// logged and fails nothing.
    pub async fn release_detached(&self, id: &ConversationId, device: &DeviceId) {
        self.conversations().slot(id).jobs_ended_on(device).await;
        match self.conversation_hosts(id).await {
            Ok(hosts) if hosts.iter().any(|host| host.device == *device) => {}
            // A conversation deleted meanwhile was released by its deletion.
            Err(HostAccessError::Missing) => {}
            Ok(_) | Err(_) => self.tell_release(id, std::slice::from_ref(device), false),
        }
    }

    /// Tells every Host the conversation reaches, primary and attached, to
    /// let go of it: for its idle watch, which this ends. Only reading the
    /// Hosts fails it.
    pub async fn release_everywhere(&self, record: &ConversationRecord) -> Result<(), StorageError> {
        let devices = self.release_devices(record).await?;
        self.tell_release(&record.id, &devices, false);
        Ok(())
    }

    /// The devices of every Host the conversation reaches, primary and
    /// attached: where its release goes. A deletion reads them before its
    /// records go, and tells them after
    /// (`crates/backend-database/src/deletions.rs`).
    pub async fn release_devices(
        &self,
        record: &ConversationRecord,
    ) -> Result<Vec<DeviceId>, StorageError> {
        let target = self.resolve_target(record).await?;
        let hosts = self.reachable_hosts(record, &target).await?;
        Ok(hosts.into_iter().map(|host| host.device).collect())
    }
}
