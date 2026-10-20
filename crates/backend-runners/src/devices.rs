//! A user's devices as their runners connect.
//!
//! Each device has one slot holding its connection, `Online` with the
//! connection's link or `Offline` with the account its runner last reported,
//! and every Host handle of the device watches that slot. A device holds one
//! live connection: a new hello with its token is refused while the held
//! connection answers its ping, and adopted once a silent one finished tearing
//! down.
//!
//! A runner reconnects by itself, so a device whose connection dropped is
//! returning for a while (`decisions/0011-a-runner-that-drops-is-awaited.md`):
//! it counts as online in everything its owner's pages are told, and an
//! operation that needs it waits for the runner, until the runner is back
//! or the wait is over. The wait starts the moment the connection fails,
//! before its cleanup ends it. Only an end the backend chose, a revocation or
//! a disconnect of the device, leaves it offline at once. The runners hear
//! only whether a peer's runner is connected, which the wait does not change.
//!
//! ```text
//!  connection drops ──▶ returning (the runners' tuning; 30 s from a start)
//!                         ├── runner back ──▶ online
//!                         └── time is up  ──▶ offline, announced
//! ```
//!
//! A Host handle is made here only for one of the ways to a Host
//! (`crates/backend-host-access/src/lib.rs`):
//!
//! - A conversation's, against a lease of its file gate.
//! - Device access, while the runner is connected.
//! - Machine access, under the Cloud's admission.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket};
use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;
use demi_backend_database::devices::{DeviceRecord, FirstHome};
use demi_backend_page_sync::{Part, UserMarks};
use demi_backend_remote_host::{
    Admission, DeviceJobs, DeviceLink, Link, LinkDriver, LinkEnd, RemoteHost, close_frame,
    socket_frame,
};
use demi_host_interface::{HostIdentity, HostKey};
use demi_runner_protocol::wire::{self, AutostartUnavailable, HostArtifact, Inbound, OperatingSystem};
use demi_runner_protocol::values::BackendUrl;
use demi_web_api_protocol::devices::{DeviceDto, DeviceKind, DeviceLogo, DeviceMark, DeviceState};
use demi_web_api_protocol::ids::{DeviceId, UserId};
use futures_util::future::ready;
use futures_util::{SinkExt as _, StreamExt as _};
use tokio::sync::watch;
use tokio::time::Instant;
use tokio_util::task::AbortOnDropHandle;

use crate::file_gate::FileLease;
use crate::host_key::{HostOwner, host_key};
use crate::install::start_command;

/// Why a revoked device's connection ended.
const REVOKED: &str = "device revoked";

/// Why a revoked device's commands are lost.
const REVOKED_LOST: &str = "its device was revoked";

/// Why the backend's shutdown ended a connection.
const SHUTTING_DOWN: &str = "backend shutting down";

/// How long after the backend starts a device the last shutdown
/// disconnected is returning (`crates/backend-remote-host/src/device_jobs.rs`).
pub const RETURN_GRACE: Duration = Duration::from_secs(30);

/// The devices whose connections the backend's last shutdown ended, whose
/// runners connect again by themselves soon after the start, and until when
/// they are returning. A device leaves the set once its runner is back.
#[derive(Debug)]
pub struct Returning {
    /// A `std` mutex, never held across a wait: a Cloud the machine
    /// manager does not run is forgotten once the start learns it.
    devices: std::sync::Mutex<HashSet<DeviceId>>,
    /// When the wait after the start ends.
    until: Instant,
}

impl Default for Returning {
    fn default() -> Self {
        Self {
            devices: std::sync::Mutex::default(),
            until: Instant::now(),
        }
    }
}

impl Returning {
    /// Takes what the last shutdown recorded, for the start now.
    pub async fn take(control: &ControlService) -> Result<Self, StorageError> {
        let devices: HashSet<DeviceId> = control
            .take_devices_ended_by_shutdown()
            .await?
            .into_iter()
            .collect();
        Ok(Self {
            devices: std::sync::Mutex::new(devices),
            until: Instant::now() + RETURN_GRACE,
        })
    }

    /// The devices whose runners the start waits for.
    pub fn devices(&self) -> Vec<DeviceId> {
        self.devices
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .cloned()
            .collect()
    }

    /// Forgets `device`, whose runner does not come back by itself, as a
    /// Cloud that does not run after an upgrade stopped it, or is back.
    pub fn forget(&self, device: &DeviceId) {
        self.devices
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(device);
    }

    /// Until when `device` is returning, when the last shutdown ended its
    /// connection and its runner is not back yet.
    fn until(&self, device: &DeviceId) -> Option<Instant> {
        self.devices
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(device)
            .then_some(self.until)
    }
}

/// The user's devices, each with its connection slot.
pub struct Devices {
    slots: RefCell<HashMap<DeviceId, Rc<DeviceSlot>>>,
    /// The number of the last download of a runner executable served.
    downloads: Cell<u64>,
    /// The devices the backend's last shutdown disconnected.
    returning: Arc<Returning>,
    /// How long a device whose connection dropped is returning.
    wait: Duration,
    /// The owner's pages, which hear when a device stops returning.
    marks: UserMarks,
    /// How many operations wait for a returning device's runner now, which a
    /// test waits on to know an operation is waiting.
    #[cfg(feature = "testing")]
    return_waits: watch::Sender<usize>,
}

/// One operation counted among those that wait for a returning device's
/// runner, until it is dropped.
#[cfg(feature = "testing")]
struct ReturnWait<'a>(&'a watch::Sender<usize>);

#[cfg(feature = "testing")]
impl<'a> ReturnWait<'a> {
    fn new(waits: &'a watch::Sender<usize>) -> Self {
        waits.send_modify(|waits| *waits += 1);
        Self(waits)
    }
}

