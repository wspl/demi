//! Account administration (`web-api.md` § Account API, `users.md` § User
//! system): the master and admins manage only roles below their own; an
//! account an administrator made or reset chooses its own password before
//! anything else opens; a disabled account signs in no way and is signed out
//! at once; a sign-in code signs a phone in and names its issuer.

use jiff::SignedDuration;
use reqwest::StatusCode;
use serde_json::{Value, json};

use demi_runner_protocol::wire::{self, Inbound};
use demi_web_api_protocol::auth::Identity;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::sessions::{HandoffCode, SessionList};
use demi_web_api_protocol::state::SyncEvent;
use demi_web_api_protocol::users::{ListedUser, Users};

use crate::runners::{RawRunner, hello};
use crate::support::{Answer, Harness, Session, TestBackend, answer};

const FORBIDDEN: (StatusCode, ErrorCode) = (StatusCode::FORBIDDEN, ErrorCode::Forbidden);

async fn create(backend: &TestBackend, actor: &Session, email: &str, role: &str) -> Answer {
    backend
        .post("/api/users", Some(actor), json!({ "email": email, "role": role }))
        .await
}

/// The id and the first password of the account a creation made.
fn made(answer: &Answer) -> (String, String) {
    assert_eq!(answer.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&answer.body));
    let body = answer.json::<Value>();
    (
        body["user"]["id"].as_str().unwrap().to_owned(),
        body["password"].as_str().unwrap().to_owned(),
    )
}

async fn change(backend: &TestBackend, actor: &Session, id: &str, body: Value) -> Answer {
    backend.patch(&format!("/api/users/{id}"), actor, body).await
}

async fn act(backend: &TestBackend, actor: &Session, id: &str, action: &str) -> Answer {
    backend
        .post(&format!("/api/users/{id}/{action}"), Some(actor), json!({}))
        .await
}

async fn listed(backend: &TestBackend, actor: &Session) -> Users {
    backend.get("/api/users", Some(actor)).await.json()
}

fn code_of(made: &HandoffCode) -> String {
    url::Url::parse(&made.url).unwrap().fragment().unwrap().to_owned()
}

/// Spends `code` from a browser that holds no session.
async fn spend(backend: &TestBackend, code: &str) -> Answer {
    answer(
        reqwest::Client::new()
            .post(format!("{}/api/auth/handoff", backend.url))
            .header("user-agent", "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 Version/18.0 Mobile/15E148 Safari/604.1")
            .header("content-type", "application/json")
            .body(json!({ "code": code }).to_string())
            .send()
            .await
            .unwrap(),
    )
    .await
}

