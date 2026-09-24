//! A user's devices as their runners connect (`runner.md` § Connection and
//! identity): each device has one slot holding its connection, `Online` with
//! the connection's link or `Offline` with the account its runner last
//! reported, and every Host handle of the device watches that slot. A device
//! holds one live connection; a second runner with its token is refused
//! until the first is gone, and one reconnecting is adopted once the old
//! connection finished tearing down.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use axum::extract::ws::{Message, WebSocket};
use demi_host_remote::{Admission, DeviceLink, Link, LinkEnd, LinkOptions, RemoteHost, host_identity};
use demi_runner_protocol::wire::{HelloErrorCode, Inbound, RunnerInfo};
use demi_shell::{HostIdentity, HostKey};
use demi_web_api::devices::DeviceDto;
use demi_web_api::ids::DeviceId;
use futures_util::future::ready;
use futures_util::{SinkExt as _, StreamExt as _};
use tokio::sync::{oneshot, watch};

use super::accept::send;
use super::policy::ShardPolicy;
use crate::shard::Shard;
use crate::storage::control::ControlService;
use crate::storage::devices::DeviceRecord;

/// Why a revoked device's connection ended; its runner hears it and stops.
const REVOKED: &str = "device revoked";

/// The user's devices, each with its connection slot.
#[derive(Default)]
pub(crate) struct Devices {
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
                *current = DeviceLink::Offline { last: Some(identity) };
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
    pub(crate) fn link(&self, device: &DeviceId) -> Option<Link> {
        self.slots.borrow().get(device).and_then(|slot| slot.live())
    }

    pub(crate) fn online(&self, device: &DeviceId) -> bool {
        self.link(device).is_some()
    }

    /// Resolves once a live connection serves the device, as a Cloud's
    /// boot waits for its runner.
    pub(crate) async fn until_online(&self, device: &DeviceId) {
        let mut link = self.slot(device).link.subscribe();
        // The slot keeps the sender while the shard lives.
        let _ = link
            .wait_for(|link| matches!(link, DeviceLink::Online(link) if !link.is_closed()))
            .await;
    }

    /// The home directory the device's runner reported when it last
    /// connected; none before it ever did since the backend started.
    pub(crate) fn home(&self, device: &DeviceId) -> Option<String> {
        let identity = self.slots.borrow().get(device).and_then(|slot| slot.identity());
        identity.map(|identity| identity.home_dir)
    }

    /// A Host on the device, which serves whenever the device's runner is
    /// connected.
    pub(crate) fn host(&self, device: &DeviceId, key: HostKey, cwd: String, admission: Admission) -> RemoteHost {
        RemoteHost::new(key, cwd, self.slot(device).link.subscribe(), admission)
    }

    /// Device access (`sessions-and-targets.md` § Every way to a Host): the
    /// device's Host for work that touches no conversation's files, only
    /// while its runner is connected. It takes nothing and wakes nothing.
    pub(crate) fn device_access(&self, device: &DeviceId) -> Option<RemoteHost> {
        self.link(device)?;
        let key = super::host_key(device, super::HostOwner::DeviceAccess, "/");
        Some(self.host(device, key, "/".into(), Admission::Free))
    }