#[cfg(feature = "testing")]
impl Drop for ReturnWait<'_> {
    fn drop(&mut self) {
        self.0.send_modify(|waits| *waits -= 1);
    }
}

/// A device's wait for its runner: until `until`, whose end `_expiry`
/// announces to the owner's pages; dropped, it stops.
struct Return {
    until: Instant,
    _expiry: AbortOnDropHandle<()>,
}

impl Return {
    /// A wait until `until`, whose end the device's pages hear of, since
    /// the device then shows offline. The runners hear only of connections,
    /// so the end tells them nothing.
    fn new(until: Instant, marks: UserMarks) -> Self {
        let expiry = tokio::spawn(async move {
            tokio::time::sleep_until(until).await;
            marks.mark(Part::Devices);
        });
        Self {
            until,
            _expiry: AbortOnDropHandle::new(expiry),
        }
    }
}

/// One device's connection, which its Hosts watch, and its jobs, which
/// outlive each connection.
struct DeviceSlot {
    link: watch::Sender<DeviceLink>,
    jobs: DeviceJobs,
    /// Set once the device is revoked: the projects that went with it,
    /// which its runner hears of as its connection ends.
    revoked: RefCell<Option<Vec<String>>>,
    /// Set while the backend ends the live connection itself, whose end
    /// then leaves the device offline at once.
    ended_by_backend: Cell<bool>,
    /// The wait for the runner since the connection last failed, which a
    /// new connection ends; a past `until` is a wait that is over.
    returning: RefCell<Option<Return>>,
    /// Starts the wait the moment the published connection fails, before
    /// its cleanup ends it; dropped with the next publication.
    failure: RefCell<Option<AbortOnDropHandle<()>>>,
    /// The update the device's runner was last sent to, until its next
    /// hello.
    updating: RefCell<Option<Updating>>,
    /// The download of a runner executable this backend serves the device
    /// now, by the number of its request.
    download: Cell<Option<(u64, Download)>>,
    /// The installation directory its runner reported when it last
    /// connected; none before it ever did since the backend started, or
    /// when its runner reported none.
    installation: RefCell<Option<String>>,
    /// The target its runner reported when it last connected, whose
    /// executables it runs; none before it ever did since the backend
    /// started, or when its runner reported none.
    target: RefCell<Option<String>>,
}

/// A runner's update as the backend shows it: until `until`, which each byte of
/// its download served moves on, unless the device's next hello comes first.
///
/// The backend counts the bytes it serves of a runner executable to the device
/// whose token the request carries, and the device's state carries the
/// progress, announced at most once a second.
///
/// - A prefetch shows the device online with its percent.
/// - From the 409 that named the executable until its next hello, the device is
///   updating, for as long as its download moves, so a slow link that keeps
///   receiving shows updating, never offline.
/// - The device shows offline once no byte was served to it for the window and
///   no hello came.
pub struct Updating {
    until: Cell<Instant>,
    /// Ends the window for the device's pages; dropped, it stops.
    expiry: RefCell<AbortOnDropHandle<()>>,
}

impl Updating {
    /// An update shown until `until`, whose end `expiry` announces.
    pub fn new(until: Instant, expiry: AbortOnDropHandle<()>) -> Self {
        Self {
            until: Cell::new(until),
            expiry: RefCell::new(expiry),
        }
    }
}

/// How much of a runner executable's compressed copy the backend has
/// served a device: `served` of its `size` bytes, counting those the
/// device had when its download resumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Download {
    pub served: u64,
    pub size: u64,
}

impl Download {
    /// The share served, in whole percent.
    pub fn percent(&self) -> u8 {
        let percent = self.served.saturating_mul(100) / self.size.max(1);
        u8::try_from(percent.min(100)).expect("at most 100")
    }
}

impl DeviceSlot {
    fn new(returning: Option<Return>) -> Rc<Self> {
        Rc::new(Self {
            link: watch::Sender::new(DeviceLink::Offline { last: None }),
            jobs: DeviceJobs::default(),
            revoked: RefCell::new(None),
            ended_by_backend: Cell::new(false),
            returning: RefCell::new(returning),
            failure: RefCell::new(None),
            updating: RefCell::new(None),
            download: Cell::new(None),
            installation: RefCell::new(None),
            target: RefCell::new(None),
        })
    }

    /// The connection, while one serves the device.
    fn live(&self) -> Option<Link> {
        match &*self.link.borrow() {
            DeviceLink::Online(link) if !link.is_closed() => Some(link.clone()),
            _ => None,
        }
    }

    /// The account the device's runner reports now, or reported when it was
    /// last connected.
    fn identity(&self) -> Option<HostIdentity> {
        match &*self.link.borrow() {
            DeviceLink::Online(link) => Some(link.identity().clone()),
            DeviceLink::Offline { last } => last.clone(),
        }
    }

    /// Waits until no connection of the device is closing: a closing one ends
    /// first, and its end marks the device offline.
    async fn settled(&self) {
        let mut watching = self.link.subscribe();
        // The slot holds the sender while anyone watches it.
        let _ = watching
            .wait_for(|link| !matches!(link, DeviceLink::Online(link) if link.is_closed()))
            .await;
    }

    /// The rest of the wait for the runner, while no connection serves the
    /// device and the wait is not over; `wait` is a whole wait, which a
    /// connection that just failed has before its watch starts it.
    fn rest(&self, wait: Duration) -> Option<Duration> {
        if self.live().is_some() {
            return None;
        }
        if let Some(returning) = &*self.returning.borrow() {
            let rest = returning.until.saturating_duration_since(Instant::now());
            return Some(rest).filter(|rest| !rest.is_zero());
        }
        let failed = matches!(&*self.link.borrow(), DeviceLink::Online(_));
        (failed && !self.ended_by_backend.get() && !wait.is_zero()).then_some(wait)
    }

