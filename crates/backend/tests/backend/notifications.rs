//! Push notifications.
//!
//! Scenarios:
//!
//! - Each session subscribes its browser with the events it chose.
//! - The backend sends what the agent's `demi notify send` says, a turn's end
//!   and a permission request to every subscription that chose them,
//!   encrypted for that browser and signed with the backend's VAPID key, but
//!   to none of a session a page of which shows the conversation.
//! - A subscription its push service says is gone, or whose session signed
//!   out, is removed.
//! - No session reaches another's.
//!
//! The push services are a stand-in on the loopback address that decrypts what
//! it is sent, reached through a transport the test gives the backend; the
//! model is the scripted family of the subagent scenarios, the device a real
//! runner. No test reaches a real push service or model.

use std::sync::Arc;

use axum::Router;
use axum::http;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode as PushStatus};
use axum::routing::post;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use demi_backend_accounts::push::PushTransport;
use demi_backend_user_shard::sync::SyncStep;
use demi_conversation_socket_protocol::{ClientFrame, ServerFrame, TranscriptPatch};
use demi_shared_types::Block;
use demi_provider_common::testing::event;
use demi_web_api_protocol::push::{PushAsk, PushEndpoint, PushKey, PushMessage};
use demi_web_api_protocol::state::SyncEvent;
use futures_util::future::BoxFuture;
use reqwest::{Method, StatusCode};
use serde_json::json;
use tokio::sync::mpsc;
use web_push_native::Auth;
use web_push_native::p256::SecretKey;
use web_push_native::p256::elliptic_curve::sec1::ToEncodedPoint;

use crate::conversations::{FIRST, Socket};
use crate::subagents::{Scripts, WAIT, background, say, shell, tree_on};
use crate::support::{Harness, MASTER_EMAIL, MASTER_PASSWORD, Session, TestBackend};

/// The push services as the backend reaches them in these tests: every
/// endpoint's host, `push.test`, is the stand-in on the loopback address,
/// but `intranet.test`, which is not a public host.
struct StandIn {
    service: String,
    client: reqwest::Client,
}

impl PushTransport for StandIn {
    fn admit<'a>(&'a self, endpoint: &'a PushEndpoint) -> BoxFuture<'a, Result<(), String>> {
        let host = endpoint.url().host_str().unwrap_or_default().to_owned();
        Box::pin(async move {
            if host == "intranet.test" {
                Err("intranet.test resolves to no public address".to_owned())
            } else {
                Ok(())
            }
        })
    }

    fn post(&self, request: http::Request<Vec<u8>>) -> BoxFuture<'_, Result<StatusCode, String>> {
        Box::pin(async move {
            let url = format!("{}{}", self.service, request.uri().path());
            let (parts, body) = request.into_parts();
            let answer = self
                .client
                .post(url)
                .headers(parts.headers)
                .body(body)
                .send()
                .await
                .map_err(|error| error.to_string())?;
            Ok(answer.status())
        })
    }
}

/// A browser as its push service knows it: the keys it decrypts with.
struct Browser {
    name: &'static str,
    secret: SecretKey,
    auth: Auth,
}

impl Browser {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            secret: SecretKey::from_slice(&rand::random::<[u8; 32]>()).expect("a random scalar is a P-256 key"),
            auth: Auth::clone_from_slice(&rand::random::<[u8; 16]>()),
        }
    }

    fn endpoint(&self) -> String {
        format!("https://push.test/push/{}", self.name)
    }

    /// What `session` answers subscribing it to `endpoint` with `events`.
    async fn put(&self, backend: &TestBackend, session: &Session, endpoint: &str, events: serde_json::Value) -> crate::support::Answer {
        backend
            .put(
                "/api/push/subscription",
                session,
                json!({
                    "endpoint": endpoint,
                    "keys": {
                        "p256dh": URL_SAFE_NO_PAD.encode(self.secret.public_key().to_encoded_point(false).as_bytes()),
                        "auth": URL_SAFE_NO_PAD.encode(self.auth),
                    },
                    "events": events,
                }),
            )
            .await
    }

    /// Subscribes it for `session` with the events `events`.
    async fn subscribe(&self, backend: &TestBackend, session: &Session, events: serde_json::Value) {
        let answer = self.put(backend, session, &self.endpoint(), events).await;
        assert_eq!(answer.status, StatusCode::NO_CONTENT, "{}", String::from_utf8_lossy(&answer.body));
    }
}

