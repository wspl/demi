//! The pages' synchronization channel end to end (`web-api.md` § Page
//! synchronization, `backend.md` § Page synchronization): the product
//! state a page receives first; what it receives when another session
//! changes something, after it reconnects, and when it falls behind; a turn
//! and its reading as other pages see them; when the channel opens and how
//! it ends; and the heartbeat each of a page's sockets sends when it is
//! quiet.

use std::time::{Duration, Instant};

use demi_conversation_socket_protocol::{ClientFrame, ServerFrame};
use demi_backend_user_shard::sync::SyncStep;
use demi_shared_types::AuthState;
use demi_provider_common::quota::ProbeCost;
use demi_provider_common::testing::{MockResponse, MockVendor};
use demi_web_api_protocol::auth::Role;
use demi_web_api_protocol::cloud::{CloudState, CloudStatus, CloudVolumes};
use demi_web_api_protocol::conversations::{ConversationStatus, ConversationSummary};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::providers::{ProviderDetails, ProviderReading, ProviderState};
use demi_web_api_protocol::settings::{InstanceMode, Preferences, Theme};
use demi_web_api_protocol::state::{ProductState, SyncEvent};
use jiff::SignedDuration;
use reqwest::{Method, StatusCode};
use serde_json::json;

use crate::accounts::{device_entry, scripts};
use crate::conversations::{FIRST, Socket, anthropic, choose, create, events, message_start, send, text_block};
use crate::support::{Harness, MASTER_EMAIL, MASTER_PASSWORD, Session, SyncChannel, TestBackend, answer};

async fn rename(backend: &TestBackend, session: &Session, id: &str, title: &str) {
    let renamed = backend
        .patch(&format!("/api/conversations/{id}"), session, json!({ "title": title }))
        .await;
    assert_eq!(renamed.status, StatusCode::OK, "{}", String::from_utf8_lossy(&renamed.body));
}

async fn pin(backend: &TestBackend, session: &Session, id: &str) {
    let pinned = backend
        .patch(&format!("/api/conversations/{id}"), session, json!({ "pinned": true }))
        .await;
    assert_eq!(pinned.status, StatusCode::OK, "{}", String::from_utf8_lossy(&pinned.body));
}

async fn theme(backend: &TestBackend, session: &Session, theme: &str) {
    let body = json!({ "appearance": { "theme": theme } });
    let set = backend.patch("/api/settings/preferences", session, body).await;
    assert_eq!(set.status, StatusCode::OK, "{}", String::from_utf8_lossy(&set.body));
}

/// The summary a message carries, if it is one of `id`.
fn summary<'a>(event: &'a SyncEvent, id: &str) -> Option<&'a ConversationSummary> {
    match event {
        SyncEvent::Conversation { conversation } if conversation.id.as_str() == id => Some(conversation),
        _ => None,
    }
}

/// Whether the message carries preferences with the theme `theme`.
fn themed(event: &SyncEvent, theme: Theme) -> bool {
    matches!(event, SyncEvent::Preferences { preferences } if preferences.appearance.theme == Some(theme))
}

/// Waits until the channel brings the summary of `id` that `wanted`
/// accepts, and answers it.
async fn until_summary(page: &mut SyncChannel, id: &str, wanted: impl Fn(&ConversationSummary) -> bool) -> ConversationSummary {
    let received = page
        .until(|event| summary(event, id).is_some_and(|conversation| wanted(conversation)))
        .await;
    summary(received.last().unwrap(), id).unwrap().clone()
}

/// How the channel's route answers an upgrade from a page at `origin`, or
/// from none, when it refuses it before upgrading.
async fn refusal(backend: &TestBackend, session: Option<&Session>, origin: Option<&str>) -> (StatusCode, ErrorCode) {
    let mut request = reqwest::Client::new()
        .request(Method::GET, format!("{}/api/sync", backend.url))
        .header("upgrade", "websocket")
        .header("connection", "Upgrade")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "MDEyMzQ1Njc4OWFiY2RlZg==");
    if let Some(session) = session {
        request = request.header("cookie", &session.cookie);
    }
    if let Some(origin) = origin {
        request = request.header("origin", origin);
    }
    answer(request.send().await.unwrap()).await.refusal()
}