    /// Starts the device's wait for its runner: `link`, still its
    /// connection, failed, and the backend did not end it.
    fn failed(&self, link: &Link, wait: Duration, marks: &UserMarks) {
        let current = matches!(&*self.link.borrow(), DeviceLink::Online(current) if current == link);
        if current && !self.ended_by_backend.get() && !wait.is_zero() && self.returning.borrow().is_none() {
            *self.returning.borrow_mut() = Some(Return::new(Instant::now() + wait, marks.clone()));
        }
    }

    /// Marks the device offline once `link` ended, unless a newer connection
    /// serves it already; unless the backend ended it, the device is
    /// returning for `wait`, whose end `marks` announces.
    fn went_offline(&self, link: &Link, identity: HostIdentity, wait: Duration, marks: &UserMarks) {
        let ended = self.link.send_if_modified(|current| {
            let ended = matches!(current, DeviceLink::Online(current) if current == link);
            if ended {
                *current = DeviceLink::Offline {
                    last: Some(identity),
                };
            }
            ended
        });
        if ended {
            if self.ended_by_backend.replace(false) || wait.is_zero() {
                self.returning.take();
            } else if self.returning.borrow().is_none() {
                *self.returning.borrow_mut() = Some(Return::new(Instant::now() + wait, marks.clone()));
            }
        }
    }

    /// Publishes `link` as the device's connection: a wait for the runner
    /// is over, and the connection's failure starts the next one, for
    /// `wait`, whose end `marks` announces.
    fn publish(self: &Rc<Self>, link: &Link, wait: Duration, marks: &UserMarks) {
        self.ended_by_backend.set(false);
        self.returning.take();
        self.link.send_replace(DeviceLink::Online(link.clone()));
        let slot = Rc::downgrade(self);
        let link = link.clone();
        let marks = marks.clone();
        let failure = tokio::task::spawn_local(async move {
            link.ended().await;
            if let Some(slot) = slot.upgrade() {
                slot.failed(&link, wait, &marks);
            }
        });
        *self.failure.borrow_mut() = Some(AbortOnDropHandle::new(failure));
    }
}

impl Devices {
    /// The devices of a user, whose pages `marks` tells of them: of these,
    /// `returning` names those the backend's last shutdown disconnected, and
    /// one whose connection drops is returning for `wait`.
    pub fn new(returning: Arc<Returning>, wait: Duration, marks: UserMarks) -> Self {
        Self {
            slots: RefCell::default(),
            downloads: Cell::new(0),
            returning,
            wait,
            marks,
            #[cfg(feature = "testing")]
            return_waits: watch::Sender::new(0),
        }
    }

    /// The device's slot; a new one is returning when the last shutdown
    /// ended the device's connection.
    fn slot(&self, device: &DeviceId) -> Rc<DeviceSlot> {
        self.slots
            .borrow_mut()
            .entry(device.clone())
            .or_insert_with(|| {
                let returning = self
                    .returning
                    .until(device)
                    .filter(|until| Instant::now() < *until)
                    .map(|until| Return::new(until, self.marks.clone()));
                DeviceSlot::new(returning)
            })
            .clone()
    }

    /// The device's jobs, which each connection of its runner takes up.
    pub fn jobs(&self, device: &DeviceId) -> DeviceJobs {
        self.slot(device).jobs.clone()
    }

    /// The device's connection, while its runner is connected.
    pub fn link(&self, device: &DeviceId) -> Option<Link> {
        self.slots.borrow().get(device).and_then(|slot| slot.live())
    }

    /// The devices whose runners are connected now, each with its
    /// connection.
    pub fn links(&self) -> Vec<(DeviceId, Link)> {
        self.slots
            .borrow()
            .iter()
            .filter_map(|(device, slot)| Some((device.clone(), slot.live()?)))
            .collect()
    }

    /// Whether the device's runner is connected now.
    pub fn connected(&self, device: &DeviceId) -> bool {
        self.link(device).is_some()
    }

    /// Whether the device counts as online: its runner is connected, or it
    /// is returning.
    pub fn online(&self, device: &DeviceId) -> bool {
        self.connected(device) || self.returning(device).is_some()
    }

    /// Whether the device's runner serves it now, replaces itself, or
    /// neither.
    pub fn state(&self, device: &DeviceId) -> DeviceState {
        if self.online(device) {
            return DeviceState::Online;
        }
        let updating = self
            .update_until(device)
            .is_some_and(|until| Instant::now() < until);
        if updating {
            DeviceState::Updating
        } else {
            DeviceState::Offline
        }
    }

    /// Shows the device as updating from now on, as `updating` makes it,
    /// unless an update shows already or showed since the device's last
    /// hello: the retries of an update do not extend its window. Whether it
    /// began.
    pub fn begin_update(&self, device: &DeviceId, updating: impl FnOnce() -> Updating) -> bool {
        let slot = self.slot(device);
        let mut current = slot.updating.borrow_mut();
        if current.is_some() {
            return false;
        }
        *current = Some(updating());
        true
    }

    /// The device's runner said hello: an update it was sent to is over.
    /// Whether one was shown.
    pub fn hello(&self, device: &DeviceId) -> bool {
        let ended = self
            .slots
            .borrow()
            .get(device)
            .and_then(|slot| slot.updating.take());
        ended.is_some_and(|updating| Instant::now() < updating.until.get())
    }

    /// When the update the device's runner was sent to stops showing,
    /// unless more of its download is served first; none without one since
    /// its last hello.
    pub fn update_until(&self, device: &DeviceId) -> Option<Instant> {
        let slots = self.slots.borrow();
        let updating = slots.get(device)?.updating.borrow();
        updating.as_ref().map(|updating| updating.until.get())
    }

