//! Setup, the session cookie and its gate, login and its lockout, and the
//! caller's own account (`backend.md` § Authentication and ownership,
//! `web-api.md` § Account API).

use std::sync::atomic::Ordering;

use demi_web_api::auth::{EmailChangeStarted, Identity, Role, SetupStatus};
use demi_web_api::error::ErrorCode;
use jiff::SignedDuration;
use reqwest::{Method, StatusCode};
use serde_json::json;

use crate::support::{
    Answer, Harness, MASTER_EMAIL, MASTER_PASSWORD, SESSION_COOKIE, Session, TestBackend, session_from,
};

fn days(count: i64) -> SignedDuration {
    SignedDuration::from_hours(24 * count)
}

async fn put(backend: &TestBackend, path: &str, session: &Session, body: serde_json::Value) -> Answer {
    backend.send(Method::PUT, path, Some(&session.cookie), Some(body)).await
}

#[tokio::test]
async fn setup_creates_the_master_once_and_signs_it_in() {
    let harness = Harness::new();
    let backend = harness.start().await;
    assert!(backend.get("/api/setup", None).await.json::<SetupStatus>().needed);

    let answer = backend
        .post(
            "/api/setup",
            None,
            json!({ "email": "  Master@Example.TEST ", "password": MASTER_PASSWORD }),
        )
        .await;
    assert_eq!(answer.status, StatusCode::CREATED);
    let master = session_from(&answer);
    assert_eq!(master.user.email.as_str(), MASTER_EMAIL);
    assert_eq!(master.user.role, Role::Master);
    assert_eq!(master.user.nickname, "");
    let token = master.cookie.strip_prefix(&format!("{SESSION_COOKIE}=")).unwrap();
    assert!(token.len() >= 40 && token.bytes().all(|byte| byte.is_ascii_alphanumeric()), "{token}");
    // The cookie lives as long as the session: 30 days from the setup.
    let attributes = cookie_attributes(&answer.session_cookies()[0]);
    for attribute in ["httponly", "samesite=lax", "path=/", "expires=sat, 24 oct 2026 08:00:00 gmt"] {
        assert!(attributes.contains(&attribute.to_owned()), "{attributes:?}");
    }
    assert!(!attributes.contains(&"secure".to_owned()));

    assert!(!backend.get("/api/setup", None).await.json::<SetupStatus>().needed);
    let again = backend
        .post("/api/setup", None, json!({ "email": "other@example.test", "password": "other-pass-1" }))
        .await;
    assert_eq!(again.refusal(), (StatusCode::NOT_FOUND, ErrorCode::AlreadySetUp));
    assert!(again.session_cookies().is_empty());

    let me = backend.get("/api/auth/me", Some(&master)).await;
    assert_eq!(me.json::<Identity>().user, master.user);
    backend.close().await;
}

#[tokio::test]
async fn concurrent_setups_create_one_master() {
    let harness = Harness::new();
    let backend = harness.start().await;
    let setup = |email: &'static str| {
        backend.post("/api/setup", None, json!({ "email": email, "password": MASTER_PASSWORD }))
    };
    let (first, second) = tokio::join!(setup("first@example.test"), setup("second@example.test"));
    let mut statuses = [first.status, second.status];
    statuses.sort();
    assert_eq!(statuses, [StatusCode::CREATED, StatusCode::NOT_FOUND]);
    backend.close().await;
}

#[tokio::test]
async fn every_other_api_path_wants_a_live_session() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;

    for (method, path) in [
        (Method::GET, "/api/auth/me"),
        (Method::GET, "/api/no-such-resource"),
        (Method::GET, "/api"),
        (Method::GET, "/api/setup/more"),
        (Method::DELETE, "/api/auth/me"),
    ] {
        let anonymous = backend.send(method.clone(), path, None, None).await;
        assert_eq!(anonymous.refusal(), (StatusCode::UNAUTHORIZED, ErrorCode::Unauthenticated), "{method} {path}");
        assert!(anonymous.session_cookies().is_empty());
    }

    let missing = backend.get("/api/no-such-resource", Some(&master)).await;
    assert_eq!(missing.refusal(), (StatusCode::NOT_FOUND, ErrorCode::NotFound));
    assert_eq!(missing.error().message, "No route for GET /api/no-such-resource");
    let wrong_method = backend.send(Method::DELETE, "/api/auth/me", Some(&master.cookie), None).await;
    assert_eq!(wrong_method.refusal(), (StatusCode::NOT_FOUND, ErrorCode::NotFound));

    // A cookie that names no session is refused and cleared.
    let forged = backend
        .send(Method::GET, "/api/auth/me", Some(&format!("{SESSION_COOKIE}=not-a-session")), None)
        .await;
    assert_eq!(forged.refusal(), (StatusCode::UNAUTHORIZED, ErrorCode::Unauthenticated));
    let cleared = cookie_attributes(&forged.session_cookies()[0]);
    assert_eq!(cleared[0], format!("{SESSION_COOKIE}="));
    assert!(cleared.contains(&"max-age=0".to_owned()), "{cleared:?}");
    backend.close().await;
}

