//! The Codex provider at its boundaries: the requests it sends to a scripted
//! backend and sign-in service, over server-sent events and a scripted
//! WebSocket, the events it makes of recorded streams, its token refresh,
//! failures, quota, catalog and device login. No test calls the real vendor.

mod auth;
mod fake_websocket;
mod login;
mod models;
mod quota;
mod request;
mod stream;
mod websocket;

use std::{sync::Arc, time::Duration};

use demi_provider::{
    InferenceRequest, Provider, ProviderEvent, ProviderRuntime, RuntimeEnv,
    credentials::{AccountMeta, CredentialPool, MemoryCredentialPool},
    quota::MemorySnapshots,
    testing::{FixedClock, MockResponse, MockVendor, jwt, sse_body},
};
use demi_provider_codex::{CodexConfig, CodexProvider, TransportMode};
use futures_util::StreamExt;
use serde_json::{Value, json};

/// When the scripted vendor answers.
pub(crate) const NOW: &str = "2026-09-18T14:00:00.000Z";
/// `NOW` in Unix seconds.
pub(crate) const NOW_SECONDS: i64 = 1_789_740_000;

pub(crate) const ACCOUNT: &str = "cred-a";

/// An access token of `acct-1` that expires at `exp`, in Unix seconds.
pub(crate) fn access_token(exp: i64) -> String {
    jwt(&json!({ "exp": exp, "https://api.openai.com/auth": { "chatgpt_account_id": "acct-1" } }))
}

/// An access token that expires in an hour.
pub(crate) fn fresh_token() -> String {
    access_token(NOW_SECONDS + 3_600)
}

/// The id token of `dev@example.com`, account `acct-1`.
pub(crate) fn secret_id_token() -> String {
    jwt(
        &json!({ "email": "dev@example.com", "https://api.openai.com/auth": { "chatgpt_account_id": "acct-1" } }),
    )
}

/// The account's secret document, signed in as `dev@example.com`.
pub(crate) fn secret(access: &str, refresh: &str, last_refresh: &str) -> String {
    json!({
        "accessToken": access,
        "refreshToken": refresh,
        "idToken": secret_id_token(),
        "accountId": "acct-1",
        "lastRefresh": last_refresh,
    })
    .to_string()
}

/// A pool holding the account with `secret`.
pub(crate) async fn pool_with(secret: String) -> MemoryCredentialPool {
    let pool = MemoryCredentialPool::new();
    let meta = AccountMeta {
        id: ACCOUNT.into(),
        label: "dev@example.com".into(),
        detail: Some("chatgpt".into()),
        updated_at: NOW.parse().unwrap(),
        source: "test".into(),
        identity_key: Some("acct-1".into()),
    };
    pool.write(meta, secret).await.unwrap();
    pool.set_active(ACCOUNT).await.unwrap();
    pool
}

/// A provider whose backend and sign-in service are the scripted vendor.
pub(crate) fn provider(
    vendor: &MockVendor,
    pool: &MemoryCredentialPool,
    transport: TransportMode,
) -> CodexProvider {
    let mut config = CodexConfig::new("codex", "Codex", Some(ACCOUNT.into()));
    config.backend_url = vendor.url("/backend-api").parse().unwrap();
    config.auth_url = vendor.url("").parse().unwrap();
    config.transport = transport;
    config.header_timeout = Duration::from_secs(5);
    config.connect_timeout = Duration::from_secs(5);
    let clock = Arc::new(FixedClock(NOW.parse().unwrap()));
    CodexProvider::new(
        config,
        Arc::new(pool.clone()),
        Arc::new(MemorySnapshots::new()),
        reqwest::Client::new(),
        clock,
    )
}

pub(crate) fn runtime_of(provider: &CodexProvider) -> Box<dyn ProviderRuntime> {
    provider
        .runtime(RuntimeEnv {
            http: reqwest::Client::new(),
        })
        .unwrap()
}

pub(crate) async fn run(
    runtime: &mut dyn ProviderRuntime,
    request: InferenceRequest,
) -> Vec<ProviderEvent> {
    runtime.run(request).collect().await
}

/// A Responses stream of `events`.
pub(crate) fn stream(events: &[Value]) -> MockResponse {
    MockResponse::event_stream(sse_body(events))
}

/// A stream that completes at once.
pub(crate) fn completed() -> MockResponse {
    stream(&[
        json!({ "type": "response.completed", "response": { "usage": { "input_tokens": 1, "output_tokens": 1 } } }),
    ])
}

/// The Codex responses path under the scripted backend.
pub(crate) const RESPONSES: &str = "/backend-api/codex/responses";
