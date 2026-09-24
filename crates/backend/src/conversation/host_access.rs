//! The conversation's host access (`sessions-and-targets.md` § Host
//! operations): the one way to a conversation's main or attached Host. An
//! operation takes the conversation's file gate, so it excludes an archive
//! or a target switch; the target is resolved, and an archived conversation
//! or a device that is not bound is refused; a Cloud's admission is taken,
//! and the file gate is let go while that waits; then the operation runs
//! once. Each conversation has one slot in its user's shard, with its file
//! gate and its open transfers.

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::rc::Rc;

use demi_gates::{ActivityGate, GateLease, Purpose};
use demi_host_remote::{Admission, RemoteHost};
use demi_shell::{HostError, HostErrorKind, HostFs, MkdirOptions};
use demi_web_api::devices::DeviceKind;
use demi_web_api::error::ErrorCode;
use demi_web_api::ids::{ConversationId, DeviceId};
use futures_util::future::join_all;
use tokio_util::sync::CancellationToken;

use super::target::ExecutionTarget;
use super::transfer::{OpenTransfer, TransferSet, TransfersClosed};
use crate::runner::{HostOwner, host_key};
use crate::shard::Shard;
use crate::storage::StorageError;
use crate::storage::conversation_index::ConversationRecord;
use crate::storage::devices::DeviceRecord;

/// Each conversation's slot, made on its first use and kept while the shard
/// lives: a second slot for one conversation would be a second file gate.
/// Slots are keyed by the id as the conversation's record spells it.
#[derive(Default)]
pub(crate) struct Conversations {
    slots: RefCell<HashMap<ConversationId, Rc<ConversationSlot>>>,
}

/// One conversation's part of its user's shard.
pub(crate) struct ConversationSlot {
    /// Every operation on the conversation's Hosts holds a lease of it; a
    /// transition reserves it.
    pub(crate) files: ActivityGate,
    /// The open file transfers and user streams.
    pub(crate) transfers: Rc<TransferSet>,
}

impl Conversations {
    pub(crate) fn slot(&self, id: &ConversationId) -> Rc<ConversationSlot> {
        self.slots
            .borrow_mut()
            .entry(id.clone())
            .or_insert_with(|| {
                Rc::new(ConversationSlot {
                    files: ActivityGate::new(),
                    transfers: TransferSet::new(),
                })
            })
            .clone()
    }

    /// Ends every conversation's open transfers and user streams, for the
    /// shard's close; they stay closed while the answer is held.
    pub(crate) async fn end_transfers(&self) -> Vec<TransfersClosed> {
        let slots: Vec<Rc<ConversationSlot>> = self.slots.borrow().values().cloned().collect();
        join_all(slots.iter().map(|slot| slot.transfers.close())).await
    }
}