#[tokio::test]
async fn the_snapshot_is_the_users_product_state() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let mut page = backend.sync(&master).await;
    let state = page.snapshot().await;
    assert_eq!(
        state,
        ProductState {
            user: master.user.clone(),
            mode: InstanceMode::Shared,
            preferences: Preferences::default(),
            providers: Vec::new(),
            workspaces: Vec::new(),
            devices: Vec::new(),
            exposes: Vec::new(),
            expose_domain: None,
            // The URL runners connect to: without a configured one, the
            // backend's own address.
            public_url: format!("{}/", backend.url),
            conversations: Vec::new(),
            // A Cloud no work used yet is not made.
            cloud: CloudStatus {
                device: None,
                state: CloudState::Unallocated,
                operation: None,
                error: None,
                volumes: None,
                limits: CloudVolumes {
                    system_bytes: 16 << 30,
                    home_bytes: 32 << 30,
                },
            },
        }
    );
    // The page's install command fetches the installer at that URL's origin,
    // which is this backend's; it has no runner releases to install.
    let installer = url::Url::parse(&state.public_url).unwrap().join("/install.sh").unwrap();
    let installer = reqwest::get(installer).await.unwrap();
    assert_eq!(installer.status(), StatusCode::SERVICE_UNAVAILABLE);
    backend.close().await;
}

#[tokio::test]
async fn a_change_in_one_session_reaches_the_other_sessions_page_as_one_message() {
    let harness = Harness::new();
    let (backend, laptop) = harness.start_set_up().await;
    let phone = backend.login(MASTER_EMAIL, MASTER_PASSWORD).await;
    create(&backend, &laptop, FIRST).await;
    let mut page = backend.sync(&phone).await;
    let state = page.snapshot().await;
    assert_eq!((state.conversations[0].title.as_str(), state.conversations[0].draft_revision), ("New conversation", 0));

    // Each change the laptop makes is one message on the phone's page: the
    // message after it is the next change's.
    rename(&backend, &laptop, FIRST, "Fix the login").await;
    let renamed = page.next().await;
    assert_eq!(summary(&renamed, FIRST).map(|conversation| conversation.title.as_str()), Some("Fix the login"));
    let draft = json!({ "base": 0, "text": "The login fails", "files": [] });
    let saved = backend.put(&format!("/api/conversations/{FIRST}/draft"), &laptop, draft).await;
    assert_eq!(saved.status, StatusCode::OK, "{}", String::from_utf8_lossy(&saved.body));
    let drafted = page.next().await;
    assert_eq!(summary(&drafted, FIRST).map(|conversation| conversation.draft_revision), Some(1));
    let nickname = backend.patch("/api/auth/me", &laptop, json!({ "nickname": "Ana" })).await;
    assert_eq!(nickname.status, StatusCode::OK, "{}", String::from_utf8_lossy(&nickname.body));
    assert!(matches!(page.next().await, SyncEvent::User { user } if user.nickname == "Ana"));
    theme(&backend, &laptop, "dark").await;
    assert!(themed(&page.next().await, Theme::Dark));
    backend.close().await;
}

#[tokio::test]
async fn a_page_that_reconnects_catches_up_on_what_changed_while_it_was_away_and_while_it_read() {
    let harness = Harness::new();
    let (backend, laptop) = harness.start_set_up().await;
    let phone = backend.login(MASTER_EMAIL, MASTER_PASSWORD).await;
    create(&backend, &laptop, FIRST).await;
    let mut away = backend.sync(&phone).await;
    away.snapshot().await;
    drop(away);
    rename(&backend, &laptop, FIRST, "Renamed while away").await;

    // The phone connects again; once its channel has read the state, and
    // before it sends it, the laptop pins the conversation.
    let held = backend.hold_sync(SyncStep::Snapshot);
    let mut page = backend.sync(&phone).await;
    held.until_arrived(1).await;
    pin(&backend, &laptop, FIRST).await;
    held.release();
    let state = page.snapshot().await;
    let read = &state.conversations[0];
    assert_eq!((read.title.as_str(), read.pinned), ("Renamed while away", false));
    // The pin, made after that read, comes after it: the summary, then the
    // order.
    let caught_up = page
        .until(|event| matches!(event, SyncEvent::ConversationOrder { .. }))
        .await;
    assert_eq!(caught_up.len(), 2, "{caught_up:?}");
    assert_eq!(summary(&caught_up[0], FIRST).map(|conversation| conversation.pinned), Some(true));
    backend.close().await;
}

