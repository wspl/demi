//! The conversation's host access: the one way to a conversation's primary or
//! attached Host.
//!
//! Each conversation has one slot in its user's shard, with its file gate, its
//! open transfers and the gate its open user streams hold.
//!
//! An operation passes through these checks:
//!
//! - It takes the conversation's file gate, so it excludes an archive or a
//!   target switch.
//! - The target is resolved, and an archived conversation or a device that is
//!   not bound is refused.
//! - In the grace after the backend starts, a device its last shutdown
//!   disconnected is waited for. A restart is not a device going away: its
//!   runners connect again by themselves within seconds.
//! - After the grace, an agent's job on the primary Host waits for its device's
//!   runner however long it takes. An attached device without a live runner is
//!   refused with its offline error.
//! - A Cloud's admission is taken.
//!
//! The file gate is let go while any of these waits. Then the operation runs
//! once.
//!
//! Admission is a loop, because the two things it waits for, the conversation's
//! file gate and a Cloud's admission, are also what a transition takes:
//!
//! ```text
//! 1. Take the conversation's file gate.        Waits while a transition
//!                                              holds it.
//! 2. Resolve the target; check the archive,    Refused: an archived
//!    the binding and the owner.                conversation, a device that
//!                                              is not bound.
//! 3. A Cloud whose admission this attempt does not hold yet: release
//!    the file gate, take the Cloud's admission (wakes a stopped Cloud,
//!    joins a boot, waits for a running reset), and go back to 1.
//! 4. Run the operation, once.
//! 5. Release the file gate and the Cloud's admission.
//! ```
//!
//! The file gate is released before waiting for a Cloud because a reset takes
//! the file gates of the conversations it holds: a caller that kept its own
//! while it waited would wait for the reset forever.
//!
//! For the same reason nothing enters a conversation's file gate while it holds
//! a lease of it. The gate is first-in, first-out, so a reservation queued
//! between the two entries would wait for the first lease while the second
//! waits behind the reservation. A conversation socket's frame is therefore
//! handled under one lease, and a frame with uploads is admitted once on the
//! primary Host through this entry, its uploads written through that admission.
//!
//! Going back to step 1 repeats every check, since while the attempt waited the
//! conversation may have been archived, switched or had its device detached.
//! Only an attempt that has not dispatched its operation waits and starts over.
//! A dispatched operation runs once and is never retried: if a reset or the
//! loss of its device cuts it off, it fails with that cause.
//!
//! An agent's command on a primary device without a live runner waits for it as
//! for a held conversation, so the agent sees a command that is slow, never one
//! that failed for the device's absence. The user's Stop ends every waiting
//! command, which never runs, since a user who stops expects nothing to start
//! later.

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::rc::Rc;

use demi_backend_blobs::ObjectError;
use demi_backend_cloud::machine::{CloudAdmission, CloudError};
use demi_backend_database::StorageError;
use demi_backend_database::conversation_index::{ConversationRecord, ExecutionTarget};
use demi_backend_database::devices::DeviceRecord;
use demi_agent_tools::duration;
use demi_backend_remote_host::{Admission, RemoteHost};
use demi_backend_runners::file_gate::{FileGate, FileLease};
use demi_host_interface::{HostError, HostErrorKind, HostFs, MkdirOptions};
use demi_runner_protocol::files::FsFailure;
use demi_shared_gates::{ActivityGate, GateLease, Purpose, SerialGate};
use demi_web_api_protocol::devices::DeviceKind;
use demi_web_api_protocol::error::ErrorCode;
use futures_util::future::LocalBoxFuture;
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
    /// Every operation on the conversation's Hosts holds a lease of it, and a
    /// transition reserves it. Only host access takes its leases, the ones a
    /// conversation's Host is made against (`Devices::conversation_host`).
    /// Other work holds the conversation through `file_gate`.
    files: FileGate,
    /// Every open user stream holds a demand lease of it: someone watches the
    /// conversation's Host, which is activity
    /// (`crates/backend-user-shard/src/lifecycle/mod.rs`). Nothing reserves it,
    /// since a transition ends the streams instead of waiting for them.
    pub streams: ActivityGate,
    /// The open file transfers and user streams.
    pub transfers: Rc<TransferSet>,
    /// Orders the changes of the conversation's model settings, one at a
    /// time; an open of its tree takes the same turn, so it opens with the
    /// selection the record holds.
    pub settings: SerialGate,
    /// Orders the sends of the conversation's messages saved for later: one
    /// is read, handed to the session and taken off the list before the
    /// next is read, so two pages never send it twice.
    pub saved: SerialGate,
    /// The admissions its running shell jobs hold, each with its job's
    /// device.
    jobs: RefCell<Vec<Rc<JobAdmission>>>,
    /// The agent's jobs whose start waits for the conversation's primary
    /// device to come back (§ Host operations).
    waiting: RefCell<Vec<Rc<WaitingJob>>>,
}

