//! The WebSocket transport and the fallback to server-sent events
//! (`providers.md` § Endpoints).

use std::{net::SocketAddr, sync::Arc, time::Duration};

use demi_core::TokenUsage;
use demi_provider::{
    ErrorCode, Provider, ProviderEvent,
    credentials::MemoryCredentialPool,
    quota::MemorySnapshots,
    testing::{FixedClock, MockResponse, MockVendor, inference_request},
};
use demi_provider_codex::{CodexConfig, CodexProvider, TransportMode};
use futures_util::StreamExt;
use serde_json::{Value, json};

use crate::{
    NOW, RESPONSES, completed,
    fake_websocket::{FakeWebSocket, Script, Step},
    fresh_token, pool_with, run, runtime_of, secret,
};

fn address(vendor: &MockVendor) -> SocketAddr {
    vendor
        .url("")
        .trim_start_matches("http://")
        .parse()
        .unwrap()
}

/// A provider whose backend is `socket` and whose sign-in service is
/// `vendor`.
fn provider(
    socket: &FakeWebSocket,
    vendor: &MockVendor,
    pool: &MemoryCredentialPool,
    transport: TransportMode,
    idle: Option<Duration>,
) -> CodexProvider {
    let mut config = CodexConfig::new("codex", "Codex", Some(crate::ACCOUNT.into()));
    config.backend_url = socket.backend_url().parse().unwrap();
    config.auth_url = vendor.url("").parse().unwrap();
    config.transport = transport;
    config.stream_idle_timeout = idle;
    let clock = Arc::new(FixedClock(NOW.parse().unwrap()));
    CodexProvider::new(
        config,
        Arc::new(pool.clone()),
        Arc::new(MemorySnapshots::new()),
        reqwest::Client::new(),
        clock,
    )
}

fn event(value: Value) -> Step {
    Step::Send(value.to_string())
}

fn delta(text: &str) -> Step {
    event(json!({ "type": "response.output_text.delta", "delta": text }))
}

fn done() -> Step {
    event(
        json!({ "type": "response.completed", "response": { "usage": { "input_tokens": 1, "output_tokens": 1 } } }),
    )
}

fn usage() -> TokenUsage {
    TokenUsage {
        input_tokens: 1,
        output_tokens: 1,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
    }
}

#[tokio::test]
async fn a_websocket_request_is_one_response_create_message_with_the_websocket_beta() {
    let vendor = MockVendor::start().await;
    let socket = FakeWebSocket::start(vec![Script::Accept(vec![delta("ws"), done()])], None).await;
    let pool = pool_with(secret(&fresh_token(), "refresh-1", NOW)).await;
    let provider = provider(&socket, &vendor, &pool, TransportMode::WebSocket, None);
    let mut request = inference_request();
    request.model_id = "gpt-5.4".into();
    let events = run(runtime_of(&provider).as_mut(), request).await;
    assert_eq!(
        events,
        [
            ProviderEvent::TextDelta("ws".into()),
            ProviderEvent::Response(usage())
        ]
    );

    let connection = &socket.connections()[0];
    let header = |name: &str| {
        connection
            .headers
            .get(name)
            .map(|value| value.to_str().unwrap())
    };
    assert_eq!(
        header("openai-beta"),
        Some("responses_websockets=2026-02-06")
    );
    assert_eq!(
        header("authorization"),
        Some(format!("Bearer {}", fresh_token()).as_str())
    );
    assert_eq!(header("chatgpt-account-id"), Some("acct-1"));
    assert_eq!((header("accept"), header("content-type")), (None, None));
    let message: Value = serde_json::from_str(&connection.received[0]).unwrap();
    assert_eq!(
        (&message["type"], &message["model"], &message["stream"]),
        (&json!("response.create"), &json!("gpt-5.4"), &json!(true))
    );
    // A terminal event closes the socket, without waiting for the server's
    // close.
    assert_eq!(
        socket.close_reason(0).await.as_deref(),
        Some("response_done")
    );
    // The handshake's answer reaches the account's quota.
    assert_eq!(
        provider.quota().unwrap().latest().unwrap().windows[0].used_percent,
        Some(12.0)
    );
}

#[tokio::test]
async fn envelopes_carry_their_events_and_a_done_envelope_is_the_completion() {
    let vendor = MockVendor::start().await;
    let socket = FakeWebSocket::start(
        vec![Script::Accept(vec![
            event(json!({ "type": "event", "event": { "type": "response.output_text.delta", "delta": "hi" } })),
            event(json!({ "type": "response.done", "response": { "usage": { "input_tokens": 1, "output_tokens": 1 } } })),
        ])],
        None,
    )
    .await;
    let pool = pool_with(secret(&fresh_token(), "refresh-1", NOW)).await;
    let provider = provider(&socket, &vendor, &pool, TransportMode::WebSocket, None);
    let events = run(runtime_of(&provider).as_mut(), inference_request()).await;
    assert_eq!(
        events,
        [
            ProviderEvent::TextDelta("hi".into()),
            ProviderEvent::Response(usage())
        ]
    );
}

#[tokio::test]
async fn a_websocket_that_closes_before_its_first_event_gives_way_to_server_sent_events() {
    let vendor = MockVendor::start().await;
    vendor.respond_at(RESPONSES, completed());
    let socket = FakeWebSocket::start(
        vec![Script::Accept(vec![Step::Close])],
        Some(address(&vendor)),
    )
    .await;
    let pool = pool_with(secret(&fresh_token(), "refresh-1", NOW)).await;
    let provider = provider(&socket, &vendor, &pool, TransportMode::Auto, None);
    let events = run(runtime_of(&provider).as_mut(), inference_request()).await;
    assert_eq!(events, [ProviderEvent::Response(usage())]);
    assert_eq!(vendor.requests().len(), 1);
    assert_eq!(
        vendor.requests()[0].header("accept"),
        Some("text/event-stream")
    );
}

