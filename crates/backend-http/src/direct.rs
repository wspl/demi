//! The signaling socket of a direct channel (`web-api.md` § Direct
//! channel, `direct-channel.md` § Making the channel): the backend
//! introduces the page to its paired device's runner, through device
//! access, and is the only check. It relays each offer to the runner and
//! its answer to the page, each side's candidates found after them to the
//! other, and the page's probes of the relay path and their answers, which
//! never wait behind an offer's answer; the socket's close closes the
//! runner's peer, and so does the user turning a plugin on or off, which the socket tells the
//! page so that it offers again with the new introduction.

use std::time::Duration;

use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use demi_backend_remote_host::{DirectAnswer, PeerEvent};
use demi_backend_runners::command_context::{Reported, reported};
use demi_backend_user_shard::shard::page_socket::PageSocket;
use demi_runner_protocol::direct::{CONNECT_TIMEOUT, Introduction, OfferRefusal};
use demi_web_api_protocol::devices::{DeviceKind, DirectMessage, DirectRequest, Unanswered};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::{DeviceId, UserId};
use futures_util::future::BoxFuture;
use futures_util::{FutureExt as _, StreamExt as _};
use futures_util::stream::SplitStream;
use garde::Validate as _;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::AppState;
use super::body::page_socket;
use super::devices::owned_device;
use super::error::ApiError;
use super::gate::AuthUser;

/// How long the backend waits for the runner's answer to an offer.
const ANSWER_TIMEOUT: Duration = CONNECT_TIMEOUT;

/// `WS /devices/:deviceId/direct`: a device that is not the caller's
/// answers 404, the Cloud 409 `not_a_paired_device`, and a paired device
/// without a connected runner 409 `device_offline`, before the upgrade.
pub(super) async fn open(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Result<Response, ApiError> {
    let device = owned_device(&state, &user.id, &id, None).await?;
    if device.kind == DeviceKind::Managed {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::NotAPairedDevice,
            "A direct channel reaches a paired device, not the Cloud",
        ));
    }
    let upgrade = upgrade.map_err(|_| {
        ApiError::new(
            StatusCode::UPGRADE_REQUIRED,
            ErrorCode::UpgradeRequired,
            "The direct channel's signaling is a WebSocket",
        )
    })?;
    let device = device.id;
    let connected = {
        let device = device.clone();
        state
            .shards
            .of(&user.id)
            .call(move |shard, _| async move { shard.devices().online(&device) })
            .await?
    };
    if !connected {
        return Err(ApiError::device_offline());
    }
    let tuning = state.services.pages;
    Ok(page_socket(upgrade).on_upgrade(move |socket| async move {
        let (sink, from_page) = socket.split();
        let signaling = Signaling {
            state,
            user: user.id,
            device,
            peer: uuid::Uuid::new_v4().simple().to_string(),
        };
        signaling.serve(PageSocket::new(sink, tuning), from_page).await;
    }))
}

/// One page's signaling socket for one device, with the peer id the
/// backend gave it.
struct Signaling {
    state: AppState,
    user: UserId,
    device: DeviceId,
    peer: String,
}

/// What an offer the backend relayed came to.
struct Offered {
    /// What the page hears: the answer, or why there is none.
    message: DirectMessage,
    /// Cancelled once the introduction is out of date.
    switched: CancellationToken,
}

/// How a signaling socket ended, which it closes with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    /// The runner's connection ended.
    HostUnreachable,
    /// The page sent a message it should not have.
    Refused,
    /// The page closed its socket, or it failed.
    PageClosed,
}

