//! The WebSocket transport and the fallback to server-sent events
//! (`crates/provider-codex/src/transport.rs`).

use std::{net::SocketAddr, sync::Arc, time::Duration};

use demi_provider_codex::{
    CodexConfig, CodexProvider, TransportMode,
    testing::{FakeWebSocket, Script, Step},
};
use demi_provider_common::{
    ErrorCode, ProviderEvent,
    credentials::MemoryCredentialPool,
    quota::MemorySnapshots,
    testing::{FixedClock, ManualClock, MockVendor, inference_request},
};
use demi_shared_types::{Clock, TokenUsage};
use serde_json::{Value, json};

use crate::{NOW, RESPONSES, completed, fresh_token, pool_with, run, runtime_of, secret};

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
    let clock = Arc::new(FixedClock(NOW.parse().unwrap()));
    provider_at(socket, vendor, pool, transport, idle, clock)
}

fn provider_at(
    socket: &FakeWebSocket,
    vendor: &MockVendor,
    pool: &MemoryCredentialPool,
    transport: TransportMode,
    idle: Option<Duration>,
    clock: Arc<dyn Clock>,
) -> CodexProvider {
    let mut config = CodexConfig::new(Some(crate::ACCOUNT.into()), crate::releases(vendor));
    config.backend_url = socket.backend_url().parse().unwrap();
    config.auth_url = vendor.url("").parse().unwrap();
    config.transport = transport;
    config.stream_idle_timeout = idle;
    CodexProvider::new(
        config,
        Arc::new(pool.clone()),
        Arc::new(MemorySnapshots::new()),
        demi_provider_common::ProviderHttp::new(reqwest::Client::new()),
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
async fn a_websocket_that_cannot_connect_sends_the_next_ten_minutes_of_requests_over_server_sent_events()
 {
    let vendor = MockVendor::start().await;
    vendor.respond_at(RESPONSES, completed());
    vendor.respond_at(RESPONSES, completed());
    let socket = FakeWebSocket::start(
        vec![Script::Drop, Script::Accept(vec![done()])],
        Some(address(&vendor)),
    )
    .await;
    let pool = pool_with(secret(&fresh_token(), "refresh-1", NOW)).await;
    let clock = Arc::new(ManualClock::new(NOW.parse().unwrap()));
    let provider = provider_at(
        &socket,
        &vendor,
        &pool,
        TransportMode::Auto,
        None,
        clock.clone(),
    );
    let mut runtime = runtime_of(&provider);
    assert_eq!(
        run(runtime.as_mut(), inference_request()).await,
        [ProviderEvent::Response(Some(usage()))]
    );
    // A new session of the same provider does not try the WebSocket again
    // within ten minutes.
    clock.advance(jiff::SignedDuration::from_secs(599));
    let mut other = runtime_of(&provider);
    assert_eq!(
        run(other.as_mut(), inference_request()).await,
        [ProviderEvent::Response(Some(usage()))]
    );
    assert_eq!(socket.connections().len(), 1);
    assert_eq!(vendor.requests().len(), 2);
    // After them, a passing failure has cost the WebSocket no longer.
    clock.advance(jiff::SignedDuration::from_secs(1));
    assert_eq!(
        run(other.as_mut(), inference_request()).await,
        [ProviderEvent::Response(Some(usage()))]
    );
    assert_eq!(socket.connections().len(), 2);
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
