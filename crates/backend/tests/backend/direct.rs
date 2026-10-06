//! A direct channel's signaling socket (`web-api.md` § Direct channel):
//! the backend introduces a page to its paired device's real runner, which
//! answers the page's offer; the socket's close closes the runner's peer;
//! the Cloud and an offline device are refused; a quiet socket hears a
//! heartbeat, and the runner's connection ending closes it.

use std::time::Duration;

use futures_util::{SinkExt as _, StreamExt as _};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

use crate::support::{Harness, Paired, Session, TestBackend};

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// A data channel's offer as a browser makes it, before its candidates.
fn offer() -> String {
    let fingerprint = (0..32).map(|_| "AB").collect::<Vec<_>>().join(":");
    [
        "v=0",
        "o=- 4611731400430051336 2 IN IP4 127.0.0.1",
        "s=-",
        "t=0 0",
        "a=group:BUNDLE 0",
        "a=msid-semantic: WMS",
        "m=application 9 UDP/DTLS/SCTP webrtc-datachannel",
        "c=IN IP4 0.0.0.0",
        "a=ice-ufrag:pAge",
        "a=ice-pwd:aPasswordOfTwentyTwoCharacters",
        "a=ice-options:trickle",
        &format!("a=fingerprint:sha-256 {fingerprint}"),
        "a=setup:actpass",
        "a=mid:0",
        "a=sctp-port:5000",
        "a=max-message-size:262144",
        "",
    ]
    .join("\r\n")
}

/// The device's signaling upgrade, from the product's origin.
fn signaling_request(
    backend: &TestBackend,
    session: &Session,
    device: &str,
) -> tokio_tungstenite::tungstenite::handshake::client::Request {
    let mut request = backend
        .ws_url(&format!("/api/devices/{device}/direct"))
        .into_client_request()
        .unwrap();
    let headers = request.headers_mut();
    headers.insert("cookie", session.cookie.parse().unwrap());
    headers.insert("origin", backend.url.parse().unwrap());
    request
}

async fn signaling(backend: &TestBackend, session: &Session, paired: &Paired) -> Socket {
    let request = signaling_request(backend, session, paired.id());
    let (socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    socket
}

/// The status a refused upgrade answers with, and its code.
async fn refused(backend: &TestBackend, session: &Session, device: &str) -> (u16, Value) {
    let request = signaling_request(backend, session, device);
    match tokio_tungstenite::connect_async(request).await {
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            let body = response.body().clone().unwrap_or_default();
            (response.status().as_u16(), serde_json::from_slice(&body).unwrap())
        }
        other => panic!("expected a refused upgrade, got {:?}", other.map(|_| ())),
    }
}

/// The next message the backend sends, or the socket's close code.
async fn next(socket: &mut Socket) -> Result<Value, u16> {
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => return Ok(serde_json::from_str(text.as_str()).unwrap()),
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
            Some(Ok(Message::Close(frame))) => {
                return Err(frame.map_or(1005, |frame| u16::from(frame.code)));
            }
            other => panic!("expected a signaling message, got {other:?}"),
        }
    }
}

async fn send_offer(socket: &mut Socket) {
    let offer = json!({ "type": "offer", "sdp": offer() }).to_string();
    socket.send(Message::Text(offer.into())).await.unwrap();
}

// A second: a paired device's real runner answers nine offers.
#[tokio::test]
async fn the_runner_answers_offers_and_a_closed_socket_frees_its_peer() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let paired = backend.pair(&master, "laptop").await;

    let mut sockets = Vec::new();
    for _ in 0..demi_runner_protocol::direct::MAX_PEERS {
        let mut socket = signaling(&backend, &master, &paired).await;
        send_offer(&mut socket).await;
        let answer = next(&mut socket).await.unwrap();
        assert_eq!(answer["type"], "answer", "{answer}");
        let sdp = answer["sdp"].as_str().unwrap();
        assert!(sdp.contains("a=candidate"), "the runner's addresses: {sdp}");
        assert!(sdp.contains("127.0.0.1"), "loopback is offered: {sdp}");
        sockets.push(socket);
    }
    let mut over = signaling(&backend, &master, &paired).await;
    send_offer(&mut over).await;
    assert_eq!(
        next(&mut over).await.unwrap(),
        json!({ "type": "unanswered", "code": "busy" })
    );

    // The page went: the backend closes its peer, and another page's
    // offer is answered.
    let mut closed = sockets.pop().unwrap();
    closed.close(None).await.unwrap();
    drop(closed);
    let answered = loop {
        send_offer(&mut over).await;
        let message = next(&mut over).await.unwrap();
        if message["type"] == "answer" {
            break message;
        }
    };
    assert_eq!(answered["type"], "answer");

    // An offer that is no data channel's is not answered.
    let invalid = json!({ "type": "offer", "sdp": "v=0\r\n" }).to_string();
    over.send(Message::Text(invalid.into())).await.unwrap();
    assert_eq!(next(&mut over).await.unwrap()["type"], "unanswered");
    // A message the page should not send closes the socket.
    over.send(Message::Text("{\"type\":\"use\"}".into())).await.unwrap();
    assert_eq!(next(&mut over).await, Err(1003));
    backend.close().await;
}

#[tokio::test]
async fn the_cloud_and_a_device_without_its_runner_are_refused() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let cloud = "0b6f1f0e-7a52-4c39-9c51-0e2d6b1c7a11";
    harness
        .control_database()
        .execute(
            "INSERT INTO devices (id, user_id, kind, name, platform, claimed_at)
             VALUES (?1, ?2, 'managed', 'Cloud', 'linux', 0)",
            rusqlite::params![cloud, master.user.id.as_str()],
        )
        .unwrap();
    let (status, body) = refused(&backend, &master, cloud).await;
    assert_eq!((status, body["code"].as_str()), (409, Some("not_a_paired_device")));

    let mut paired = backend.pair(&master, "laptop").await;
    let id = paired.id().to_owned();
    paired.runner.stop().await;
    backend.until_online(&master, &id, false).await;
    let (status, body) = refused(&backend, &master, &id).await;
    assert_eq!((status, body["code"].as_str()), (409, Some("device_offline")));
    backend.close().await;
}

/// A quiet socket hears a heartbeat (0.2 s here), and the runner's
/// connection ending closes the socket with `host_unreachable`.
#[tokio::test]
async fn a_quiet_socket_hears_a_heartbeat_and_closes_when_the_runner_goes() {
    let mut harness = Harness::new();
    harness.pages.heartbeat = Duration::from_millis(200);
    let (backend, master) = harness.start_set_up().await;
    let mut paired = backend.pair(&master, "laptop").await;
    let mut socket = signaling(&backend, &master, &paired).await;
    assert_eq!(next(&mut socket).await.unwrap(), json!({ "type": "heartbeat" }));

    paired.runner.stop().await;
    let closed = loop {
        match next(&mut socket).await {
            Ok(message) => assert_eq!(message["type"], "heartbeat"),
            Err(code) => break code,
        }
    };
    assert_eq!(closed, 1011);
    backend.close().await;
}