    /// The backend served the device more of a runner executable, as
    /// request `request`, now `download`: an update it was sent to shows
    /// until `until`, and one whose window had ended shows again, its end
    /// announced by the new `expiry`. Whether the device's state changed.
    pub fn download_moved(
        &self,
        device: &DeviceId,
        request: u64,
        download: Download,
        until: Instant,
        expiry: impl FnOnce() -> AbortOnDropHandle<()>,
    ) -> bool {
        let slot = self.slot(device);
        slot.download.set(Some((request, download)));
        let updating = slot.updating.borrow();
        let Some(updating) = updating.as_ref() else {
            return false;
        };
        let shows_again = Instant::now() >= updating.until.get() && !self.online(device);
        updating.until.set(until);
        if shows_again {
            *updating.expiry.borrow_mut() = expiry();
        }
        shows_again
    }

    /// The target the device's runner reported when it last connected.
    pub fn target(&self, device: &DeviceId) -> Option<String> {
        let slots = self.slots.borrow();
        slots.get(device)?.target.borrow().clone()
    }

    /// A number for a new download of a runner executable, which its
    /// progress names.
    pub fn download_request(&self) -> u64 {
        self.downloads.set(self.downloads.get() + 1);
        self.downloads.get()
    }

    /// Request `request`'s download ended; the device shows no download
    /// unless another request serves it one.
    pub fn download_ended(&self, device: &DeviceId, request: u64) {
        if let Some(slot) = self.slots.borrow().get(device)
            && slot.download.get().is_some_and(|(current, _)| current == request)
        {
            slot.download.set(None);
        }
    }

    /// The download of a runner executable the backend serves the device
    /// now, if any.
    pub fn download(&self, device: &DeviceId) -> Option<Download> {
        let slots = self.slots.borrow();
        slots.get(device)?.download.get().map(|(_, download)| download)
    }

    /// Resolves once a live connection serves the device, as a Cloud's
    /// boot waits for its runner.
    pub async fn until_online(&self, device: &DeviceId) {
        let mut link = self.slot(device).link.subscribe();
        // The slot keeps the sender while the shard lives.
        let _ = link
            .wait_for(|link| matches!(link, DeviceLink::Online(link) if !link.is_closed()))
            .await;
    }

    /// How long an operation that starts now waits for `device`'s runner:
    /// the rest of its wait while it is returning; none otherwise.
    pub fn returning(&self, device: &DeviceId) -> Option<Duration> {
        self.slot(device).rest(self.wait)
    }

    /// Resolves once `device`'s runner is connected, or after `rest`, the
    /// rest of the wait `returning` answered, whichever comes first.
    pub async fn returned(&self, device: &DeviceId, rest: Duration) {
        #[cfg(feature = "testing")]
        let _waiting = ReturnWait::new(&self.return_waits);
        // A runner still not back after the wait: the operation answers as
        // it does for any device that is away.
        let _ = tokio::time::timeout(rest, self.until_online(device)).await;
    }

    /// Whether the device's runner is connected, once a returning device's
    /// runner is back or its wait is over.
    pub async fn reached(&self, device: &DeviceId) -> bool {
        if let Some(rest) = self.returning(device) {
            self.returned(device, rest).await;
        }
        self.connected(device)
    }

    /// How many operations wait for a returning device's runner now.
    #[cfg(feature = "testing")]
    pub fn return_waits(&self) -> watch::Receiver<usize> {
        self.return_waits.subscribe()
    }

    /// Waits until no connection of the device is closing: a closing one
    /// ends first, and its end marks the device offline.
    pub async fn settled(&self, device: &DeviceId) {
        self.slot(device).settled().await;
    }

    /// The home directory the device's runner reported when it last
    /// connected; none before it ever did since the backend started.
    pub fn home(&self, device: &DeviceId) -> Option<String> {
        let identity = self
            .slots
            .borrow()
            .get(device)
            .and_then(|slot| slot.identity());
        identity.map(|identity| identity.home_dir)
    }

    /// A Host on the device, which serves whenever the device's runner is
    /// connected.
    fn host(
        &self,
        device: &DeviceId,
        key: HostKey,
        cwd: String,
        admission: Admission,
    ) -> RemoteHost {
        let slot = self.slot(device);
        RemoteHost::new(key, cwd, slot.link.subscribe(), slot.jobs.clone(), admission)
    }

    /// The conversation's Host on the device, starting work in `cwd`, for
    /// the operation that holds `files`, a lease of the conversation's file
    /// gate (`crates/backend-host-access/src/access.rs`): the handle is the
    /// conversation's the lease names.
    pub fn conversation_host(
        &self,
        device: &DeviceId,
        files: &FileLease,
        cwd: String,
        admission: Admission,
    ) -> RemoteHost {
        let key = host_key(device, HostOwner::Conversation(files.conversation()), &cwd);
        self.host(device, key, cwd, admission)
    }

    /// Machine access (`crates/backend-host-access/src/lib.rs`): the
    /// Cloud's Host starting in `home`, whose operations take `admission`,
    /// the Cloud's; it touches no conversation's files.
    pub fn machine_host(
        &self,
        device: &DeviceId,
        home: String,
        admission: Admission,
    ) -> RemoteHost {
        let key = host_key(device, HostOwner::MachineAccess, &home);
        self.host(device, key, home, admission)
    }

    /// Device access (`crates/backend-host-access/src/lib.rs`): the
    /// device's Host for work that touches no conversation's files, while
    /// its runner is connected, once a returning device's runner is back.
    /// It takes nothing and wakes nothing.
    pub async fn device_access(&self, device: &DeviceId) -> Option<RemoteHost> {
        if !self.reached(device).await {
            return None;
        }
        Some(device_access(device, &self.slot(device)))
    }

