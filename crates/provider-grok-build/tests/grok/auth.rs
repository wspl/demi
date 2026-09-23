//! The account's sign-in: its status, and the OIDC refresh of its tokens
//! before a request, after a refusal with HTTP 401, and one at a time
//! (`providers.md` § Token refresh).

use demi_core::{AuthState, TokenUsage};
use demi_provider::{
    ErrorCode, Provider, ProviderEvent,
    credentials::{AccountMeta, CredentialPool, MemoryCredentialPool},
    testing::{MockResponse, MockVendor, inference_request},
};
use serde_json::{Value, json};

use crate::{ACCOUNT, CHAT, chat, form, json_answer, pool_with, provider, run, runtime_of, secret};

async fn stored(pool: &MemoryCredentialPool) -> Value {
    serde_json::from_str(&pool.document(ACCOUNT).read().await.unwrap().unwrap().text).unwrap()
}

#[tokio::test]
async fn a_request_refused_with_401_refreshes_once_for_its_principal_and_is_sent_again() {
    let vendor = MockVendor::start().await;
    vendor.respond_at(CHAT, MockResponse::status(401).chunk("unauthorized"));
    vendor.respond_at("/oauth2/token", json_answer(200, json!({ "access_token": "refreshed-token", "refresh_token": "refresh-2", "expires_in": 3600, "token_type": "Bearer" })));
    vendor.respond_at(
        CHAT,
        chat(&[json!({ "choices": [{ "delta": { "content": "ok" } }] })]),
    );
    let pool = pool_with(secret(
        &vendor,
        json!({ "principal": { "kind": "User", "id": "user-1" } }),
    ))
    .await;
    let provider = provider(&vendor, &pool, Some(ACCOUNT));
    let events = run(runtime_of(&provider).as_mut(), inference_request()).await;
    assert_eq!(
        events,
        [
            ProviderEvent::TextDelta("ok".into()),
            ProviderEvent::Response(TokenUsage::default())
        ]
    );

    let requests = vendor.requests();
    let paths: Vec<&str> = requests.iter().map(|request| request.uri.path()).collect();
    assert_eq!(paths, [CHAT, "/oauth2/token", CHAT]);
    let refresh = form(&requests[1]);
    for field in [
        "grant_type=refresh_token",
        "refresh_token=refresh-1",
        "client_id=client-1",
        "principal_type=User",
        "principal_id=user-1",
    ] {
        assert!(refresh.contains(field), "{refresh}");
    }
    assert_eq!(
        requests[2].header("authorization"),
        Some("Bearer refreshed-token")
    );
    let document = stored(&pool).await;
    assert_eq!(
        (&document["accessToken"], &document["refreshToken"]),
        (&json!("refreshed-token"), &json!("refresh-2"))
    );
    assert_eq!(document["expiresAt"], json!("2026-09-18T15:00:00.000Z"));
    assert_eq!(document["email"], json!("user@example.com"));
}

#[tokio::test]
async fn a_refused_token_that_another_refresher_replaced_meanwhile_is_not_refreshed_again() {
    let vendor = MockVendor::start().await;
    vendor.respond_at(CHAT, MockResponse::status(401));
    vendor.respond_at(CHAT, chat(&[]));
    let pool = pool_with(secret(&vendor, json!({}))).await;
    let provider = provider(&vendor, &pool, Some(ACCOUNT));
    let mut runtime = runtime_of(&provider);
    // Another refresher holds the account's turn and stores new tokens once
    // the request with the old ones has gone out.
    let doc = pool.document(ACCOUNT);
    let turn = doc.refresh_turn().await;
    let other = async {
        vendor.received(1).await;
        let version = doc.read().await.unwrap().unwrap().version;
        let rotated = secret(
            &vendor,
            json!({ "accessToken": "rotated-token", "refreshToken": "refresh-2" }),
        );
        assert!(doc.replace(rotated, version).await.unwrap());
        drop(turn);
    };
    let (events, ()) = tokio::join!(run(runtime.as_mut(), inference_request()), other);
    assert_eq!(events, [ProviderEvent::Response(TokenUsage::default())]);
    let requests = vendor.requests();
    let paths: Vec<&str> = requests.iter().map(|request| request.uri.path()).collect();
    assert_eq!(paths, [CHAT, CHAT]);
    assert_eq!(
        requests[1].header("authorization"),
        Some("Bearer rotated-token")
    );
}

#[tokio::test]
async fn a_token_expiring_soon_is_refreshed_first_and_one_without_a_refresh_token_is_used_as_it_is()
{
    let vendor = MockVendor::start().await;
    vendor.respond_at(
        "/oauth2/token",
        json_answer(
            200,
            json!({ "access_token": "fresh", "expires_in": "3600" }),
        ),
    );
    vendor.respond_at(CHAT, chat(&[]));
    let pool = pool_with(secret(
        &vendor,
        json!({ "expiresAt": "2026-09-18T14:01:00.000Z" }),
    ))
    .await;
    run(
        runtime_of(&provider(&vendor, &pool, Some(ACCOUNT))).as_mut(),
        inference_request(),
    )
    .await;
    let requests = vendor.requests();
    assert_eq!(
        (requests[0].uri.path(), requests[1].header("authorization")),
        ("/oauth2/token", Some("Bearer fresh"))
    );
    // The refresh token the answer did not replace stays.
    assert_eq!(stored(&pool).await["refreshToken"], json!("refresh-1"));

    let vendor = MockVendor::start().await;
    vendor.respond_at(CHAT, chat(&[]));
    let pool = pool_with(secret(
        &vendor,
        json!({ "expiresAt": "2026-09-18T14:01:00.000Z", "refreshToken": null }),
    ))
    .await;
    run(
        runtime_of(&provider(&vendor, &pool, Some(ACCOUNT))).as_mut(),
        inference_request(),
    )
    .await;
    assert_eq!(vendor.requests().len(), 1);
}

