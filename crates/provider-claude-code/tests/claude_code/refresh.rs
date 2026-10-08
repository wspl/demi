//! The account's tokens, which only the backend refreshes
//! (`claude-code.md` § Accounts and sign-in, `providers.md` § Token
//! refresh): a process starts with a token refreshed first when it expires
//! within five minutes; the CLI's `oauth_token_refresh` after a 401 is
//! answered with the account's token, refreshed when the process's token is
//! still the stored one, so processes that ask together spend the
//! single-use refresh token once; a refusal answers no token with its
//! reason; and the usage probe uses the access token, refreshed first when
//! due.

use demi_provider_common::credentials::{CredentialPool, MemoryCredentialPool};
use demi_provider_common::Provider;
use demi_provider_common::testing::{MockResponse, MockVendor};
use serde_json::{Value, json};

use crate::cli::*;

/// An account of `pool` whose access token expires within five minutes of
/// the clock.
async fn expiring(pool: &MemoryCredentialPool) -> String {
    seed(pool, sign_in(TOKEN, REFRESH, "2026-09-24T08:04:00Z")).await
}

/// The token endpoint's answer: a new access token valid for an hour, and a
/// new refresh token.
fn refreshed(token: &str) -> MockResponse {
    MockResponse::status(200)
        .header("content-type", "application/json")
        .chunk(
            json!({
                "access_token": token,
                "refresh_token": "sk-ant-ort01-next-refresh",
                "expires_in": 3600,
                "scope": "user:inference user:profile",
            })
            .to_string(),
        )
}

/// The refresh grant the vendor received, as the CLI sends it.
fn grant() -> Value {
    json!({
        "grant_type": "refresh_token",
        "refresh_token": REFRESH,
        "client_id": "9d1c250a-e61b-44d9-88ed-5944d1962f5e",
        "scope": "user:inference user:profile",
    })
}

/// The stored sign-in of `account`.
async fn stored(pool: &MemoryCredentialPool, account: &str) -> Value {
    let document = pool.document(account).read().await.unwrap().unwrap();
    serde_json::from_str(&document.text).unwrap()
}

#[tokio::test(flavor = "local")]
async fn a_process_starts_with_a_token_refreshed_first_when_it_expires_within_five_minutes() {
    let vendor = MockVendor::start().await;
    vendor.respond_at("/v1/oauth/token", refreshed("sk-ant-oat01-fresh"));
    let pool = MemoryCredentialPool::new();
    let account = expiring(&pool).await;
    let token = vendor.url("/v1/oauth/token");
    let urls = Urls {
        token: &token,
        ..NOWHERE
    };
    let provider = provider_of(&pool, &account, &urls);
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let (_, cli) = tokio::join!(
        all_events(runtime.run(request_without_tools(vec![user("hi")]))),
        async {
            let mut cli = starts.next().await;
            cli.read().await;
            cli.result(1, 1);
            cli
        }
    );
    assert_eq!(
        cli.spawn.descriptors[0].bytes.as_ref(),
        b"sk-ant-oat01-fresh"
    );
    let requests = vendor.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].json(), grant());
    let signed_in = stored(&pool, &account).await;
    assert_eq!(signed_in["accessToken"], "sk-ant-oat01-fresh");
    assert_eq!(signed_in["refreshToken"], "sk-ant-ort01-next-refresh");
    assert_eq!(signed_in["expiresAt"], "2026-09-24T09:00:00.000Z");
    assert_eq!(signed_in["accountId"], "account-1");
}

