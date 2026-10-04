//! A user's plugins (`plugins.md` § A user's plugins, `web-api.md` § A
//! user's plugins): a plugin turned off leaves the user's pages at once and
//! refuses its page calls, while an open conversation keeps the commands its
//! tree opened with and offers a reload, which opens the tree with the
//! commands of the plugins the user has on.

use demi_conversation_socket_protocol::ServerFrame;
use demi_provider_common::testing::{MockResponse, MockVendor};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::state::SyncEvent;
use reqwest::StatusCode;
use serde_json::json;

use crate::conversations::{FIRST, Socket, answer, anthropic, choose, create};
use crate::support::{Harness, Session, TestBackend};

async fn switch(
    backend: &TestBackend,
    session: &Session,
    plugin: &str,
    enabled: bool,
) -> StatusCode {
    let path = format!("/api/plugins/{plugin}");
    backend
        .put(&path, session, json!({ "enabled": enabled }))
        .await
        .status
}

/// The system prompt of the provider request numbered `index`.
fn system(vendor: &MockVendor, index: usize) -> String {
    vendor.requests()[index].json()["system"].to_string()
}

#[tokio::test]
async fn an_open_conversation_keeps_its_commands_until_a_reload_opens_it_with_the_plugins_on() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut page = backend.sync(&master).await;
    page.snapshot().await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    vendor.respond(answer(&["one"], 1, 1));
    socket.chat("m1", "first").await;
    assert!(system(&vendor, 0).contains("demi expose"));

    // Turned off: every page learns it, and the open conversation says a
    // reload would change it.
    assert_eq!(
        switch(&backend, &master, "expose", false).await,
        StatusCode::NO_CONTENT
    );
    // One batch: the conversation's summary goes before the plugin list.
    let changes = page
        .until(|event| matches!(event, SyncEvent::Plugins { .. }))
        .await;
    assert!(
        changes.iter().any(|event| {
            matches!(event, SyncEvent::Conversation { conversation } if conversation.plugins_changed)
        }),
        "{changes:?}"
    );
    let Some(SyncEvent::Plugins { plugins }) = changes.last() else {
        unreachable!("the wait ends at the plugin list")
    };
    let expose = plugins.iter().find(|plugin| plugin.id == "expose").unwrap();
    assert!(!expose.enabled);

    // Until the reload, the tree keeps the commands it opened with.
    vendor.respond(answer(&["two"], 1, 1));
    socket.chat("m2", "second").await;
    assert!(system(&vendor, 1).contains("demi expose"));

    // A reload closes the tree, the socket opens it again, and it offers
    // the commands of the plugins on.
    let reloaded = backend
        .post(
            &format!("/api/conversations/{FIRST}/reload"),
            Some(&master),
            json!({}),
        )
        .await;
    assert_eq!(reloaded.status, StatusCode::NO_CONTENT);
    socket
        .until(|frame| matches!(frame, ServerFrame::Closed))
        .await;
    page.until(|event| {
        matches!(event, SyncEvent::Conversation { conversation } if !conversation.plugins_changed)
    })
    .await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    vendor.respond(answer(&["three"], 1, 1));
    socket.chat("m3", "third").await;
    let reopened = system(&vendor, 2);
    assert!(!reopened.contains("demi expose"), "{reopened}");
    assert!(reopened.contains("demi host"), "{reopened}");

    // A tree that works is not reloaded; the turn goes on.
    vendor.respond(MockResponse::event_stream(": thinking\n\n").stay_open());
    socket
        .send(&crate::conversations::send("m4", "fourth"))
        .await;
    socket
        .until(|frame| matches!(frame, ServerFrame::Phase { phase } if *phase != demi_shared_types::SessionPhase::Idle))
        .await;
    let refused = backend
        .post(
            &format!("/api/conversations/{FIRST}/reload"),
            Some(&master),
            json!({}),
        )
        .await;
    assert_eq!(
        refused.refusal(),
        (StatusCode::CONFLICT, ErrorCode::TurnInFlight)
    );
    socket.stop().await;
    backend.close().await;
}

#[tokio::test]
async fn a_plugin_turned_off_leaves_the_page_and_refuses_its_calls_until_it_is_on_again() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let renew = || async {
        backend
            .post(
                "/api/plugins/expose/calls/renew",
                Some(&master),
                json!({ "expose": "k7x2maqw4p3s6tavaw2y4z6aab" }),
            )
            .await
    };

    assert_eq!(
        switch(&backend, &master, "expose", false).await,
        StatusCode::NO_CONTENT
    );
    let state = backend.sync(&master).await.snapshot().await;
    assert!(!state.plugin_states.contains_key("expose"), "{state:?}");
    let expose = state
        .plugins
        .iter()
        .find(|plugin| plugin.id == "expose")
        .unwrap();
    assert!(!expose.enabled);
    assert_eq!(
        renew().await.refusal(),
        (StatusCode::CONFLICT, ErrorCode::PluginDisabled)
    );

    assert_eq!(
        switch(&backend, &master, "expose", true).await,
        StatusCode::NO_CONTENT
    );
    let state = backend.sync(&master).await.snapshot().await;
    assert!(state.plugin_states.contains_key("expose"), "{state:?}");
    assert_eq!(
        renew().await.error().reason.as_deref(),
        Some("expose_not_found")
    );

    let unknown = backend
        .put("/api/plugins/nope", &master, json!({ "enabled": false }))
        .await;
    assert_eq!(
        unknown.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::UnknownPlugin)
    );
    backend.close().await;
}
