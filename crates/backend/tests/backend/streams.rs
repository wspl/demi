//! User streams end to end (`web-api.md` § User streams): the page's
//! socket, the backend's admission and relay, the runner's service stream
//! and a resident native service. The streams bind the runner's test
//! fixture package: `echo` answers what the page sends, `where` reports its
//! context and directory.

use std::time::Duration;

use demi_web_api::error::ErrorCode;
use futures_util::{SinkExt as _, StreamExt as _};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::tungstenite::Message;

use crate::support::{Harness, Paired, Session, TestBackend, answer};

const CONVERSATION: &str = "5d4c3b2a-8f3a-4c1e-9d2b-7a1c2e3f4a01";

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// A signed-in master whose conversation runs in the home of a paired
/// device.
async fn conversation(harness: &Harness) -> (TestBackend, Session, Paired) {
    let (backend, master) = harness.start_set_up().await;
    let created = backend
        .post("/api/conversations", Some(&master), json!({ "id": CONVERSATION }))
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
    let laptop = backend.pair(&master, "laptop").await;
    let home = laptop.runner.home_dir().to_str().unwrap().to_owned();
    let target = json!({ "target": { "kind": "device", "deviceId": laptop.id(), "path": home } });
    let moved = backend
        .patch(&format!("/api/conversations/{CONVERSATION}"), &master, target)
        .await;
    assert_eq!(moved.status, StatusCode::OK, "{}", String::from_utf8_lossy(&moved.body));
    (backend, master, laptop)
}

fn path(name: &str) -> String {
    format!("/api/conversations/{CONVERSATION}/streams/{name}")
}

