//! A user's devices as their runners connect (`runner.md` § Connection and
//! identity): each device has one slot holding its connection, `Online` with
//! the connection's link or `Offline` with the account its runner last
//! reported, and every Host handle of the device watches that slot. A device
//! holds one live connection; a new hello with its token is refused while
//! the held connection answers its ping, and adopted once a silent one
//! finished tearing down. A Host handle is made here only for one
//! of the ways to a Host (`sessions-and-targets.md` § Every way to a Host):
//! a conversation's against a lease of its file gate, device access while
//! the runner is connected, and machine access under the Cloud's admission.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket};
use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;
use demi_backend_database::devices::DeviceRecord;
use demi_backend_page_sync::{Part, UserMarks};
use demi_backend_remote_host::{Admission, DeviceLink, Link, LinkDriver, LinkEnd, RemoteHost};
use demi_host_interface::{HostIdentity, HostKey};
use demi_runner_protocol::wire::{self, HostArtifact, Inbound, OperatingSystem};
use demi_runner_protocol::values::BackendUrl;
use demi_web_api_protocol::devices::{DeviceDto, DeviceKind, DeviceState};
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

/// Why the backend's shutdown ended a connection.
const SHUTTING_DOWN: &str = "backend shutting down";

/// How long after the backend starts an operation waits for the runner of a
/// device the last shutdown disconnected (`sessions-and-targets.md`
/// § Recovery and persistence).
pub const RETURN_GRACE: Duration = Duration::from_secs(30);

/// The devices whose connections the backend's last shutdown ended, whose
/// runners connect again by themselves soon after the start, and until when
/// an operation waits for them.
#[derive(Debug, Default)]
pub struct Returning {
    devices: HashSet<DeviceId>,
    /// When the grace after the start ends.
    until: jiff::Timestamp,
}

impl Returning {
    /// Takes what the last shutdown recorded, for the start at `started`.
    pub async fn take(control: &ControlService, started: jiff::Timestamp) -> Result<Self, StorageError> {
        let devices: HashSet<DeviceId> = control
            .take_devices_ended_by_shutdown()
            .await?
            .into_iter()
            .collect();
        let until = started
            .checked_add(RETURN_GRACE)
            .map_err(StorageError::Time)?;
        Ok(Self { devices, until })
    }

    /// The rest of the grace at `now` for `device`, when the last shutdown
    /// ended its connection.
    fn rest(&self, device: &DeviceId, now: jiff::Timestamp) -> Option<Duration> {
        if !self.devices.contains(device) {
            return None;
        }
        Duration::try_from(self.until.duration_since(now))
            .ok()
            .filter(|rest| !rest.is_zero())
    }
}

/// The user's devices, each with its connection slot.
#[derive(Default)]
pub struct Devices {
    slots: RefCell<HashMap<DeviceId, Rc<DeviceSlot>>>,
    /// The devices the backend's last shutdown disconnected.
    returning: Arc<Returning>,
}

/// One device's connection, which its Hosts watch.
struct DeviceSlot {
    link: watch::Sender<DeviceLink>,
    /// Set once the device is revoked: the projects that went with it,
    /// which its runner hears of as its connection ends.
    revoked: RefCell<Option<Vec<String>>>,
    /// The update the device's runner was last sent to, until its next
    /// hello.
    updating: RefCell<Option<Updating>>,
    /// The installation directory its runner reported when it last
    /// connected; none before it ever did since the backend started, or
    /// when its runner reported none.
    installation: RefCell<Option<String>>,
}

/// A runner's update as the backend shows it (`runner.md` § Runner
/// updates): until `until`, unless the device's next hello comes first.
pub struct Updating {
    until: Instant,
    /// Ends the window for the device's pages; dropped, it stops.
    _expiry: AbortOnDropHandle<()>,
}

impl Updating {
    /// An update shown until `until`, whose end `expiry` announces.
    pub fn new(until: Instant, expiry: AbortOnDropHandle<()>) -> Self {
        Self {
            until,
            _expiry: expiry,
        }
    }
}

