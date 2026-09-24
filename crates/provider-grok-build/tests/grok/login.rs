//! Grok Build's device login and the account it stores (`providers.md` §
//! Login and publication). The login waits at least a second between polls,
//! so these tests run on a paused clock; no other timer is pending while
//! they wait on the scripted issuer.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use demi_core::LoginPending;
use demi_provider::{
    Provider,
    credentials::{AccountsCapability, CredentialPool, LoginError, MemoryCredentialPool},
    testing::{MockResponse, MockVendor},
};
use serde_json::{Value, json};
use tokio::sync::Notify;

use crate::{form, json_answer, provider, token};

const DEVICE_CODE: &str = "/oauth2/device/code";
const TOKEN: &str = "/oauth2/token";
const USER: &str = "/v1/user";
const CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";

/// The issuer's device-code answer, with `fields` over one that polls at
/// once and lives ten minutes.
fn device_code(fields: Value) -> MockResponse {
    let mut answer = json!({
        "device_code": "dev_code_1",
        "user_code": "GROK-1234",
        "verification_uri": "https://auth.x.ai/activate",
        "interval": 0,
        "expires_in": 600,
    });
    for (key, value) in fields.as_object().unwrap() {
        answer[key] = value.clone();
    }
    json_answer(200, answer)
}

fn confirmed(access_token: &str, id_token: Option<String>) -> MockResponse {
    let mut answer =
        json!({ "access_token": access_token, "refresh_token": "rt_1", "expires_in": 3600 });
    if let Some(id_token) = id_token {
        answer["id_token"] = json!(id_token);
    }
    json_answer(200, answer)
}

fn pending() -> MockResponse {
    json_answer(400, json!({ "error": "authorization_pending" }))
}

fn paths(vendor: &MockVendor) -> Vec<String> {
    vendor
        .requests()
        .iter()
        .map(|request| request.uri.path().to_owned())
        .collect()
}

async fn stored(pool: &MemoryCredentialPool, id: &str) -> Value {
    serde_json::from_str(&pool.document(id).read().await.unwrap().unwrap().text).unwrap()
}

#[tokio::test(start_paused = true)]
async fn a_device_login_follows_the_grok_clis_contract_and_stores_the_user() {
    let vendor = MockVendor::start().await;
    vendor.respond_at(DEVICE_CODE, device_code(json!({ "verification_uri_complete": "https://auth.x.ai/activate?user_code=GROK-1234" })));
    vendor.respond_at(TOKEN, pending());
    let id_token = token(json!({ "sub": "user_1", "email": "id@example.com" }));
    vendor.respond_at(TOKEN, confirmed("at_1", Some(id_token)));
    vendor.respond_at(
        USER,
        json_answer(
            200,
            json!({ "userId": "user_1", "firstName": "G", "email": "g@example.com" }),
        ),
    );
    let pool = MemoryCredentialPool::new();
    let provider = provider(&vendor, &pool, None);
    let accounts = provider.accounts().unwrap();
    assert_eq!(
        accounts.capability(),
        AccountsCapability {
            login: true,
            add: false
        }
    );

    let shown = Arc::new(Mutex::new(Vec::new()));
    let report = {
        let shown = shown.clone();
        move |pending: LoginPending| shown.lock().unwrap().push(pending)
    };
    let account = accounts.login(&report).await.unwrap();
    let shown = shown.lock().unwrap().clone();
    assert_eq!(shown.len(), 1);
    assert_eq!(
        shown[0].verification_url,
        "https://auth.x.ai/activate?user_code=GROK-1234"
    );
    assert_eq!(shown[0].user_code.as_deref(), Some("GROK-1234"));
    assert_eq!(
        shown[0].expires_at,
        Some("2026-09-18T14:10:00.000Z".parse().unwrap())
    );

    // The proxy's details of the user win over the id token's.
    assert_eq!(account.label, "g@example.com");
    assert_eq!(
        accounts.active().await.unwrap().as_deref(),
        Some(account.id.as_str())
    );
    let issuer = vendor.url("");
    assert_eq!(
        stored(&pool, &account.id).await,
        json!({
            "accessToken": "at_1",
            "refreshToken": "rt_1",
            "expiresAt": "2026-09-18T15:00:00.000Z",
            "issuer": format!("{issuer}/"),
            "clientId": CLIENT_ID,
            "userId": "user_1",
            "email": "g@example.com",
        })
    );
    let listed = pool.list().await.unwrap();
    assert_eq!(listed[0].identity_key, Some(format!("{issuer}::user_1")));

    let requests = vendor.requests();
    assert_eq!(paths(&vendor), [DEVICE_CODE, TOKEN, TOKEN, USER]);
    let scope = "openid+profile+email+offline_access+grok-cli%3Aaccess+api%3Aaccess+conversations%3Aread+conversations%3Awrite+workspaces%3Aread+workspaces%3Awrite";
    assert_eq!(
        form(&requests[0]),
        format!("client_id={CLIENT_ID}&scope={scope}&referrer=grok-build")
    );
    assert_eq!(
        form(&requests[1]),
        format!(
            "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code&device_code=dev_code_1&client_id={CLIENT_ID}"
        )
    );
    for request in &requests[..3] {
        assert_eq!(
            (
                request.header("x-grok-client-surface"),
                request.header("x-grok-client-version")
            ),
            (Some("ui"), Some("1.0.5"))
        );
    }
    let user = &requests[3];
    assert_eq!(
        (
            user.header("authorization"),
            user.header("x-xai-token-auth")
        ),
        (Some("Bearer at_1"), Some("xai-grok-cli"))
    );
    assert_eq!(user.header("x-grok-client-mode"), Some("interactive"));
}

