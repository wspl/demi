//! The account's sign-in: its status, the refresh of its tokens before a
//! request or after a request refused with HTTP 401, one refresh at a time
//! (`crates/provider-codex/src/auth.rs`).

use demi_provider_codex::TransportMode;
use demi_provider_common::{
    CallEvent, CallOutcome, ErrorCode, ProviderEvent, VendorCall,
    credentials::{CredentialPool, MemoryCredentialPool},
    testing::{MockResponse, MockVendor, every_event, inference_request},
};
use demi_shared_types::TokenUsage;
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
    vendor.respond_at(
        RESPONSES,
        crate::stream(&[json!({ "type": "response.completed", "response": {
            "id": "resp_2", "usage": { "input_tokens": 1, "output_tokens": 1 } } })]),
    );
    let pool = pool_with(secret(&fresh_token(), "refresh-1", NOW)).await;
    let provider = provider(&vendor, &pool, TransportMode::Sse);
    let events = every_event(runtime_of(&provider).run(inference_request())).await;
    let usage = TokenUsage {
        input_tokens: 1,
        output_tokens: 1,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
    };
    // Each attempt is a call of the usage ledger's: the refused one with its
    // code, then the one sent again, under the response's id.
    assert_eq!(
        events,
        [
            VendorCall::refused(Some(ErrorCode::AuthExpired)),
            ProviderEvent::Call(CallEvent::Started),
            ProviderEvent::Call(CallEvent::Ended(VendorCall {
                vendor_id: Some("resp_2".into()),
                usage: Some(usage),
                cache_write_1h_tokens: 0,
                outcome: CallOutcome::Answered,
            })),
            ProviderEvent::Response(Some(usage)),
        ]
    );

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