#[tokio::test]
async fn a_request_over_https_gets_a_secure_cookie() {
    let harness = Harness::new();
    let (backend, _) = harness.start_set_up().await;
    let forwarded = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!("{}/api/auth/login", backend.url))
        .header("x-forwarded-proto", "https, http")
        .body(json!({ "email": MASTER_EMAIL, "password": MASTER_PASSWORD }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(forwarded.status(), StatusCode::OK);
    let cookie = forwarded.headers().get("set-cookie").unwrap().to_str().unwrap().to_owned();
    assert!(cookie_attributes(&cookie).contains(&"secure".to_owned()), "{cookie}");
    backend.close().await;
}

#[tokio::test]
async fn login_locks_out_after_five_failures_and_logout_ends_the_session() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;

    let wrong = backend.login_answer(MASTER_EMAIL, "nope-nope").await;
    assert_eq!(wrong.refusal(), (StatusCode::UNAUTHORIZED, ErrorCode::InvalidCredentials));
    assert!(wrong.session_cookies().is_empty());

    let signed_in = backend.login(MASTER_EMAIL, MASTER_PASSWORD).await;
    assert_eq!(signed_in.user.id, master.user.id);
    assert_ne!(signed_in.cookie, master.cookie);

    // The success cleared the count: five fresh failures lock the address,
    // and then even the right password is refused.
    for _ in 0..5 {
        let failed = backend.login_answer(MASTER_EMAIL, "nope-nope").await;
        assert_eq!(failed.status, StatusCode::UNAUTHORIZED);
    }
    let locked = backend.login_answer(MASTER_EMAIL, MASTER_PASSWORD).await;
    assert_eq!(locked.refusal(), (StatusCode::TOO_MANY_REQUESTS, ErrorCode::TooManyAttempts));
    // The lock never touches sessions already open.
    assert_eq!(backend.get("/api/auth/me", Some(&signed_in)).await.status, StatusCode::OK);

    let out = backend.post("/api/auth/logout", Some(&signed_in), json!({})).await;
    assert_eq!(out.status, StatusCode::NO_CONTENT);
    assert!(cookie_attributes(&out.session_cookies()[0]).contains(&"max-age=0".to_owned()));
    assert_eq!(backend.get("/api/auth/me", Some(&signed_in)).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(backend.get("/api/auth/me", Some(&master)).await.status, StatusCode::OK);
    backend.close().await;
}

#[tokio::test]
async fn an_address_without_an_account_is_locked_out_too() {
    let harness = Harness::new();
    let (backend, _) = harness.start_set_up().await;
    for _ in 0..5 {
        let failed = backend.login_answer("nobody@example.test", "nope-nope").await;
        assert_eq!(failed.refusal(), (StatusCode::UNAUTHORIZED, ErrorCode::InvalidCredentials));
    }
    let locked = backend.login_answer("NOBODY@example.test", "nope-nope").await;
    assert_eq!(locked.refusal(), (StatusCode::TOO_MANY_REQUESTS, ErrorCode::TooManyAttempts));
    backend.login(MASTER_EMAIL, MASTER_PASSWORD).await;
    backend.close().await;
}

#[tokio::test]
async fn a_user_changes_their_own_password_with_the_current_one() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;

    let wrong = put(&backend, "/api/auth/password", &master, json!({ "current": "wrong-wrong", "next": "second-pass-2" })).await;
    assert_eq!(wrong.refusal(), (StatusCode::UNAUTHORIZED, ErrorCode::InvalidCredentials));
    let short = put(&backend, "/api/auth/password", &master, json!({ "current": MASTER_PASSWORD, "next": "short" })).await;
    assert_eq!(short.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody));
    assert!(short.error().message.contains("next"), "{}", short.error().message);
    let changed = put(&backend, "/api/auth/password", &master, json!({ "current": MASTER_PASSWORD, "next": "second-pass-2" })).await;
    assert_eq!(changed.status, StatusCode::NO_CONTENT);

    let old = backend.login_answer(MASTER_EMAIL, MASTER_PASSWORD).await;
    assert_eq!(old.status, StatusCode::UNAUTHORIZED);
    assert_eq!(backend.login(MASTER_EMAIL, "second-pass-2").await.user.id, master.user.id);
    backend.close().await;
}