#[tokio::test]
async fn concurrent_requests_refresh_one_account_once_and_share_the_new_token() {
    let vendor = MockVendor::start().await;
    vendor.respond_at(
        "/oauth2/token",
        json_answer(200, json!({ "access_token": "fresh", "expires_in": 3600 })),
    );
    vendor.respond_at(CHAT, chat(&[]));
    vendor.respond_at(CHAT, chat(&[]));
    let pool = pool_with(secret(
        &vendor,
        json!({ "expiresAt": "2026-09-18T14:01:00.000Z" }),
    ))
    .await;
    let provider = provider(&vendor, &pool, Some(ACCOUNT));
    let (mut first, mut second) = (runtime_of(&provider), runtime_of(&provider));
    tokio::join!(
        run(first.as_mut(), inference_request()),
        run(second.as_mut(), inference_request())
    );
    let requests = vendor.requests();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.uri.path() == "/oauth2/token")
            .count(),
        1
    );
    for request in requests.iter().filter(|request| request.uri.path() == CHAT) {
        assert_eq!(request.header("authorization"), Some("Bearer fresh"));
    }
}

#[tokio::test]
async fn a_refused_refresh_fails_the_run_as_a_failed_refresh() {
    let vendor = MockVendor::start().await;
    vendor.respond_at(
        "/oauth2/token",
        json_answer(400, json!({ "error": "invalid_grant" })),
    );
    let pool = pool_with(secret(
        &vendor,
        json!({ "expiresAt": "2026-09-18T14:01:00.000Z" }),
    ))
    .await;
    let events = run(
        runtime_of(&provider(&vendor, &pool, Some(ACCOUNT))).as_mut(),
        inference_request(),
    )
    .await;
    let [ProviderEvent::Error(failure)] = events.as_slice() else {
        panic!("{events:?}");
    };
    assert_eq!(
        (failure.message.as_str(), failure.code.clone()),
        (
            "Grok token refresh failed with HTTP 400",
            Some(ErrorCode::AuthRefreshFailed)
        )
    );

    // An answer without an access token is a failed refresh too.
    let vendor = MockVendor::start().await;
    vendor.respond_at(
        "/oauth2/token",
        json_answer(200, json!({ "token_type": "Bearer", "expires_in": 3600 })),
    );
    let pool = pool_with(secret(
        &vendor,
        json!({ "expiresAt": "2026-09-18T14:01:00.000Z" }),
    ))
    .await;
    let events = run(
        runtime_of(&provider(&vendor, &pool, Some(ACCOUNT))).as_mut(),
        inference_request(),
    )
    .await;
    let [ProviderEvent::Error(failure)] = events.as_slice() else {
        panic!("{events:?}");
    };
    assert!(
        failure
            .message
            .starts_with("Grok token refresh failed: the response is malformed"),
        "{}",
        failure.message
    );
    assert_eq!(failure.code, Some(ErrorCode::AuthRefreshFailed));
    assert!(
        vendor
            .requests()
            .iter()
            .all(|request| request.uri.path() != CHAT)
    );
}

#[tokio::test]
async fn the_status_names_the_account_and_an_unreadable_or_absent_one_is_reported() {
    let vendor = MockVendor::start().await;
    let pool = pool_with(secret(&vendor, json!({}))).await;
    let signed_in = provider(&vendor, &pool, Some(ACCOUNT));
    let expected = AuthState::Authenticated {
        account_label: Some("user@example.com".into()),
    };
    assert_eq!(signed_in.auth_status().await, expected);
    // The provider stands for its account, whichever one is active.
    let other = AccountMeta {
        id: "cred-b".into(),
        label: "other".into(),
        detail: None,
        updated_at: crate::NOW.parse().unwrap(),
        source: "test".into(),
        identity_key: None,
    };
    pool.write(other, "{}".into()).await.unwrap();
    pool.set_active("cred-b").await.unwrap();
    assert_eq!(signed_in.auth_status().await, expected);
    // A field of the wrong type is corrupt material, refused without its
    // value.
    let corrupt = pool_with(secret(
        &vendor,
        json!({ "refreshToken": 7, "accessToken": "sk-secret-token" }),
    ))
    .await;
    let provider_of_corrupt = provider(&vendor, &corrupt, Some(ACCOUNT));
    let AuthState::Error { message } = provider_of_corrupt.auth_status().await else {
        panic!("a corrupt document reads as usable");
    };
    assert!(
        message.contains("refreshToken") && !message.contains("sk-secret-token"),
        "{message}"
    );
    let events = run(
        runtime_of(&provider_of_corrupt).as_mut(),
        inference_request(),
    )
    .await;
    assert!(
        matches!(events.as_slice(), [ProviderEvent::Error(failure)] if failure.code == Some(ErrorCode::AuthInvalid))
    );
    let staged = provider(&vendor, &MemoryCredentialPool::new(), None);
    assert_eq!(
        staged.auth_status().await,
        AuthState::Unauthenticated {
            message: Some("No Grok account is signed in".into())
        }
    );
    let events = run(runtime_of(&staged).as_mut(), inference_request()).await;
    assert!(
        matches!(events.as_slice(), [ProviderEvent::Error(failure)] if failure.code == Some(ErrorCode::AuthMissing))
    );
    assert!(vendor.requests().is_empty());
}