impl DeviceSlot {
    fn new() -> Rc<Self> {
        Rc::new(Self {
            link: watch::Sender::new(DeviceLink::Offline { last: None }),
            revoked: RefCell::new(None),
            updating: RefCell::new(None),
            installation: RefCell::new(None),
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

    /// Marks the device offline once `link` ended, unless a newer connection
    /// serves it already.
    fn went_offline(&self, link: &Link, identity: HostIdentity) {
        self.link.send_if_modified(|current| {
            let ended = matches!(current, DeviceLink::Online(current) if current == link);
            if ended {
                *current = DeviceLink::Offline {
                    last: Some(identity),
                };
            }
            ended
        });
    }
}

impl Devices {
    /// The devices of a user, of whom `returning` names those the backend's
    /// last shutdown disconnected.
    pub fn new(returning: Arc<Returning>) -> Self {
        Self {
            slots: RefCell::default(),
            returning,
        }
    }

    fn slot(&self, device: &DeviceId) -> Rc<DeviceSlot> {
        self.slots
            .borrow_mut()
            .entry(device.clone())
            .or_insert_with(DeviceSlot::new)
            .clone()
    }

    /// The device's connection, while its runner is connected.
    pub fn link(&self, device: &DeviceId) -> Option<Link> {
        self.slots.borrow().get(device).and_then(|slot| slot.live())
    }

    pub fn online(&self, device: &DeviceId) -> bool {
        self.link(device).is_some()
    }

    /// Whether the device's runner serves it now, replaces itself, or
    /// neither.
    pub fn state(&self, device: &DeviceId) -> DeviceState {
        if self.online(device) {
            return DeviceState::Online;
        }
        let updating = self.slots.borrow().get(device).is_some_and(|slot| {
            slot.updating
                .borrow()
                .as_ref()
                .is_some_and(|updating| Instant::now() < updating.until)
        });
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
        ended.is_some_and(|updating| Instant::now() < updating.until)
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

    /// How long an operation that starts at `now` waits for `device`'s
    /// runner (`sessions-and-targets.md` § Recovery and persistence): the
    /// rest of the grace after the start, for a device the backend's last
    /// shutdown disconnected whose runner is not back yet; none otherwise.
    pub fn returning(&self, device: &DeviceId, now: jiff::Timestamp) -> Option<Duration> {
        if self.online(device) {
            return None;
        }
        self.returning.rest(device, now)
    }

    /// Resolves once `device`'s runner is connected, or after `rest`, the
    /// rest of the grace `returning` answered, whichever comes first.
    pub async fn returned(&self, device: &DeviceId, rest: Duration) {
        // A runner still not back after the grace: the operation answers as
        // it does for any device that is away.
        let _ = tokio::time::timeout(rest, self.until_online(device)).await;
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
        RemoteHost::new(key, cwd, self.slot(device).link.subscribe(), admission)
    }

    /// The conversation's Host on the device, starting work in `cwd`, for
    /// the operation that holds `files`, a lease of the conversation's file
    /// gate (`sessions-and-targets.md` § Host operations): the handle is the
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

    /// Machine access (`sessions-and-targets.md` § Every way to a Host): the
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

    /// Device access (`sessions-and-targets.md` § Every way to a Host): the
    /// device's Host for work that touches no conversation's files, only
    /// while its runner is connected. It takes nothing and wakes nothing.
    pub fn device_access(&self, device: &DeviceId) -> Option<RemoteHost> {
        self.link(device)?;
        let key = host_key(device, HostOwner::DeviceAccess, "/");
        Some(self.host(device, key, "/".into(), Admission::Free))
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
        installation: Option<String>,
    ) -> Serving {
        let slot = self.slot(device);
        slot.installation.replace(installation);
        slot.link.send_replace(DeviceLink::Online(link.clone()));
        tracing::info!(device = %device, "runner connected");
        Serving {
            slot,
            device: device.clone(),
            identity: link.identity().clone(),
            link,
            driver,
            seen,
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
        DeviceDto {
            state: self.state(&device.id),
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

    /// Ends the device's connection, whose runner then reconnects.
    pub fn disconnect(&self, device: &DeviceId, reason: &str) {
        if let Some(link) = self.link(device) {
            link.disconnect(reason);
        }
    }

    /// Ends the connection of a revoked device, whose runner hears that it
    /// was revoked, with the names of the `projects` that went with it, and
    /// removes itself.
    pub fn revoke(&self, device: &DeviceId, projects: Vec<String>) {
        self.slot(device).revoked.replace(Some(projects));
        self.disconnect(device, REVOKED);
    }

    /// Ends every connection as the backend shuts down, and records the
    /// devices whose connections it ended: their runners connect again by
    /// themselves, and the next start waits for them (`sessions-and-targets.md`
    /// § Recovery and persistence).
    pub async fn shut_down(&self, control: &ControlService) {
        let links: Vec<(DeviceId, Link)> = self
            .slots
            .borrow()
            .iter()
            .filter_map(|(device, slot)| slot.live().map(|link| (device.clone(), link)))
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

/// A bound connection, served over its socket by the task that adopted it.
pub struct Serving {
    slot: Rc<DeviceSlot>,
    device: DeviceId,
    identity: HostIdentity,
    link: Link,
    driver: LinkDriver,
    seen: DeviceRecorder,
}

impl Serving {
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
        } = self;
        let (mut outgoing, incoming) = socket.split();
        let incoming = incoming.filter_map(|message| {
            ready(match message {
                Ok(Message::Binary(frame)) => Some(Ok(frame.to_vec())),
                Ok(Message::Text(_)) => Some(Err("the runner sent a text frame".to_owned())),
                Ok(Message::Ping(_) | Message::Pong(_) | Message::Close(_)) => None,
                Err(error) => Some(Err(error.to_string())),
            })
        });
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
            LinkEnd::Refused(reason) => {
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
        let _ = outgoing.send(Message::Close(None)).await;
        slot.went_offline(&link, identity);
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

    /// Records what `device`'s runner said in its hello: its operating
    /// system and its release, and that it was connected just now.
    pub async fn hello(&self, device: DeviceId, os: OperatingSystem, version: String) {
        if let Err(error) = self.control.set_device_runner(device.clone(), os, version).await {
            // The agent's context block names the Host without its system,
            // and Settings shows the device without them, until the next
            // hello records them.
            tracing::warn!(device = %device, error = &error as &dyn std::error::Error, "operating system and runner release not recorded");
        }
        self.touch(device).await;
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

    use super::*;

    const WINDOW: Duration = Duration::from_secs(5 * 60);

    /// An update shown from now for the window, whose expiry does nothing.
    fn updating() -> Updating {
        let expiry = tokio::spawn(std::future::pending::<()>());
        Updating::new(Instant::now() + WINDOW, AbortOnDropHandle::new(expiry))
    }

    /// A device whose update keeps failing shows as updating for the window
    /// from the first 409, whatever its retries, and then as offline until
    /// its next hello; after the hello an update shows again
    /// (`runner.md` § Runner updates).
    #[tokio::test(start_paused = true)]
    async fn an_update_shows_from_its_first_409_for_the_window_and_retries_do_not_extend_it() {
        let devices = Devices::default();
        let device = DeviceId::try_from("6a50f29f-4ac3-4121-acac-b4200f48f915").unwrap();
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
        // The next hello ends the update; a later 409 shows a new one.
        assert!(!devices.hello(&device));
        assert!(devices.begin_update(&device, updating));
        assert_eq!(devices.state(&device), DeviceState::Updating);
        assert!(devices.hello(&device));
        assert_eq!(devices.state(&device), DeviceState::Offline);
    }
}
