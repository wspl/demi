//! The account's sign-in: its status, the refresh of its tokens before a
//! request or after a request refused with HTTP 401, one refresh at a time
//! (`providers.md` § Token refresh).

use demi_core::{AuthState, TokenUsage};
use demi_provider::{
    ErrorCode, Provider, ProviderEvent,
    credentials::{AccountMeta, CredentialPool, MemoryCredentialPool},
    testing::{MockResponse, MockVendor, inference_request},
};
use demi_provider_codex::TransportMode;
use serde_json::{Value, json};

use crate::{
    NOW, NOW_SECONDS, RESPONSES, access_token, completed, fresh_token, pool_with, provider, run,
    runtime_of, secret,
};

/// The token endpoint's answer with `access`.
fn refreshed(access: &str, refresh: &str) -> MockResponse {
    let body = json!({ "access_token": access, "refresh_token": refresh, "id_token": crate::secret_id_token() });
    MockResponse::status(200)
        .header("content-type", "application/json")
        .chunk(body.to_string())
}

async fn stored(pool: &MemoryCredentialPool) -> Value {
    serde_json::from_str(
        &pool
            .document(crate::ACCOUNT)
            .read()
            .await
            .unwrap()
            .unwrap()
            .text,
    )
    .unwrap()
}

#[tokio::test]
async fn a_request_refused_with_401_refreshes_once_and_is_sent_again() {
    let vendor = MockVendor::start().await;
    vendor.respond_at(
        RESPONSES,
        MockResponse::status(401).chunk(r#"{"error":{"message":"expired"}}"#),
    );
    vendor.respond_at("/oauth/token", refreshed("new-access", "refresh-2"));
    vendor.respond_at(RESPONSES, completed());
    let pool = pool_with(secret(&fresh_token(), "refresh-1", NOW)).await;
    let provider = provider(&vendor, &pool, TransportMode::Sse);
    let events = run(runtime_of(&provider).as_mut(), inference_request()).await;
    let usage = TokenUsage {
        input_tokens: 1,
        output_tokens: 1,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
    };
    assert_eq!(events, [ProviderEvent::Response(usage)]);

    let requests = vendor.requests();
    let paths: Vec<&str> = requests.iter().map(|request| request.uri.path()).collect();
    assert_eq!(paths, [RESPONSES, "/oauth/token", RESPONSES]);
    assert_eq!(
        requests[0].header("authorization"),
        Some(format!("Bearer {}", fresh_token()).as_str())
    );
    assert_eq!(
        requests[2].header("authorization"),
        Some("Bearer new-access")
    );
    assert_eq!(
        requests[1].json(),
        json!({ "client_id": "app_EMoamEEZ73f0CkXaXp7hrann", "grant_type": "refresh_token", "refresh_token": "refresh-1" })
    );
    // The refreshed sign-in is stored, still for its account.
    let document = stored(&pool).await;
    assert_eq!(
        (&document["accessToken"], &document["refreshToken"]),
        (&json!("new-access"), &json!("refresh-2"))
    );
    assert_eq!(
        (&document["accountId"], &document["lastRefresh"]),
        (&json!("acct-1"), &json!(NOW))
    );
}

#[tokio::test]
async fn a_refused_token_that_another_refresher_replaced_meanwhile_is_not_refreshed_again() {
    let vendor = MockVendor::start().await;
    vendor.respond_at(RESPONSES, MockResponse::status(401));
    vendor.respond_at(RESPONSES, completed());
    let pool = pool_with(secret(&fresh_token(), "refresh-1", NOW)).await;
    let provider = provider(&vendor, &pool, TransportMode::Sse);
    let mut runtime = runtime_of(&provider);
    // Another refresher holds the account's turn and stores new tokens once
    // the request with the old ones has gone out.
    let doc = pool.document(crate::ACCOUNT);
    let turn = doc.refresh_turn().await;
    let other = async {
        vendor.received(1).await;
        let version = doc.read().await.unwrap().unwrap().version;
        assert!(
            doc.replace(secret("rotated-access", "refresh-2", NOW), version)
                .await
                .unwrap()
        );
        drop(turn);
    };
    let (events, ()) = tokio::join!(run(runtime.as_mut(), inference_request()), other);
    assert!(
        matches!(events.as_slice(), [ProviderEvent::Response(_)]),
        "{events:?}"
    );
    let requests = vendor.requests();
    let paths: Vec<&str> = requests.iter().map(|request| request.uri.path()).collect();
    assert_eq!(paths, [RESPONSES, RESPONSES]);
    assert_eq!(
        requests[1].header("authorization"),
        Some("Bearer rotated-access")
    );
}

#[tokio::test]
async fn a_second_refusal_after_the_refresh_fails_the_run() {
    let vendor = MockVendor::start().await;
    vendor.respond_at(RESPONSES, MockResponse::status(401));
    vendor.respond_at("/oauth/token", refreshed("new-access", "refresh-2"));
    vendor.respond_at(RESPONSES, MockResponse::status(401).chunk("still expired"));
    let pool = pool_with(secret(&fresh_token(), "refresh-1", NOW)).await;
    let events = run(
        runtime_of(&provider(&vendor, &pool, TransportMode::Sse)).as_mut(),
        inference_request(),
    )
    .await;
    let [ProviderEvent::Error(failure)] = events.as_slice() else {
        panic!("{events:?}");
    };
    assert_eq!(failure.code, Some(ErrorCode::AuthExpired));
    assert_eq!(
        failure.message,
        "Codex API request failed with HTTP 401: still expired"
    );
    assert_eq!(vendor.requests().len(), 3);
}

#[tokio::test]
async fn a_sign_in_expiring_soon_or_long_unrefreshed_is_refreshed_before_the_request() {
    let expiring = secret(&access_token(NOW_SECONDS + 60), "refresh-1", NOW);
    let stale = secret(&fresh_token(), "refresh-1", "2026-09-10T14:00:00.000Z");
    for document in [expiring, stale] {
        let vendor = MockVendor::start().await;
        vendor.respond_at("/oauth/token", refreshed("new-access", "refresh-2"));
        vendor.respond_at(RESPONSES, completed());
        let pool = pool_with(document).await;
        run(
            runtime_of(&provider(&vendor, &pool, TransportMode::Sse)).as_mut(),
            inference_request(),
        )
        .await;
        let requests = vendor.requests();
        assert_eq!(requests[0].uri.path(), "/oauth/token");
        assert_eq!(
            requests[1].header("authorization"),
            Some("Bearer new-access")
        );
    }
    // A sign-in refreshed seven days ago with a token good for an hour is used
    // as it is.
    let vendor = MockVendor::start().await;
    vendor.respond_at(RESPONSES, completed());
    let pool = pool_with(secret(
        &fresh_token(),
        "refresh-1",
        "2026-09-11T14:00:01.000Z",
    ))
    .await;
    run(
        runtime_of(&provider(&vendor, &pool, TransportMode::Sse)).as_mut(),
        inference_request(),
    )
    .await;
    assert_eq!(vendor.requests().len(), 1);
}

#[tokio::test]
async fn concurrent_requests_refresh_one_account_once() {
    let vendor = MockVendor::start().await;
    vendor.respond_at("/oauth/token", refreshed("new-access", "refresh-2"));
    vendor.respond_at(RESPONSES, completed());
    vendor.respond_at(RESPONSES, completed());
    let pool = pool_with(secret(&access_token(NOW_SECONDS + 60), "refresh-1", NOW)).await;
    let provider = provider(&vendor, &pool, TransportMode::Sse);
    let (mut first, mut second) = (runtime_of(&provider), runtime_of(&provider));
    let (one, two) = tokio::join!(
        run(first.as_mut(), inference_request()),
        run(second.as_mut(), inference_request())
    );
    assert!(
        matches!(one.last(), Some(ProviderEvent::Response(_)))
            && matches!(two.last(), Some(ProviderEvent::Response(_)))
    );
    let requests = vendor.requests();
    let refreshes = requests
        .iter()
        .filter(|request| request.uri.path() == "/oauth/token")
        .count();
    assert_eq!(refreshes, 1);
    for request in requests
        .iter()
        .filter(|request| request.uri.path() == RESPONSES)
    {
        assert_eq!(request.header("authorization"), Some("Bearer new-access"));
    }
}

#[tokio::test]
async fn a_refused_refresh_fails_the_run_as_a_failed_refresh() {
    let vendor = MockVendor::start().await;
    vendor.respond_at(
        "/oauth/token",
        MockResponse::status(400).chunk(r#"{"error":"invalid_grant","refresh_token":"refresh-1"}"#),
    );
    let pool = pool_with(secret(&access_token(NOW_SECONDS + 60), "refresh-1", NOW)).await;
    let events = run(
        runtime_of(&provider(&vendor, &pool, TransportMode::Sse)).as_mut(),
        inference_request(),
    )
    .await;
    let [ProviderEvent::Error(failure)] = events.as_slice() else {
        panic!("{events:?}");
    };
    assert_eq!(failure.code, Some(ErrorCode::AuthRefreshFailed));
    assert_eq!(failure.message, "Codex token refresh failed with HTTP 400");
}

#[tokio::test]
async fn the_status_names_the_account_and_an_unreadable_or_absent_one_is_reported() {
    let vendor = MockVendor::start().await;
    let pool = pool_with(secret(&fresh_token(), "refresh-1", NOW)).await;
    let signed_in = provider(&vendor, &pool, TransportMode::Sse);
    assert_eq!(
        signed_in.auth_status().await,
        AuthState::Authenticated {
            account_label: Some("dev@example.com".into())
        }
    );
    // The provider stands for its account, whichever one is active.
    let other = AccountMeta {
        id: "cred-b".into(),
        label: "other".into(),
        detail: None,
        updated_at: NOW.parse().unwrap(),
        source: "test".into(),
        identity_key: None,
    };
    pool.write(other, "{}".into()).await.unwrap();
    pool.set_active("cred-b").await.unwrap();
    assert_eq!(
        signed_in.auth_status().await,
        AuthState::Authenticated {
            account_label: Some("dev@example.com".into())
        }
    );

    // A corrupt document is refused, never repaired, and its tokens never
    // reach the message.
    let corrupt =
        pool_with(json!({ "accessToken": "sk-secret-token", "refreshToken": 7 }).to_string()).await;
    let provider = provider(&vendor, &corrupt, TransportMode::Sse);
    let AuthState::Error { message } = provider.auth_status().await else {
        panic!("a corrupt document reads as usable");
    };
    assert!(
        message.contains("refreshToken") && !message.contains("sk-secret-token"),
        "{message}"
    );
    let events = run(runtime_of(&provider).as_mut(), inference_request()).await;
    assert!(
        matches!(events.as_slice(), [ProviderEvent::Error(failure)] if failure.code == Some(ErrorCode::AuthInvalid))
    );

    // A provider built to log in has no account yet.
    let mut config = demi_provider_codex::CodexConfig::new("codex", "Codex", None);
    config.backend_url = vendor.url("/backend-api").parse().unwrap();
    let clock = std::sync::Arc::new(demi_provider::testing::FixedClock(NOW.parse().unwrap()));
    let snapshots = std::sync::Arc::new(demi_provider::quota::MemorySnapshots::new());
    let staged = demi_provider_codex::CodexProvider::new(
        config,
        std::sync::Arc::new(MemoryCredentialPool::new()),
        snapshots,
        reqwest::Client::new(),
        clock,
    );
    assert_eq!(
        staged.auth_status().await,
        AuthState::Unauthenticated {
            message: Some("No Codex account is signed in".into())
        }
    );
    let events = run(runtime_of(&staged).as_mut(), inference_request()).await;
    assert!(
        matches!(events.as_slice(), [ProviderEvent::Error(failure)] if failure.code == Some(ErrorCode::AuthMissing))
    );
    assert!(vendor.requests().is_empty());
}