/// Cost: one backend with a few accounts; a second or two of password hashing.
#[tokio::test]
async fn every_route_acts_only_on_a_role_below_the_callers() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let ana = backend.make_user(&master, "ana@example.test", "admin").await;
    let mei = backend.make_user(&master, "mei@example.test", "admin").await;
    let bo = backend.make_user(&master, "bo@example.test", "user").await;
    let (ana_id, mei_id, bo_id, master_id) = (
        ana.user.id.to_string(),
        mei.user.id.to_string(),
        bo.user.id.to_string(),
        master.user.id.to_string(),
    );

    // A user manages nothing; the lists are an administrator's.
    assert_eq!(backend.get("/api/users", Some(&bo)).await.refusal(), FORBIDDEN);
    assert_eq!(create(&backend, &bo, "x@example.test", "user").await.refusal(), FORBIDDEN);
    assert_eq!(act(&backend, &bo, &bo_id, "password").await.refusal(), FORBIDDEN);

    // The master makes admins; an admin makes users only; no one makes a master.
    assert_eq!(create(&backend, &ana, "dave@example.test", "admin").await.refusal(), FORBIDDEN);
    made(&create(&backend, &ana, "carol@example.test", "user").await);
    made(&create(&backend, &master, "erin@example.test", "admin").await);
    assert_eq!(
        create(&backend, &master, "m@example.test", "master").await.refusal(),
        (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody)
    );
    assert_eq!(
        create(&backend, &master, "Bo@Example.test", "user").await.refusal(),
        (StatusCode::CONFLICT, ErrorCode::EmailTaken)
    );

    // First passwords, role changes, disabling, signing out and sign-in codes
    // go down the ranks, never sideways or up; whom a request names is
    // answered before its body.
    for (actor, target, allowed) in [
        (&ana, &bo_id, true),
        (&ana, &mei_id, false),
        (&ana, &master_id, false),
        (&master, &mei_id, true),
        (&master, &master_id, false),
    ] {
        for action in ["password", "sign-out", "sign-in-code"] {
            let answered = act(&backend, actor, target, action).await;
            if allowed {
                assert!(answered.status.is_success(), "{action}: {}", String::from_utf8_lossy(&answered.body));
            } else {
                assert_eq!(answered.refusal(), FORBIDDEN, "{action}");
            }
        }
        let unchecked = change(&backend, actor, target, json!({ "nonsense": 1 })).await;
        assert_eq!(
            unchecked.refusal(),
            if allowed { (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody) } else { FORBIDDEN }
        );
    }
    assert_eq!(
        act(&backend, &master, "nobody", "password").await.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::UserNotFound)
    );

    // Only the master changes roles: a role is given only below the giver's own.
    assert_eq!(change(&backend, &ana, &bo_id, json!({ "role": "admin" })).await.status, StatusCode::FORBIDDEN);
    assert_eq!(change(&backend, &master, &mei_id, json!({ "role": "user" })).await.status, StatusCode::NO_CONTENT);
    assert_eq!(change(&backend, &master, &bo_id, json!({ "role": "admin" })).await.status, StatusCode::NO_CONTENT);
    assert_eq!(
        change(&backend, &master, &bo_id, json!({ "role": "master" })).await.refusal(),
        (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody)
    );
    assert_eq!(
        change(&backend, &master, &bo_id, json!({ "role": "user", "disabled": true })).await.refusal(),
        (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody)
    );
    // A user the master makes an admin acts as one, and acts on no peer.
    let fay = backend.make_user(&master, "fay@example.test", "user").await;
    assert_eq!(act(&backend, &fay, &ana_id, "sign-out").await.refusal(), FORBIDDEN);
    assert_eq!(
        change(&backend, &master, fay.user.id.as_str(), json!({ "role": "admin" })).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(act(&backend, &fay, &ana_id, "sign-out").await.refusal(), FORBIDDEN);
    let gus = backend.make_user(&master, "gus@example.test", "user").await;
    assert_eq!(act(&backend, &fay, gus.user.id.as_str(), "sign-out").await.status, StatusCode::NO_CONTENT);
    assert_eq!(
        change(&backend, &ana, gus.user.id.as_str(), json!({ "disabled": true })).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(change(&backend, &ana, &ana_id, json!({ "disabled": true })).await.refusal(), FORBIDDEN);
    backend.close().await;
}

/// Cost: one backend; a few seconds of password hashing.
#[tokio::test]
async fn an_account_made_or_reset_chooses_its_own_password_before_anything_else_opens() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let created = create(&backend, &master, "bo@example.test", "user").await;
    let (bo_id, first) = made(&created);
    assert_eq!(first.len(), 20);
    assert!(first.chars().all(|c| c.is_ascii_alphanumeric()), "{first}");
    assert_eq!(created.json::<Value>()["user"]["choosesPassword"], json!(true));

    // It has not signed in: no activity, a row that says it has not chosen.
    let row = |users: &Users| users.users.iter().find(|listed| listed.user.id.as_str() == bo_id).cloned().unwrap();
    let before = row(&listed(&backend, &master).await);
    assert_eq!((before.user.disabled, before.user.chooses_password, before.last_active_at), (false, true, None));

    // The first password signs in, and only the choice, who it is, signing
    // out and the passkey routes answer; every other route, a socket too,
    // is refused.
    let bo = backend.login("bo@example.test", &first).await;
    let blocked = (StatusCode::FORBIDDEN, ErrorCode::PasswordChoiceRequired);
    assert_eq!(backend.get("/api/auth/me", Some(&bo)).await.status, StatusCode::OK);
    assert_eq!(backend.get("/api/settings", Some(&bo)).await.refusal(), blocked);
    assert_eq!(backend.get("/api/auth/sessions", Some(&bo)).await.refusal(), blocked);
    assert_eq!(backend.get("/api/auth/passkeys", Some(&bo)).await.status, StatusCode::OK);
    let me: Identity = backend.get("/api/auth/me", Some(&bo)).await.json();
    assert!(me.user.chooses_password);
    // The choice is the new password alone, 8 to 1024 characters.
    for body in [
        json!({ "current": first, "next": "chosen-by-bo-1" }),
        json!({ "next": "short" }),
    ] {
        let refused = backend.put("/api/auth/password", &bo, body).await;
        assert_eq!(refused.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody));
    }
    assert_eq!(backend.get("/api/settings", Some(&bo)).await.refusal(), blocked);
    let chosen = backend.put("/api/auth/password", &bo, json!({ "next": "chosen-by-bo-1" })).await;
    assert_eq!(chosen.status, StatusCode::NO_CONTENT);

    // Then it is an account like any other: the first password is gone, its
    // own signs in, and the next change takes the current password.
    assert_eq!(backend.get("/api/settings", Some(&bo)).await.status, StatusCode::OK);
    assert_eq!(
        backend.login_answer("bo@example.test", &first).await.refusal(),
        (StatusCode::UNAUTHORIZED, ErrorCode::InvalidCredentials)
    );
    backend.login("bo@example.test", "chosen-by-bo-1").await;
    let again = backend.put("/api/auth/password", &bo, json!({ "next": "another-one-1" })).await;
    assert_eq!(again.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody));
    let after = row(&listed(&backend, &master).await);
    assert_eq!((after.user.chooses_password, after.last_active_at.is_some()), (false, true));

    // A new first password ends the one before at once, and its owner must
    // choose again: the open session stays, its channel ends and is refused.
    let mut page = backend.sync(&bo).await;
    page.snapshot().await;
    let rotated = act(&backend, &master, &bo_id, "password").await;
    assert_eq!(rotated.status, StatusCode::OK);
    let second = rotated.json::<Value>()["password"].as_str().unwrap().to_owned();
    assert_ne!(second, first);
    assert_eq!(page.closed().await, (4002, "session_ended".to_owned()));
    assert_eq!(backend.get("/api/settings", Some(&bo)).await.refusal(), blocked);
    assert_eq!(
        backend.login_answer("bo@example.test", "chosen-by-bo-1").await.refusal(),
        (StatusCode::UNAUTHORIZED, ErrorCode::InvalidCredentials)
    );
    let bo_again = backend.login("bo@example.test", &second).await;
    assert_eq!(backend.get("/api/settings", Some(&bo_again)).await.refusal(), blocked);
    // Another new password stops the one shown before it.
    let third = act(&backend, &master, &bo_id, "password").await.json::<Value>()["password"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        backend.login_answer("bo@example.test", &second).await.refusal(),
        (StatusCode::UNAUTHORIZED, ErrorCode::InvalidCredentials)
    );
    backend.login("bo@example.test", &third).await;
    backend.close().await;
}

