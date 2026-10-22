//! Grok Build's device login and the account it stores
//! (`crates/provider-grok-build/src/login.rs`).
//!
//! The login waits at least a second between polls, so these tests run on a
//! paused clock; no other timer is pending while they wait on the scripted
//! issuer.

use std::time::Duration;

use demi_provider_common::{
    Provider,
    credentials::MemoryCredentialPool,
    testing::{MockResponse, MockVendor, login_io},
};
use demi_shared_types::LoginPending;
use serde_json::{Value, json};

use crate::{json_answer, provider};

const DEVICE_CODE: &str = "/oauth2/device/code";
const TOKEN: &str = "/oauth2/token";
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
    provider.accounts().unwrap().login(login_io(&report)).await.unwrap();
    // Two seconds, then seven, then seven again.
    let waited = started.elapsed();
    assert!(
        (Duration::from_secs(16)..Duration::from_millis(16_100)).contains(&waited),
        "{waited:?}"
    );
}
