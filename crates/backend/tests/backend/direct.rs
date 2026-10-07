//! A direct channel's signaling socket (`web-api.md` § Direct channel):
//! the backend introduces a page to its paired device's real runner, which
//! answers the page's offer; the socket's close closes the runner's peer;
//! the Cloud and an offline device are refused; a quiet socket hears a
//! heartbeat, and the runner's connection ending closes it. A page in
//! process opens the user's streams on the channel: one installs its service
//! from the backend as a relay stream does, but not another stream's
//! package, and a plugin turned off ends its streams with the peer, whose
//! next introduction lacks them.

use std::time::Duration;

use futures_util::{SinkExt as _, StreamExt as _};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

use bytes::Bytes;
use chromiumoxide::cdp::browser_protocol::network::CookieParam;
use demi_command_package_browser_chrome::tabs::environment::{LaunchOptions, with_browser};
use demi_command_package_browser_chrome::driver::numbers::TabNumbers;
use demi_command_protocol::CommandLocale;
use demi_command_sdk::testing::counting_numbers;
use demi_runner_direct::testing::{Heard, Page};
use reqwest::StatusCode;
use tokio_util::sync::CancellationToken;

use crate::streams::{CONVERSATION, conversation};
use crate::support::{EXTRA, Harness, Paired, Session, TestBackend};
use demi_command_protocol::host_target;

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

/// A page in process connected to the device's runner, introduced through
/// `socket`.
async fn connected_page(socket: &mut Socket) -> Page {
    let mut page = Page::connect(async |sdp| {
        let offer = json!({ "type": "offer", "sdp": sdp }).to_string();
        socket.send(Message::Text(offer.into())).await.unwrap();
        let answer = next(socket).await.unwrap();
        match answer["sdp"].as_str() {
            Some(sdp) => Ok(sdp.to_owned()),
            None => Err(answer),
        }
    })
    .await
    .expect("the runner answers");
    assert!(page.wait_connected().await, "the peer connects");
    page
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

// About a second: a paired device's real runner installs the fixture's
// service from the backend for a direct stream, and two pages in process
// connect to it over loopback.
#[tokio::test]
async fn a_direct_stream_installs_its_service_but_not_another_streams_and_a_plugin_turned_off_ends_it() {
    let harness = Harness::new().with_native_fixture().with_extra_package();
    let (backend, master, laptop) = conversation(&harness).await;
    let mut socket = signaling(&backend, &master, &laptop).await;
    let page = connected_page(&mut socket).await;
    let cwd = laptop.runner.home_dir().to_str().unwrap().to_owned();
    let header = |stream: &str, args: Value| {
        json!({ "op": "stream", "conversation": CONVERSATION, "cwd": cwd, "stream": stream, "args": args })
    };

    // The device does not hold the fixture's service: the direct stream
    // asks the backend for it before it starts, as a stream the backend
    // opened would, and no relay stream opens. While it runs, it may
    // install only what a relay stream of its name may: the artifact of
    // the extra package, which another of the user's streams binds, is
    // refused, so the invocation prints no path.
    let extra = &EXTRA.descriptor.targets[host_target()];
    let args = json!({ "sha256": extra.sha256, "size": extra.size });
    let mut install = page.open(header("install", args)).await;
    assert_eq!(install.next().await.json(), json!({ "ok": true }));
    let printed = String::from_utf8(install.bytes_to_end().await).unwrap();
    assert_eq!(printed, "", "nothing was installed");
    let mut direct = page.open(header("echo", json!({}))).await;
    assert_eq!(direct.next().await.json(), json!({ "ok": true }));
    direct.binary(b"ping");
    assert_eq!(direct.next().await, Heard::Binary(Bytes::from_static(b"ping")));

    // Turned off, the plugin's stream ends with the peer, and the socket
    // tells the page to offer again.
    let switched = backend
        .put("/api/plugins/fixture", &master, json!({ "enabled": false }))
        .await;
    assert_eq!(switched.status, StatusCode::NO_CONTENT);
    assert_eq!(direct.next().await, Heard::Closed);
    assert_eq!(next(&mut socket).await.unwrap(), json!({ "type": "closed" }));
    // The next introduction carries the user's streams as they are now.
    let page = connected_page(&mut socket).await;
    let mut gone = page.open(header("echo", json!({}))).await;
    assert_eq!(gone.next().await.json()["error"]["code"], "unknown_stream");
    backend.close().await;
}

/// A page that connects to `deviceId`'s runner directly, as the web app's
/// direct channel does, and reads `length` bytes of `path` from `offset` on
/// a `read` channel: the runner's answer, the bytes in hexadecimal, and the
/// milliseconds the connection took.
const DIRECT_PAGE: &str = r#"<!doctype html><title>Direct</title><script>
async function readDirectly(deviceId, path, offset, length) {
  const started = performance.now()
  const socket = new WebSocket(`ws://${location.host}/api/devices/${deviceId}/direct`)
  await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onclose = reject })
  const connection = new RTCPeerConnection({ iceServers: [] })
  connection.createDataChannel('direct')
  await connection.setLocalDescription(await connection.createOffer())
  const answered = new Promise((resolve) => {
    socket.onmessage = (event) => {
      const message = JSON.parse(event.data)
      if (message.type !== 'heartbeat') resolve(message)
    }
  })
  socket.send(JSON.stringify({ type: 'offer', sdp: connection.localDescription.sdp }))
  const answer = await answered
  if (answer.type !== 'answer') return { failed: JSON.stringify(answer) }
  await connection.setRemoteDescription({ type: 'answer', sdp: answer.sdp })
  await new Promise((resolve, reject) => {
    const change = () => {
      if (connection.connectionState === 'connected') resolve()
      if (connection.connectionState === 'failed') reject(new Error('failed'))
    }
    connection.onconnectionstatechange = change
    change()
  })
  const connectedMs = performance.now() - started
  const channel = connection.createDataChannel('read')
  channel.binaryType = 'arraybuffer'
  channel.onopen = () => channel.send(JSON.stringify({ op: 'read', conversation: 'c1', cwd: '/', path, offset, length }))
  const messages = []
  await new Promise((resolve) => {
    channel.onmessage = (event) => messages.push(event.data)
    channel.onclose = resolve
  })
  const bytes = messages.slice(1).flatMap((part) => [...new Uint8Array(part)])
  connection.close()
  socket.close()
  return {
    answer: messages[0],
    hex: bytes.map((byte) => byte.toString(16).padStart(2, '0')).join(''),
    connectedMs,
  }
}
</script>"#;