#[tokio::test]
async fn a_websocket_that_cannot_connect_sends_later_requests_over_server_sent_events() {
    let vendor = MockVendor::start().await;
    vendor.respond_at(RESPONSES, completed());
    vendor.respond_at(RESPONSES, completed());
    let socket = FakeWebSocket::start(
        vec![Script::Drop, Script::Accept(vec![done()])],
        Some(address(&vendor)),
    )
    .await;
    let pool = pool_with(secret(&fresh_token(), "refresh-1", NOW)).await;
    let provider = provider(&socket, &vendor, &pool, TransportMode::Auto, None);
    let mut runtime = runtime_of(&provider);
    assert_eq!(
        run(runtime.as_mut(), inference_request()).await,
        [ProviderEvent::Response(usage())]
    );
    // A new session of the same provider does not try the WebSocket again.
    let mut other = runtime_of(&provider);
    assert_eq!(
        run(other.as_mut(), inference_request()).await,
        [ProviderEvent::Response(usage())]
    );
    assert_eq!(socket.connections().len(), 1);
    assert_eq!(vendor.requests().len(), 2);
}

#[tokio::test]
async fn a_websocket_failure_after_its_first_event_is_the_runs_failure() {
    let vendor = MockVendor::start().await;
    let socket = FakeWebSocket::start(
        vec![Script::Accept(vec![delta("ws"), Step::Drop])],
        Some(address(&vendor)),
    )
    .await;
    let pool = pool_with(secret(&fresh_token(), "refresh-1", NOW)).await;
    let provider = provider(&socket, &vendor, &pool, TransportMode::Auto, None);
    let events = run(runtime_of(&provider).as_mut(), inference_request()).await;
    let [
        ProviderEvent::TextDelta(text),
        ProviderEvent::Error(failure),
    ] = events.as_slice()
    else {
        panic!("{events:?}");
    };
    assert_eq!(
        (text.as_str(), failure.code.clone()),
        ("ws", Some(ErrorCode::Overloaded))
    );
    assert!(
        failure.message.starts_with("Codex WebSocket failed"),
        "{}",
        failure.message
    );
    assert!(
        vendor.requests().is_empty(),
        "the run fell back after an event"
    );
}

#[tokio::test]
async fn a_handshake_refused_with_401_refreshes_the_token_once() {
    let vendor = MockVendor::start().await;
    let refreshed = json!({ "access_token": "new-access", "refresh_token": "refresh-2" });
    vendor.respond_at(
        "/oauth/token",
        MockResponse::status(200).chunk(refreshed.to_string()),
    );
    let socket = FakeWebSocket::start(
        vec![Script::Refuse(401), Script::Accept(vec![done()])],
        None,
    )
    .await;
    let pool = pool_with(secret(&fresh_token(), "refresh-1", NOW)).await;
    let provider = provider(&socket, &vendor, &pool, TransportMode::WebSocket, None);
    let events = run(runtime_of(&provider).as_mut(), inference_request()).await;
    assert_eq!(events, [ProviderEvent::Response(usage())]);
    let connections = socket.connections();
    assert_eq!(
        connections[1].headers.get("authorization").unwrap(),
        "Bearer new-access"
    );
}

#[tokio::test]
async fn a_socket_that_idles_fails_the_run_and_is_closed() {
    let vendor = MockVendor::start().await;
    let socket = FakeWebSocket::start(vec![Script::Accept(vec![delta("ws")])], None).await;
    let pool = pool_with(secret(&fresh_token(), "refresh-1", NOW)).await;
    let provider = provider(
        &socket,
        &vendor,
        &pool,
        TransportMode::WebSocket,
        Some(Duration::from_millis(100)),
    );
    let events = run(runtime_of(&provider).as_mut(), inference_request()).await;
    let [ProviderEvent::TextDelta(_), ProviderEvent::Error(failure)] = events.as_slice() else {
        panic!("{events:?}");
    };
    assert_eq!(
        (failure.message.as_str(), failure.code.clone()),
        (
            "Codex WebSocket stream idled for 100ms",
            Some(ErrorCode::Overloaded)
        )
    );
    assert_eq!(
        socket.close_reason(0).await.as_deref(),
        Some("idle_timeout")
    );
}

#[tokio::test]
async fn cancelling_closes_the_socket_as_aborted_and_ends_the_run_without_an_event() {
    let vendor = MockVendor::start().await;
    let socket = FakeWebSocket::start(vec![Script::Accept(vec![delta("hel")])], None).await;
    let pool = pool_with(secret(&fresh_token(), "refresh-1", NOW)).await;
    let provider = provider(&socket, &vendor, &pool, TransportMode::WebSocket, None);
    let mut runtime = runtime_of(&provider);
    let request = inference_request();
    let cancel = request.cancel.clone();
    let mut events = runtime.run(request);
    assert_eq!(
        events.next().await,
        Some(ProviderEvent::TextDelta("hel".into()))
    );
    cancel.cancel();
    assert_eq!(events.next().await, None);
    assert_eq!(socket.close_reason(0).await.as_deref(), Some("aborted"));
}