/// A job whose start waits for its offline primary device, `device`, and
/// why it will never start, once a move of the conversation decided that
/// (§ Switch the primary target).
struct WaitingJob {
    device: DeviceId,
    failed: RefCell<Option<String>>,
    /// Cancelled once `failed` is set.
    ended: CancellationToken,
}

/// A job's registration as waiting for its device, until it drops.
struct HeldWait {
    slot: Rc<ConversationSlot>,
    entry: Rc<WaitingJob>,
}

impl HeldWait {
    /// Why the job will never start, once a move decided it.
    fn failed(&self) -> Option<String> {
        self.entry.failed.borrow().clone()
    }

    /// Resolves with why the job will never start, once a move decides it.
    async fn until_failed(&self) -> String {
        self.entry.ended.cancelled().await;
        self.failed().unwrap_or_default()
    }
}

impl Drop for HeldWait {
    fn drop(&mut self) {
        self.slot
            .waiting
            .borrow_mut()
            .retain(|entry| !Rc::ptr_eq(entry, &self.entry));
    }
}

/// A running job's admission, held until the job ends, unless a move away
/// from the job's offline Host leaves the job to that Host's runner
/// without it (`crates/backend-host-access/src/transition.rs`).
struct JobAdmission {
    device: DeviceId,
    admitted: RefCell<Option<Admitted>>,
}

/// A job's hold of its admission, registered with its conversation's slot
/// until it drops.
pub(crate) struct HeldJob {
    slot: Rc<ConversationSlot>,
    entry: Rc<JobAdmission>,
}

impl Drop for HeldJob {
    fn drop(&mut self) {
        self.slot
            .jobs
            .borrow_mut()
            .retain(|entry| !Rc::ptr_eq(entry, &self.entry));
    }
}

impl ConversationSlot {
    /// Holds `admitted`, a job's on `device`, while the job runs.
    pub(crate) fn hold_job(self: &Rc<Self>, device: DeviceId, admitted: Admitted) -> HeldJob {
        let entry = Rc::new(JobAdmission {
            device,
            admitted: RefCell::new(Some(admitted)),
        });
        self.jobs.borrow_mut().push(entry.clone());
        HeldJob {
            slot: self.clone(),
            entry,
        }
    }

    /// Lets go of the admissions of the jobs running on `device`, an offline
    /// Host a move leaves: the jobs run on there, left to its runner, and
    /// no longer hold the conversation's file gate.
    pub fn leave_jobs_on(&self, device: &DeviceId) {
        let left: Vec<Admitted> = self
            .jobs
            .borrow()
            .iter()
            .filter(|entry| entry.device == *device)
            .filter_map(|entry| entry.admitted.take())
            .collect();
        drop(left);
    }

    /// Registers a job as waiting for `device`, the conversation's offline
    /// primary device.
    fn wait_for_device(self: &Rc<Self>, device: DeviceId) -> HeldWait {
        let entry = Rc::new(WaitingJob {
            device,
            failed: RefCell::new(None),
            ended: CancellationToken::new(),
        });
        self.waiting.borrow_mut().push(entry.clone());
        HeldWait {
            slot: self.clone(),
            entry,
        }
    }

    /// Fails the jobs that wait for `device`, which a move left: none of
    /// them was sent, and each ends with `reason`.
    pub fn fail_waiting_for(&self, device: &DeviceId, reason: &str) {
        for entry in self.waiting.borrow().iter().filter(|entry| entry.device == *device) {
            entry.failed.replace(Some(reason.to_owned()));
            entry.ended.cancel();
        }
    }

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
                    saved: SerialGate::new(),
                    jobs: RefCell::default(),
                    waiting: RefCell::default(),
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
/// activity (`crates/backend-user-shard/src/lifecycle/mod.rs`).
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
    /// Whether it is the conversation's primary Host, whose `root` is the
    /// conversation's working tree.
    pub primary: bool,
    pub open: OpenTransfer,
    /// A stream's lease of the conversation's stream gate; none for a call.
    pub watching: Option<GateLease>,
}

/// What ends an admission's waits: the requester leaving, and, for a file
/// transfer, a transition that ends the conversation's transfers. An agent's
/// job also says how it waits for an offline primary device, `device`.
#[derive(Clone, Copy)]
pub struct Waits<'a> {
    pub cancel: &'a CancellationToken,
    pub ended: Option<&'a CancellationToken>,
    pub device: Option<DeviceWait<'a>>,
}

/// How an agent's job waits while the conversation's primary Host is a
/// paired device without a live runner (§ Host operations): what shows the
/// job waiting for the device it names, or no longer waiting with `None`,
/// which the wait awaits before it goes on.
#[derive(Clone, Copy)]
pub struct DeviceWait<'a> {
    pub shown: &'a dyn ShowWait,
}