/// Why the conversation's host access admitted no operation, or why the Host
/// failed one.
#[derive(Debug, thiserror::Error)]
pub(crate) enum HostAccessError {
    /// The user has no conversation of the id.
    #[error("No such conversation")]
    Missing,
    #[error(transparent)]
    Refused(#[from] Refusal),
    #[error(transparent)]
    Cloud(#[from] CloudUnavailable),
    /// The requester left while the admission waited.
    #[error("the request went away")]
    Cancelled,
    /// The Host failed an operation.
    #[error(transparent)]
    Host(#[from] HostError),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

impl HostAccessError {
    /// The code and HTTP status the error answers with.
    pub(crate) fn code(&self) -> (ErrorCode, u16) {
        match self {
            Self::Missing => (ErrorCode::ConversationNotFound, 404),
            Self::Refused(Refusal::Archived) => (ErrorCode::ConversationArchived, 409),
            Self::Refused(Refusal::NotAttached) => (ErrorCode::HostNotAttached, 404),
            Self::Refused(Refusal::Busy) => (ErrorCode::ConversationBusy, 409),
            Self::Refused(Refusal::Stopped) => (ErrorCode::HostStopped, 409),
            Self::Refused(Refusal::DeviceGone) => (ErrorCode::DeviceNotFound, 404),
            Self::Cloud(_) => (ErrorCode::CloudUnavailable, 503),
            Self::Host(error) => host_error_code(error),
            Self::Cancelled | Self::Storage(_) => (ErrorCode::InternalError, 500),
        }
    }
}

/// The code and HTTP status of a Host's failure of a file operation:
/// nothing at the path answers 404, no permission 403, a listing over the
/// runner's message limit 413.
pub(crate) fn host_error_code(error: &HostError) -> (ErrorCode, u16) {
    match &error.kind {
        HostErrorKind::Offline => (ErrorCode::DeviceOffline, 409),
        HostErrorKind::Unavailable => (ErrorCode::CloudUnavailable, 503),
        HostErrorKind::TooLarge => (ErrorCode::DirectoryTooLarge, 413),
        HostErrorKind::Failed { code } => match code.as_deref() {
            Some("ENOENT") => (ErrorCode::FsError, 404),
            Some("EACCES" | "EPERM") => (ErrorCode::FsError, 403),
            _ => (ErrorCode::HostOperationFailed, 500),
        },
        HostErrorKind::Protocol | HostErrorKind::Interrupted => (ErrorCode::HostOperationFailed, 500),
    }
}

/// What the conversation's state refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum Refusal {
    #[error("Conversation is archived")]
    Archived,
    /// The device is neither the main Host nor attached.
    #[error("No such host in this conversation")]
    NotAttached,
    /// A transition is ending the conversation's transfers and streams.
    #[error("The conversation is changing; its file transfers and streams are closed")]
    Busy,
    /// The Cloud is stopped, and the operation does not wake it.
    #[error("The Cloud is stopped")]
    Stopped,
    /// The conversation's target names a device the user no longer has,
    /// such as a revoked one.
    #[error("The conversation's device no longer exists")]
    DeviceGone,
}

/// The Cloud cannot start (`managed-hosts.md`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub(crate) struct CloudUnavailable(pub(crate) String);

/// A hold of the Cloud's admission: the Cloud keeps running while an
/// admitted operation holds it.
#[expect(dead_code, reason = "the Cloud's lifecycle admits a Cloud")]
pub(crate) struct CloudAdmission {
    device: DeviceId,
    /// What each operation of the admitted Host takes: a lease of the
    /// Cloud's admission, refused while the Cloud changes state.
    per_operation: Admission,
    held: GateLease,
}

/// A conversation's Host as an admitted operation reaches it.
#[derive(Clone)]
pub(crate) struct ConversationHost {
    pub(crate) host: RemoteHost,
    /// Where the conversation's work starts on this Host.
    pub(crate) root: String,
    /// The home directory the device's runner reported when it last
    /// connected.
    pub(crate) home: Option<String>,
}

/// An admitted operation's hold. The fields drop in order: the Host, the
/// file lease, then the Cloud's admission.
pub(super) struct Admitted {
    pub(super) host: ConversationHost,
    _files: GateLease,
    _cloud: Option<CloudAdmission>,
}

/// A user stream's hold on the conversation's main Host: registered with
/// the conversation's open transfers, it holds neither the file gate nor a
/// Cloud awake.
pub(super) struct StreamAccess {
    /// The conversation's id as its record spells it.
    pub(super) conversation: ConversationId,
    pub(super) host: ConversationHost,
    pub(super) device: DeviceId,
    pub(super) open: OpenTransfer,
}

/// What ends an admission's waits: the requester leaving, and, for a file
/// transfer, a transition that ends the conversation's transfers.
#[derive(Clone, Copy)]
pub(super) struct Waits<'a> {
    pub(super) cancel: &'a CancellationToken,
    pub(super) ended: Option<&'a CancellationToken>,
}

impl<'a> Waits<'a> {
    pub(super) fn request(cancel: &'a CancellationToken) -> Self {
        Self { cancel, ended: None }
    }

    /// `work`'s outcome, unless the requester leaves or a transition ends
    /// the transfer first; dropping `work` must be safe.
    pub(super) async fn wait<T>(self, work: impl Future<Output = T>) -> Result<T, HostAccessError> {
        let ended = async {
            match self.ended {
                Some(ended) => ended.cancelled().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            biased;
            () = self.cancel.cancelled() => Err(HostAccessError::Cancelled),
            () = ended => Err(Refusal::Busy.into()),
            value = work => Ok(value),
        }
    }
}

/// The Host an operation reaches, before any Cloud admission.
struct Selected {
    device: DeviceRecord,
    root: String,
    /// Whether the directory is made before the operation, as a Cloud
    /// session directory is.
    prepare: bool,
}

/// A Host the conversation reaches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReachableHost {
    pub(crate) name: String,
    pub(crate) device: DeviceId,
    /// Where a shell there starts.
    pub(crate) path: String,
    pub(crate) role: HostRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HostRole {
    Main,
    Attached,
}

impl Shard {
    /// The user's conversation of `id`, in whichever case it is spelled; a
    /// conversation of another user answers as a missing one. Every shard
    /// entry that names a conversation starts here, which makes it the one
    /// check that a request reaches its owner's conversation.
    pub(crate) async fn owned_conversation(&self, id: &ConversationId) -> Result<ConversationRecord, HostAccessError> {
        let record = self.services().control.conversation(id.clone()).await?;
        record
            .filter(|record| record.owner == *self.user())
            .ok_or(HostAccessError::Missing)
    }

    /// Runs `operation` once on the conversation's Host: its main Host, or
    /// the bound device `device` names. The operation holds the
    /// conversation's file gate, and a Cloud's admission, until it returns.
    /// An admission that waits ends when `cancel` does.
    pub(crate) async fn with_host<T>(
        &self,
        id: &ConversationId,
        device: Option<&DeviceId>,
        cancel: &CancellationToken,
        operation: impl AsyncFnOnce(&ConversationHost) -> T,
    ) -> Result<T, HostAccessError> {
        let admitted = self.admit_host(id, device, Waits::request(cancel)).await?;
        let output = operation(&admitted.host).await;
        drop(admitted);
        Ok(output)
    }

    /// Admits an operation on the conversation's Host (§ Host operations):
    /// each attempt takes the file gate and checks the conversation again,
    /// since while it waited the conversation may have been archived or
    /// switched, or its device detached.
    pub(super) async fn admit_host(
        &self,
        id: &ConversationId,
        device: Option<&DeviceId>,
        waits: Waits<'_>,
    ) -> Result<Admitted, HostAccessError> {
        let record = self.owned_conversation(id).await?;
        let slot = self.conversations().slot(&record.id);
        let mut cloud: Option<CloudAdmission> = None;
        loop {
            let files = waits.wait(slot.files.enter(Purpose::Demand)).await?;
            let selected = self.select_host(&record.id, device, true).await?;
            let admitted = match selected.device.kind {
                DeviceKind::User => {
                    cloud = None;
                    true
                }
                DeviceKind::Managed => cloud.as_ref().is_some_and(|cloud| cloud.device == selected.device.id),
            };
            if !admitted {
                // Waiting for the Cloud holds no file lease: a reset takes
                // the file gates of the conversations it holds, and would
                // wait for this one forever.
                drop(files);
                drop(cloud.take());
                cloud = Some(waits.wait(self.enter_cloud(&selected.device)).await??);
                continue;
            }
            let host = self.open_host(&record.id, &selected, cloud.as_ref()).await?;
            return Ok(Admitted {
                host,
                _files: files,
                _cloud: cloud,
            });
        }
    }

    /// Admits a user stream, or a one-shot user call that must not wake the
    /// Host (§ Host operations): like any operation, except that a stopped
    /// Cloud is refused instead of woken, and that the admitted stream lets
    /// go of the file gate. It stays registered with the conversation's
    /// transfers, so a transition ends it.
    pub(super) async fn admit_stream(
        &self,
        id: &ConversationId,
        cancel: &CancellationToken,
    ) -> Result<StreamAccess, HostAccessError> {
        let record = self.owned_conversation(id).await?;
        let slot = self.conversations().slot(&record.id);
        // As for a file transfer: no await between the check and the
        // registration.
        let open = slot.transfers.open().map_err(|_| Refusal::Busy)?;
        let waits = Waits {
            cancel,
            ended: Some(&open.ended),
        };
        let files = waits.wait(slot.files.enter(Purpose::Demand)).await?;
        let mut selected = self.select_host(&record.id, None, false).await?;
        if !self.devices().online(&selected.device.id) {
            return Err(match selected.device.kind {
                DeviceKind::Managed => Refusal::Stopped.into(),
                DeviceKind::User => HostError::offline("The device has no live runner").into(),
            });
        }
        // A stream shows what the conversation's work left: it makes no
        // directory.
        selected.prepare = false;
        let host = self.open_host(&record.id, &selected, None).await?;
        drop(files);
        Ok(StreamAccess {
            conversation: record.id,
            host,
            device: selected.device.id,
            open,
        })
    }

    /// Takes the Cloud's admission: it wakes a stopped Cloud, joins a boot
    /// under way, or waits for a running reset to finish. The Cloud's
    /// lifecycle holds the admission; a backend that runs none starts no
    /// Cloud.
    async fn enter_cloud(&self, device: &DeviceRecord) -> Result<CloudAdmission, CloudUnavailable> {
        Err(CloudUnavailable(format!("The Cloud {} cannot start", device.id)))
    }

    /// The Host an operation reaches and where its work starts, found
    /// without waiting for any device. `allocate` makes the user's Cloud
    /// device on its first use; without it, a Cloud never made counts as
    /// stopped.
    async fn select_host(
        &self,
        id: &ConversationId,
        device: Option<&DeviceId>,
        allocate: bool,
    ) -> Result<Selected, HostAccessError> {
        let control = &self.services().control;
        let record = self.owned_conversation(id).await?;
        if record.archived {
            return Err(Refusal::Archived.into());
        }
        let target = self.resolve_target(&record).await?;
        let named = match device {
            None => None,
            Some(device) => Some(
                self.reachable_hosts(&record, &target)
                    .await?
                    .into_iter()
                    .find(|host| host.device == *device)
                    .ok_or(Refusal::NotAttached)?,
            ),
        };
        let found = match (&named, &target) {
            (Some(host), _) => control.device(host.device.clone()).await?,
            (None, ExecutionTarget::Cloud { .. }) if allocate => {
                Some(control.managed_device_or_create(record.owner.clone()).await?)
            }
            (None, ExecutionTarget::Cloud { .. }) => {
                Some(control.managed_device(record.owner.clone()).await?.ok_or(Refusal::Stopped)?)
            }
            (None, ExecutionTarget::Device { device_id, .. } | ExecutionTarget::Workspace { device_id, .. }) => {
                control.device(device_id.clone()).await?
            }
        };
        let device = found
            .filter(|device| device.user == record.owner)
            .ok_or(Refusal::DeviceGone)?;
        let (root, prepare) = match named.filter(|host| host.role == HostRole::Attached) {
            Some(attached) => (attached.path, false),
            None => (target.path().to_owned(), matches!(target, ExecutionTarget::Cloud { .. })),
        };
        Ok(Selected { device, root, prepare })
    }

    /// The admitted Host, its Cloud session directory made first.
    async fn open_host(
        &self,
        id: &ConversationId,
        selected: &Selected,
        cloud: Option<&CloudAdmission>,
    ) -> Result<ConversationHost, HostAccessError> {
        let device = &selected.device.id;
        let admission = cloud.map_or(Admission::Free, |cloud| cloud.per_operation.clone());
        let key = host_key(device, HostOwner::Conversation(id), &selected.root);
        let host = self.devices().host(device, key, selected.root.clone(), admission);
        if selected.prepare {
            HostFs::mkdir(&host, &selected.root, MkdirOptions { recursive: true }).await?;
        }
        Ok(ConversationHost {
            host,
            root: selected.root.clone(),
            home: self.devices().home(device),
        })
    }

    /// The Hosts browser access and `demi host shell --host` accept
    /// (§ Attached hosts): the main Host, and the attached ones. A shell on
    /// the main Host starts in the conversation's directory there; one on an
    /// attached Host where the last shell there ended, or in its home before
    /// one ran.
    pub(crate) async fn reachable_hosts(
        &self,
        record: &ConversationRecord,
        target: &ExecutionTarget,
    ) -> Result<Vec<ReachableHost>, StorageError> {
        let control = &self.services().control;
        let main = target.device();
        let mut hosts = Vec::new();
        if let Some(device) = main {
            let name = control
                .device(device.clone())
                .await?
                .map_or_else(|| device.to_string(), |record| record.name);
            hosts.push(ReachableHost {
                name,
                device: device.clone(),
                path: target.path().to_owned(),
                role: HostRole::Main,
            });
        }
        for attached in control.attached_hosts(record.id.clone()).await? {
            if Some(&attached.device) == main {
                continue;
            }
            let path = attached
                .cwd
                .or_else(|| self.devices().home(&attached.device))
                .unwrap_or_default();
            hosts.push(ReachableHost {
                name: attached.name,
                device: attached.device,
                path,
                role: HostRole::Attached,
            });
        }
        Ok(hosts)
    }

    /// Lifecycle access (§ Lifecycle access): the conversation release, sent
    /// to `device` when it is a connected paired device. It takes no file
    /// gate and wakes nothing: a Cloud reclaims everything when it stops, and
    /// a device whose runner is not connected lost the conversation's state
    /// with its connection.
    pub(crate) async fn release_on(&self, id: &ConversationId, device: &DeviceId) -> Result<(), HostAccessError> {
        let Some(record) = self.services().control.device(device.clone()).await? else {
            return Ok(());
        };
        if record.kind == DeviceKind::Managed {
            return Ok(());
        }
        let Some(link) = self.devices().link(device) else {
            return Ok(());
        };
        Ok(link.release_conversation(id.as_str()).await?)
    }
}
