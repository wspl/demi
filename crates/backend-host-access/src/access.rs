//! The conversation's host access (`sessions-and-targets.md` § Host
//! operations): the one way to a conversation's primary or attached Host. An
//! operation takes the conversation's file gate, so it excludes an archive
//! or a target switch; the target is resolved, and an archived conversation
//! or a device that is not bound is refused; in the grace after the
//! backend starts, a device its last shutdown disconnected is waited for
//! (§ Recovery and persistence); a Cloud's admission is taken; the file gate
//! is let go while either waits; then the operation runs once. Each
//! conversation has one slot in its user's shard, with its file gate, its
//! open transfers and the gate its open user streams hold.

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::rc::Rc;

use demi_backend_blobs::ObjectError;
use demi_backend_cloud::machine::{CloudAdmission, CloudError};
use demi_backend_database::StorageError;
use demi_backend_database::conversation_index::{ConversationRecord, ExecutionTarget};
use demi_backend_database::devices::DeviceRecord;
use demi_backend_remote_host::{Admission, RemoteHost};
use demi_backend_runners::file_gate::{FileGate, FileLease};
use demi_host_interface::{HostError, HostErrorKind, HostFs, MkdirOptions};
use demi_runner_protocol::files::FsFailure;
use demi_shared_gates::{ActivityGate, GateLease, Purpose, SerialGate};
use demi_web_api_protocol::devices::DeviceKind;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use futures_util::future::join_all;
use tokio_util::sync::CancellationToken;

use crate::HostShard;
use crate::transfer::{OpenTransfer, TransferSet, TransfersClosed};

/// Each conversation's slot, made on its first use and kept while the shard
/// lives: a second slot for one conversation would be a second file gate.
/// Slots are keyed by the id as the conversation's record spells it.
#[derive(Default)]
pub struct Conversations {
    slots: RefCell<HashMap<ConversationId, Rc<ConversationSlot>>>,
}

/// One conversation's part of its user's shard.
pub struct ConversationSlot {
    /// Every operation on the conversation's Hosts holds a lease of it; a
    /// transition reserves it. Only host access takes its leases, the ones
    /// a conversation's Host is made against (`Devices::conversation_host`);
    /// other work holds the conversation through `file_gate`.
    files: FileGate,
    /// Every open user stream holds a demand lease of it: someone watches
    /// the conversation's Host, which is activity (`resource-lifecycle.md`
    /// § Activity). Nothing reserves it, since a transition ends the streams
    /// instead of waiting for them.
    pub streams: ActivityGate,
    /// The open file transfers and user streams.
    pub transfers: Rc<TransferSet>,
    /// Orders the changes of the conversation's model settings, one at a
    /// time; an open of its tree takes the same turn, so it opens with the
    /// selection the record holds (`web-api.md` § Sidebar mutations, read
    /// state and page synchronization).
    pub settings: SerialGate,
}

impl ConversationSlot {
    /// The conversation's file gate, which a transition reserves, an idle
    /// watch reads, and work that holds the conversation without reaching
    /// its Host, such as a frame of its socket, enters. A lease of it names
    /// no conversation, so it makes no Host.
    pub fn file_gate(&self) -> &ActivityGate {
        self.files.gate()
    }
}

impl Conversations {
    pub fn slot(&self, id: &ConversationId) -> Rc<ConversationSlot> {
        self.slots
            .borrow_mut()
            .entry(id.clone())
            .or_insert_with(|| {
                Rc::new(ConversationSlot {
                    files: FileGate::new(id.clone()),
                    streams: ActivityGate::new(),
                    transfers: TransferSet::new(),
                    settings: SerialGate::new(),
                })
            })
            .clone()
    }

    /// Ends every conversation's open transfers and user streams, for the
    /// shard's close; they stay closed while the answer is held.
    pub async fn end_transfers(&self) -> Vec<TransfersClosed> {
        let slots: Vec<Rc<ConversationSlot>> = self.slots.borrow().values().cloned().collect();
        join_all(slots.iter().map(|slot| slot.transfers.close())).await
    }
}

