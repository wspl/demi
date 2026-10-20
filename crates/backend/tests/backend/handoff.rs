//! Handing the account to a phone by a code, and the account's signed-in
//! browsers, at the edge (`crates/backend-accounts/src/sessions.rs`): a code
//! needs a session verified in the last five minutes, opens a page of the
//! product once within ten minutes, in the phone's own session of the account
//! when it has one and else in a session of its own, which the laptop's page
//! learns of from the `sessions` part and can sign out, its pages' sockets with
//! it.

use jiff::SignedDuration;
use reqwest::StatusCode;
use serde_json::{Value, json};

use demi_web_api_protocol::auth::{Identity, Role};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::sessions::{HandoffCode, SessionDto, SessionList};
use demi_web_api_protocol::state::SyncEvent;

use crate::conversations::{FIRST, Socket, create};
use std::time::Duration;

use demi_backend_user_shard::shard::ShardPlacement;

use crate::support::{
    Answer, Harness, MASTER_PASSWORD, Session, TestBackend, answer, elapse, session_from,
};

const IPHONE_SAFARI: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1";

/// Asks for a code that opens `to`.
async fn make_code(backend: &TestBackend, laptop: &Session, to: &str) -> Answer {
    backend
        .post("/api/auth/handoff/code", Some(laptop), json!({ "to": to }))
        .await
}

/// The code a made code's URL carries after `#`.
fn code_of(made: &HandoffCode) -> String {
    let url = url::Url::parse(&made.url).unwrap();
    url.fragment().unwrap().to_owned()
}

/// Spends `code` from a phone's browser, which holds no session.
async fn spend(backend: &TestBackend, code: &str) -> Answer {
    spend_signed_in(backend, code, None).await
}

/// Spends `code` from a phone's browser, which holds `held`'s session
/// cookie when given.
async fn spend_signed_in(backend: &TestBackend, code: &str, held: Option<&Session>) -> Answer {
    let mut request = reqwest::Client::new()
        .post(format!("{}/api/auth/handoff", backend.url))
        .header("user-agent", IPHONE_SAFARI)
        .header("content-type", "application/json")
        .body(json!({ "code": code }).to_string());
    if let Some(held) = held {
        request = request.header("cookie", &held.cookie);
    }
    answer(request.send().await.unwrap()).await
}

/// The sessions part's first list that holds a session the code `made`
/// opened, skipping the messages before it.
async fn handed_off(page: &mut crate::support::SyncChannel, made: &HandoffCode) -> Vec<SessionDto> {
    let carries = |sessions: &Vec<SessionDto>| {
        sessions
            .iter()
            .any(|session| session.handoff_id.as_ref() == Some(&made.id))
    };
    let received = page
        .until(|event| matches!(event, SyncEvent::Sessions { sessions } if carries(sessions)))
        .await;
    match received.into_iter().last() {
        Some(SyncEvent::Sessions { sessions }) => sessions,
        other => unreachable!("{other:?}"),
    }
}

async fn verify(backend: &TestBackend, laptop: &Session, password: &str) -> Answer {
    backend
        .post("/api/auth/verify", Some(laptop), json!({ "password": password }))
        .await
}

/// The sessions part's list, skipping the messages before it.
async fn sessions_part(page: &mut crate::support::SyncChannel) -> Vec<SessionDto> {
    let received = page
        .until(|event| matches!(event, SyncEvent::Sessions { .. }))
        .await;
    match received.into_iter().last() {
        Some(SyncEvent::Sessions { sessions }) => sessions,
        other => unreachable!("{other:?}"),
    }
}