#[tokio::test]
async fn a_session_slides_while_used_and_expires_when_silent() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;

    harness.clock.advance(days(10));
    let early = backend.get("/api/auth/me", Some(&master)).await;
    assert_eq!(early.status, StatusCode::OK);
    assert!(early.session_cookies().is_empty());

    // 16 days in, less than 15 remain: the session renews to 30 days from now.
    harness.clock.advance(days(6));
    let renewed = backend.get("/api/auth/me", Some(&master)).await;
    assert_eq!(renewed.status, StatusCode::OK);
    let cookie = cookie_attributes(&renewed.session_cookies()[0]);
    assert_eq!(cookie[0], master.cookie.to_lowercase());
    assert!(cookie.contains(&"expires=mon, 09 nov 2026 08:00:00 gmt".to_owned()), "{cookie:?}");

    // Past the first expiry, inside the renewed one.
    harness.clock.advance(days(20));
    assert_eq!(backend.get("/api/auth/me", Some(&master)).await.status, StatusCode::OK);

    harness.clock.advance(days(31));
    let expired = backend.get("/api/auth/me", Some(&master)).await;
    assert_eq!(expired.refusal(), (StatusCode::UNAUTHORIZED, ErrorCode::Unauthenticated));
    assert!(cookie_attributes(&expired.session_cookies()[0]).contains(&"max-age=0".to_owned()));
    backend.close().await;
}

#[tokio::test]
async fn email_identity_is_normalized_and_the_nickname_persists() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;

    let signed_in = backend.login(&format!("  {}  ", MASTER_EMAIL.to_uppercase()), MASTER_PASSWORD).await;
    assert_eq!(signed_in.user.email.as_str(), MASTER_EMAIL);
    let invalid = backend.login_answer("invalid", MASTER_PASSWORD).await;
    assert_eq!(invalid.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody));
    assert!(invalid.error().message.contains("email"), "{}", invalid.error().message);

    let renamed = backend
        .send(Method::PATCH, "/api/auth/me", Some(&signed_in.cookie), Some(json!({ "nickname": "  New name  " })))
        .await;
    let user = renamed.json::<Identity>().user;
    assert_eq!((user.email.as_str(), user.nickname.as_str()), (MASTER_EMAIL, "New name"));
    assert_eq!(backend.get("/api/auth/me", Some(&master)).await.json::<Identity>().user.nickname, "New name");
    for refused in [json!({ "nickname": " " }), json!({ "nickname": "Ana", "role": "admin" })] {
        let answer = backend
            .send(Method::PATCH, "/api/auth/me", Some(&signed_in.cookie), Some(refused))
            .await;
        assert_eq!(answer.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody));
    }

    let unavailable = backend
        .post("/api/auth/email", Some(&signed_in), json!({ "email": "next@example.test", "password": MASTER_PASSWORD }))
        .await;
    assert_eq!(unavailable.refusal(), (StatusCode::SERVICE_UNAVAILABLE, ErrorCode::MailUnavailable));
    backend.close().await;
}