#[tokio::test(flavor = "local")]
async fn processes_that_ask_for_a_fresh_token_together_get_one_refresh_and_the_same_token() {
    let vendor = MockVendor::start().await;
    vendor.respond_at("/v1/oauth/token", refreshed("sk-ant-oat01-fresh"));
    let token = vendor.url("/v1/oauth/token");
    let urls = Urls {
        token: &token,
        ..NOWHERE
    };
    let (provider, pool) = provider_with(TOKEN, &urls).await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut first = runtime_of(&provider, &placement);
    let mut second = runtime_of(&provider, &placement);
    // Each process's CLI was refused with the token both hold and asks its
    // host for a fresh one; neither is answered before both asked.
    let asks = async {
        let mut clis = [starts.next().await, starts.next().await];
        for cli in &mut clis {
            cli.read().await;
        }
        for (index, cli) in clis.iter().enumerate() {
            cli.say(json!({
                "type": "control_request",
                "request_id": format!("refresh-{index}"),
                "request": { "subtype": "oauth_token_refresh" },
            }));
        }
        let mut answers = Vec::new();
        for cli in &mut clis {
            answers.push(cli.read().await);
            cli.result(1, 1);
        }
        answers
    };
    let (_, _, answers) = tokio::join!(
        all_events(first.run(request_without_tools(vec![user("one")]))),
        all_events(second.run(request_without_tools(vec![user("two")]))),
        asks,
    );
    for (index, answer) in answers.iter().enumerate() {
        assert_eq!(
            *answer,
            json!({
                "type": "control_response",
                "response": {
                    "subtype": "success",
                    "request_id": format!("refresh-{index}"),
                    "response": { "accessToken": "sk-ant-oat01-fresh" },
                },
            })
        );
    }
    // The single-use refresh token was spent once.
    assert_eq!(vendor.requests().len(), 1);
    assert_eq!(vendor.requests()[0].json(), grant());
    let account = pool.active().await.unwrap().unwrap();
    assert_eq!(
        stored(&pool, &account).await["accessToken"],
        "sk-ant-oat01-fresh"
    );
}

#[tokio::test(flavor = "local")]
async fn a_refresh_the_vendor_refuses_answers_no_token_with_its_reason_and_the_run_goes_on() {
    let vendor = MockVendor::start().await;
    vendor.respond_at(
        "/v1/oauth/token",
        MockResponse::status(400).chunk(r#"{"error":"invalid_grant"}"#),
    );
    let token = vendor.url("/v1/oauth/token");
    let urls = Urls {
        token: &token,
        ..NOWHERE
    };
    let (provider, pool) = provider_with(TOKEN, &urls).await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let (events, answer) = tokio::join!(
        all_events(runtime.run(request_without_tools(vec![user("hi")]))),
        async {
            let mut cli = starts.next().await;
            cli.read().await;
            cli.say(json!({
                "type": "control_request",
                "request_id": "refresh",
                "request": { "subtype": "oauth_token_refresh" },
            }));
            let answer = cli.read().await;
            cli.text("ok");
            cli.result(1, 1);
            answer
        }
    );
    assert_eq!(
        answer["response"]["response"],
        json!({ "accessToken": null, "reason": "refresh_failed" })
    );
    assert_eq!(events.len(), 2, "{events:?}");
    let account = pool.active().await.unwrap().unwrap();
    assert_eq!(stored(&pool, &account).await["accessToken"], TOKEN);
}

#[tokio::test(flavor = "local")]
async fn the_usage_probe_uses_the_access_token_refreshed_first_when_due() {
    let vendor = MockVendor::start().await;
    vendor.respond_at("/v1/oauth/token", refreshed("sk-ant-oat01-fresh"));
    vendor.respond_at(
        "/api/oauth/usage",
        MockResponse::status(200)
            .header("content-type", "application/json")
            .chunk(json!({ "five_hour": { "utilization": 12 } }).to_string()),
    );
    let pool = MemoryCredentialPool::new();
    let account = expiring(&pool).await;
    let (token, usage) = (vendor.url("/v1/oauth/token"), vendor.url("/api/oauth/usage"));
    let urls = Urls {
        token: &token,
        usage: &usage,
        ..NOWHERE
    };
    let provider = provider_of(&pool, &account, &urls);
    let snapshot = provider.quota().unwrap().probe().await.unwrap();
    assert_eq!(snapshot.windows[0].used_percent, Some(12.0));
    let requests = vendor.requests();
    let paths: Vec<&str> = requests
        .iter()
        .map(|request| request.uri.path())
        .collect();
    assert_eq!(paths, ["/v1/oauth/token", "/api/oauth/usage"]);
    assert_eq!(
        requests[1].header("authorization"),
        Some("Bearer sk-ant-oat01-fresh")
    );
}