/// What the push service received: the browser it was for, the VAPID
/// signature's header and the encrypted message.
struct Pushed {
    browser: String,
    authorization: String,
    body: Vec<u8>,
}

/// A push service on the loopback address, and a backend's transport to
/// it: each message is passed to the test, and answered 201, or 410 for the
/// browser named `gone`, as a push service answers for a subscription the
/// browser ended.
async fn push_service() -> (Arc<StandIn>, mpsc::UnboundedReceiver<Pushed>) {
    let (sender, received) = mpsc::unbounded_channel();
    async fn receive(
        State(sender): State<mpsc::UnboundedSender<Pushed>>,
        Path(browser): Path<String>,
        headers: HeaderMap,
        body: Bytes,
    ) -> PushStatus {
        let status = if browser == "gone" { PushStatus::GONE } else { PushStatus::CREATED };
        let authorization = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        // The test outlives the service's answers.
        let _ = sender.send(Pushed { browser, authorization, body: body.to_vec() });
        status
    }
    let app = Router::new().route("/push/{browser}", post(receive)).with_state(sender);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let service = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let transport = StandIn {
        service,
        client: reqwest::Client::new(),
    };
    (Arc::new(transport), received)
}

/// The next message the push service received, decrypted with its
/// browser's keys.
async fn next(received: &mut mpsc::UnboundedReceiver<Pushed>, browsers: &[&Browser]) -> (String, String, PushMessage) {
    let pushed = received.recv().await.expect("the push service runs");
    let browser = browsers
        .iter()
        .find(|browser| browser.name == pushed.browser)
        .expect("a subscribed browser");
    let plain = web_push_native::decrypt(pushed.body, &browser.secret, &browser.auth).expect("the message decrypts");
    let message = serde_json::from_slice(&plain).expect("a push message");
    (pushed.browser, pushed.authorization, message)
}

/// The next `count` messages, by browser name.
async fn next_each(
    received: &mut mpsc::UnboundedReceiver<Pushed>,
    browsers: &[&Browser],
    count: usize,
) -> Vec<(String, PushMessage)> {
    let mut each = Vec::new();
    for _ in 0..count {
        let (browser, _, message) = next(received, browsers).await;
        each.push((browser, message));
    }
    each.sort_by(|left, right| left.0.cmp(&right.0));
    each
}

fn names(each: &[(String, PushMessage)]) -> Vec<&str> {
    each.iter().map(|(browser, _)| browser.as_str()).collect()
}

/// Waits until the sessions part, as `page` is sent it, says `count`
/// sessions' browsers have notifications on, and answers that list.
async fn notifying(page: &mut crate::support::SyncChannel, count: usize) -> Vec<demi_web_api_protocol::sessions::SessionDto> {
    let on = |sessions: &Vec<demi_web_api_protocol::sessions::SessionDto>| {
        sessions.iter().filter(|session| session.notifications).count() == count
    };
    let received = page
        .until(|event| matches!(event, SyncEvent::Sessions { sessions } if on(sessions)))
        .await;
    match received.into_iter().last() {
        Some(SyncEvent::Sessions { sessions }) => sessions,
        other => unreachable!("{other:?}"),
    }
}

fn all_events() -> serde_json::Value {
    json!({ "turnFinishes": true, "turnFails": true, "needsPermission": true, "agentNotifies": true })
}

