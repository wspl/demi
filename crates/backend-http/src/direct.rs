//! The signaling socket of a direct channel (`web-api.md` § Direct
//! channel, `direct-channel.md` § Making the channel): the backend
//! introduces the page to its paired device's runner, through device
//! access, and is the only check. It relays each offer to the runner and
//! its answer to the page, and each side's candidates found after them to
//! the other; the socket's close closes the runner's peer, and
//! so does the user turning a plugin on or off, which the socket tells the
//! page so that it offers again with the new introduction.

use std::time::Duration;

use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use demi_backend_remote_host::DirectAnswer;
use demi_backend_runners::command_context::{Reported, reported};
use demi_backend_user_shard::shard::page_socket::PageSocket;
use demi_runner_protocol::direct::{CONNECT_TIMEOUT, Introduction, OfferRefusal};
use demi_web_api_protocol::devices::{DeviceKind, DirectMessage, DirectRequest, Unanswered};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::{DeviceId, UserId};
use futures_util::StreamExt as _;
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
    /// The candidates the runner finds after its answer.
    candidates: mpsc::UnboundedReceiver<String>,
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
        // The candidates the runner finds for the peer after its answer.
        let mut found: Option<mpsc::UnboundedReceiver<String>> = None;
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
                    found = None;
                    self.close_peer().await;
                    if page.send(text(&DirectMessage::Closed)).await.is_err() {
                        break End::PageClosed;
                    }
                    continue;
                }
                candidate = async {
                    match &mut found {
                        Some(found) => found.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    let Some(candidate) = candidate else {
                        // The peer ended: the runner finds nothing more.
                        found = None;
                        continue;
                    };
                    if page.send(text(&DirectMessage::Candidate { candidate })).await.is_err() {
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
            let sdp = match request {
                DirectRequest::Offer { sdp } => sdp,
                DirectRequest::Candidate { candidate } => {
                    // A candidate before any offer has no peer to go to.
                    if offered {
                        self.candidate(candidate).await;
                    }
                    continue;
                }
            };
            offered = true;
            let answer = tokio::select! {
                biased;
                () = &mut link_ended => break End::HostUnreachable,
                answer = self.offer(sdp) => answer,
            };
            let message = match answer {
                Some(Offered { message, switched, candidates }) => {
                    // A new offer replaced the peer, and only an answered
                    // one made another.
                    let answered = matches!(message, DirectMessage::Answer { .. });
                    introduced = answered.then_some(switched);
                    found = answered.then_some(candidates);
                    message
                }
                None => break End::HostUnreachable,
            };
            if page.send(text(&message)).await.is_err() {
                break End::PageClosed;
            }
        };
        if offered {
            self.close_peer().await;
        }
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
                // Listening before the offer goes, so nothing the runner
                // finds is missed.
                let candidates = link.direct_candidates(&peer);
                let offer = link.direct_offer(&peer, sdp, introduction, stun);
                let answered = match tokio::time::timeout(ANSWER_TIMEOUT, offer).await {
                    Ok(Ok(answer)) => Ok(answer),
                    Ok(Err(_)) => return None,
                    Err(_) => Err(Unanswered::Timeout),
                };
                Some((answered, switched, candidates))
            })
            .await
            .ok()
            .flatten()?;
        let (answered, switched, candidates) = answered;
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
        Some(Offered {
            message,
            switched,
            candidates,
        })
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