    /// Publishes a new connection of the device, which its Hosts may use at
    /// once: what they send waits in the connection's queue until it serves.
    /// `seen` records when the device was last seen, once the connection
    /// ends; its start is its caller's to record.
    pub fn bind(
        &self,
        device: &DeviceId,
        link: Link,
        driver: LinkDriver,
        seen: DeviceRecorder,
        reported: Reported,
    ) -> Serving {
        let slot = self.slot(device);
        slot.installation.replace(reported.installation);
        slot.target.replace(reported.target);
        self.returning.forget(device);
        slot.publish(&link, self.wait, &self.marks);
        tracing::info!(device = %device, "runner connected");
        Serving {
            slot,
            device: device.clone(),
            identity: link.identity().clone(),
            link,
            driver,
            seen,
            wait: self.wait,
            marks: self.marks.clone(),
        }
    }

    /// The device as the web app sees it, whose runner connects to
    /// `backend`, the backend's public URL once it listens.
    pub fn dto(&self, device: DeviceRecord, backend: Option<&BackendUrl>) -> DeviceDto {
        let installation = self
            .slots
            .borrow()
            .get(&device.id)
            .and_then(|slot| slot.installation.borrow().clone());
        let start_command = match (device.kind, backend) {
            (DeviceKind::User, Some(backend)) => Some(start_command(
                backend.url(),
                installation.as_deref(),
                self.home(&device.id).as_deref(),
                device.platform,
            )),
            _ => None,
        };
        let mark = match device.kind {
            DeviceKind::User => Some(DeviceMark {
                logo: DeviceLogo::of(device.platform, device.os.as_ref()),
                number: device.mark_number,
            }),
            DeviceKind::Managed => None,
        };
        DeviceDto {
            mark,
            state: self.state(&device.id),
            download_percent: self.download(&device.id).map(|download| download.percent()),
            home: self.home(&device.id),
            installed: device.installed,
            id: device.id,
            kind: device.kind,
            name: device.name,
            platform: device.platform,
            claimed_at: device.claimed_at,
            last_seen_at: device.last_seen_at,
            os: device.os,
            runner_version: device.runner_version,
            route: device.route,
            start_command,
            autostart_unavailable: device.autostart_unavailable,
        }
    }

    /// `user`'s devices as the web app sees them: the paired ones oldest
    /// first, then the Cloud once its first use made it.
    pub async fn device_list(
        &self,
        control: &ControlService,
        user: &UserId,
        backend: Option<&BackendUrl>,
    ) -> Result<Vec<DeviceDto>, StorageError> {
        let mut devices = control.paired_devices(user.clone()).await?;
        if let Some(cloud) = control.managed_device(user.clone()).await? {
            devices.push(cloud);
        }
        Ok(devices
            .into_iter()
            .map(|device| self.dto(device, backend))
            .collect())
    }

    /// Ends the device's connection, whose runner then reconnects, and
    /// leaves the device offline at once: a returning one stops returning.
    pub fn disconnect(&self, device: &DeviceId, reason: &str) {
        let slot = self.slot(device);
        // A connection still closing ends as the backend's too.
        let connection = match &*slot.link.borrow() {
            DeviceLink::Online(link) => Some(link.clone()),
            DeviceLink::Offline { .. } => None,
        };
        let was_returning = slot.rest(self.wait).is_some();
        if let Some(link) = connection {
            slot.ended_by_backend.set(true);
            link.disconnect(reason);
        }
        if was_returning {
            slot.returning.take();
            self.marks.mark(Part::Devices);
        }
    }

    /// Ends the connection of a revoked device, whose runner hears that it
    /// was revoked, with the names of the `projects` that went with it, and
    /// removes itself.
    pub fn revoke(&self, device: &DeviceId, projects: Vec<String>) {
        let slot = self.slot(device);
        slot.revoked.replace(Some(projects));
        self.disconnect(device, REVOKED);
        // The device's commands go with it.
        slot.jobs.lose_all(REVOKED_LOST);
    }

    /// Ends every connection as the backend shuts down, and records the
    /// devices whose connections it ended: their runners connect again by
    /// themselves, and the next start waits for them (`crates/backend-remote-host/src/device_jobs.rs`).
    pub async fn shut_down(&self, control: &ControlService) {
        let links: Vec<(DeviceId, Link)> = self
            .slots
            .borrow()
            .iter()
            .filter_map(|(device, slot)| {
                let link = slot.live()?;
                // No operation waits for a runner during the shutdown; the
                // next start waits for it instead.
                slot.ended_by_backend.set(true);
                Some((device.clone(), link))
            })
            .collect();
        for (_, link) in &links {
            link.disconnect(SHUTTING_DOWN);
        }
        let ended = links.into_iter().map(|(device, _)| device).collect();
        if let Err(error) = control.set_devices_ended_by_shutdown(ended).await {
            // The next start then answers for these devices at once, as for
            // any device that is away, instead of waiting for their runners.
            tracing::warn!(error = &error as &dyn std::error::Error, "the devices the shutdown disconnected were not recorded");
        }
    }
}

/// What a device's runner reported in its hello that the backend keeps for
/// the device while it runs.
pub struct Reported {
    /// Its installation directory.
    pub installation: Option<String>,
    /// The target whose executables it runs.
    pub target: Option<String>,
}

/// Device access (`crates/backend-host-access/src/lib.rs`) to `device`,
/// whose slot is `slot`.
fn device_access(device: &DeviceId, slot: &DeviceSlot) -> RemoteHost {
    let key = host_key(device, HostOwner::DeviceAccess, "/");
    RemoteHost::new(key, "/".into(), slot.link.subscribe(), slot.jobs.clone(), Admission::Free)
}

/// A bound connection, served over its socket by the task that adopted it.
pub struct Serving {
    slot: Rc<DeviceSlot>,
    device: DeviceId,
    identity: HostIdentity,
    link: Link,
    driver: LinkDriver,
    seen: DeviceRecorder,
    /// How long the device is returning once the connection drops.
    wait: Duration,
    /// The owner's pages, which hear when the device stops returning.
    marks: UserMarks,
}

