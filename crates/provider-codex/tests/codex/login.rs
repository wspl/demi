//! Codex's device login and the accounts it stores (`providers.md` § Login
//! and publication).

use std::sync::{Arc, Mutex};

use demi_provider_codex::{CodexConfig, CodexProvider};
use demi_provider_common::{
    Provider,
    credentials::{AccountsCapability, CredentialPool, LoginError, LoginKind, MemoryCredentialPool},
    quota::MemorySnapshots,
    testing::{FixedClock, MockResponse, MockVendor, jwt, login_io},
};
use demi_shared_types::LoginPending;
use serde_json::{Value, json};

use crate::NOW;

/// A provider built to log in: no account yet, over a staged pool.
fn staged(vendor: &MockVendor, pool: &MemoryCredentialPool) -> CodexProvider {
    let mut config = CodexConfig::new(None);
    config.auth_url = vendor.url("").parse().unwrap();
    let clock = Arc::new(FixedClock(NOW.parse().unwrap()));
    CodexProvider::new(
        config,
        Arc::new(pool.clone()),
        Arc::new(MemorySnapshots::new()),
        reqwest::Client::new(),
        clock,
    )
}

fn json_answer(status: u16, body: Value) -> MockResponse {
    MockResponse::status(status)
        .header("content-type", "application/json")
        .chunk(body.to_string())
}

fn access() -> String {
    jwt(
        &json!({ "email": "device@example.com", "https://api.openai.com/auth": { "chatgpt_account_id": "acct-device" } }),
    )
}

#[tokio::test]
async fn a_device_login_shows_its_code_once_polls_until_confirmed_and_stores_the_account() {
    let vendor = MockVendor::start().await;
    vendor.respond(json_answer(
        200,
        json!({ "device_auth_id": "dev_auth_1", "user_code": "WXYZ-9876", "interval": "0" }),
    ));
    vendor.respond(json_answer(403, json!({})));
    vendor.respond(json_answer(200, json!({ "authorization_code": "authz_1", "code_challenge": "challenge", "code_verifier": "verifier_1" })));
    vendor.respond(json_answer(200, json!({ "id_token": jwt(&json!({ "email": "device@example.com" })), "access_token": access(), "refresh_token": "refresh_1" })));
    let pool = MemoryCredentialPool::new();
    let provider = staged(&vendor, &pool);
    let accounts = provider.accounts().unwrap();
    assert_eq!(
        accounts.capability(),
        AccountsCapability {
            login: Some(LoginKind::Device)
        }
    );

    let shown = Arc::new(Mutex::new(Vec::new()));
    let report = {
        let shown = shown.clone();
        move |pending: LoginPending| shown.lock().unwrap().push(pending)
    };
    let account = accounts.login(login_io(&report)).await.unwrap();
    let shown = shown.lock().unwrap().clone();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].verification_url, vendor.url("/codex/device"));
    assert_eq!(shown[0].user_code.as_deref(), Some("WXYZ-9876"));

    assert_eq!(account.label, "device@example.com");
    assert_eq!(
        accounts.active().await.unwrap().as_deref(),
        Some(account.id.as_str())
    );
    let stored: Value = serde_json::from_str(
        &pool
            .document(&account.id)
            .read()
            .await
            .unwrap()
            .unwrap()
            .text,
    )
    .unwrap();
    assert_eq!(
        (
            &stored["accountId"],
            &stored["refreshToken"],
            &stored["lastRefresh"]
        ),
        (&json!("acct-device"), &json!("refresh_1"), &json!(NOW))
    );

    let requests = vendor.requests();
    let paths: Vec<&str> = requests.iter().map(|request| request.uri.path()).collect();
    assert_eq!(
        paths,
        [
            "/api/accounts/deviceauth/usercode",
            "/api/accounts/deviceauth/token",
            "/api/accounts/deviceauth/token",
            "/oauth/token"
        ]
    );
    assert_eq!(
        requests[0].json(),
        json!({ "client_id": "app_EMoamEEZ73f0CkXaXp7hrann" })
    );
    assert_eq!(
        requests[1].json(),
        json!({ "device_auth_id": "dev_auth_1", "user_code": "WXYZ-9876" })
    );
    let exchange = String::from_utf8(requests[3].body.to_vec()).unwrap();
    for field in [
        "grant_type=authorization_code",
        "code=authz_1",
        "code_verifier=verifier_1",
        "client_id=app_EMoamEEZ73f0CkXaXp7hrann",
    ] {
        assert!(exchange.contains(field), "{exchange}");
    }
    let callback = format!(
        "redirect_uri={}",
        url_encode(&vendor.url("/deviceauth/callback"))
    );
    assert!(exchange.contains(&callback), "{exchange}");
}

fn url_encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'*' => {
                (byte as char).to_string()
            }
            b' ' => "+".into(),
            other => format!("%{other:02X}"),
        })
        .collect()
}

#[tokio::test]
async fn a_device_code_without_a_user_code_or_an_unavailable_login_fails() {
    let vendor = MockVendor::start().await;
    vendor.respond(json_answer(200, json!({ "device_auth_id": "dev_auth_1" })));
    vendor.respond(json_answer(404, json!({})));
    let pool = MemoryCredentialPool::new();
    let provider = staged(&vendor, &pool);
    let report = |_: LoginPending| {};
    let missing = provider
        .accounts()
        .unwrap()
        .login(login_io(&report))
        .await
        .unwrap_err();
    assert_eq!(
        missing,
        LoginError::Failed("Device code response is malformed: user_code is missing".into())
    );
    let unavailable = provider
        .accounts()
        .unwrap()
        .login(login_io(&report))
        .await
        .unwrap_err();
    assert_eq!(
        unavailable,
        LoginError::Unavailable("Device-code login is not enabled for this Codex account".into())
    );
    assert!(pool.list().await.unwrap().is_empty());
}

#[tokio::test]
async fn dropping_a_login_cancels_it_between_polls_and_stores_nothing() {
    let vendor = MockVendor::start().await;
    vendor.respond(json_answer(
        200,
        json!({ "device_auth_id": "dev_auth_1", "usercode": "CODE-1", "interval": 5 }),
    ));
    vendor.respond(json_answer(403, json!({})));
    let pool = MemoryCredentialPool::new();
    let provider = staged(&vendor, &pool);
    let report = |_: LoginPending| {};
    // The login is dropped once its first poll went out; the next would
    // follow five seconds later.
    tokio::select! {
        _ = provider.accounts().unwrap().login(login_io(&report)) => panic!("the login ended without the user"),
        () = vendor.received(2) => {}
    }
    assert_eq!(vendor.requests().len(), 2);
    assert!(pool.list().await.unwrap().is_empty());
}
