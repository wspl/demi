//! The user's port forwards in their shard (`port-forwarding.md`): what
//! Settings › Port Forwarding and `demi host forward` read and change, what
//! each runner is told of the forwards it is an end of, what the Listen On
//! runners announce of them, and the backend's part in a connection: the
//! relay's pipes from one runner to the other, and the introduction of two
//! runners to each other, as a page is introduced to a runner
//! (`direct-channel.md` § Making the channel). Every way to the two Hosts
//! is device access: a stopped Cloud is not woken, and a connection to an
//! offline Host is refused at once (`sessions-and-targets.md` § Every way
//! to a Host).

use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use demi_backend_database::StorageError;
use demi_backend_database::devices::DeviceRecord;
use demi_backend_database::port_forwards::{PortForwardChange, PortForwardRecord, PortForwardRefusal};
use demi_backend_page_sync::Part;
use demi_backend_remote_host::{DirectAnswer, ForwardEvent, Link, PeerEvent};
use demi_runner_protocol::direct::{CONNECT_TIMEOUT, DialRefusal, Introduced, OfferRefusal};
use demi_runner_protocol::forward::{DEADLINE, Forward, ForwardRunState};
use demi_web_api_protocol::devices::{DeviceKind, DeviceRoute};
use demi_web_api_protocol::ids::{DeviceId, PortForwardId};
use demi_web_api_protocol::port_forwards::{PortForwardDto, PortForwardPatch};
use tokio_util::sync::CancellationToken;

use super::Shard;

/// How long the backend waits for the other runner's answer to an offer.
const ANSWER_TIMEOUT: Duration = CONNECT_TIMEOUT;

/// What the shard keeps of the user's forwards while it runs: what each
/// Listen On runner last announced, and the peers it introduced between two
/// runners, by the peer's id.
#[derive(Default)]
pub(crate) struct ForwardsHeld {
    /// What each runner announced of each forward, by the forward and the
    /// runner: a forward shows what its current Listen On said, so a late
    /// word of the Host it listened on before changes nothing.
    states: std::cell::RefCell<HashMap<(String, DeviceId), ForwardRunState>>,
    /// Each introduced peer: what ends its relay of candidates, and its
    /// offering and answering devices, the only ones that may close it.
    peers: std::cell::RefCell<HashMap<String, IntroducedPeer>>,
    /// The forwards each connected runner was told last; a runner not
    /// listed holds none, as each connection of it starts with none.
    told: std::cell::RefCell<HashMap<DeviceId, Vec<Forward>>>,
}

/// A peer the shard introduced between two runners.
struct IntroducedPeer {
    closed: CancellationToken,
    offering: DeviceId,
    answering: DeviceId,
}