    /// The device as the browser sees it.
    pub(crate) fn dto(&self, device: DeviceRecord) -> DeviceDto {
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

    /// Ends the device's connection, whose runner then reconnects.
    pub(crate) fn disconnect(&self, device: &DeviceId, reason: &str) {
        if let Some(link) = self.link(device) {
            link.disconnect(reason);
        }
    }

    /// Ends every connection, for `reason`.
    pub(crate) fn disconnect_all(&self, reason: &str) {
        let links: Vec<Link> = self.slots.borrow().values().filter_map(|slot| slot.live()).collect();
        for link in links {
            link.disconnect(reason);
        }
    }
}

impl Shard {
    /// Takes the socket of a runner that presented `device`'s token.
    pub(crate) async fn adopt_runner(self: Rc<Self>, device: DeviceRecord, runner: RunnerInfo, mut socket: WebSocket) {
        let slot = self.devices().slot(&device.id);
        slot.settled().await;
        // From this check until the link is published nothing awaits, so two
        // runners of one device never both become online.
        if slot.live().is_some() {
            tracing::warn!(
                device = %device.id,
                "runner hello refused (already_connected): device {} already has a live connection [{}, {}]",
                device.id,
                runner.name,
                runner.platform
            );
            let refusal = Inbound::HelloError {
                code: HelloErrorCode::AlreadyConnected,
                reason: format!("device {} already has a live connection", device.id),
            };
            // A runner that went away needs no refusal.
            if send(&mut socket, &refusal).await.is_ok() {
                let _ = socket.send(Message::Close(None)).await;
            }
            return;
        }
        let identity = host_identity(&runner.identity);
        let welcome = Inbound::HelloOk {
            device_id: device.id.to_string(),
        };
        // A closing shard takes no runner; dropping the socket closes it.
        let Some(serving) = self.bind(&slot, &device.id, identity) else {
            return;
        };
        // A runner that went away before its welcome ends its connection at
        // once, as the connection reads its socket.
        let _ = send(&mut socket, &welcome).await;
        serving.serve(socket).await;
    }

    /// Takes the socket of a runner a claim just paired with `device`, and
    /// answers the device as it is once the runner is bound.
    pub(crate) async fn adopt_claimed(
        self: Rc<Self>,
        device: DeviceRecord,
        runner: RunnerInfo,
        socket: WebSocket,
        bound: oneshot::Sender<DeviceDto>,
    ) {
        let slot = self.devices().slot(&device.id);
        // A closing shard takes no runner, and the claim that waits for the
        // answer deletes the device it made.
        let Some(serving) = self.bind(&slot, &device.id, host_identity(&runner.identity)) else {
            return;
        };
        // The claim that waits for this answer may have gone; the runner is
        // paired all the same.
        let _ = bound.send(self.devices().dto(device));
        serving.serve(socket).await;
    }

    /// Publishes a new connection of the device, which its Hosts may use at
    /// once: what they send waits in the connection's queue until it serves.
    /// None once the shard is closing, whose close ends every connection it
    /// published before.
    fn bind(self: &Rc<Self>, slot: &Rc<DeviceSlot>, device: &DeviceId, identity: HostIdentity) -> Option<Serving> {
        if self.is_closing() {
            return None;
        }
        let (link, driver) = Link::new(LinkOptions {
            device: device.to_string(),
            identity: identity.clone(),
            pipes: self.pipes().clone(),
            policy: Rc::new(ShardPolicy::new(self, device.clone())),
            ping: self.services().runners.ping,
        });
        slot.link.send_replace(DeviceLink::Online(link.clone()));
        tracing::info!(device = %device, "runner connected");
        let control = self.services().control.clone();
        let seen = device.clone();
        self.tasks().spawn_local(async move { touch_seen(&control, seen).await });
        Some(Serving {
            slot: slot.clone(),
            device: device.clone(),
            identity,
            link,
            driver,
            control: self.services().control.clone(),
        })
    }

    /// Revokes a device: its exposes end with their connections, its row
    /// goes with its attachments, and its runner hears that it was revoked
    /// and stops for good.
    pub(crate) async fn revoke_device(&self, device: DeviceId) -> Result<(), crate::storage::StorageError> {
        self.destroy_exposes_on(&device).await;
        self.services().control.delete_device(device.clone()).await?;
        self.devices().disconnect(&device, REVOKED);
        Ok(())
    }

    /// The user's devices as the browser sees them: the paired ones oldest
    /// first, then the Cloud once its first use made it.
    pub(crate) async fn device_list(&self) -> Result<Vec<DeviceDto>, crate::storage::StorageError> {
        let control = &self.services().control;
        let mut devices = control.paired_devices(self.user().clone()).await?;
        if let Some(cloud) = control.managed_device(self.user().clone()).await? {
            devices.push(cloud);
        }
        Ok(devices.into_iter().map(|device| self.devices().dto(device)).collect())
    }
}

#[cfg(test)]
impl Shard {
    /// Connects `device` through a link nothing serves, whose runner works
    /// in `home`, for a test that needs the device online; the device stays
    /// online while the answer is held.
    pub(crate) fn connect_for_tests(self: &Rc<Self>, device: &DeviceId, home: &str) -> demi_host_remote::LinkDriver {
        let slot = self.devices().slot(device);
        let identity = HostIdentity {
            uid: 501,
            gid: 20,
            hostname: "test".into(),
            home_dir: home.into(),
        };
        self.bind(&slot, device, identity).expect("an open shard binds").driver
    }