/// Cost: one backend; a few seconds of password hashing.
#[tokio::test]
async fn a_disabled_account_signs_in_no_way_and_is_signed_out_at_once_until_enabled() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let bo = backend.make_user(&master, "bo@example.test", "user").await;
    let bo_id = bo.user.id.to_string();
    let mut page = backend.sync(&bo).await;
    page.snapshot().await;
    // A code made for Bo before he is disabled.
    let code: HandoffCode = act(&backend, &master, &bo_id, "sign-in-code").await.json();

    let disabled = change(&backend, &master, &bo_id, json!({ "disabled": true })).await;
    assert_eq!(disabled.status, StatusCode::NO_CONTENT);
    // His pages close and his session is gone.
    assert_eq!(page.closed().await, (4002, "session_ended".to_owned()));
    assert_eq!(
        backend.get("/api/auth/me", Some(&bo)).await.refusal(),
        (StatusCode::UNAUTHORIZED, ErrorCode::Unauthenticated)
    );
    let account = listed(&backend, &master).await.users.into_iter().find(|listed| listed.user.id == bo.user.id).unwrap();
    assert!(account.user.disabled);

    // Neither his password nor a code signs him in, and the code is spent.
    let disabled_refusal = (StatusCode::FORBIDDEN, ErrorCode::AccountDisabled);
    assert_eq!(backend.login_answer("bo@example.test", "bo-pass-1").await.refusal(), disabled_refusal);
    let spent = spend(&backend, &code_of(&code)).await;
    assert_eq!(spent.refusal(), disabled_refusal);
    assert!(spent.session_cookies().is_empty());
    // A wrong password is still only a wrong password.
    assert_eq!(
        backend.login_answer("bo@example.test", "wrong-password").await.refusal(),
        (StatusCode::UNAUTHORIZED, ErrorCode::InvalidCredentials)
    );

    // Enabling undoes it: nothing was removed.
    assert_eq!(change(&backend, &master, &bo_id, json!({ "disabled": false })).await.status, StatusCode::NO_CONTENT);
    let back = backend.login("bo@example.test", "bo-pass-1").await;
    assert_eq!(backend.get("/api/settings", Some(&back)).await.status, StatusCode::OK);
    backend.close().await;
}