/// Why the conversation's host access admitted no operation, or why the Host
/// failed one.
#[derive(Debug, thiserror::Error)]
pub enum HostAccessError {
    /// The user has no conversation of the id.
    #[error("No such conversation")]
    Missing,
    #[error(transparent)]
    Refused(#[from] Refusal),
    #[error(transparent)]
    Cloud(#[from] CloudError),
    /// The requester left while the admission waited.
    #[error("the request went away")]
    Cancelled,
    /// The Host failed an operation.
    #[error(transparent)]
    Host(#[from] HostError),
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Objects(#[from] ObjectError),
    /// A put of a fitted upload image through the agent's view of the blob
    /// namespace failed.
    #[error(transparent)]
    Store(#[from] demi_agent_store::StoreError),
}

impl HostAccessError {
    /// The code and HTTP status the error answers with.
    pub fn code(&self) -> (ErrorCode, u16) {
        match self {
            Self::Missing => (ErrorCode::ConversationNotFound, 404),
            Self::Refused(Refusal::Archived) => (ErrorCode::ConversationArchived, 409),
            Self::Refused(Refusal::NotAttached) => (ErrorCode::HostNotAttached, 404),
            Self::Refused(Refusal::Busy) => (ErrorCode::ConversationBusy, 409),
            Self::Refused(Refusal::Stopped) => (ErrorCode::HostStopped, 409),
            Self::Refused(Refusal::DeviceGone) => (ErrorCode::DeviceNotFound, 404),
            Self::Cloud(error) => error.code(),
            Self::Host(error) => host_error_code(error),
            Self::Cancelled | Self::Storage(_) | Self::Objects(_) | Self::Store(_) => {
                (ErrorCode::InternalError, 500)
            }
        }
    }
}

/// The code and HTTP status of a Host's failure of a file operation:
/// nothing at the path answers 404, no permission 403, a listing over the
/// runner's message limit 413.
pub fn host_error_code(error: &HostError) -> (ErrorCode, u16) {
    match &error.kind {
        HostErrorKind::Offline => (ErrorCode::DeviceOffline, 409),
        HostErrorKind::Unavailable => (ErrorCode::CloudUnavailable, 503),
        HostErrorKind::TooLarge => (ErrorCode::DirectoryTooLarge, 413),
        HostErrorKind::Failed { code } => match FsFailure::of(code.as_deref()) {
            FsFailure::NotFound => (ErrorCode::FsError, 404),
            FsFailure::Forbidden => (ErrorCode::FsError, 403),
            FsFailure::Failed => (ErrorCode::HostOperationFailed, 500),
        },
        HostErrorKind::Protocol | HostErrorKind::Interrupted => {
            (ErrorCode::HostOperationFailed, 500)
        }
    }
}

/// What the conversation's state refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    #[error("Conversation is archived")]
    Archived,
    /// The device is neither the primary Host nor attached.
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

/// A conversation's Host as an admitted operation reaches it.
#[derive(Clone)]
pub struct ConversationHost {
    pub host: RemoteHost,
    /// Where the conversation's work starts on this Host.
    pub root: String,
    /// The home directory the device's runner reported when it last
    /// connected.
    pub home: Option<String>,
}

/// An admitted operation's hold. The fields drop in order: the Host, the
/// file lease, then the Cloud's admission.
pub struct Admitted {
    pub host: ConversationHost,
    _files: FileLease,
    _cloud: Option<CloudAdmission>,
}

/// What an admission that never wakes the Host is to the conversation's
/// activity (`resource-lifecycle.md` § Activity).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Attention {
    /// An open user stream: someone watches the Host, and the conversation
    /// is active until the stream ends.
    Watches,
    /// An operation of the user's on what runs there, such as closing a
    /// conversation browser tab: activity that ends at once.
    Operates,
    /// A look at what runs there, such as listing the conversation
    /// browser's tabs: no activity.
    Looks,
}

/// A user stream's hold on the conversation's primary Host: registered with
/// the conversation's open transfers, it holds no file gate, and a stream
/// holds its lease of the conversation's stream gate.
pub(crate) struct StreamAccess {
    /// The conversation's id as its record spells it.
    pub conversation: ConversationId,
    pub host: ConversationHost,
    pub device: DeviceId,
    pub open: OpenTransfer,
    /// A stream's lease of the conversation's stream gate; none for a call.
    pub watching: Option<GateLease>,
}

/// What ends an admission's waits: the requester leaving, and, for a file
/// transfer, a transition that ends the conversation's transfers.
#[derive(Clone, Copy)]
pub struct Waits<'a> {
    pub cancel: &'a CancellationToken,
    pub ended: Option<&'a CancellationToken>,
}

impl<'a> Waits<'a> {
    pub fn request(cancel: &'a CancellationToken) -> Self {
        Self {
            cancel,
            ended: None,
        }
    }

    /// `work`'s outcome, unless the requester leaves or a transition ends
    /// the transfer first; dropping `work` must be safe.
    pub async fn wait<T>(self, work: impl Future<Output = T>) -> Result<T, HostAccessError> {
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
pub struct ReachableHost {
    pub name: String,
    pub device: DeviceId,
    /// Where a shell there starts.
    pub path: String,
    pub role: HostRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostRole {
    Primary,
    Attached,
}

impl dyn HostShard + '_ {
    /// The user's conversation of `id`, in whichever case it is spelled; a
    /// conversation of another user answers as a missing one. Every shard
    /// entry that names a conversation starts here, which makes it the one
    /// check that a request reaches its owner's conversation.
    pub async fn owned_conversation(
        &self,
        id: &ConversationId,
    ) -> Result<ConversationRecord, HostAccessError> {
        let record = self.control().conversation(id.clone()).await?;
        record
            .filter(|record| record.owner == *self.user())
            .ok_or(HostAccessError::Missing)
    }

    /// Runs `operation` once on the conversation's Host: its primary Host, or
    /// the bound device `device` names. The operation holds the
    /// conversation's file gate, and a Cloud's admission, until it returns.
    /// An admission that waits ends when `cancel` does.
    pub async fn with_host<T>(
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
    pub async fn admit_host(
        &self,
        id: &ConversationId,
        device: Option<&DeviceId>,
        waits: Waits<'_>,
    ) -> Result<Admitted, HostAccessError> {
        let record = self.owned_conversation(id).await?;
        let slot = self.conversations().slot(&record.id);
        let mut cloud: Option<CloudAdmission> = None;
        let mut waited_for_return = false;
        loop {
            let files = waits.wait(slot.files.enter(Purpose::Demand)).await?;
            let selected = self.select_host(&record.id, device, true).await?;
            if !waited_for_return && let Some(rest) = self.returning(&selected.device.id) {
                // As for the Cloud below, the wait holds no file lease.
                drop(files);
                waited_for_return = true;
                waits.wait(self.devices().returned(&selected.device.id, rest)).await?;
                continue;
            }
            let admitted = match selected.device.kind {
                DeviceKind::User => {
                    cloud = None;
                    true
                }
                DeviceKind::Managed => cloud
                    .as_ref()
                    .is_some_and(|cloud| cloud.device == selected.device.id),
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
            let host = self.open_host(&files, &selected, cloud.as_ref()).await?;
            self.track_idle(&record.id);
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
    /// transfers, so a transition ends it. `attention` says what it is to
    /// the conversation's activity: a look holds the file gate without
    /// demand while it is admitted, and a stream takes its lease of the
    /// stream gate before it lets go of the file gate, so an idle watch that
    /// reserved the file gate sees every stream admitted before it.
    pub(crate) async fn admit_stream(
        &self,
        id: &ConversationId,
        attention: Attention,
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
        let purpose = match attention {
            Attention::Watches | Attention::Operates => Purpose::Demand,
            Attention::Looks => Purpose::Maintenance,
        };
        let mut waited_for_return = false;
        let (files, mut selected) = loop {
            let files = waits.wait(slot.files.enter(purpose)).await?;
            let selected = self.select_host(&record.id, None, false).await?;
            match self.returning(&selected.device.id) {
                Some(rest) if !waited_for_return => {
                    drop(files);
                    waited_for_return = true;
                    waits.wait(self.devices().returned(&selected.device.id, rest)).await?;
                }
                _ => break (files, selected),
            }
        };
        if !self.devices().online(&selected.device.id) {
            return Err(match selected.device.kind {
                DeviceKind::Managed => Refusal::Stopped.into(),
                DeviceKind::User => HostError::offline("The device has no live runner").into(),
            });
        }
        // A stream shows what the conversation's work left: it makes no
        // directory.
        selected.prepare = false;
        let host = self.open_host(&files, &selected, None).await?;
        self.track_idle(&record.id);
        let watching = match attention {
            Attention::Watches => Some(slot.streams.enter(Purpose::Demand).await),
            Attention::Operates | Attention::Looks => None,
        };
        drop(files);
        Ok(StreamAccess {
            conversation: record.id,
            host,
            device: selected.device.id,
            open,
            watching,
        })
    }

    /// How long an operation on `device` waits for its runner first: the
    /// rest of the grace after the backend started, when the last shutdown
    /// disconnected the device and its runner is not back yet
    /// (`sessions-and-targets.md` § Recovery and persistence). The wait
    /// comes once per admission, before anything answers that the device is
    /// offline or the Cloud stopped, and holds no file lease, so a
    /// transition does not wait for it.
    fn returning(&self, device: &DeviceId) -> Option<std::time::Duration> {
        self.devices().returning(device, self.clock().now().to_jiff())
    }

    /// Takes the Cloud's admission: it wakes a stopped Cloud, joins a boot
    /// under way, or waits for a running reset to finish (`managed`).
    async fn enter_cloud(&self, device: &DeviceRecord) -> Result<CloudAdmission, CloudError> {
        self.cloud_shard().admit_cloud(device).await
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
        let control = self.control();
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
                Some(self.cloud_shard().cloud_device().await?)
            }
            (None, ExecutionTarget::Cloud { .. }) => Some(
                control
                    .managed_device(record.owner.clone())
                    .await?
                    .ok_or(Refusal::Stopped)?,
            ),
            (
                None,
                ExecutionTarget::Device { device_id, .. }
                | ExecutionTarget::Workspace { device_id, .. },
            ) => control.device(device_id.clone()).await?,
        };
        let device = found
            .filter(|device| device.user == record.owner)
            .ok_or(Refusal::DeviceGone)?;
        let (root, prepare) = match named.filter(|host| host.role == HostRole::Attached) {
            Some(attached) => (attached.path, false),
            None => (
                target.path().to_owned(),
                matches!(target, ExecutionTarget::Cloud { .. }),
            ),
        };
        Ok(Selected {
            device,
            root,
            prepare,
        })
    }

    /// The admitted Host of the operation that holds `files`, its Cloud
    /// session directory made first.
    async fn open_host(
        &self,
        files: &FileLease,
        selected: &Selected,
        cloud: Option<&CloudAdmission>,
    ) -> Result<ConversationHost, HostAccessError> {
        let device = &selected.device.id;
        let admission = cloud.map_or(Admission::Free, |cloud| cloud.per_operation.clone());
        let host =
            self.devices()
                .conversation_host(device, files, selected.root.clone(), admission);
        if selected.prepare {
            HostFs::mkdir(&host, &selected.root, MkdirOptions { recursive: true }).await?;
        }
        Ok(ConversationHost {
            host,
            root: selected.root.clone(),
            home: self.devices().home(device),
        })
    }

    /// The Hosts the web app's host routes and `demi host shell --host` accept
    /// (§ Attached hosts): the primary Host, and the attached ones. A shell on
    /// the primary Host starts in the conversation's directory there; one on an
    /// attached Host where the last shell there ended, or in its home before
    /// one ran.
    /// The Hosts the user's conversation `id` reaches: its primary Host, then
    /// its attached ones.
    pub async fn conversation_hosts(
        &self,
        id: &ConversationId,
    ) -> Result<Vec<ReachableHost>, HostAccessError> {
        let record = self.owned_conversation(id).await?;
        let target = self.resolve_target(&record).await?;
        Ok(self.reachable_hosts(&record, &target).await?)
    }

    pub(crate) async fn reachable_hosts(
        &self,
        record: &ConversationRecord,
        target: &ExecutionTarget,
    ) -> Result<Vec<ReachableHost>, StorageError> {
        let control = self.control();
        let primary = target.device();
        let mut hosts = Vec::new();
        if let Some(device) = primary {
            let name = control
                .device(device.clone())
                .await?
                .map_or_else(|| device.to_string(), |record| record.name);
            hosts.push(ReachableHost {
                name,
                device: device.clone(),
                path: target.path().to_owned(),
                role: HostRole::Primary,
            });
        }
        for attached in control.attached_hosts(record.id.clone()).await? {
            if Some(&attached.device) == primary {
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

    /// Lifecycle access (§ Lifecycle access): the release of `conversation`,
    /// by the name the runner knows it by, sent to `device` while its runner
    /// is connected, a paired device's or a running Cloud's. It takes no file
    /// gate and wakes nothing: a device whose runner is not connected, a
    /// stopped Cloud among them, lost the conversation's services with its
    /// connection, and holds no files of it (`resource-lifecycle.md`
    /// § Conversation release). A release that fails is logged with the Host
    /// and the reason, and fails nothing that sent it, since nothing is left
    /// for it (`resource-lifecycle.md` § A release that fails).
    pub(crate) async fn release_on(&self, conversation: &str, device: &DeviceId) {
        let Some(link) = self.devices().link(device) else {
            return;
        };
        if let Err(error) = link.release_conversation(conversation).await {
            tracing::warn!(device = %device, conversation, "the conversation release failed: {error}");
        }
    }
}