#[tokio::test]
async fn a_page_that_falls_behind_receives_each_changed_part_once_as_it_is_when_it_reads() {
    let harness = Harness::new();
    let (backend, laptop) = harness.start_set_up().await;
    let phone = backend.login(MASTER_EMAIL, MASTER_PASSWORD).await;
    create(&backend, &laptop, FIRST).await;
    let mut page = backend.sync(&phone).await;
    page.snapshot().await;

    // The phone's page takes nothing while the laptop renames the
    // conversation fifty times and changes a preference.
    let held = backend.hold_sync(SyncStep::Changes);
    for number in 1..=50 {
        rename(&backend, &laptop, FIRST, &format!("Title {number}")).await;
    }
    theme(&backend, &laptop, "dark").await;
    held.until_arrived(1).await;
    held.release();
    // It receives the conversation once, as it is now, then the preference.
    let latest = page.next().await;
    assert_eq!(summary(&latest, FIRST).map(|conversation| conversation.title.as_str()), Some("Title 50"));
    assert!(themed(&page.next().await, Theme::Dark));
    // It was not closed for falling behind.
    rename(&backend, &laptop, FIRST, "Caught up").await;
    let renamed = page.next().await;
    assert_eq!(summary(&renamed, FIRST).map(|conversation| conversation.title.as_str()), Some("Caught up"));
    backend.close().await;
}

#[tokio::test]
async fn a_turn_shows_on_every_page_as_it_runs_and_stops_and_reading_it_on_one_clears_it_on_all() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, laptop) = harness.start_set_up().await;
    let phone = backend.login(MASTER_EMAIL, MASTER_PASSWORD).await;
    let provider = anthropic(&backend, &laptop, &vendor).await;
    create(&backend, &laptop, FIRST).await;
    choose(&backend, &laptop, FIRST, &provider, "claude-opus-4-8").await;
    let mut page = backend.sync(&phone).await;
    page.snapshot().await;

    // The answer streams and stays open, so the turn runs until the laptop
    // stops it.
    let frames = [
        vec![message_start(json!({ "input_tokens": 12, "output_tokens": 0 }))],
        text_block(0, &["Hello"]),
    ]
    .concat();
    vendor.respond(MockResponse::event_stream(events(&frames)).stay_open());
    let mut socket = Socket::connect(&backend, &laptop, FIRST).await;
    socket.open().await;
    socket.send(&send("m1", "Say hello")).await;
    until_summary(&mut page, FIRST, |conversation| conversation.status == ConversationStatus::Running).await;
    socket.send(&ClientFrame::Abort {}).await;
    let stopped = until_summary(&mut page, FIRST, |conversation| {
        conversation.status == ConversationStatus::Stopped
    })
    .await;
    assert!(stopped.unread && stopped.revision > 0, "{stopped:?}");

    let read = json!({ "revision": stopped.revision });
    let acknowledged = backend
        .post(&format!("/api/conversations/{FIRST}/read"), Some(&laptop), read)
        .await;
    assert_eq!(acknowledged.status, StatusCode::NO_CONTENT);
    until_summary(&mut page, FIRST, |conversation| !conversation.unread).await;
    backend.close().await;
}

fn details(reading: &ProviderReading) -> &ProviderDetails {
    let ProviderReading::Read(details) = reading else {
        panic!("the entry could not be read: {reading:?}");
    };
    details
}

/// The entries a providers message carries, once one does.
async fn until_providers(page: &mut SyncChannel, count: usize) -> Vec<ProviderState> {
    let received = page
        .until(|event| matches!(event, SyncEvent::Providers { providers } if providers.len() == count))
        .await;
    let Some(SyncEvent::Providers { providers }) = received.into_iter().last() else {
        unreachable!("the last message is the one accepted");
    };
    providers
}

#[tokio::test]
async fn an_entry_reaches_the_page_of_every_user_who_infers_with_it_and_only_a_configuring_user_sees_its_accounts() {
    let scripts = scripts(Some(ProbeCost::Free));
    let harness = Harness::new().with_families(scripts.families.clone());
    let (backend, master) = harness.start_set_up().await;
    harness.add_user("reader@example.test", "reader-pass-1", Role::User);
    let reader = backend.login("reader@example.test", "reader-pass-1").await;
    let mut masters = backend.sync(&master).await;
    assert!(masters.snapshot().await.providers.is_empty());
    let mut readers = backend.sync(&reader).await;
    readers.snapshot().await;

    // The master adds entries of a shared instance, which every user infers
    // with: every user's page receives them.
    let device = device_entry(&backend, &master, &scripts).await;
    let keyed = json!({ "source": "custom", "providerType": "anthropic", "label": "Work", "apiKey": "sk-state" });
    let keyed = backend.post("/api/providers", Some(&master), keyed).await;
    assert_eq!(keyed.status, StatusCode::CREATED);
    let seen = until_providers(&mut masters, 2).await;
    let labels: Vec<&str> = seen.iter().map(|entry| entry.provider.label.as_str()).collect();
    assert_eq!(labels, ["device subscription", "Work"]);
    assert!(!serde_json::to_string(&seen).unwrap().contains("sk-state"));
    let subscription = details(&seen[0].details);
    assert_eq!(subscription.accounts.len(), 1);
    assert_eq!(subscription.active.as_ref().map(|id| id.as_str()), Some(subscription.accounts[0].account.id.as_str()));
    assert!(matches!(details(&seen[1].details).auth, AuthState::Authenticated { .. }));
    assert_eq!(seen[0].provider.id, device.id);
    // A user who only infers sees no accounts, plan or usage.
    let seen = until_providers(&mut readers, 2).await;
    let subscription = details(&seen[0].details);
    assert_eq!((subscription.accounts.len(), subscription.active.clone(), subscription.quota.clone()), (0, None, None));
    assert_eq!(subscription.auth, AuthState::Authenticated { account_label: None });
    backend.close().await;
}