#[tokio::test]
async fn a_verified_laptop_hands_the_account_to_a_phone_once() {
    let harness = Harness::new();
    let (backend, laptop) = harness.start_set_up().await;
    let mut laptop_page = backend.sync(&laptop).await;
    laptop_page.snapshot().await;

    // A sign-in verifies its session: for five minutes it makes codes, and
    // then no more until it is verified again, so a browser left signed in
    // cannot hand the account on.
    let fresh = make_code(&backend, &laptop, "/chat").await;
    assert_eq!(fresh.status, StatusCode::OK);
    harness.clock.advance(SignedDuration::from_mins(5));
    let unverified = make_code(&backend, &laptop, "/chat").await;
    assert_eq!(unverified.refusal(), (StatusCode::FORBIDDEN, ErrorCode::VerificationRequired));
    let wrong = verify(&backend, &laptop, "not the password").await;
    assert_eq!(wrong.refusal(), (StatusCode::UNAUTHORIZED, ErrorCode::InvalidCredentials));
    let verified = verify(&backend, &laptop, MASTER_PASSWORD).await;
    assert_eq!(verified.status, StatusCode::NO_CONTENT);

    // A code never opens another site.
    for elsewhere in ["//evil.example/chat", "https://evil.example/chat", "/\\evil.example"] {
        let refused = make_code(&backend, &laptop, elsewhere).await;
        assert_eq!(refused.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody), "{elsewhere}");
    }

    let to = format!("/chat/{FIRST}");
    let made = make_code(&backend, &laptop, &to).await;
    assert_eq!(made.status, StatusCode::OK, "{}", String::from_utf8_lossy(&made.body));
    let made: HandoffCode = made.json();
    // The code travels after `#`, at the backend's public URL.
    assert!(made.url.starts_with(&format!("{}/handoff#", backend.url)), "{}", made.url);
    assert_eq!(made.expires_in_ms, 10 * 60 * 1000);

    // The phone spends it: a session of its own, and the page to open.
    let spent = spend(&backend, &code_of(&made)).await;
    assert_eq!(spent.status, StatusCode::OK, "{}", String::from_utf8_lossy(&spent.body));
    assert_eq!(spent.json::<Value>(), json!({ "to": to }));
    let cookies = spent.session_cookies();
    let [cookie] = cookies.as_slice() else {
        panic!("one session cookie: {cookies:?}");
    };
    let phone = Session {
        cookie: cookie.split(';').next().unwrap().to_owned(),
        user: laptop.user.clone(),
    };
    let me = backend.get("/api/auth/me", Some(&phone)).await;
    assert_eq!(me.json::<Identity>().user, laptop.user);
    assert_ne!(phone.cookie, laptop.cookie);

    // The laptop's page learns of the session the code made, named for the
    // phone's browser.
    let listed = sessions_part(&mut laptop_page).await;
    let made_by_code: Vec<&SessionDto> = listed
        .iter()
        .filter(|session| session.handoff_id.as_ref() == Some(&made.id))
        .collect();
    let [phone_session] = made_by_code.as_slice() else {
        panic!("one session made by the code: {listed:?}");
    };
    assert_eq!(phone_session.name, "Safari on iPhone");
    assert!(!phone_session.current);

    // A code is spent once.
    let again = spend(&backend, &code_of(&made)).await;
    assert_eq!(again.refusal(), (StatusCode::UNAUTHORIZED, ErrorCode::HandoffExpired));
    assert!(again.session_cookies().is_empty());

    // A new code ends the one before.
    let first: HandoffCode = make_code(&backend, &laptop, "/chat").await.json();
    let second: HandoffCode = make_code(&backend, &laptop, "/chat").await.json();
    let ended = spend(&backend, &code_of(&first)).await;
    assert_eq!(ended.refusal(), (StatusCode::UNAUTHORIZED, ErrorCode::HandoffExpired));

    // A code lasts ten minutes, and a verification five.
    harness.clock.advance(SignedDuration::from_mins(10));
    let expired = spend(&backend, &code_of(&second)).await;
    assert_eq!(expired.refusal(), (StatusCode::UNAUTHORIZED, ErrorCode::HandoffExpired));
    let lapsed = make_code(&backend, &laptop, "/chat").await;
    assert_eq!(lapsed.refusal(), (StatusCode::FORBIDDEN, ErrorCode::VerificationRequired));
    backend.close().await;
}

