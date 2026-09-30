//! A user's devices as their runners connect (`runner.md` § Connection and
//! identity): each device has one slot holding its connection, `Online` with
//! the connection's link or `Offline` with the account its runner last
//! reported, and every Host handle of the device watches that slot. A device
//! holds one live connection; a second runner with its token is refused
//! until the first is gone, and one reconnecting is adopted once the old
//! connection finished tearing down. A Host handle is made here only for one
//! of the ways to a Host (`sessions-and-targets.md` § Every way to a Host):
//! a conversation's against a lease of its file gate, device access while
//! the runner is connected, and machine access under the Cloud's admission.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use axum::extract::ws::{Message, WebSocket};
use demi_backend_storage::StorageError;
use demi_backend_storage::control::ControlService;
use demi_backend_storage::devices::DeviceRecord;
use demi_backend_sync::{Part, UserMarks};
use demi_host_remote::{Admission, DeviceLink, Link, LinkDriver, LinkEnd, RemoteHost};
use demi_runner_protocol::wire::{self, HelloErrorCode, Inbound};
use demi_shell::{HostIdentity, HostKey};
use demi_web_api::devices::DeviceDto;
use demi_web_api::ids::{DeviceId, UserId};
use futures_util::future::ready;
use futures_util::{SinkExt as _, StreamExt as _};
use tokio::sync::watch;

use crate::file_gate::FileLease;
use crate::host_key::{HostOwner, host_key};

/// Why a revoked device's connection ended; its runner hears it and stops.
const REVOKED: &str = "device revoked";

/// The user's devices, each with its connection slot.
#[derive(Default)]
pub struct Devices {
    slots: RefCell<HashMap<DeviceId, Rc<DeviceSlot>>>,
}

/// One device's connection, which its Hosts watch.
struct DeviceSlot {
    link: watch::Sender<DeviceLink>,
}

impl DeviceSlot {
    fn new() -> Rc<Self> {
        Rc::new(Self {
            link: watch::Sender::new(DeviceLink::Offline { last: None }),
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

    /// Resolves once a live connection serves the device, as a Cloud's
    /// boot waits for its runner.
    pub async fn until_online(&self, device: &DeviceId) {
        let mut link = self.slot(device).link.subscribe();
        // The slot keeps the sender while the shard lives.
        let _ = link
            .wait_for(|link| matches!(link, DeviceLink::Online(link) if !link.is_closed()))
            .await;
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
        seen: LastSeen,
    ) -> Serving {
        let slot = self.slot(device);
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

    /// The device as the browser sees it.
    pub fn dto(&self, device: DeviceRecord) -> DeviceDto {
        DeviceDto {
            online: self.online(&device.id),
            home: self.home(&device.id),
            id: device.id,
            kind: device.kind,
            name: device.name,
            platform: device.platform,
            claimed_at: device.claimed_at,
            last_seen_at: device.last_seen_at,
        }
    }

    /// `user`'s devices as the browser sees them: the paired ones oldest
    /// first, then the Cloud once its first use made it.
    pub async fn device_list(
        &self,
        control: &ControlService,
        user: &UserId,
    ) -> Result<Vec<DeviceDto>, StorageError> {
        let mut devices = control.paired_devices(user.clone()).await?;
        if let Some(cloud) = control.managed_device(user.clone()).await? {
            devices.push(cloud);
        }
        Ok(devices.into_iter().map(|device| self.dto(device)).collect())
    }

    /// Ends the device's connection, whose runner then reconnects.
    pub fn disconnect(&self, device: &DeviceId, reason: &str) {
        if let Some(link) = self.link(device) {
            link.disconnect(reason);
        }
    }

    /// Ends the connection of a revoked device, whose runner hears that it
    /// was revoked and stops for good.
    pub fn revoke(&self, device: &DeviceId) {
        self.disconnect(device, REVOKED);
    }

    /// Ends every connection, for `reason`.
    pub fn disconnect_all(&self, reason: &str) {
        let links: Vec<Link> = self
            .slots
            .borrow()
            .values()
            .filter_map(|slot| slot.live())
            .collect();
        for link in links {
            link.disconnect(reason);
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
    seen: LastSeen,
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
        let end = driver
            .serve(
                incoming,
                (&mut outgoing).with(|frame: Vec<u8>| {
                    ready(Ok::<_, axum::Error>(Message::Binary(frame.into())))
                }),
            )
            .await;
        match &end {
            LinkEnd::Refused(reason) => {
                tracing::warn!(device = %device, "runner connection closed: {reason}")
            }
            LinkEnd::Closed(reason) | LinkEnd::Disconnected(reason) => {
                tracing::info!(device = %device, "runner connection ended: {reason}");
            }
        }
        if end == LinkEnd::Disconnected(REVOKED.into()) {
            let refusal = Inbound::HelloError {
                code: HelloErrorCode::Revoked,
                reason: REVOKED.into(),
            };
            let frame = wire::encode(&refusal)
                .expect("the backend's own runner messages encode")
                .into_bytes();
            // A runner that went away needs no refusal.
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

/// Where a connection records that its device's runner was connected just
/// now, which its owner's pages show with whether it is online: a
/// connection records it as it starts and as it ends. Cloning it is cheap.
#[derive(Clone)]
pub struct LastSeen {
    control: ControlService,
    /// The owner's pages, which show whether the device is online.
    marks: UserMarks,
}

impl LastSeen {
    pub fn new(control: ControlService, marks: UserMarks) -> Self {
        Self { control, marks }
    }

    /// Records that `device`'s runner was connected just now.
    pub async fn touch(&self, device: DeviceId) {
        if let Err(error) = self.control.touch_device_seen(device.clone()).await {
            // The time is shown to the user and decides nothing.
            tracing::warn!(device = %device, error = &error as &dyn std::error::Error, "last-seen time not recorded");
        }
        self.marks.mark(Part::Devices);
    }
}