// About two seconds: a real device runs the shell jobs of four turns.
#[tokio::test]
async fn notifications_reach_each_session_that_chose_them_but_one_whose_page_shows_the_conversation() {
    let scripts = Arc::new(Scripts::default());
    let (transport, mut received) = push_service().await;
    let harness = Harness::new().with_push_transport(transport);
    let (_harness, backend, master, _device, _root, _provider) = tree_on(&scripts, harness).await;
    let key: PushKey = backend.get("/api/push/key", Some(&master)).await.json();

    // Each browser is signed in with a session of its own.
    let watching = Browser::new("watching");
    let gone = Browser::new("gone");
    let quiet = Browser::new("quiet");
    let shared = Browser::new("shared");
    let gone_session = backend.login(MASTER_EMAIL, MASTER_PASSWORD).await;
    let quiet_session = backend.login(MASTER_EMAIL, MASTER_PASSWORD).await;
    let shared_session = backend.login(MASTER_EMAIL, MASTER_PASSWORD).await;
    // Each session says whether its browser has notifications on, and the
    // sessions part announces each change to a page of another session.
    let viewer = backend.login(MASTER_EMAIL, MASTER_PASSWORD).await;
    let mut page = backend.sync(&viewer).await;
    watching.subscribe(&backend, &master, all_events()).await;
    let sessions = notifying(&mut page, 1).await;
    assert!(sessions.iter().all(|session| !session.current || !session.notifications), "{sessions:?}");
    gone.subscribe(&backend, &gone_session, all_events()).await;
    quiet
        .subscribe(
            &backend,
            &quiet_session,
            json!({ "turnFinishes": true, "turnFails": true, "needsPermission": true, "agentNotifies": false }),
        )
        .await;
    notifying(&mut page, 3).await;
    let ended = backend
        .send(Method::DELETE, "/api/push/subscription", Some(&quiet_session.cookie), None)
        .await;
    assert_eq!(ended.status, StatusCode::NO_CONTENT);
    notifying(&mut page, 2).await;
    quiet
        .subscribe(
            &backend,
            &quiet_session,
            json!({ "turnFinishes": true, "turnFails": true, "needsPermission": true, "agentNotifies": false }),
        )
        .await;
    notifying(&mut page, 3).await;

    // No session reaches another's subscription: an endpoint another live
    // session holds is refused, and a session's DELETE removes its own.
    let taken = shared.put(&backend, &shared_session, &watching.endpoint(), all_events()).await;
    assert_eq!(taken.status, StatusCode::CONFLICT);
    assert_eq!(taken.json::<serde_json::Value>()["code"], "push_endpoint_taken");
    // An endpoint whose host is not a public one is refused.
    let private = shared
        .put(&backend, &shared_session, "https://intranet.test/push/shared", all_events())
        .await;
    assert_eq!(private.status, StatusCode::BAD_REQUEST, "{}", String::from_utf8_lossy(&private.body));
    let insecure = shared.put(&backend, &shared_session, "http://push.test/push/shared", all_events()).await;
    assert_eq!(insecure.status, StatusCode::BAD_REQUEST);
    let none = backend
        .send(Method::DELETE, "/api/push/subscription", Some(&shared_session.cookie), None)
        .await;
    assert_eq!(none.status, StatusCode::NO_CONTENT);
    // A browser whose session signs out is sent nothing more of the account.
    shared.subscribe(&backend, &shared_session, all_events()).await;
    let signed_out = backend.post("/api/auth/logout", Some(&shared_session), json!({})).await;
    assert_eq!(signed_out.status, StatusCode::NO_CONTENT, "{}", String::from_utf8_lossy(&signed_out.body));
    let browsers = [&watching, &gone, &quiet, &shared];

    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    scripts.root(
        FIRST,
        vec![
            shell("t1", "demi notify send 'Build finished' 'All 42 tests pass'"),
            say("**All done.** The build is green."),
        ],
    );
    socket.chat("m1", "Build it").await;
    let asked = scripts.asked(|session| session == FIRST).pop().unwrap();
    // `send` answers at once with the browsers it goes to, the gone one
    // among them: the push service has not answered yet.
    assert!(asked.contains("Sending to 2 browsers."), "{asked}");

    // It reaches the two browsers that take the agent's notifications.
    let sent = next_each(&mut received, &browsers, 2).await;
    assert_eq!(names(&sent), ["gone", "watching"]);
    for (browser, message) in &sent {
        let PushMessage::Agent { id, .. } = message else {
            panic!("{browser} was sent {message:?}");
        };
        let expected = PushMessage::Agent {
            conversation_id: FIRST.try_into().unwrap(),
            id: id.clone(),
            title: "Build finished".to_owned(),
            body: "All 42 tests pass".to_owned(),
        };
        assert_eq!(message, &expected);
    }

    // The turn's end goes after it, once the push service said the gone
    // one is gone, to the two browsers left that chose it, with the start
    // of the answer and signed with the key the page subscribes with.
    let mut ends = Vec::new();
    for _ in 0..2 {
        let (browser, authorization, message) = next(&mut received, &browsers).await;
        assert!(authorization.starts_with("vapid t="), "{authorization}");
        assert!(authorization.ends_with(&format!("k={}", key.public_key)), "{authorization}");
        ends.push((browser, message));
    }
    ends.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(names(&ends), ["quiet", "watching"]);
    // The removed one's session no longer has notifications on: of the
    // four browsers, the shared one signed out and the gone one is gone.
    notifying(&mut page, 2).await;
    for (browser, message) in &ends {
        let PushMessage::TurnFinished { conversation_id, answer_start, .. } = message else {
            panic!("{browser} was sent {message:?}");
        };
        assert_eq!(conversation_id.as_str(), FIRST);
        assert_eq!(answer_start.as_deref(), Some("**All done.** The build is green."));
    }

    // While a page of a session shows the conversation, that session's
    // browser is sent nothing of it.
    let mut page = backend.sync(&master).await;
    page.snapshot().await;
    let noted = backend.hold_sync(SyncStep::Showing);
    page.show(Some(FIRST)).await;
    noted.until_arrived(1).await;
    noted.release();
    scripts.root(FIRST, vec![say("Still green.")]);
    socket.chat("m2", "Check again").await;
    let (browser, _, message) = next(&mut received, &browsers).await;
    assert_eq!(browser, "quiet");
    assert!(matches!(message, PushMessage::TurnFinished { .. }), "{message:?}");

    // Once it shows none, it is sent them again; a browser that turned
    // notifications off is sent nothing more.
    let noted = backend.hold_sync(SyncStep::Showing);
    page.show(None).await;
    noted.until_arrived(1).await;
    noted.release();
    let off = backend
        .send(Method::DELETE, "/api/push/subscription", Some(&quiet_session.cookie), None)
        .await;
    assert_eq!(off.status, StatusCode::NO_CONTENT);
    scripts.root(FIRST, vec![shell("t2", "demi notify status"), say("Checked.")]);
    socket.chat("m3", "Status?").await;
    let asked = scripts.asked(|session| session == FIRST).pop().unwrap();
    assert!(asked.contains("Reachable: yes\\nBrowsers: 1\\nAgent notifications: 1"), "{asked}");
    let (browser, _, message) = next(&mut received, &browsers).await;
    assert_eq!(browser, "watching");
    assert!(matches!(&message, PushMessage::TurnFinished { answer_start, .. } if answer_start.as_deref() == Some("Checked.")), "{message:?}");
    assert!(received.try_recv().is_err(), "nothing more was pushed");
}

