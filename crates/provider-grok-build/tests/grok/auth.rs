//! The account's sign-in: its status, and the OIDC refresh of its tokens
//! before a request, after a refusal with HTTP 401, and one at a time
//! (`crates/provider-grok-build/src/auth.rs`).

use demi_provider_common::{
    CallEvent, CallOutcome, ErrorCode, ProviderEvent, VendorCall,
    credentials::{CredentialPool, MemoryCredentialPool},
    testing::{MockResponse, MockVendor, every_event, inference_request},
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
        chat(&[json!({ "id": "chatcmpl-2", "choices": [{ "delta": { "content": "ok" } }] })]),
    );
    let pool = pool_with(secret(
        &vendor,
        json!({ "principal": { "kind": "User", "id": "user-1" } }),
    ))
    .await;
    let provider = provider(&vendor, &pool, Some(ACCOUNT));
    let events = every_event(runtime_of(&provider).run(inference_request())).await;
    // Each attempt is a call of the usage ledger's: the refused one with its
    // code, then the one sent again, under the completion's id.
    assert_eq!(
        events,
        [
            VendorCall::refused(Some(ErrorCode::AuthExpired)),
            ProviderEvent::Call(CallEvent::Started),
            ProviderEvent::TextDelta("ok".into()),
            ProviderEvent::Call(CallEvent::Ended(VendorCall {
                vendor_id: Some("chatcmpl-2".into()),
                usage: None,
                cache_write_1h_tokens: 0,
                outcome: CallOutcome::Answered,
            })),
            ProviderEvent::Response(None)
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