// Seconds: Chrome for Testing starts, connects to a paired device's real
// runner over loopback, and reads a range of a file of 300 KB.
#[tokio::test]
#[ignore = "the browser suite: needs DEMI_TEST_CHROME, and DEMI_TEST_CHROME_RUNTIME on Linux (scenarios.md § Browser suite)"]
async fn chrome_connects_to_the_runner_directly_and_reads_a_range_of_a_file() {
    let harness = Harness::new().with_web(&[
        ("index.html", DIRECT_PAGE),
        ("build.json", r#"{ "build": "direct" }"#),
    ]);
    let (backend, master) = harness.start_set_up().await;
    let paired = backend.pair(&master, "laptop").await;
    let file = paired.runner.home_dir().join("clip.bin");
    let content: Vec<u8> = (0..300_000u32).map(|index| (index * 7) as u8).collect();
    std::fs::write(&file, &content).unwrap();

    // Chrome starts as the browser suite starts it, on Linux with the
    // Chrome runtime (`browser.md` § Browser distribution).
    let options = LaunchOptions::pinned(
        demi_command_package_browser_chrome::driver::testing::installation(),
        CommandLocale {
            time_zone: "UTC".into(),
            languages: vec!["en-US".into()],
        },
        demi_command_protocol::ColorScheme::Light,
    )
    .unwrap();
    let (name, value) = master.cookie.split_once('=').unwrap();
    let cookie = CookieParam::builder()
        .name(name)
        .value(value)
        .url(backend.url.as_str())
        .build()
        .unwrap();
    let page_url = format!("{}/index.html", backend.url);
    let read_call = format!(
        "readDirectly({:?}, {:?}, 123456, 70000)",
        paired.id(),
        file.to_str().unwrap()
    );
    let read = with_browser(
        options,
        TabNumbers::new(counting_numbers(), "conversation".into()),
        CancellationToken::new(),
        |browser| async move {
            let tab = browser
                .open("about:blank", &CancellationToken::new(), Duration::from_secs(10))
                .await?;
            let page = tab.page();
            page.set_cookie(cookie).await?;
            page.goto(page_url).await?;
            // The navigation answers before the page's script ran.
            loop {
                let defined = page.evaluate("typeof readDirectly").await?;
                if defined.value() == Some(&json!("function")) {
                    break;
                }
            }
            let read = page.evaluate(read_call).await?;
            Ok(read.value().cloned().expect("readDirectly answers a value"))
        },
    )
    .await
    .unwrap();
    assert!(read.get("failed").is_none(), "{read}");
    let answer: Value = serde_json::from_str(read["answer"].as_str().unwrap()).unwrap();
    assert_eq!(answer["ok"], true);
    assert_eq!(answer["size"], 300_000);
    let expected: String = content[123_456..193_456]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(read["hex"].as_str().unwrap(), expected, "the range, byte for byte");
    eprintln!("the direct channel connected in {} ms", read["connectedMs"]);
    backend.close().await;
}