// About two seconds: a real device runs the turns, a stop and a Resume.
#[tokio::test]
async fn each_turn_notifies_as_it_ended_and_work_that_ends_none_notifies_nothing() {
    let scripts = Arc::new(Scripts::default());
    let (transport, mut received) = push_service().await;
    let harness = Harness::new().with_push_transport(transport);
    let (_harness, backend, master, _device, _root, _provider) = tree_on(&scripts, harness).await;
    let browser = Browser::new("laptop");
    browser.subscribe(&backend, &master, all_events()).await;
    let browsers = [&browser];
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    // A turn that fails says so.
    scripts.root(FIRST, vec![vec![event::error("The model broke", None)]]);
    socket.chat("m1", "Try").await;
    let (_, _, failed) = next(&mut received, &browsers).await;
    assert!(matches!(failed, PushMessage::TurnFailed { .. }), "{failed:?}");

    // A turn the user stops notifies nothing; the Resume that finishes it
    // notifies of its answer.
    scripts.root(FIRST, vec![shell("t1", WAIT)]);
    socket.send(&crate::conversations::send("m2", "Work")).await;
    socket
        .until(|frame| match frame {
            ServerFrame::TranscriptPatch { patches, .. } => patches
                .iter()
                .any(|patch| matches!(patch, TranscriptPatch::Add { value: Block::ToolCall(_), .. })),
            _ => false,
        })
        .await;
    socket.stop().await;
    // Its answer fills most of the window, so a compaction may follow.
    scripts.root(FIRST, vec![vec![event::text("Resumed and done."), event::response(60_000, 1)]]);
    socket.send(&ClientFrame::Resume {}).await;
    socket.until_idle().await;
    let (_, _, resumed) = next(&mut received, &browsers).await;
    let PushMessage::TurnFinished { turn, answer_start, .. } = &resumed else {
        panic!("{resumed:?}");
    };
    assert_eq!(answer_start.as_deref(), Some("Resumed and done."));
    let resumed_turn = turn.clone();

    // A compaction ends no turn: nothing goes until the next turn's end,
    // which is that turn's own.
    scripts.root(FIRST, vec![say("The user asked for work.")]);
    socket.send(&ClientFrame::Compact { node: None }).await;
    socket.until_idle().await;
    scripts.root(FIRST, vec![say("Next answer.")]);
    socket.chat("m3", "Next").await;
    let (_, _, next_end) = next(&mut received, &browsers).await;
    let PushMessage::TurnFinished { turn, answer_start, .. } = &next_end else {
        panic!("{next_end:?}");
    };
    assert_ne!(turn, &resumed_turn);
    assert_eq!(answer_start.as_deref(), Some("Next answer."));
    assert!(received.try_recv().is_err(), "nothing more was pushed");
}