fn days(count: i64) -> SignedDuration {
    SignedDuration::from_hours(24 * count)
}

#[tokio::test]
async fn the_channel_opens_for_a_signed_in_page_of_the_product_and_ends_with_its_session_or_the_backend() {
    let harness = Harness::new();
    let (backend, laptop) = harness.start_set_up().await;
    let product = backend.url.clone();
    let foreign = refusal(&backend, Some(&laptop), Some("https://a1b2c3.expose.localhost")).await;
    assert_eq!(foreign, (StatusCode::FORBIDDEN, ErrorCode::ForbiddenOrigin));
    let anonymous = refusal(&backend, None, Some(&product)).await;
    assert_eq!(anonymous, (StatusCode::UNAUTHORIZED, ErrorCode::Unauthenticated));
    let plain = backend.get_with("/api/sync", &laptop, &[("origin", &product)]).await;
    assert_eq!(plain.refusal(), (StatusCode::UPGRADE_REQUIRED, ErrorCode::UpgradeRequired));

    // Signing out closes the channels of that session, and no other's.
    let phone = backend.login(MASTER_EMAIL, MASTER_PASSWORD).await;
    let tablet = backend.login(MASTER_EMAIL, MASTER_PASSWORD).await;
    let mut laptop_page = backend.sync(&laptop).await;
    laptop_page.snapshot().await;
    let mut phone_page = backend.sync(&phone).await;
    phone_page.snapshot().await;
    let out = backend.post("/api/auth/logout", Some(&phone), json!({})).await;
    assert_eq!(out.status, StatusCode::NO_CONTENT);
    assert_eq!(phone_page.closed().await, (4002, "session_ended".to_owned()));

    // A channel never renews its session, not even as it opens: the
    // tablet's page opens 20 days after its sign-in, when a request would
    // renew the session, and closes as the session ends 30 days after the
    // sign-in. The laptop's requests renew the laptop's.
    harness.clock.advance(days(20));
    theme(&backend, &laptop, "dark").await;
    assert!(themed(&laptop_page.next().await, Theme::Dark));
    let mut tablet_page = backend.sync(&tablet).await;
    tablet_page.snapshot().await;
    harness.clock.advance(days(11));
    theme(&backend, &laptop, "light").await;
    assert!(themed(&laptop_page.next().await, Theme::Light));
    assert_eq!(tablet_page.closed().await, (4002, "session_ended".to_owned()));

    // A page sends nothing on its channel.
    laptop_page.send_text("hello").await;
    assert_eq!(laptop_page.closed().await, (1008, "unexpected_message".to_owned()));

    // At shutdown the channels close first.
    let mut last = backend.sync(&laptop).await;
    last.snapshot().await;
    let (closed, ()) = tokio::join!(last.closed(), backend.close());
    assert_eq!(closed, (1001, "backend_closing".to_owned()));
}

/// A page tells a quiet socket from one that died without a close by the
/// heartbeat each of its sockets sends once it has sent nothing else for the
/// interval (`web-application.md` § Liveness and reconnection): the channel
/// after its snapshot, and a conversation socket whose page never opened
/// the conversation. Each waits for its heartbeat, 0.2 s here.
#[tokio::test]
async fn each_socket_of_a_page_sends_a_heartbeat_once_it_was_quiet_for_the_interval() {
    let mut harness = Harness::new();
    harness.pages.heartbeat = Duration::from_millis(200);
    let (backend, master) = harness.start_set_up().await;
    create(&backend, &master, FIRST).await;

    let connected = Instant::now();
    let mut page = backend.sync(&master).await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    page.snapshot().await;
    let (event, frame) = tokio::join!(page.next(), socket.frame());
    assert_eq!(event, SyncEvent::Heartbeat);
    assert_eq!(frame, ServerFrame::Heartbeat);
    assert!(
        connected.elapsed() >= harness.pages.heartbeat,
        "no heartbeat comes before the interval"
    );
    backend.close().await;
}