impl Signaling {
    async fn serve(self, mut page: PageSocket, mut from_page: SplitStream<WebSocket>) {
        let link_ended = self.link_ended();
        tokio::pin!(link_ended);
        let mut offered = false;
        // What ends the introduction the runner's peer was made with: the
        // user's next turn of a plugin on or off.
        let mut introduced: Option<CancellationToken> = None;
        // What the runner says of the peer: its candidates found after its
        // answer, and its answers to the page's probes.
        let Some(mut said) = self.peer_events().await else {
            page.close(CloseFrame {
                code: 1011,
                reason: "host_unreachable".into(),
            })
            .await;
            return;
        };
        // The offer waiting for the runner's answer, which the page's other
        // messages, its probes above all, do not wait behind.
        let mut answering: Option<BoxFuture<'_, Option<Offered>>> = None;
        // The page's candidates that came while its offer waited: they go
        // once the runner made the peer they belong to.
        let mut held: Vec<String> = Vec::new();
        let end = loop {
            let switched = introduced.clone();
            let request = tokio::select! {
                biased;
                () = &mut link_ended => break End::HostUnreachable,
                () = async {
                    match switched {
                        Some(switched) => switched.cancelled_owned().await,
                        None => std::future::pending().await,
                    }
                } => {
                    // The peer's streams are no longer the user's: the
                    // runner closes it, and the page offers again.
                    introduced = None;
                    self.close_peer().await;
                    if page.send(text(&DirectMessage::Closed)).await.is_err() {
                        break End::PageClosed;
                    }
                    continue;
                }
                answer = async {
                    match answering.as_mut() {
                        Some(answer) => answer.await,
                        None => std::future::pending().await,
                    }
                } => {
                    answering = None;
                    let Some(Offered { message, switched }) = answer else {
                        break End::HostUnreachable;
                    };
                    // A new offer replaced the peer, and only an answered
                    // one made another, which the held candidates are for.
                    let answered = matches!(message, DirectMessage::Answer { .. });
                    introduced = answered.then_some(switched);
                    let candidates = std::mem::take(&mut held);
                    if page.send(text(&message)).await.is_err() {
                        break End::PageClosed;
                    }
                    if answered {
                        for candidate in candidates {
                            self.candidate(candidate).await;
                        }
                    }
                    continue;
                }
                event = said.recv() => {
                    let message = match event {
                        Some(PeerEvent::Candidate(candidate)) => DirectMessage::Candidate { candidate },
                        Some(PeerEvent::Pong(id)) => DirectMessage::Pong { id },
                        // The runner's connection ended, which the link's
                        // end says next.
                        None => {
                            link_ended.as_mut().await;
                            break End::HostUnreachable;
                        }
                    };
                    if page.send(text(&message)).await.is_err() {
                        break End::PageClosed;
                    }
                    continue;
                }
                message = from_page.next() => match message {
                    Some(Ok(Message::Text(text))) => match request_of(text.as_str()) {
                        Some(request) => request,
                        None => break End::Refused,
                    },
                    Some(Ok(Message::Binary(_))) => break End::Refused,
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                    Some(Ok(Message::Close(_)) | Err(_)) | None => break End::PageClosed,
                },
                () = page.silent() => {
                    if page.send(text(&DirectMessage::Heartbeat)).await.is_err() {
                        break End::PageClosed;
                    }
                    continue;
                }
            };
            match request {
                DirectRequest::Offer { sdp } => {
                    // An offer that replaces one still waiting drops it, with
                    // the candidates that came for it.
                    offered = true;
                    held.clear();
                    answering = Some(self.offer(sdp).boxed());
                }
                DirectRequest::Candidate { candidate } if answering.is_some() => held.push(candidate),
                // A candidate before any offer has no peer to go to.
                DirectRequest::Candidate { candidate } => {
                    if offered {
                        self.candidate(candidate).await;
                    }
                }
                DirectRequest::Ping { id } => self.ping(id).await,
            }
        };
        if offered {
            self.close_peer().await;
        }
        self.forget_peer().await;
        let close = match end {
            End::HostUnreachable => Some((1011, "host_unreachable")),
            End::Refused => Some((1003, "invalid_message")),
            End::PageClosed => None,
        };
        if let Some((code, reason)) = close {
            let frame = CloseFrame {
                code,
                reason: reason.into(),
            };
            page.close(frame).await;
        }
    }

    /// Resolves once the device's runner connection ends, or at once when
    /// none serves it.
    async fn link_ended(&self) {
        let device = self.device.clone();
        let waited = self
            .state
            .shards
            .of(&self.user)
            .call(move |shard, cancel| async move {
                let Some(link) = shard.devices().link(&device) else {
                    return;
                };
                tokio::select! {
                    () = link.ended() => {}
                    () = cancel.cancelled() => {}
                }
            })
            .await;
        // A shard that is gone has no runner connection either.
        let _ = waited;
    }