/// A phone already signed in to the account opens the page in the session
/// it has: no second session, no cookie, and the laptop learns of the code
/// from the phone's session.
#[tokio::test]
async fn a_phone_signed_in_to_the_account_keeps_its_session() {
    let harness = Harness::new();
    let (backend, laptop) = harness.start_set_up().await;
    let phone = session_from(&backend.login_answer(crate::support::MASTER_EMAIL, MASTER_PASSWORD).await);
    let before: SessionList = backend.get("/api/auth/sessions", Some(&phone)).await.json();
    let [phone_before, _] = before.sessions.as_slice() else {
        panic!("two sessions: {before:?}");
    };
    assert!(phone_before.current);
    let mut laptop_page = backend.sync(&laptop).await;
    laptop_page.snapshot().await;

    let made: HandoffCode = make_code(&backend, &laptop, "/chat").await.json();
    let spent = spend_signed_in(&backend, &code_of(&made), Some(&phone)).await;
    assert_eq!(spent.status, StatusCode::OK, "{}", String::from_utf8_lossy(&spent.body));
    assert_eq!(spent.json::<Value>(), json!({ "to": "/chat" }));
    assert!(spent.session_cookies().is_empty(), "{:?}", spent.session_cookies());

    // Still two browsers, the phone's own session carrying the code.
    let listed = handed_off(&mut laptop_page, &made).await;
    assert_eq!(listed.len(), 2, "{listed:?}");
    let phone_session = listed
        .iter()
        .find(|session| session.handoff_id.as_ref() == Some(&made.id))
        .unwrap();
    assert_eq!(phone_session.id, phone_before.id);
    assert_eq!(backend.get("/api/auth/me", Some(&phone)).await.status, StatusCode::OK);

    // The code is spent all the same.
    let again = spend_signed_in(&backend, &code_of(&made), Some(&phone)).await;
    assert_eq!(again.refusal(), (StatusCode::UNAUTHORIZED, ErrorCode::HandoffExpired));
    backend.close().await;
}

/// A phone signed in to another account signs that one out, its pages with
/// it, and takes this one in a new session.
#[tokio::test]
async fn a_phone_signed_in_to_another_account_takes_this_one() {
    let harness = Harness::new();
    let (backend, laptop) = harness.start_set_up().await;
    harness.add_user("bob@example.test", "bob's password", Role::User);
    let bob = backend.login("bob@example.test", "bob's password").await;
    let mut bob_page = backend.sync(&bob).await;
    bob_page.snapshot().await;
    let mut laptop_page = backend.sync(&laptop).await;
    laptop_page.snapshot().await;

    let made: HandoffCode = make_code(&backend, &laptop, "/chat").await.json();
    let spent = spend_signed_in(&backend, &code_of(&made), Some(&bob)).await;
    assert_eq!(spent.status, StatusCode::OK, "{}", String::from_utf8_lossy(&spent.body));

    // Bob's session is gone, its page closed with it.
    assert_eq!(bob_page.closed().await, (4002, "session_ended".to_owned()));
    let me = backend.get("/api/auth/me", Some(&bob)).await;
    assert_eq!(me.refusal(), (StatusCode::UNAUTHORIZED, ErrorCode::Unauthenticated));

    // The phone holds a new session of this account, made by the code.
    let cookies = spent.session_cookies();
    let [cookie] = cookies.as_slice() else {
        panic!("one session cookie: {cookies:?}");
    };
    let phone = Session {
        cookie: cookie.split(';').next().unwrap().to_owned(),
        user: laptop.user.clone(),
    };
    assert_eq!(backend.get("/api/auth/me", Some(&phone)).await.json::<Identity>().user, laptop.user);
    let listed = handed_off(&mut laptop_page, &made).await;
    assert_eq!(listed.len(), 2, "{listed:?}");
    backend.close().await;
}