impl Serving {
    /// Device access (`crates/backend-host-access/src/lib.rs`) over the
    /// device's connections, this one first.
    pub fn device_access(&self) -> RemoteHost {
        device_access(&self.device, &self.slot)
    }

    /// The connection's driver without a socket, for a test that plays the
    /// runner over a transport of its own; the device stays online while
    /// the driver serves.
    #[cfg(feature = "testing")]
    pub fn into_driver(self) -> LinkDriver {
        self.driver
    }

    /// Serves the connection until either end closes it, then marks the
    /// device offline and records when it was last seen.
    pub async fn serve(self, socket: WebSocket) {
        let Serving {
            slot,
            device,
            identity,
            link,
            driver,
            seen,
            wait,
            marks,
        } = self;
        let (mut outgoing, incoming) = socket.split();
        let incoming = incoming.filter_map(|message| ready(socket_frame(message)));
        let served = driver.serve(
            incoming,
            (&mut outgoing)
                .with(|frame: Vec<u8>| ready(Ok::<_, axum::Error>(Message::Binary(frame.into())))),
        );
        // What the runner's cache holds, which the device's record keeps;
        // the watch ends with the connection.
        let mut installed = link.watch_installed();
        let recorder = seen.clone();
        let held = {
            let device = device.clone();
            async move {
                while installed.changed().await.is_ok() {
                    let reported = installed.borrow_and_update().clone();
                    if let Some(artifacts) = reported {
                        recorder.installed(device.clone(), artifacts).await;
                    }
                }
                std::future::pending::<()>().await
            }
        };
        let end = tokio::select! {
            end = served => end,
            () = held => unreachable!("the installed artifacts are watched until the connection ends"),
        };
        match &end {
            LinkEnd::Refused(reason) | LinkEnd::RunnerRefused(reason) => {
                tracing::warn!(device = %device, "runner connection closed: {reason}")
            }
            LinkEnd::Closed(reason) | LinkEnd::Disconnected(reason) => {
                tracing::info!(device = %device, "runner connection ended: {reason}");
            }
        }
        if let Some(projects) = slot.revoked.take() {
            let frame = wire::encode(&Inbound::Revoked { projects })
                .expect("the backend's own runner messages encode")
                .into_bytes();
            // A runner that went away has nothing left to remove; it learns
            // of the revocation when its token is refused.
            let _ = outgoing.send(Message::Binary(frame.into())).await;
        }
        // The connection is over whether or not the close reaches the runner.
        let _ = outgoing.send(Message::Close(close_frame(&end))).await;
        slot.went_offline(&link, identity, wait, &marks);
        seen.touch(device).await;
    }
}

/// Sends one message on a runner's socket that nothing else writes to yet,
/// such as the answer to its hello.
pub async fn send(socket: &mut WebSocket, message: &Inbound) -> Result<(), axum::Error> {
    let frame = wire::encode(message)
        .expect("the backend's own runner messages encode")
        .into_bytes();
    socket.send(Message::Binary(frame.into())).await
}

/// Where a connection records what it learns of its device, which its
/// owner's pages show: that its runner was connected just now, as the
/// connection starts and as it ends, the operating system and the runner
/// release its hello names, and what the runner reports its artifact cache
/// holds. Cloning it is cheap.
#[derive(Clone)]
pub struct DeviceRecorder {
    control: ControlService,
    /// The owner's pages, which show whether the device is online.
    marks: UserMarks,
}

impl DeviceRecorder {
    pub fn new(control: ControlService, marks: UserMarks) -> Self {
        Self { control, marks }
    }

    /// Records what `device`'s runner said in its hello: its operating system,
    /// its release, its instance, its home and why it will not start by itself
    /// after a restart, and that it was connected just now. It advances the
    /// execution contexts the report changes
    /// (`crates/backend-database/src/selection.rs`).
    ///
    /// Answers the directories the runner resolves before a paired device's
    /// first home is recorded, when an earlier release left any
    /// (`crates/server/src/main.rs`).
    pub async fn hello(
        &self,
        device: DeviceId,
        os: OperatingSystem,
        version: String,
        instance: u64,
        home: String,
        autostart_unavailable: Option<AutostartUnavailable>,
    ) -> Option<FirstHome> {
        let first = match self
            .control
            .set_device_runner(device.clone(), os, version, instance, home, autostart_unavailable)
            .await
        {
            Ok(first) => first,
            Err(error) => {
            // The agent's context block names the Host without its system,
            // or with the one recorded before, and Settings shows the device
            // without them, until the next hello records them; a node that
            // read the context before reads no new block until another
            // change advances it.
            tracing::warn!(device = %device, error = &error as &dyn std::error::Error, "operating system and runner release not recorded");
                None
            }
        };
        self.touch(device).await;
        first
    }

    /// Records `first`'s home as `device`'s once its runner resolved each
    /// of `first`'s directories, `resolved`, and shows the pages the
    /// conversations and projects that moved.
    pub async fn settle_first_home(&self, device: DeviceId, first: FirstHome, resolved: HashMap<String, String>) {
        match self.control.settle_first_home(device.clone(), first.home, resolved).await {
            Ok(settled) => {
                if settled.projects_changed {
                    self.marks.mark(Part::Workspaces);
                }
                for conversation in settled.moved {
                    self.marks.mark(Part::Conversation(conversation));
                }
                self.marks.mark(Part::Devices);
            }
            // The home stays unrecorded, so the runner's next hello settles
            // it again.
            Err(error) => {
                tracing::warn!(device = %device, error = &error as &dyn std::error::Error, "first home not recorded");
            }
        }
    }

    /// Records that `device`'s runner was connected just now.
    pub async fn touch(&self, device: DeviceId) {
        if let Err(error) = self.control.touch_device_seen(device.clone()).await {
            // The time is shown to the user and decides nothing.
            tracing::warn!(device = %device, error = &error as &dyn std::error::Error, "last-seen time not recorded");
        }
        self.marks.mark(Part::Devices);
    }