/// Why a change of a forward was refused.
#[derive(Debug, thiserror::Error)]
pub enum PortForwardError {
    #[error(transparent)]
    Refused(#[from] PortForwardRefusal),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// What a page's or a command's patch changes.
fn change_of(patch: PortForwardPatch) -> PortForwardChange {
    {
        PortForwardChange {
            name: patch.name.map(|name| name.into_string()),
            listen_device: patch.listen_device,
            listen_port: patch.listen_port,
            connect_device: patch.connect_device,
            connect_address: patch.connect_address,
            enabled: patch.enabled,
        }
    }
}

/// Whether the runners of two devices may talk directly: neither is a
/// Cloud, which runs beside the backend, and neither's route is Relay Only
/// (`port-forwarding.md` § The path of a connection).
fn may_talk_directly(one: &DeviceRecord, other: &DeviceRecord) -> bool {
    [one, other]
        .iter()
        .all(|device| device.kind == DeviceKind::User && device.route != DeviceRoute::Server)
}

impl Shard {
    /// The user's forwards as a page lists them, oldest first, each with
    /// what its Listen On runner last announced of it.
    pub async fn port_forward_list(&self) -> Result<Vec<PortForwardDto>, StorageError> {
        let records = self.services().control.port_forwards(self.user().clone()).await?;
        Ok(records.into_iter().map(|record| self.forward_dto(record)).collect())
    }

    fn forward_dto(&self, record: PortForwardRecord) -> PortForwardDto {
        let state = self
            .forwards
            .states
            .borrow()
            .get(&(record.id.to_string(), record.listen_device.clone()))
            .cloned();
        PortForwardDto {
            id: record.id,
            name: record.name,
            listen_device: record.listen_device,
            listen_port: record.listen_port,
            connect_device: record.connect_device,
            connect_address: record.connect_address,
            enabled: record.enabled,
            state,
            created_at: record.created_at,
        }
    }

    /// `POST /port-forwards` and `demi host forward add`: a forward from its
    /// defaults with `patch` over them, listening by default on the first
    /// of the user's Hosts as the devices list orders them.
    pub async fn create_port_forward(&self, patch: PortForwardPatch) -> Result<PortForwardDto, PortForwardError> {
        let first = self.host_records().await?.into_iter().next().map(|device| device.id);
        let stored = self
            .services()
            .control
            .create_port_forward(self.user().clone(), first, change_of(patch))
            .await??;
        self.forwards_changed().await;
        Ok(self.forward_dto(stored))
    }

    /// `PATCH /port-forwards/:id`.
    pub async fn change_port_forward(
        &self,
        id: PortForwardId,
        patch: PortForwardPatch,
    ) -> Result<PortForwardDto, PortForwardError> {
        let stored = self
            .services()
            .control
            .change_port_forward(self.user().clone(), id, change_of(patch))
            .await??;
        self.forwards_changed().await;
        Ok(self.forward_dto(stored))
    }

    /// `DELETE /port-forwards/:id` and `demi host forward remove`: its
    /// listener closes and its connections end.
    pub async fn remove_port_forward(&self, id: PortForwardId) -> Result<(), PortForwardError> {
        let removed = self
            .services()
            .control
            .remove_port_forward(self.user().clone(), id.clone())
            .await?;
        if !removed {
            return Err(PortForwardRefusal::NotFound.into());
        }
        self.forwards.states.borrow_mut().retain(|(forward, _), _| forward != id.as_str());
        self.forwards_changed().await;
        Ok(())
    }

    /// `POST /port-forwards/:id/retry`: Try Again, which its Listen On
    /// runner does when its port was in use; nothing while that runner is
    /// offline, whose next start tries.
    pub async fn retry_port_forward(&self, id: PortForwardId) -> Result<(), PortForwardError> {
        let records = self.services().control.port_forwards(self.user().clone()).await?;
        let forward = records
            .into_iter()
            .find(|record| record.id == id)
            .ok_or(PortForwardRefusal::NotFound)?;
        if let Some(link) = self.devices().link(&forward.listen_device) {
            link.forward_retry(forward.id.as_str());
        }
        Ok(())
    }

    /// The user's Hosts in the devices list's order: the paired devices
    /// oldest first, then the Cloud.
    pub(crate) async fn host_records(&self) -> Result<Vec<DeviceRecord>, StorageError> {
        let control = &self.services().control;
        let mut devices = control.paired_devices(self.user().clone()).await?;
        devices.extend(control.managed_device(self.user().clone()).await?);
        Ok(devices)
    }

    /// The user's forwards or Hosts changed: every page receives the
    /// forwards, and every connected runner the forwards it is an end of.
    pub(crate) async fn forwards_changed(&self) {
        self.mark(Part::PortForwards);
        let (records, devices) = match (
            self.services().control.port_forwards(self.user().clone()).await,
            self.host_records().await,
        ) {
            (Ok(records), Ok(devices)) => (records, devices),
            (Err(error), _) | (_, Err(error)) => {
                // The runners keep the forwards they were told last, until
                // the next change or connection tells them again.
                tracing::warn!(error = &error as &dyn std::error::Error, "the runners were not told of the user's forwards");
                return;
            }
        };
        // Each runner hears its forwards when they differ from what it was
        // told last, so one that has none hears nothing.
        for device in &devices {
            let Some(link) = self.devices().link(&device.id) else {
                continue;
            };
            let forwards = self.runner_forwards(&device.id, &records, &devices);
            let mut told = self.forwards.told.borrow_mut();
            if told.get(&device.id).map_or(forwards.is_empty(), |last| *last == forwards) {
                continue;
            }
            told.insert(device.id.clone(), forwards.clone());
            link.forwards(forwards, self.services().stun.clone());
        }
    }

    /// The forwards that run with `device` at an end: on, with a Connect To.
    fn runner_forwards(&self, device: &DeviceId, records: &[PortForwardRecord], devices: &[DeviceRecord]) -> Vec<Forward> {
        let find = |id: &DeviceId| devices.iter().find(|device| device.id == *id);
        records
            .iter()
            .filter(|record| record.enabled)
            .filter_map(|record| {
                let connect = record.connect_device.as_ref()?;
                let listens = record.listen_device == *device;
                let connects = connect == device;
                if !listens && !connects {
                    return None;
                }
                let peer = if listens { connect } else { &record.listen_device };
                let direct = match (find(&record.listen_device), find(connect)) {
                    (Some(listen), Some(connect)) => may_talk_directly(listen, connect),
                    _ => false,
                };
                Some(Forward {
                    id: record.id.to_string(),
                    listen: listens.then_some(record.listen_port),
                    connect: connects.then(|| record.connect_address.to_string()),
                    peer: peer.to_string(),
                    peer_online: self.devices().online(peer),
                    direct,
                })
            })
            .collect()
    }

    /// `device`'s runner connected or its connection ended: what it
    /// announced is gone, its pages hear whether it runs, and every runner
    /// with a forward to it hears whether it is there.
    pub(crate) async fn forwards_presence(&self, device: &DeviceId, connected: bool) {
        // A runner that connected holds none, as does one that is gone; a
        // connection that a newer one replaced leaves what that one was told.
        let online = self.devices().online(device);
        if connected || !online {
            self.forwards.told.borrow_mut().remove(device);
        }
        if !online {
            self.forwards
                .states
                .borrow_mut()
                .retain(|(_, listen), _| listen != device);
        }
        self.forwards_changed().await;
    }

    /// What the runner of `device` said of a forward or of a peer it made
    /// with another runner.
    pub(crate) fn forward_event(self: &Rc<Self>, device: &DeviceId, event: ForwardEvent) {
        match event {
            ForwardEvent::State { forward, state } => {
                let mut states = self.forwards.states.borrow_mut();
                let key = (forward, device.clone());
                match state {
                    Some(state) => {
                        states.insert(key, state);
                    }
                    None => {
                        states.remove(&key);
                    }
                }
                drop(states);
                self.mark(Part::PortForwards);
            }
            ForwardEvent::Open { id, forward } => {
                let shard = self.clone();
                let device = device.clone();
                self.tasks().spawn_local(async move { shard.open_forwarded(device, id, forward).await });
            }
            ForwardEvent::Dial { id, peer, device: other, sdp } => {
                let shard = self.clone();
                let device = device.clone();
                self.tasks()
                    .spawn_local(async move { shard.introduce(device, id, peer, other, sdp).await });
            }
            ForwardEvent::Closed { peer } => {
                // Only the pair's own runners close their peer.
                let mut peers = self.forwards.peers.borrow_mut();
                let ours = peers
                    .get(&peer)
                    .is_some_and(|introduced| introduced.offering == *device || introduced.answering == *device);
                if ours && let Some(introduced) = peers.remove(&peer) {
                    introduced.closed.cancel();
                }
            }
        }
    }

    /// A connection of `forward` on `device`, its Listen On, that goes
    /// through the relay: a pipe each way between the two runners, the
    /// Connect To runner told to connect, or a refusal at once when it is
    /// offline or the forward does not run.
    async fn open_forwarded(&self, device: DeviceId, id: String, forward: String) {
        let Some(link) = self.devices().link(&device) else {
            return;
        };
        let records = match self.services().control.port_forwards(self.user().clone()).await {
            Ok(records) => records,
            Err(error) => {
                link.forward_opened(id, Err(error.to_string()));
                return;
            }
        };
        let record = records
            .into_iter()
            .find(|record| record.id.as_str() == forward && record.listen_device == device && record.enabled);
        let Some((record, connect)) = record.and_then(|record| {
            let connect = record.connect_device.clone()?;
            Some((record, connect))
        }) else {
            link.forward_opened(id, Err("The forward does not run".into()));
            return;
        };
        let Some(other) = self.devices().link(&connect) else {
            link.forward_opened(id, Err("The Host it connects to is offline".into()));
            return;
        };
        // Both runners have the forward's deadline to open both pipes.
        let up = self.pipes().mint_within(Some(device.as_str()), Some(connect.as_str()), DEADLINE);
        let down = self.pipes().mint_within(Some(connect.as_str()), Some(device.as_str()), DEADLINE);
        other.forward_connect(record.id.as_str(), up.wire_ref(), down.wire_ref());
        link.forward_opened(id, Ok((down.wire_ref(), up.wire_ref())));
    }

    /// Introduces `device`'s runner, which offered peer `peer` with `sdp`,
    /// to the runner of `other`, the other end of one of its forwards, as
    /// the signaling socket introduces a page; relays each side's
    /// candidates to the other until the peer ends, either connection ends,
    /// or the offering runner says it closed.
    async fn introduce(&self, device: DeviceId, id: String, peer: String, other: String, sdp: String) {
        let Some(link) = self.devices().link(&device) else {
            return;
        };
        let refuse = |code: DialRefusal, message: &str| link.direct_dialed(id.clone(), Err((code, message.to_owned())));
        let Ok(other) = DeviceId::try_from(other) else {
            refuse(DialRefusal::Unavailable, "No such Host");
            return;
        };
        let allowed = match (self.services().control.port_forwards(self.user().clone()).await, self.host_records().await) {
            (Ok(records), Ok(devices)) => {
                let find = |id: &DeviceId| devices.iter().find(|device| device.id == *id);
                let forwarded = records.iter().any(|record| {
                    record.enabled && record.listen_device == device && record.connect_device.as_ref() == Some(&other)
                });
                let direct = match (find(&device), find(&other)) {
                    (Some(one), Some(two)) => may_talk_directly(one, two),
                    _ => false,
                };
                forwarded && direct
            }
            _ => false,
        };
        if !allowed {
            refuse(DialRefusal::Unavailable, "The two Hosts do not talk directly");
            return;
        }
        let Some(answering) = self.devices().link(&other) else {
            refuse(DialRefusal::Unavailable, "The Host it connects to is offline");
            return;
        };
        let closed = CancellationToken::new();
        {
            let mut peers = self.forwards.peers.borrow_mut();
            // A peer id is its offering runner's: another's is refused.
            if peers.get(&peer).is_some_and(|introduced| introduced.offering != device) {
                drop(peers);
                refuse(DialRefusal::Unavailable, "The peer belongs to another Host");
                return;
            }
            let introduced = IntroducedPeer {
                closed: closed.clone(),
                offering: device.clone(),
                answering: other.clone(),
            };
            if let Some(replaced) = peers.insert(peer.clone(), introduced) {
                replaced.closed.cancel();
            }
        }
        let offering_said = link.direct_peer(&peer);
        let answering_said = answering.direct_peer(&peer);
        let introduced = Introduced::Runner {
            device: device.to_string(),
        };
        let offer = answering.direct_offer(&peer, sdp, introduced, self.services().stun.clone());
        match tokio::time::timeout(ANSWER_TIMEOUT, offer).await {
            Ok(Ok(DirectAnswer::Answer(sdp))) => link.direct_dialed(id, Ok(sdp)),
            Ok(Ok(DirectAnswer::Refused(code, message))) => {
                let code = match code {
                    OfferRefusal::Busy => DialRefusal::Busy,
                    OfferRefusal::InvalidOffer => DialRefusal::InvalidOffer,
                };
                link.direct_dialed(id, Err((code, message)));
                self.forget_introduced(&peer, &link, &answering);
                return;
            }
            Ok(Err(error)) => {
                link.direct_dialed(id, Err((DialRefusal::Unavailable, error.to_string())));
                self.forget_introduced(&peer, &link, &answering);
                return;
            }
            Err(_) => {
                link.direct_dialed(id, Err((DialRefusal::Unavailable, "The other runner did not answer in time".into())));
                answering.direct_close(&peer);
                self.forget_introduced(&peer, &link, &answering);
                return;
            }
        }
        relay_candidates(&peer, &link, offering_said, &answering, answering_said, closed).await;
        self.forget_introduced(&peer, &link, &answering);
    }

    /// The introduced peer `peer` is over: neither connection hears more of
    /// it.
    fn forget_introduced(&self, peer: &str, offering: &Link, answering: &Link) {
        self.forwards.peers.borrow_mut().remove(peer);
        offering.direct_forget(peer);
        answering.direct_forget(peer);
    }
}

/// Relays each runner's candidates of peer `peer` to the other until the
/// offering runner says the peer closed, or either connection ends; then
/// the answering runner closes its side, and the offering runner too when
/// its connection still stands.
async fn relay_candidates(
    peer: &str,
    offering: &Link,
    mut offering_said: tokio::sync::mpsc::UnboundedReceiver<PeerEvent>,
    answering: &Link,
    mut answering_said: tokio::sync::mpsc::UnboundedReceiver<PeerEvent>,
    closed: CancellationToken,
) {
    loop {
        tokio::select! {
            () = closed.cancelled() => break,
            () = offering.ended() => break,
            () = answering.ended() => break,
            said = offering_said.recv() => match said {
                Some(PeerEvent::Candidate(candidate)) => answering.direct_candidate(peer, candidate),
                Some(PeerEvent::Pong(_)) => {}
                None => break,
            },
            said = answering_said.recv() => match said {
                Some(PeerEvent::Candidate(candidate)) => offering.direct_candidate(peer, candidate),
                Some(PeerEvent::Pong(_)) => {}
                None => break,
            },
        }
    }
    answering.direct_close(peer);
    offering.direct_close(peer);
}