// A few seconds: a real device runs one command that waits for the user.
#[tokio::test]
async fn a_permission_request_notifies_once() {
    let scripts = Arc::new(Scripts::default());
    let (transport, mut received) = push_service().await;
    let harness = Harness::new().with_push_transport(transport);
    let (_harness, backend, master, _device, _root, _provider) = tree_on(&scripts, harness).await;
    let browser = Browser::new("laptop");
    browser.subscribe(&backend, &master, all_events()).await;
    let browsers = [&browser];
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    scripts.root(FIRST, vec![background("t1", "demi skills add acme/tools"), say("I asked the user")]);
    socket.chat("m1", "Install acme/tools").await;
    let (_, _, asked) = next(&mut received, &browsers).await;
    let PushMessage::Permission { conversation_id, ask, .. } = &asked else {
        panic!("{asked:?}");
    };
    assert_eq!(conversation_id.as_str(), FIRST);
    let PushAsk::Permission { categories, host } = ask else {
        panic!("{ask:?}");
    };
    assert_eq!(categories.iter().map(|category| category.id.as_str()).collect::<Vec<_>>(), ["skills.manage"]);
    assert_eq!(host, &None);
    // Then the turn's end, and no second notification of the request.
    let (_, _, ended) = next(&mut received, &browsers).await;
    assert!(matches!(ended, PushMessage::TurnFinished { .. }), "{ended:?}");
    assert!(received.try_recv().is_err(), "nothing more was pushed");
}

// About half a second: a real device runs one shell job.
#[tokio::test]
async fn the_agent_sends_at_most_three_notifications_a_minute() {
    let scripts = Arc::new(Scripts::default());
    let (_harness, backend, master, _device, _root, _provider) = tree_on(&scripts, Harness::new()).await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    scripts.root(
        FIRST,
        vec![
            shell(
                "t1",
                "for n in 1 2 3 4; do demi notify send \"Step $n\"; echo exit=$?; done; demi notify status",
            ),
            say("done"),
        ],
    );
    socket.chat("m1", "Go").await;
    let asked = scripts.asked(|session| session == FIRST).pop().unwrap();
    assert_eq!(asked.matches("exit=0").count(), 3, "{asked}");
    assert!(
        // The transcript as the model was asked it, its newlines escaped.
        asked.contains("demi notify send: too many notifications; at most 3 a minute\\nexit=1"),
        "{asked}"
    );
    // With no browser subscribed, `send` says so and `status` says where
    // the user turns notifications on.
    assert!(asked.contains("demi notify send: warning: no browser of the user takes the agent's notifications now"), "{asked}");
    assert!(asked.contains("Reachable: no\\nBrowsers: 0\\nAgent notifications: 0"), "{asked}");
    assert!(
        asked.contains("demi notify status: warning: notifications are off in every browser of the user; the user turns them on in Settings › Notifications"),
        "{asked}"
    );
}