    /// Connects `device` through a runner the test plays, working in `home`:
    /// it answers each ping, and each conversation release once
    /// `on_release` ran for the released conversation, so what that reads
    /// is the state before the release was answered.
    pub(crate) fn play_runner_for_tests(
        self: &Rc<Self>,
        device: &DeviceId,
        home: &str,
        on_release: impl Fn(demi_web_api::ids::ConversationId) -> futures_util::future::LocalBoxFuture<'static, ()> + 'static,
    ) {
        use demi_runner_protocol::wire::{self, Outbound};
        let driver = self.connect_for_tests(device, home);
        let (answers, answered) = tokio::sync::mpsc::channel::<Result<Vec<u8>, String>>(8);
        let (frames, mut sent) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
        let incoming = futures_util::stream::unfold(answered, |mut answered| async move {
            answered.recv().await.map(|frame| (frame, answered))
        });
        let outgoing = futures_util::sink::unfold(frames, |frames, frame: Vec<u8>| async move {
            frames.send(frame).await.map_err(|error| error.to_string())?;
            Ok::<_, String>(frames)
        });
        tokio::task::spawn_local(driver.serve(incoming, outgoing));
        tokio::task::spawn_local(async move {
            while let Some(frame) = sent.recv().await {
                let answer = match wire::decode::<Inbound>(&frame) {
                    Ok(Inbound::Ping {}) => Outbound::Pong { jobs: 0 },
                    Ok(Inbound::ConversationRelease { id, conversation_id }) => {
                        let conversation = demi_web_api::ids::ConversationId::try_from(conversation_id)
                            .expect("a release names a conversation");
                        on_release(conversation).await;
                        Outbound::ConversationReleased { id, error: None }
                    }
                    _ => continue,
                };
                let answer = wire::encode(&answer).expect("the test runner's answers encode");
                // The link ends with the test.
                let _ = answers.send(Ok(answer.into_bytes())).await;
            }
        });
    }
}

/// A bound connection, served over its socket by the task that adopted it.
struct Serving {
    slot: Rc<DeviceSlot>,
    device: DeviceId,
    identity: HostIdentity,
    link: Link,
    driver: demi_host_remote::LinkDriver,
    control: ControlService,
}

impl Serving {
    /// Serves the connection until either end closes it, then marks the
    /// device offline and records when it was last seen.
    async fn serve(self, socket: WebSocket) {
        let Serving {
            slot,
            device,
            identity,
            link,
            driver,
            control,
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
                (&mut outgoing).with(|frame: Vec<u8>| ready(Ok::<_, axum::Error>(Message::Binary(frame.into())))),
            )
            .await;
        match &end {
            LinkEnd::Refused(reason) => tracing::warn!(device = %device, "runner connection closed: {reason}"),
            LinkEnd::Closed(reason) | LinkEnd::Disconnected(reason) => {
                tracing::info!(device = %device, "runner connection ended: {reason}");
            }
        }
        if end == LinkEnd::Disconnected(REVOKED.into()) {
            let refusal = Inbound::HelloError {
                code: HelloErrorCode::Revoked,
                reason: REVOKED.into(),
            };
            let frame = demi_runner_protocol::wire::encode(&refusal)
                .expect("the backend's own runner messages encode")
                .into_bytes();
            // A runner that went away needs no refusal.
            let _ = outgoing.send(Message::Binary(frame.into())).await;
        }
        // The connection is over whether or not the close reaches the runner.
        let _ = outgoing.send(Message::Close(None)).await;
        slot.went_offline(&link, identity);
        touch_seen(&control, device).await;
    }
}

/// Records that the device's runner was connected just now.
async fn touch_seen(control: &ControlService, device: DeviceId) {
    if let Err(error) = control.touch_device_seen(device.clone()).await {
        // The time is shown to the user and decides nothing.
        tracing::warn!(device = %device, error = &error as &dyn std::error::Error, "last-seen time not recorded");
    }
}