/// Cost: one backend; a few seconds of password hashing.
#[tokio::test]
async fn a_sign_in_code_signs_a_phone_in_and_names_the_administrator_who_made_it() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let ana = backend.make_user(&master, "ana@example.test", "admin").await;
    let (bo_id, _) = made(&create(&backend, &ana, "bo@example.test", "user").await);

    let first: HandoffCode = act(&backend, &ana, &bo_id, "sign-in-code").await.json();
    assert_eq!(first.expires_in_ms, 10 * 60 * 1000);
    assert!(first.url.starts_with(&format!("{}/handoff#", backend.url)), "{}", first.url);
    // A new code ends the one before; the code is spent once.
    let second: HandoffCode = act(&backend, &ana, &bo_id, "sign-in-code").await.json();
    assert_eq!(
        spend(&backend, &code_of(&first)).await.refusal(),
        (StatusCode::UNAUTHORIZED, ErrorCode::HandoffExpired)
    );
    let spent = spend(&backend, &code_of(&second)).await;
    assert_eq!(spent.status, StatusCode::OK, "{}", String::from_utf8_lossy(&spent.body));
    assert_eq!(spent.json::<Value>(), json!({ "to": "/" }));
    assert_eq!(
        spend(&backend, &code_of(&second)).await.refusal(),
        (StatusCode::UNAUTHORIZED, ErrorCode::HandoffExpired)
    );
    let cookie = spent.session_cookies().remove(0).split(';').next().unwrap().to_owned();
    let phone = Session { cookie, user: ana.user.clone() };

    // The phone is Bo's, still to choose his password; once he has, his list
    // of browsers names Ana on it.
    let me: Identity = backend.get("/api/auth/me", Some(&phone)).await.json();
    assert_eq!(me.user.id.as_str(), bo_id);
    assert!(me.user.chooses_password);
    let chosen = backend.put("/api/auth/password", &phone, json!({ "next": "chosen-by-bo-1" })).await;
    assert_eq!(chosen.status, StatusCode::NO_CONTENT);
    let sessions: SessionList = backend.get("/api/auth/sessions", Some(&phone)).await.json();
    let [only] = sessions.sessions.as_slice() else {
        panic!("one session: {sessions:?}");
    };
    // Ana has no nickname: her email names her.
    assert_eq!(only.issued_by.as_deref(), Some("ana@example.test"));
    assert!(only.handoff_id.is_some());
    // A session Bo signs in with his password has no issuer.
    let laptop = backend.login("bo@example.test", "chosen-by-bo-1").await;
    let sessions: SessionList = backend.get("/api/auth/sessions", Some(&laptop)).await.json();
    assert_eq!(sessions.sessions.iter().filter(|s| s.issued_by.is_none()).count(), 1);

    // A sign-in code asks the administrator's own verification, as a handoff
    // code does.
    harness.clock.advance(SignedDuration::from_mins(5));
    assert_eq!(
        act(&backend, &ana, &bo_id, "sign-in-code").await.refusal(),
        (StatusCode::FORBIDDEN, ErrorCode::VerificationRequired)
    );
    for (action, status) in [("password", StatusCode::FORBIDDEN), ("sign-in-code", StatusCode::FORBIDDEN)] {
        assert_eq!(act(&backend, &ana, &bo_id, action).await.status, status);
    }
    assert_eq!(
        create(&backend, &ana, "eve@example.test", "user").await.refusal(),
        (StatusCode::FORBIDDEN, ErrorCode::VerificationRequired)
    );
    backend.close().await;
}

/// Cost: one backend; a few seconds of password hashing.
#[tokio::test]
async fn signing_an_account_out_everywhere_ends_every_session_it_has() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let bo = backend.make_user(&master, "bo@example.test", "user").await;
    let bo_phone = backend.login("bo@example.test", "bo-pass-1").await;
    let mut page = backend.sync(&bo).await;
    page.snapshot().await;

    let out = act(&backend, &master, bo.user.id.as_str(), "sign-out").await;
    assert_eq!(out.status, StatusCode::NO_CONTENT);
    assert_eq!(page.closed().await, (4002, "session_ended".to_owned()));
    for session in [&bo, &bo_phone] {
        assert_eq!(
            backend.get("/api/auth/me", Some(session)).await.refusal(),
            (StatusCode::UNAUTHORIZED, ErrorCode::Unauthenticated)
        );
    }
    // Nothing is removed: he signs in again.
    backend.login("bo@example.test", "bo-pass-1").await;
    backend.close().await;
}