#[tokio::test]
async fn an_email_change_needs_a_delivered_unexpired_single_use_code_and_survives_restart() {
    let harness = Harness::new().with_mail();
    let (backend, master) = harness.start_set_up().await;

    let wrong = backend
        .post("/api/auth/email", Some(&master), json!({ "email": "next@example.test", "password": "wrong-password" }))
        .await;
    assert_eq!(wrong.refusal(), (StatusCode::UNAUTHORIZED, ErrorCode::InvalidCredentials));
    let own = start_email_change(&backend, &master, MASTER_EMAIL).await;
    assert_eq!(own.refusal(), (StatusCode::CONFLICT, ErrorCode::EmailTaken));

    let started = start_email_change(&backend, &master, "NEXT@example.test").await;
    assert_eq!(started.status, StatusCode::ACCEPTED);
    let challenge = started.json::<EmailChangeStarted>().challenge;
    let mail = harness.mailbox.sent()[0].clone();
    assert_eq!(mail.email.as_str(), "next@example.test");
    assert_eq!(challenge.email, mail.email);
    assert_eq!(challenge.expires_at.to_string(), "2026-09-24T08:10:00.000Z");
    assert!(!String::from_utf8_lossy(&started.body).contains(&mail.code));
    let again = start_email_change(&backend, &master, "next@example.test").await;
    assert_eq!(again.refusal(), (StatusCode::TOO_MANY_REQUESTS, ErrorCode::TooManyAttempts));
    for malformed in ["abcdef", "١٢٣٤٥٦"] {
        let refused = backend
            .post("/api/auth/email/confirm", Some(&master), json!({ "id": challenge.id, "code": malformed }))
            .await;
        assert_eq!(refused.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody));
    }

    backend.close().await;
    let backend = harness.start().await;
    let master = backend.login(MASTER_EMAIL, MASTER_PASSWORD).await;
    let confirm = json!({ "id": challenge.id, "code": mail.code });
    let confirmed = backend.post("/api/auth/email/confirm", Some(&master), confirm.clone()).await;
    assert_eq!(confirmed.status, StatusCode::OK);
    assert_eq!(confirmed.json::<Identity>().user.email.as_str(), "next@example.test");
    let reused = backend.post("/api/auth/email/confirm", Some(&master), confirm).await;
    assert_eq!(reused.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidCode));
    assert_eq!(backend.login_answer(MASTER_EMAIL, MASTER_PASSWORD).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(backend.login("next@example.test", MASTER_PASSWORD).await.user.id, master.user.id);

    // After the cooldown a new code goes out; ten minutes later it is dead.
    harness.clock.advance(SignedDuration::from_mins(1));
    let later = start_email_change(&backend, &master, "later@example.test").await;
    let challenge = later.json::<EmailChangeStarted>().challenge;
    harness.clock.advance(SignedDuration::from_mins(10));
    let code = harness.mailbox.sent()[1].code.clone();
    let expired = backend
        .post("/api/auth/email/confirm", Some(&master), json!({ "id": challenge.id, "code": code }))
        .await;
    assert_eq!(expired.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidCode));
    backend.close().await;
}

#[tokio::test]
async fn a_failed_delivery_allows_a_retry_and_wrong_codes_or_a_new_password_end_a_challenge() {
    let harness = Harness::new().with_mail();
    let (backend, master) = harness.start_set_up().await;
    let start = || backend.post("/api/auth/email", Some(&master), json!({ "email": "new@example.test", "password": MASTER_PASSWORD }));

    harness.mailbox.failing.store(true, Ordering::SeqCst);
    assert_eq!(start().await.refusal(), (StatusCode::SERVICE_UNAVAILABLE, ErrorCode::MailFailed));
    // The failed code holds back no retry.
    harness.mailbox.failing.store(false, Ordering::SeqCst);
    let challenge = start().await.json::<EmailChangeStarted>().challenge;
    let code = harness.mailbox.sent()[0].code.clone();
    let wrong = if code == "000000" { "111111" } else { "000000" };
    for _ in 0..5 {
        let refused = backend
            .post("/api/auth/email/confirm", Some(&master), json!({ "id": challenge.id, "code": wrong }))
            .await;
        assert_eq!(refused.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidCode));
    }
    let used_up = backend
        .post("/api/auth/email/confirm", Some(&master), json!({ "id": challenge.id, "code": code }))
        .await;
    assert_eq!(used_up.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidCode));

    harness.clock.advance(SignedDuration::from_mins(1));
    let challenge = start().await.json::<EmailChangeStarted>().challenge;
    let code = harness.mailbox.sent()[1].code.clone();
    let changed = put(&backend, "/api/auth/password", &master, json!({ "current": MASTER_PASSWORD, "next": "new-password-2" })).await;
    assert_eq!(changed.status, StatusCode::NO_CONTENT);
    let ended = backend
        .post("/api/auth/email/confirm", Some(&master), json!({ "id": challenge.id, "code": code }))
        .await;
    assert_eq!(ended.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidCode));
    backend.close().await;
}

async fn start_email_change(backend: &TestBackend, session: &Session, email: &str) -> Answer {
    let body = json!({ "email": email, "password": MASTER_PASSWORD });
    backend.post("/api/auth/email", Some(session), body).await
}

/// A `Set-Cookie` value's parts, lowercased: the name and value first, then
/// each attribute.
fn cookie_attributes(cookie: &str) -> Vec<String> {
    cookie.split(';').map(|part| part.trim().to_lowercase()).collect()
}
