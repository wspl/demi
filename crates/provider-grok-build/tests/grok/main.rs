//! The Grok Build provider at its boundaries: the requests it sends to a
//! scripted chat proxy and issuer, the events it makes of recorded streams,
//! its token refresh, catalog, quota and device login. No test calls the real
//! vendor.

mod auth;
mod login;
mod models;
mod quota;
mod request;

use std::sync::Arc;

use demi_provider::{
    InferenceRequest, Provider, ProviderEvent, ProviderRuntime, RuntimeEnv,
    credentials::{AccountMeta, CredentialPool, MemoryCredentialPool},
    quota::MemorySnapshots,
    testing::{FixedClock, MockResponse, MockVendor, RecordedRequest, jwt},
};
use demi_provider_grok_build::{GrokConfig, GrokProvider};
use futures_util::StreamExt;
use serde_json::{Value, json};

pub(crate) const NOW: &str = "2026-09-18T14:00:00.000Z";
pub(crate) const ACCOUNT: &str = "cred-g";
pub(crate) const CHAT: &str = "/v1/chat/completions";

/// The account's secret document, with `fields` over a signed-in user whose
/// token expires in 2030.
pub(crate) fn secret(vendor: &MockVendor, fields: Value) -> String {
    let mut document = json!({
        "accessToken": "session-token",
        "refreshToken": "refresh-1",
        "expiresAt": "2030-01-01T00:00:00.000Z",
        "issuer": vendor.url(""),
        "clientId": "client-1",
        "userId": "user-1",
        "email": "user@example.com",
    });
    for (key, value) in fields.as_object().unwrap() {
        if value.is_null() {
            document.as_object_mut().unwrap().remove(key);
        } else {
            document[key] = value.clone();
        }
    }
    document.to_string()
}

pub(crate) async fn pool_with(secret: String) -> MemoryCredentialPool {
    let pool = MemoryCredentialPool::new();
    let meta = AccountMeta {
        id: ACCOUNT.into(),
        label: "user@example.com".into(),
        detail: Some("oidc".into()),
        updated_at: NOW.parse().unwrap(),
        source: "test".into(),
        identity_key: Some("user-1".into()),
    };
    pool.write(meta, secret).await.unwrap();
    pool.set_active(ACCOUNT).await.unwrap();
    pool
}

/// A provider whose chat proxy and issuer are the scripted vendor.
pub(crate) fn provider(
    vendor: &MockVendor,
    pool: &MemoryCredentialPool,
    account: Option<&str>,
) -> GrokProvider {
    let mut config = GrokConfig::new("grok-build", "Grok Build", account.map(str::to_owned));
    config.proxy_url = vendor.url("/v1").parse().unwrap();
    config.issuer_url = vendor.url("").parse().unwrap();
    let clock = Arc::new(FixedClock(NOW.parse().unwrap()));
    // No idle-connection timer, so a paused clock only moves at the login's
    // own waits.
    let http = reqwest::Client::builder()
        .pool_idle_timeout(None)
        .build()
        .unwrap();
    GrokProvider::new(
        config,
        Arc::new(pool.clone()),
        Arc::new(MemorySnapshots::new()),
        http,
        clock,
    )
}

pub(crate) fn runtime_of(provider: &GrokProvider) -> Box<dyn ProviderRuntime> {
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

/// A chat stream of `chunks`, closed by `[DONE]`.
pub(crate) fn chat(chunks: &[Value]) -> MockResponse {
    let mut body: String = chunks
        .iter()
        .map(|chunk| format!("data: {chunk}\n\n"))
        .collect();
    body.push_str("data: [DONE]\n\n");
    MockResponse::event_stream(body)
}

pub(crate) fn json_answer(status: u16, body: Value) -> MockResponse {
    MockResponse::status(status)
        .header("content-type", "application/json")
        .chunk(body.to_string())
}

/// The form a request sent, as its encoded text.
pub(crate) fn form(request: &RecordedRequest) -> String {
    String::from_utf8(request.body.to_vec()).unwrap()
}

/// A token whose claims are `claims`.
pub(crate) fn token(claims: Value) -> String {
    jwt(&claims)
}