/// Cost: one backend and one real runner; a few seconds.
#[tokio::test(flavor = "multi_thread")]
async fn disabling_an_account_ends_its_runner_connections_and_turns_its_runners_away_until_enabled() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let bo = backend.make_user(&master, "bo@example.test", "user").await;
    let mut laptop = backend.pair(&bo, "laptop").await;
    let token = laptop.token().await;
    laptop.runner.stop().await;
    backend.until_online(&bo, laptop.id(), false).await;

    // The runner is welcomed while Bo's account is enabled, and its
    // connection ends the moment the account is disabled.
    let mut runner = RawRunner::connect(&backend).await;
    runner.send(&hello(wire::VERSION, Some(&token), None)).await;
    assert!(matches!(runner.next().await, Some(Inbound::HelloOk { .. })));
    let bo_id = bo.user.id.to_string();
    assert_eq!(change(&backend, &master, &bo_id, json!({ "disabled": true })).await.status, StatusCode::NO_CONTENT);
    runner.closed().await;

    // It reconnects and is closed on without a word, which a runner takes
    // for a lost connection and tries again.
    let mut again = RawRunner::connect(&backend).await;
    again.send(&hello(wire::VERSION, Some(&token), None)).await;
    assert_eq!(again.next().await, None);

    assert_eq!(change(&backend, &master, &bo_id, json!({ "disabled": false })).await.status, StatusCode::NO_CONTENT);
    let mut back = RawRunner::connect(&backend).await;
    back.send(&hello(wire::VERSION, Some(&token), None)).await;
    assert!(matches!(back.next().await, Some(Inbound::HelloOk { .. })));
    backend.close().await;
}

/// The accounts of the first `users` message of `page` that `holds`, skipping
/// the earlier ones its changes sent.
async fn users_where(page: &mut crate::support::SyncChannel, holds: impl Fn(&[ListedUser]) -> bool) -> Vec<ListedUser> {
    let received = page
        .until(|event| matches!(event, SyncEvent::Users { users } if holds(users)))
        .await;
    match received.into_iter().last() {
        Some(SyncEvent::Users { users }) => users,
        other => unreachable!("{other:?}"),
    }
}

fn account<'a>(users: &'a [ListedUser], email: &str) -> &'a ListedUser {
    users.iter().find(|listed| listed.user.email.as_str() == email).unwrap()
}

/// Cost: one backend; a few seconds of password hashing.
#[tokio::test]
async fn the_accounts_reach_only_administrators_channels_and_follow_each_change() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let ana = backend.make_user(&master, "ana@example.test", "admin").await;
    let bo = backend.make_user(&master, "bo@example.test", "user").await;
    let mut master_page = backend.sync(&master).await;
    let snapshot = master_page.snapshot().await;
    assert_eq!(snapshot.users.as_ref().map(Vec::len), Some(3));
    let mut ana_page = backend.sync(&ana).await;
    assert_eq!(ana_page.snapshot().await.users.as_ref().map(Vec::len), Some(3));
    // A user's state has no accounts.
    let mut bo_page = backend.sync(&bo).await;
    assert_eq!(bo_page.snapshot().await.users, None);

    // A change reaches every administrator's page at once; the user's own
    // page hears nothing of it.
    let bo_id = bo.user.id.to_string();
    let (_, first) = made(&create(&backend, &master, "cy@example.test", "user").await);
    for page in [&mut master_page, &mut ana_page] {
        let users = users_where(page, |users| users.len() == 4).await;
        assert!(account(&users, "cy@example.test").user.chooses_password);
    }
    // Signing in, choosing a password, disabling and enabling each change it.
    let cy = backend.login("cy@example.test", &first).await;
    users_where(&mut master_page, |users| account(users, "cy@example.test").last_active_at.is_some()).await;
    let chosen = backend.put("/api/auth/password", &cy, json!({ "next": "cy chose this 1" })).await;
    assert_eq!(chosen.status, StatusCode::NO_CONTENT);
    users_where(&mut master_page, |users| !account(users, "cy@example.test").user.chooses_password).await;
    assert_eq!(change(&backend, &master, &bo_id, json!({ "disabled": true })).await.status, StatusCode::NO_CONTENT);
    users_where(&mut ana_page, |users| account(users, "bo@example.test").user.disabled).await;
    assert_eq!(change(&backend, &master, &bo_id, json!({ "disabled": false })).await.status, StatusCode::NO_CONTENT);
    users_where(&mut ana_page, |users| !account(users, "bo@example.test").user.disabled).await;
    backend.close().await;
}