    /// Records that `device`'s artifact cache holds `artifacts`.
    pub async fn installed(&self, device: DeviceId, artifacts: Vec<HostArtifact>) {
        if let Err(error) = self.control.set_device_installed(device.clone(), artifacts).await {
            // The list is shown to the user and offered to plugins; the next
            // report records it again.
            tracing::warn!(device = %device, error = &error as &dyn std::error::Error, "installed artifacts not recorded");
        }
        self.marks.mark(Part::Devices);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use demi_backend_page_sync::{Registration, SyncRegistry};
    use demi_backend_remote_host::testing::{CommandPolicy, TestDevice, TestLink};
    use demi_host_interface::CommandSet;

    use super::*;

    const WINDOW: Duration = Duration::from_secs(2 * 60);

    /// How long a device whose connection dropped is returning here.
    const WAIT: Duration = Duration::from_secs(10);

    fn device_id() -> DeviceId {
        DeviceId::try_from("6a50f29f-4ac3-4121-acac-b4200f48f915").unwrap()
    }

    /// A user's devices, of which `returning` names those the last shutdown
    /// disconnected, and a watch of the user's pages.
    fn devices(returning: Returning) -> (Devices, Registration) {
        let registry = SyncRegistry::default();
        let user = UserId::try_from("2b1f0c4e-5d6a-4b7c-8d9e-0f1a2b3c4d5e").unwrap();
        let marks = registry.of(&user);
        let watch = registry.watch(&user);
        (Devices::new(Arc::new(returning), WAIT, marks), watch)
    }

    /// A connection of the device's runner, published as `bind` publishes
    /// it.
    fn connect(devices: &Devices, runner: &TestDevice, device: &DeviceId) -> TestLink {
        let connection = runner.connect(None);
        devices.slot(device).publish(connection.link(), WAIT, &devices.marks);
        connection
    }

    /// Ends `connection` as its runner going away does, and as `serve` then
    /// records it.
    async fn drop_connection(devices: &Devices, device: &DeviceId, connection: TestLink) {
        let link = connection.link().clone();
        connection.close().await;
        devices.slot(device).went_offline(&link, link.identity().clone(), WAIT, &devices.marks);
    }

    /// Ends `connection` after the backend ended it, as `serve` records it.
    async fn ended(devices: &Devices, device: &DeviceId, connection: TestLink) {
        let link = connection.link().clone();
        connection.ended().await;
        devices.slot(device).went_offline(&link, link.identity().clone(), WAIT, &devices.marks);
    }

    /// Whether the user's pages were told the devices changed since the
    /// last look.
    fn devices_marked(watch: &Registration) -> bool {
        watch.take().parts.contains(&Part::Devices)
    }

    /// A connection that drops and comes back within the wait: the device
    /// shows online throughout, its pages hear of no change once the wait
    /// would have ended, and device access asked for meanwhile goes on over
    /// the new connection.
    #[tokio::test(flavor = "local", start_paused = true)]
    async fn a_drop_the_runner_returns_from_shows_no_change_and_an_operation_meanwhile_goes_on() {
        let (devices, watch) = devices(Returning::default());
        let devices = Rc::new(devices);
        let device = device_id();
        let runner = TestDevice::new(CommandPolicy::new(CommandSet::new()));
        let first = connect(&devices, &runner, &device);
        drop_connection(&devices, &device, first).await;
        assert_eq!(devices.state(&device), DeviceState::Online);
        assert!(devices.online(&device) && !devices.connected(&device));
        assert_eq!(devices.returning(&device), Some(WAIT));

        let operation = tokio::task::spawn_local({
            let devices = devices.clone();
            let device = device.clone();
            async move { devices.device_access(&device).await.is_some() }
        });
        tokio::task::yield_now().await;
        assert!(!operation.is_finished(), "it waits for the runner");
        let started = Instant::now();
        let _second = connect(&devices, &runner, &device);
        assert!(operation.await.unwrap(), "it goes on once the runner is back");
        assert_eq!(Instant::now(), started, "with no wait beyond the runner's");

        watch.take();
        tokio::time::sleep(WAIT * 2).await;
        assert!(!devices_marked(&watch));
        assert_eq!(devices.state(&device), DeviceState::Online);
    }

    /// A connection that failed is waited for at once, before its cleanup
    /// ends it and records the device offline: the device shows online, and
    /// device access waits for the runner instead of answering offline.
    #[tokio::test(flavor = "local", start_paused = true)]
    async fn a_failed_connection_is_waited_for_before_its_cleanup_ends() {
        let (devices, _watch) = devices(Returning::default());
        let devices = Rc::new(devices);
        let device = device_id();
        let runner = TestDevice::new(CommandPolicy::new(CommandSet::new()));
        let connection = connect(&devices, &runner, &device);
        // The connection is torn down; `serve` has not recorded its end.
        connection.close().await;
        assert!(!devices.connected(&device));
        assert_eq!(devices.state(&device), DeviceState::Online);
        assert_eq!(devices.returning(&device), Some(WAIT));

        let operation = tokio::task::spawn_local({
            let devices = devices.clone();
            let device = device.clone();
            async move { devices.device_access(&device).await.is_some() }
        });
        tokio::task::yield_now().await;
        assert!(!operation.is_finished(), "it waits for the runner");
        let _back = connect(&devices, &runner, &device);
        assert!(operation.await.unwrap());
    }

    /// A runner that does not come back: the device shows online until the
    /// wait ends, then offline, which its pages hear of then, and the
    /// operation that waited fails as on an offline device.
    #[tokio::test(flavor = "local", start_paused = true)]
    async fn a_drop_the_runner_does_not_return_from_is_offline_after_the_wait_and_the_waiting_operation_fails() {
        let (devices, watch) = devices(Returning::default());
        let devices = Rc::new(devices);
        let device = device_id();
        let runner = TestDevice::new(CommandPolicy::new(CommandSet::new()));
        let connection = connect(&devices, &runner, &device);
        drop_connection(&devices, &device, connection).await;
        let dropped = Instant::now();
        assert_eq!(devices.state(&device), DeviceState::Online);
        watch.take();

        let operation = tokio::task::spawn_local({
            let devices = devices.clone();
            let device = device.clone();
            async move { devices.device_access(&device).await.is_some() }
        });
        tokio::task::yield_now().await;
        assert!(!operation.is_finished(), "it waits for the runner");
        watch.marked().await;
        assert!(devices_marked(&watch));
        assert_eq!(Instant::now() - dropped, WAIT);
        assert_eq!(devices.state(&device), DeviceState::Offline);
        assert_eq!(devices.returning(&device), None);
        assert!(!operation.await.unwrap(), "it fails as offline");
    }

    /// A revocation and a disconnect of the device leave it offline at
    /// once; a disconnect of a returning device ends its wait, which its
    /// pages hear of.
    #[tokio::test(flavor = "local", start_paused = true)]
    async fn a_revocation_and_a_disconnect_are_offline_at_once() {
        let (devices, watch) = devices(Returning::default());
        let device = device_id();
        let runner = TestDevice::new(CommandPolicy::new(CommandSet::new()));

        let connection = connect(&devices, &runner, &device);
        devices.disconnect(&device, "the account was disabled");
        ended(&devices, &device, connection).await;
        assert_eq!(devices.state(&device), DeviceState::Offline);
        assert_eq!(devices.returning(&device), None);

        let connection = connect(&devices, &runner, &device);
        drop_connection(&devices, &device, connection).await;
        assert_eq!(devices.state(&device), DeviceState::Online);
        watch.take();
        devices.disconnect(&device, "Cloud stopped");
        assert!(devices_marked(&watch));
        assert_eq!(devices.state(&device), DeviceState::Offline);

        let connection = connect(&devices, &runner, &device);
        devices.revoke(&device, Vec::new());
        ended(&devices, &device, connection).await;
        assert_eq!(devices.state(&device), DeviceState::Offline);
        assert_eq!(devices.returning(&device), None);
    }

    /// A device the last shutdown disconnected shows online for the wait
    /// from the start, and offline once it ends, which its pages hear of.
    #[tokio::test(flavor = "local", start_paused = true)]
    async fn a_device_the_restart_disconnected_shows_online_during_its_wait() {
        let device = device_id();
        let started = Instant::now();
        let (devices, watch) = devices(Returning {
            devices: std::sync::Mutex::new(HashSet::from([device.clone()])),
            until: started + RETURN_GRACE,
        });
        assert_eq!(devices.state(&device), DeviceState::Online);
        assert_eq!(devices.returning(&device), Some(RETURN_GRACE));
        watch.marked().await;
        assert!(devices_marked(&watch));
        assert_eq!(Instant::now() - started, RETURN_GRACE);
        assert_eq!(devices.state(&device), DeviceState::Offline);
    }

    /// An update shown from now for the window, whose expiry does nothing.
    fn updating() -> Updating {
        let expiry = tokio::spawn(std::future::pending::<()>());
        Updating::new(Instant::now() + WINDOW, AbortOnDropHandle::new(expiry))
    }

    /// A device whose update keeps failing shows as updating for the window
    /// from the first 409, whatever its retries, and then as offline until
    /// its next hello; a download served to it moves the window on, and
    /// shows a device whose window ended as updating again; after the hello
    /// an update shows again (`crates/runner/src/update.rs`).
    #[tokio::test(start_paused = true)]
    async fn an_update_shows_from_its_first_409_while_its_download_moves() {
        let (devices, _watch) = devices(Returning::default());
        let device = device_id();
        let expiry = || AbortOnDropHandle::new(tokio::spawn(std::future::pending::<()>()));
        assert_eq!(devices.state(&device), DeviceState::Offline);
        assert!(devices.begin_update(&device, updating));
        assert_eq!(devices.state(&device), DeviceState::Updating);
        tokio::time::advance(WINDOW - Duration::from_secs(60)).await;
        // A retry's 409 a minute before the window ends.
        assert!(!devices.begin_update(&device, updating));
        assert_eq!(devices.state(&device), DeviceState::Updating);
        tokio::time::advance(Duration::from_secs(60)).await;
        assert_eq!(devices.state(&device), DeviceState::Offline);
        assert!(!devices.begin_update(&device, updating));
        assert_eq!(devices.state(&device), DeviceState::Offline);
        // A byte served shows it again, for a window from that byte.
        let download = Download { served: 1, size: 4 };
        let until = Instant::now() + WINDOW;
        assert!(devices.download_moved(&device, 1, download, until, expiry));
        assert_eq!(devices.state(&device), DeviceState::Updating);
        assert_eq!(devices.download(&device), Some(download));
        tokio::time::advance(WINDOW - Duration::from_secs(1)).await;
        let download = Download { served: 2, size: 4 };
        let until = Instant::now() + WINDOW;
        assert!(!devices.download_moved(&device, 1, download, until, expiry));
        tokio::time::advance(WINDOW - Duration::from_secs(1)).await;
        assert_eq!(devices.state(&device), DeviceState::Updating);
        devices.download_ended(&device, 1);
        assert_eq!(devices.download(&device), None);
        // The next hello ends the update; a later 409 shows a new one.
        assert!(devices.hello(&device));
        assert_eq!(devices.state(&device), DeviceState::Offline);
        assert!(devices.begin_update(&device, updating));
        assert_eq!(devices.state(&device), DeviceState::Updating);
        assert!(devices.hello(&device));
        assert_eq!(devices.state(&device), DeviceState::Offline);
    }
}