#[tokio::test(start_paused = true)]
async fn tokens_that_act_for_a_team_make_the_team_the_accounts_user_without_an_email() {
    let vendor = MockVendor::start().await;
    vendor.respond_at(DEVICE_CODE, device_code(json!({})));
    let access =
        token(json!({ "sub": "user-42", "principal_type": "Team", "principal_id": "team-123" }));
    vendor.respond_at(
        TOKEN,
        confirmed(
            &access,
            Some(token(
                json!({ "sub": "user-42", "email": "member@example.com" }),
            )),
        ),
    );
    vendor.respond_at(USER, json_answer(404, json!({})));
    let pool = MemoryCredentialPool::new();
    let provider = provider(&vendor, &pool, None);
    let report = |_: LoginPending| {};
    let account = provider.accounts().unwrap().login(&report).await.unwrap();
    assert_eq!(account.label, "team-123");
    let document = stored(&pool, &account.id).await;
    assert_eq!(
        (&document["userId"], &document["principal"]),
        (
            &json!("team-123"),
            &json!({ "kind": "Team", "id": "team-123" })
        )
    );
    assert!(document.get("email").is_none(), "{document}");
}

#[tokio::test(start_paused = true)]
async fn a_slow_down_waits_five_seconds_longer_before_each_later_poll() {
    let vendor = MockVendor::start().await;
    vendor.respond_at(DEVICE_CODE, device_code(json!({ "interval": 2 })));
    vendor.respond_at(TOKEN, json_answer(400, json!({ "error": "slow_down" })));
    vendor.respond_at(TOKEN, pending());
    vendor.respond_at(TOKEN, confirmed("at_1", None));
    let pool = MemoryCredentialPool::new();
    let provider = provider(&vendor, &pool, None);
    let report = |_: LoginPending| {};
    let started = tokio::time::Instant::now();
    provider.accounts().unwrap().login(&report).await.unwrap();
    // Two seconds, then seven, then seven again.
    let waited = started.elapsed();
    assert!(
        (Duration::from_secs(16)..Duration::from_millis(16_100)).contains(&waited),
        "{waited:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_login_the_user_never_confirms_ends_after_ten_minutes_whatever_the_code_allows() {
    let vendor = MockVendor::start().await;
    vendor.respond_at(
        DEVICE_CODE,
        device_code(json!({ "interval": 60, "expires_in": 1800 })),
    );
    for _ in 0..9 {
        vendor.respond_at(TOKEN, pending());
    }
    let pool = MemoryCredentialPool::new();
    let provider = provider(&vendor, &pool, None);
    let shown = Arc::new(Mutex::new(Vec::new()));
    let report = {
        let shown = shown.clone();
        move |pending: LoginPending| shown.lock().unwrap().push(pending.expires_at)
    };
    let started = tokio::time::Instant::now();
    let failure = provider
        .accounts()
        .unwrap()
        .login(&report)
        .await
        .unwrap_err();
    assert_eq!(
        failure,
        LoginError::Failed("Grok device login timed out before the user confirmed".into())
    );
    assert_eq!(started.elapsed(), Duration::from_secs(600));
    assert_eq!(
        *shown.lock().unwrap(),
        [Some("2026-09-18T14:10:00.000Z".parse().unwrap())]
    );
    // A poll a minute from the start to the ninth, none at the deadline.
    assert_eq!(vendor.requests().len(), 10);
    assert!(pool.list().await.unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_refused_login_or_an_address_a_browser_cannot_trust_fails_and_stores_nothing() {
    let vendor = MockVendor::start().await;
    vendor.respond_at(DEVICE_CODE, device_code(json!({})));
    vendor.respond_at(TOKEN, json_answer(400, json!({ "error": "access_denied" })));
    vendor.respond_at(
        DEVICE_CODE,
        device_code(json!({ "verification_uri": "http://auth.example.com/activate" })),
    );
    vendor.respond_at(
        DEVICE_CODE,
        device_code(json!({ "verification_uri": null })),
    );
    let pool = MemoryCredentialPool::new();
    let provider = provider(&vendor, &pool, None);
    let accounts = provider.accounts().unwrap();
    let report = |_: LoginPending| panic!("a failed login showed a code");
    let refused = |_: LoginPending| {};
    assert_eq!(
        accounts.login(&refused).await.unwrap_err(),
        LoginError::Failed("Grok device login failed: access_denied".into())
    );
    let insecure = accounts.login(&report).await.unwrap_err();
    assert_eq!(
        insecure,
        LoginError::Failed("Grok device code failed: the response is malformed at verification_uri: a field is missing, unknown or of the wrong type".into())
    );
    let unnamed = accounts.login(&report).await.unwrap_err();
    assert_eq!(
        unnamed,
        LoginError::Failed(
            "Grok device code failed: the response names no verification_uri".into()
        )
    );
    assert!(pool.list().await.unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn dropping_a_login_while_it_waits_for_the_user_stops_it_and_stores_nothing() {
    let vendor = MockVendor::start().await;
    vendor.respond_at(DEVICE_CODE, device_code(json!({ "interval": 5 })));
    vendor.respond_at(TOKEN, pending());
    let pool = MemoryCredentialPool::new();
    let provider = provider(&vendor, &pool, None);
    let shown = Arc::new(Notify::new());
    let report = {
        let shown = shown.clone();
        move |_: LoginPending| shown.notify_one()
    };
    tokio::select! {
        _ = provider.accounts().unwrap().login(&report) => panic!("the login ended without the user"),
        () = shown.notified() => {}
    }
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(paths(&vendor), [DEVICE_CODE]);
    assert!(pool.list().await.unwrap().is_empty());
}