/// Cost: one backend; a few seconds of password hashing.
#[tokio::test]
async fn an_account_made_an_admin_starts_to_receive_the_accounts() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let bo = backend.make_user(&master, "bo@example.test", "user").await;
    let mut bo_page = backend.sync(&bo).await;
    assert_eq!(bo_page.snapshot().await.users, None);
    let promoted = change(&backend, &master, bo.user.id.as_str(), json!({ "role": "admin" })).await;
    assert_eq!(promoted.status, StatusCode::NO_CONTENT);
    assert_eq!(users_where(&mut bo_page, |users| users.len() == 2).await.len(), 2);
    backend.close().await;
}

/// Cost: one backend; a few seconds of password hashing.
#[tokio::test]
async fn an_accounts_last_activity_outlives_its_sessions() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let bo = backend.make_user(&master, "bo@example.test", "user").await;
    let row = |users: &Users| users.users.iter().find(|listed| listed.user.id == bo.user.id).cloned().unwrap();
    let active = row(&listed(&backend, &master).await).last_active_at;
    assert!(active.is_some());
    harness.clock.advance(SignedDuration::from_hours(2));
    // The page's next request writes its session's activity, and the account's.
    assert_eq!(backend.get("/api/auth/me", Some(&bo)).await.status, StatusCode::OK);
    let later = row(&listed(&backend, &master).await).last_active_at;
    assert!(later > active, "{later:?} {active:?}");
    // Disabling ends the sessions; the account still says when it was last used.
    assert_eq!(change(&backend, &master, bo.user.id.as_str(), json!({ "disabled": true })).await.status, StatusCode::NO_CONTENT);
    assert_eq!(row(&listed(&backend, &master).await).last_active_at, later);
    backend.close().await;
}

/// Cost: one backend; a few seconds of password hashing.
#[tokio::test]
async fn the_channel_of_an_account_choosing_its_password_carries_the_account_alone_until_it_is_chosen() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let (_, first) = made(&create(&backend, &master, "bo@example.test", "user").await);
    let one = backend.login("bo@example.test", &first).await;
    let two = backend.login("bo@example.test", &first).await;
    // Both pages' channels open and say only who the account is.
    let mut waiting = backend.sync(&one).await;
    let mut choosing = backend.sync(&two).await;
    for page in [&mut waiting, &mut choosing] {
        match page.next().await {
            SyncEvent::User { user } => assert!(user.chooses_password),
            other => panic!("the account alone first: {other:?}"),
        }
    }
    // Its conversation sockets stay shut.
    assert_eq!(
        backend.get("/api/conversations", Some(&one)).await.refusal(),
        (StatusCode::FORBIDDEN, ErrorCode::PasswordChoiceRequired)
    );

    // One page chooses; the other learns it from its own channel, which then
    // sends the whole state.
    let chosen = backend.put("/api/auth/password", &two, json!({ "next": "bo chose this 1" })).await;
    assert_eq!(chosen.status, StatusCode::NO_CONTENT);
    for page in [&mut waiting, &mut choosing] {
        match page.next().await {
            SyncEvent::User { user } => assert!(!user.chooses_password),
            other => panic!("the changed account: {other:?}"),
        }
        let state = page.snapshot().await;
        assert!(!state.user.chooses_password);
    }
    assert_eq!(backend.get("/api/settings", Some(&one)).await.status, StatusCode::OK);
    backend.close().await;
}

/// Cost: one backend; a few seconds of password hashing.
#[tokio::test]
async fn a_disabled_accounts_sign_in_is_refused_without_a_request_id_to_trace() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let bo = backend.make_user(&master, "bo@example.test", "user").await;
    assert_eq!(
        change(&backend, &master, bo.user.id.as_str(), json!({ "disabled": true })).await.status,
        StatusCode::NO_CONTENT
    );
    let refused = backend.login_answer("bo@example.test", "bo-pass-1").await;
    assert_eq!(refused.refusal(), (StatusCode::FORBIDDEN, ErrorCode::AccountDisabled));
    backend.close().await;
}