    /// Relays an offer to the runner, as the peer of this socket, with what
    /// the runner needs of the user: the streams of the plugins they have on,
    /// their locale and their color scheme, and the STUN servers to ask.
    /// Answers what to tell the page, what is cancelled once the
    /// introduction is out of date, and the candidates the runner finds
    /// after its answer; none when the runner's connection ended.
    async fn offer(&self, sdp: String) -> Option<Offered> {
        let device = self.device.clone();
        let peer = self.peer.clone();
        let streams = self.state.services.user_streams.clone();
        let stun = self.state.services.stun.clone();
        let Reported {
            locale,
            color_scheme,
        } = reported(&self.state.services.control, &self.user)
            .await
            .ok()?;
        let answered = self
            .state
            .shards
            .of(&self.user)
            .call(move |shard, _| async move {
                // Taken before the streams are read, so a change while they
                // are read is not missed.
                let switched = shard.plugins().next_switch();
                let mut bound = std::collections::BTreeMap::new();
                for (name, binding) in streams.iter() {
                    // A stream of a plugin the user has off does not exist.
                    if matches!(shard.plugins().stream_end(name).await, Ok(Some(_))) {
                        bound.insert(name.to_owned(), binding.clone());
                    }
                }
                let introduction = Introduction {
                    streams: bound,
                    locale,
                    color_scheme,
                };
                let link = shard.devices().link(&device)?;
                let offer = link.direct_offer(&peer, sdp, introduction, stun);
                let answered = match tokio::time::timeout(ANSWER_TIMEOUT, offer).await {
                    Ok(Ok(answer)) => Ok(answer),
                    Ok(Err(_)) => return None,
                    Err(_) => Err(Unanswered::Timeout),
                };
                Some((answered, switched))
            })
            .await
            .ok()
            .flatten()?;
        let (answered, switched) = answered;
        let message = match answered {
            Ok(DirectAnswer::Answer(sdp)) => DirectMessage::Answer { sdp },
            Ok(DirectAnswer::Refused(code, message)) => {
                tracing::debug!(device = %self.device, "a runner refused an offer: {message}");
                let code = match code {
                    OfferRefusal::Busy => Unanswered::Busy,
                    OfferRefusal::InvalidOffer => Unanswered::InvalidOffer,
                };
                DirectMessage::Unanswered { code }
            }
            Err(code) => DirectMessage::Unanswered { code },
        };
        Some(Offered { message, switched })
    }

    /// What the runner says of this socket's peer from now on; none when the
    /// device's runner is no longer connected.
    async fn peer_events(&self) -> Option<mpsc::UnboundedReceiver<PeerEvent>> {
        let device = self.device.clone();
        let peer = self.peer.clone();
        self.state
            .shards
            .of(&self.user)
            .call(move |shard, _| async move { Some(shard.devices().link(&device)?.direct_peer(&peer)) })
            .await
            .ok()
            .flatten()
    }

    /// Relays the page's probe `id` of the relay path to the runner.
    async fn ping(&self, id: u32) {
        let device = self.device.clone();
        let peer = self.peer.clone();
        let relayed = self
            .state
            .shards
            .of(&self.user)
            .call(move |shard, _| async move {
                if let Some(link) = shard.devices().link(&device) {
                    link.direct_ping(&peer, id);
                }
            })
            .await;
        // A shard that is gone has no runner connection left to probe; the
        // page counts the probe as late.
        let _ = relayed;
    }

    /// Relays a candidate the page found after its offer to the runner's
    /// peer.
    async fn candidate(&self, candidate: String) {
        let device = self.device.clone();
        let peer = self.peer.clone();
        let relayed = self
            .state
            .shards
            .of(&self.user)
            .call(move |shard, _| async move {
                if let Some(link) = shard.devices().link(&device) {
                    link.direct_candidate(&peer, candidate);
                }
            })
            .await;
        // A shard that is gone has no runner connection left to tell.
        let _ = relayed;
    }

    /// The socket ended: what the runner says of its peer goes nowhere.
    async fn forget_peer(&self) {
        let device = self.device.clone();
        let peer = self.peer.clone();
        let forgot = self
            .state
            .shards
            .of(&self.user)
            .call(move |shard, _| async move {
                if let Some(link) = shard.devices().link(&device) {
                    link.direct_forget(&peer);
                }
            })
            .await;
        // A shard that is gone holds no link to forget it in.
        let _ = forgot;
    }

    /// The page went: the runner closes its peer.
    async fn close_peer(&self) {
        let device = self.device.clone();
        let peer = self.peer.clone();
        let closed = self
            .state
            .shards
            .of(&self.user)
            .call(move |shard, _| async move {
                if let Some(link) = shard.devices().link(&device) {
                    link.direct_close(&peer);
                }
            })
            .await;
        // A shard that is gone has no runner connection left to tell.
        let _ = closed;
    }
}

/// A valid message of the page's.
fn request_of(message: &str) -> Option<DirectRequest> {
    let request: DirectRequest = serde_json::from_str(message).ok()?;
    request.validate().ok()?;
    Some(request)
}

fn text(message: &DirectMessage) -> String {
    serde_json::to_string(message).expect("a signaling message serializes")
}