/// The page's socket on the stream `name`, from the product's origin.
async fn socket(backend: &TestBackend, session: &Session, name: &str) -> Socket {
    let mut request = backend.ws_url(&path(name)).into_client_request().unwrap();
    let headers = request.headers_mut();
    headers.insert("cookie", session.cookie.parse().unwrap());
    headers.insert("origin", backend.url.parse().unwrap());
    let (socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    socket
}

/// What the page receives until the backend closes the socket, and the
/// close's code and reason.
async fn received(socket: &mut Socket) -> (Vec<u8>, u16, String) {
    let mut bytes = Vec::new();
    loop {
        let message = tokio::time::timeout(Duration::from_secs(20), socket.next())
            .await
            .expect("the stream ends within twenty seconds");
        match message {
            Some(Ok(Message::Binary(chunk))) => bytes.extend_from_slice(&chunk),
            Some(Ok(Message::Close(Some(close)))) => return (bytes, close.code.into(), close.reason.to_string()),
            Some(Ok(_)) => {}
            other => panic!("the socket ended without a close: {other:?}"),
        }
    }
}

/// How the route answers an upgrade before upgrading, from `origin`.
async fn refusal(backend: &TestBackend, session: &Session, name: &str, origin: Option<&str>) -> (StatusCode, ErrorCode) {
    let mut headers = vec![
        ("upgrade", "websocket"),
        ("connection", "Upgrade"),
        ("sec-websocket-version", "13"),
        ("sec-websocket-key", "MDEyMzQ1Njc4OWFiY2RlZg=="),
    ];
    if let Some(origin) = origin {
        headers.push(("origin", origin));
    }
    answer(backend.response(Method::GET, &path(name), session, &headers, None).await)
        .await
        .refusal()
}

#[tokio::test]
async fn a_page_opens_a_user_stream_on_the_conversations_host_with_the_users_context_and_directory() {
    let harness = Harness::new().with_native_fixture();
    let (backend, master, laptop) = conversation(&harness).await;
    let mut place = socket(&backend, &master, "where").await;
    let (bytes, code, reason) = received(&mut place).await;
    assert_eq!((code, reason.as_str()), (1000, "completed"));
    let reported: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(reported["context"]["conversation"], json!(CONVERSATION));
    assert_eq!(reported["context"]["caller"], json!({ "kind": "user" }));
    assert_eq!(reported["context"]["locale"], json!({ "timeZone": "UTC", "languages": ["en-US"] }));
    assert_eq!(reported["cwd"], json!(laptop.runner.home_dir().to_str().unwrap()));
    assert_eq!(reported["value"], Value::Null);

    // Bytes go both ways as they are, in order, whatever the message
    // boundaries; the page closing its socket ends the stream.
    let mut echo = socket(&backend, &master, "echo").await;
    let payload: Vec<u8> = (0..3 * 1024 * 1024).map(|index: usize| (index * 7 % 256) as u8).collect();
    let (mut to_backend, mut from_backend) = echo.split();
    let sending = async {
        for chunk in payload.chunks(100_000) {
            to_backend.send(Message::Binary(chunk.to_vec().into())).await.unwrap();
        }
    };
    let receiving = async {
        let mut echoed = Vec::new();
        while echoed.len() < payload.len() {
            match from_backend.next().await {
                Some(Ok(Message::Binary(chunk))) => echoed.extend_from_slice(&chunk),
                other => panic!("the echo ended early: {other:?}"),
            }
        }
        echoed
    };
    let ((), echoed) = tokio::join!(sending, receiving);
    assert!(echoed == payload, "the echo differs from what was sent");
    echo = to_backend.reunite(from_backend).unwrap();
    echo.close(None).await.unwrap();
    backend.close().await;
}

#[tokio::test]
async fn the_route_refuses_a_foreign_origin_an_unknown_stream_an_archived_conversation_and_an_offline_device() {
    let harness = Harness::new().with_native_fixture();
    let (backend, master, mut laptop) = conversation(&harness).await;
    let product = backend.url.clone();
    let foreign = refusal(&backend, &master, "echo", Some("https://elsewhere.example")).await;
    assert_eq!(foreign, (StatusCode::FORBIDDEN, ErrorCode::ForbiddenOrigin));
    let unnamed = refusal(&backend, &master, "echo", None).await;
    assert_eq!(unnamed, (StatusCode::FORBIDDEN, ErrorCode::ForbiddenOrigin));
    let unknown = refusal(&backend, &master, "browser", Some(&product)).await;
    assert_eq!(unknown, (StatusCode::NOT_FOUND, ErrorCode::UnknownStream));
    let plain = backend.get(&path("echo"), Some(&master)).await;
    assert_eq!(plain.refusal(), (StatusCode::FORBIDDEN, ErrorCode::ForbiddenOrigin));
    let request = backend.response(Method::GET, &path("echo"), &master, &[("origin", &product)], None).await;
    assert_eq!(answer(request).await.refusal(), (StatusCode::UPGRADE_REQUIRED, ErrorCode::UpgradeRequired));
    let activity = format!("/api/conversations/{CONVERSATION}/activity");
    assert_eq!(backend.post(&activity, Some(&master), json!({})).await.status, StatusCode::NO_CONTENT);

    laptop.runner.kill().await;
    backend.until_online(&master, laptop.id(), false).await;
    let offline = refusal(&backend, &master, "echo", Some(&product)).await;
    assert_eq!(offline, (StatusCode::CONFLICT, ErrorCode::DeviceOffline));
    laptop.runner.start_again();
    backend.until_online(&master, laptop.id(), true).await;

    let archived = backend
        .patch(&format!("/api/conversations/{CONVERSATION}"), &master, json!({ "archived": true }))
        .await;
    assert_eq!(archived.status, StatusCode::OK, "{}", String::from_utf8_lossy(&archived.body));
    let refused = refusal(&backend, &master, "echo", Some(&product)).await;
    assert_eq!(refused, (StatusCode::CONFLICT, ErrorCode::ConversationArchived));
    let inactive = backend.post(&activity, Some(&master), json!({})).await;
    assert_eq!(inactive.refusal(), (StatusCode::CONFLICT, ErrorCode::ConversationArchived));
    backend.close().await;
}

#[tokio::test]
async fn an_archive_ends_the_conversations_open_streams() {
    let harness = Harness::new().with_native_fixture();
    let (backend, master, _laptop) = conversation(&harness).await;
    let mut echo = socket(&backend, &master, "echo").await;
    // The stream is open once it answers.
    echo.send(Message::Binary(b"ping".to_vec().into())).await.unwrap();
    match echo.next().await {
        Some(Ok(Message::Binary(bytes))) => assert_eq!(&bytes[..], b"ping"),
        other => panic!("the echo did not answer: {other:?}"),
    }
    let archived = backend
        .patch(&format!("/api/conversations/{CONVERSATION}"), &master, json!({ "archived": true }))
        .await;
    assert_eq!(archived.status, StatusCode::OK, "{}", String::from_utf8_lossy(&archived.body));
    let (_, code, reason) = received(&mut echo).await;
    assert_eq!((code, reason.as_str()), (4000, "conversation_changed"));
    backend.close().await;
}