/// Shows a job waiting for the device `host` names, or no longer waiting
/// with `None`.
pub trait ShowWait {
    fn shown(&self, host: Option<String>) -> LocalBoxFuture<'_, ()>;
}

impl<'a> Waits<'a> {
    pub fn request(cancel: &'a CancellationToken) -> Self {
        Self {
            cancel,
            ended: None,
            device: None,
        }
    }

    /// The waits of an agent's job, which waits for its offline primary
    /// device as `device` says.
    pub fn job(cancel: &'a CancellationToken, device: DeviceWait<'a>) -> Self {
        Self {
            cancel,
            ended: None,
            device: Some(device),
        }
    }

    /// The waits of a file transfer or a user stream, which also end when a
    /// transition ends the conversation's transfers, `ended`.
    pub fn until_ended(cancel: &'a CancellationToken, ended: &'a CancellationToken) -> Self {
        Self {
            cancel,
            ended: Some(ended),
            device: None,
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
    /// Whether it is the conversation's primary Host. An agent's job waits for
    /// it while its runner is away. Its other operations are admitted, as a
    /// command taken up again is, and the Host answers them offline. An
    /// attached Host is refused.
    primary: bool,
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
        // Registered while an agent's job waits for its device, and kept
        // until the admission ends, so that a move decided while the job
        // came back to the file gate still fails it.
        let mut waiting: Option<HeldWait> = None;
        loop {
            let files = waits.wait(slot.files.enter(Purpose::Demand)).await?;
            if let Some(reason) = waiting.as_ref().and_then(HeldWait::failed) {
                return Err(HostError::new(HostErrorKind::Unavailable, reason).into());
            }
            let selected = self.select_host(&record.id, device, true).await?;
            let away = selected.device.kind == DeviceKind::User
                && !self.devices().connected(&selected.device.id);
            if !waited_for_return && let Some(rest) = self.returning(&selected.device.id) {
                // As for the Cloud below, the wait holds no file lease.
                drop(files);
                waited_for_return = true;
                waits.wait(self.devices().returned(&selected.device.id, rest)).await?;
                // A runner not back when the wait ends is away: the operation
                // that waited for it fails, rather than waiting on for the
                // device or its Cloud.
                if !self.devices().connected(&selected.device.id) {
                    return Err(self.away(&selected.device));
                }
                continue;
            }
            // An agent's job waits for its primary device however long it
            // takes, holding no file lease, so a move can fail it. An attached
            // device without a live runner is refused with its offline error.
            // Any other operation on the primary Host is admitted, and its Host
            // answers it offline at once (§ Host operations).
            if away && selected.primary && let Some(wait) = waits.device {
                drop(files);
                let held = waiting.get_or_insert_with(|| slot.wait_for_device(selected.device.id.clone()));
                wait.shown.shown(Some(selected.device.name.clone())).await;
                let woke = waits
                    .wait(async {
                        tokio::select! {
                            () = self.devices().until_online(&selected.device.id) => None,
                            reason = held.until_failed() => Some(reason),
                        }
                    })
                    .await;
                wait.shown.shown(None).await;
                if let Some(reason) = woke? {
                    return Err(HostError::new(HostErrorKind::Unavailable, reason).into());
                }
                continue;
            }
            if away && !selected.primary {
                return Err(self.offline(&selected.device).into());
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
    /// Host (§ Host operations), on the conversation's primary Host or the
    /// bound device `device` names.
    ///
    /// It works like any operation, with these differences:
    ///
    /// - A stopped Cloud is refused instead of woken.
    /// - The admitted stream lets go of the file gate.
    /// - It stays registered with the conversation's transfers, so a transition
    ///   ends it.
    ///
    /// `attention` says what it is to the conversation's activity. A look holds
    /// the file gate without demand while it is admitted. A stream takes its
    /// lease of the stream gate before it lets go of the file gate, so an idle
    /// watch that reserved the file gate sees every stream admitted before it.
    pub(crate) async fn admit_stream(
        &self,
        id: &ConversationId,
        device: Option<&DeviceId>,
        attention: Attention,
        cancel: &CancellationToken,
    ) -> Result<StreamAccess, HostAccessError> {
        let record = self.owned_conversation(id).await?;
        let slot = self.conversations().slot(&record.id);
        // As for a file transfer: no await between the check and the
        // registration.
        let open = slot.transfers.open().map_err(|_| Refusal::Busy)?;
        let waits = Waits::until_ended(cancel, &open.ended);
        let purpose = match attention {
            Attention::Watches | Attention::Operates => Purpose::Demand,
            Attention::Looks => Purpose::Maintenance,
        };
        let mut waited_for_return = false;
        let (files, mut selected) = loop {
            let files = waits.wait(slot.files.enter(purpose)).await?;
            let selected = self.select_host(&record.id, device, false).await?;
            match self.returning(&selected.device.id) {
                Some(rest) if !waited_for_return => {
                    drop(files);
                    waited_for_return = true;
                    waits.wait(self.devices().returned(&selected.device.id, rest)).await?;
                }
                _ => break (files, selected),
            }
        };
        if !self.devices().connected(&selected.device.id) {
            return Err(self.away(&selected.device));
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
            primary: selected.primary,
            open,
            watching,
        })
    }

    /// The offline error of a paired device whose runner is not connected
    /// (§ Host operations), worded for the model so it can decide what to
    /// do: the device's name, how long its runner has been disconnected
    /// since its record last saw it, and what becomes of the commands
    /// running there.
    pub fn offline(&self, device: &DeviceRecord) -> HostError {
        // A device never seen has been away as long as it has been known.
        let since = device.last_seen_at.unwrap_or(device.claimed_at).as_millisecond();
        let away = u64::try_from(self.clock().now().as_millisecond() - since).unwrap_or(0);
        HostError::offline(format!(
            "{} is offline: its runner has been disconnected for {}, so nothing can run there now; commands already running there keep running and report when it is back.",
            device.name,
            duration(away),
        ))
    }

    /// The refusal of an operation on `device`, whose runner is not
    /// connected: a stopped Cloud, or a paired device's offline error.
    fn away(&self, device: &DeviceRecord) -> HostAccessError {
        match device.kind {
            DeviceKind::Managed => Refusal::Stopped.into(),
            DeviceKind::User => self.offline(device).into(),
        }
    }

    /// How long an operation on `device` waits for its runner first: the rest
    /// of its wait while the device is returning, after its connection
    /// dropped or the backend restarted (`crates/backend-runners/src/devices.rs`).
    ///
    /// The wait comes once per admission, before anything answers that the
    /// device is offline or the Cloud stopped. It holds no file lease, so a
    /// transition does not wait for it.
    fn returning(&self, device: &DeviceId) -> Option<std::time::Duration> {
        self.devices().returning(device)
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
            .filter(|device| device.is_host_of(&record.owner))
            .ok_or(Refusal::DeviceGone)?;
        let (root, prepare, primary) = match named.filter(|host| host.role == HostRole::Attached) {
            Some(attached) => (attached.path, false, false),
            None => (
                target.path().to_owned(),
                matches!(target, ExecutionTarget::Cloud { .. }),
                true,
            ),
        };
        Ok(Selected {
            device,
            root,
            prepare,
            primary,
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

    /// The Hosts the user's conversation `id` reaches, which the web app's host
    /// routes and `demi host shell --host` accept
    /// (`crates/backend-database/src/conversation_index.rs`): its primary Host,
    /// then its attached ones.
    ///
    /// A command on the primary Host starts in the conversation's directory
    /// there. A command on an attached Host starts in the directory fixed when
    /// it was attached, or in its home when none was.
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

    /// Lifecycle access (§ Lifecycle access): sends the release of
    /// `conversation`, by the name the runner knows it by, to `device` while
    /// its runner is connected, a paired device's or a running Cloud's.
    ///
    /// It takes no file gate and wakes nothing: a device whose runner is not
    /// connected, a stopped Cloud among them, lost the conversation's services
    /// with its connection, and holds no files of it. A release that fails is
    /// logged with the Host and the reason, and fails nothing that sent it,
    /// since nothing is left for it
    /// (`crates/backend-user-shard/src/lifecycle/conversations.rs`).
    pub(crate) async fn release_on(&self, conversation: &str, device: &DeviceId) {
        let Some(link) = self.devices().link(device) else {
            return;
        };
        if let Err(error) = link.release_conversation(conversation).await {
            tracing::warn!(device = %device, conversation, "the conversation release failed: {error}");
        }
    }

    /// Lifecycle access (§ Lifecycle access): stops the services of
    /// `packages` on every device whose runner is connected, which ends what
    /// they hold for each of the user's conversations, as turning their plugin
    /// off asks (`crates/backend-plugins/src/lib.rs`).
    ///
    /// It wakes nothing, for the reason [`Self::release_on`] gives: a device
    /// that is not connected holds no services. A release that fails is
    /// logged with the Host and the reason, and fails nothing: the services
    /// end with the runner's next connection at the latest.
    pub async fn release_packages(&self, packages: &[String]) {
        let releases = self.devices().links().into_iter().map(|(device, link)| async move {
            if let Err(error) = link.release_packages(packages.to_vec()).await {
                tracing::warn!(device = %device, ?packages, "the package release failed: {error}");
            }
        });
        join_all(releases).await;
    }
}