#[tokio::test]
async fn signing_another_browser_out_closes_its_pages() {
    let harness = Harness::new();
    let (backend, laptop) = harness.start_set_up().await;
    create(&backend, &laptop, FIRST).await;
    let phone = session_from(&backend.login_answer(crate::support::MASTER_EMAIL, MASTER_PASSWORD).await);
    let mut laptop_page = backend.sync(&laptop).await;
    laptop_page.snapshot().await;
    let mut phone_page = backend.sync(&phone).await;
    phone_page.snapshot().await;
    let mut phone_socket = Socket::connect(&backend, &phone, FIRST).await;

    // Each browser lists both, newest first, and marks its own.
    let listed: SessionList = backend.get("/api/auth/sessions", Some(&laptop)).await.json();
    let [newest, oldest] = listed.sessions.as_slice() else {
        panic!("two sessions: {listed:?}");
    };
    assert_eq!((newest.current, oldest.current), (false, true));
    let (phone_id, laptop_id) = (newest.id.clone(), oldest.id.clone());

    // The browser's own session ends with Sign Out, not here; a session the
    // account does not have is not found.
    let own = backend
        .delete(&format!("/api/auth/sessions/{}", laptop_id.as_str()), &laptop)
        .await;
    assert_eq!(own.refusal(), (StatusCode::CONFLICT, ErrorCode::CurrentSession));
    let unknown = backend.delete("/api/auth/sessions/nothing", &laptop).await;
    assert_eq!(unknown.refusal(), (StatusCode::NOT_FOUND, ErrorCode::SessionNotFound));

    let out = backend
        .delete(&format!("/api/auth/sessions/{}", phone_id.as_str()), &laptop)
        .await;
    assert_eq!(out.status, StatusCode::NO_CONTENT);
    // The phone's pages close at once, their channel and conversation
    // sockets alike, and its next request is signed out.
    assert_eq!(phone_page.closed().await, (4002, "session_ended".to_owned()));
    assert_eq!(phone_socket.closed().await, Some(4002));
    let me = backend.get("/api/auth/me", Some(&phone)).await;
    assert_eq!(me.refusal(), (StatusCode::UNAUTHORIZED, ErrorCode::Unauthenticated));
    // The laptop's list drops it.
    let listed = sessions_part(&mut laptop_page).await;
    assert_eq!(listed.iter().map(|session| &session.id).collect::<Vec<_>>(), [&laptop_id]);
    backend.close().await;
}

/// A conversation socket closes with its session as it expires
/// (`crates/backend-http/src/gate.rs`): at the expiry it holds from its
/// open, unless a request renewed the session meanwhile, by the shard's
/// timer on the paused clock.
#[tokio::test(flavor = "local")]
async fn a_conversation_socket_closes_as_its_session_expires() {
    let mut harness = Harness::new();
    harness.shards = Some(ShardPlacement::Inline);
    let (backend, laptop) = harness.start_set_up().await;
    create(&backend, &laptop, FIRST).await;
    let phone = session_from(&backend.login_answer(crate::support::MASTER_EMAIL, MASTER_PASSWORD).await);
    let mut laptop_socket = Socket::connect(&backend, &laptop, FIRST).await;
    let mut phone_socket = Socket::connect(&backend, &phone, FIRST).await;

    // Twenty days on, a request of the laptop's renews its session; the
    // phone makes none.
    harness.clock.advance(SignedDuration::from_hours(20 * 24));
    elapse(Duration::from_secs(20 * 24 * 3600)).await;
    assert_eq!(backend.get("/api/auth/me", Some(&laptop)).await.status, StatusCode::OK);

    // At thirty days the phone's session expires, and its socket closes;
    // the laptop's stays open and goes on sending its heartbeat.
    harness.clock.advance(SignedDuration::from_hours(10 * 24));
    elapse(Duration::from_secs(10 * 24 * 3600)).await;
    assert_eq!(phone_socket.closed().await, Some(4002));
    elapse(Duration::from_secs(60)).await;
    laptop_socket
        .until(|frame| matches!(frame, demi_conversation_socket_protocol::ServerFrame::Heartbeat))
        .await;
    backend.close().await;
}
