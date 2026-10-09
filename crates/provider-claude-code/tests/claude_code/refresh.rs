//! The account's tokens, which only the backend refreshes
//! (`claude-code.md` § Accounts and sign-in, `providers.md` § Token
//! refresh): a request whose process would start, or whose kept process
//! holds a token expiring within thirty minutes, refreshes first and starts
//! a new process; a turn the vendor refuses for its token is run once more
//! in a new process with a refreshed token, the agent seeing it once;
//! processes refused together spend the single-use refresh token once; a
//! refresh the vendor refuses fails the request; and the usage probe uses
//! the access token, refreshed first when due.

use demi_provider_common::credentials::{CredentialPool, MemoryCredentialPool};
use std::sync::Arc;

use demi_provider_common::testing::{ManualClock, MockResponse, MockVendor};
use demi_provider_common::{ErrorCode, Provider, ProviderEvent};
use demi_shared_types::TokenUsage;
use serde_json::{Value, json};

use crate::cli::*;

/// An account of `pool` whose access token expires within thirty minutes
/// of the clock.
async fn expiring(pool: &MemoryCredentialPool) -> String {
    seed(pool, sign_in(TOKEN, REFRESH, "2026-09-24T08:20:00Z")).await
}

/// The token endpoint's answer: a new access token valid for eight hours,
/// and a new refresh token.
fn refreshed(token: &str) -> MockResponse {
    MockResponse::status(200)
        .header("content-type", "application/json")
        .chunk(
            json!({
                "access_token": token,
                "refresh_token": "sk-ant-ort01-next-refresh",
                "expires_in": 28_800,
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

/// The token a scripted process read from its descriptor.
fn token_of(cli: &Cli) -> &[u8] {
    cli.spawn.descriptors[0].bytes.as_ref()
}

fn response(input: u64, output: u64) -> ProviderEvent {
    ProviderEvent::Response(TokenUsage {
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
    })
}

#[tokio::test(flavor = "local")]
async fn a_token_expiring_within_thirty_minutes_is_refreshed_and_a_kept_process_holding_it_replaced()
 {
    let vendor = MockVendor::start().await;
    vendor.respond_at("/v1/oauth/token", refreshed("sk-ant-oat01-fresh"));
    let pool = MemoryCredentialPool::new();
    let account = seed(&pool, sign_in(TOKEN, REFRESH, "2026-09-24T09:00:00Z")).await;
    let token = vendor.url("/v1/oauth/token");
    let urls = Urls {
        token: &token,
        ..NOWHERE
    };
    let now = Arc::new(ManualClock::new(NOW.parse().unwrap()));
    let provider = provider_at(&pool, &account, &urls, now.clone());
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let first = request_without_tools(vec![user("hi")]);
    let (_, first_cli) = tokio::join!(all_events(runtime.run(first)), async {
        let cli = starts.next().await;
        cli.result(1, 1);
        cli
    });
    // An hour left: the process starts with the stored token, and is kept.
    assert_eq!(token_of(&first_cli), TOKEN.as_bytes());
    assert!(vendor.requests().is_empty());

    // Twenty minutes left: the next request refreshes first and starts a new
    // process, which resumes the transcript, and the kept one goes.
    now.set("2026-09-24T08:40:00Z".parse().unwrap());
    let second = request_without_tools(vec![user("hi"), user("again")]);
    let (_, second_cli) = tokio::join!(all_events(runtime.run(second)), async {
        let cli = starts.next().await;
        cli.result(1, 1);
        cli
    });
    assert_eq!(token_of(&second_cli), b"sk-ant-oat01-fresh");
    assert!(first_cli.end().is_some());
    assert!(first_cli.config_removed());
    let requests = vendor.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].json(), grant());
    let signed_in = stored(&pool, &account).await;
    assert_eq!(signed_in["accessToken"], "sk-ant-oat01-fresh");
    assert_eq!(signed_in["refreshToken"], "sk-ant-ort01-next-refresh");
    assert_eq!(signed_in["expiresAt"], "2026-09-24T16:40:00.000Z");
    assert_eq!(signed_in["accountId"], "account-1");
}

#[tokio::test(flavor = "local")]
async fn a_turn_refused_for_its_token_runs_once_more_in_a_new_process_and_the_agent_sees_it_once() {
    let vendor = MockVendor::start().await;
    vendor.respond_at("/v1/oauth/token", refreshed("sk-ant-oat01-fresh"));
    let token = vendor.url("/v1/oauth/token");
    let urls = Urls {
        token: &token,
        ..NOWHERE
    };
    let (provider, _pool) = provider_with(TOKEN, &urls).await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let (events, (refused, answered)) = tokio::join!(
        all_events(runtime.run(request_without_tools(vec![user("hi")]))),
        async {
            let refused = starts.next().await;
            refused.refused();
            let answered = starts.next().await;
            // The new process resumes the transcript, as any new one does.
            assert_eq!(answered.messages(), refused.messages());
            answered.text("hello");
            answered.result(3, 1);
            (refused, answered)
        }
    );
    // Nothing of the refused attempt reaches the agent.
    assert_eq!(
        events,
        [ProviderEvent::TextDelta("hello".into()), response(3, 1)]
    );
    assert_eq!(token_of(&refused), TOKEN.as_bytes());
    assert_eq!(token_of(&answered), b"sk-ant-oat01-fresh");
    assert!(refused.end().is_some());
    assert!(refused.config_removed());
    assert_ne!(refused.config_dir(), answered.config_dir());
    assert_eq!(vendor.requests().len(), 1);

    // Refused again with the fresh token: the turn is sent again once only,
    // and fails with the CLI's words.
    vendor.respond_at("/v1/oauth/token", refreshed("sk-ant-oat01-fresher"));
    let (events, ()) = tokio::join!(
        all_events(runtime.run(request_without_tools(vec![
            user("hi"),
            user("again")
        ]))),
        async {
            let mut kept = answered;
            kept.read().await;
            kept.refused();
            let retried = starts.next().await;
            retried.refused();
        }
    );
    let [ProviderEvent::Error(failure)] = events.as_slice() else {
        panic!("the second refusal did not fail the turn: {events:?}");
    };
    assert_eq!(
        failure.message,
        "Failed to authenticate. API Error: 401 OAuth token has expired"
    );
    assert_eq!(placement.starts(), 3);
}

#[tokio::test(flavor = "local")]
async fn processes_refused_together_get_one_refresh_and_the_same_fresh_token() {
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
    // The vendor refuses both processes' token before either is answered.
    let refusals = async {
        let clis = [starts.next().await, starts.next().await];
        for cli in &clis {
            cli.refused();
        }
        let mut tokens = Vec::new();
        for _ in 0..2 {
            let retried = starts.next().await;
            tokens.push(token_of(&retried).to_vec());
            retried.result(1, 1);
        }
        tokens
    };
    let (one, two, tokens) = tokio::join!(
        all_events(first.run(request_without_tools(vec![user("one")]))),
        all_events(second.run(request_without_tools(vec![user("two")]))),
        refusals,
    );
    assert_eq!((one, two), (vec![response(1, 1)], vec![response(1, 1)]));
    assert_eq!(tokens, [b"sk-ant-oat01-fresh".to_vec(), b"sk-ant-oat01-fresh".to_vec()]);
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
async fn a_refresh_the_vendor_refuses_fails_the_request_and_starts_no_process() {
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
    let (events, ()) = tokio::join!(
        all_events(runtime.run(request_without_tools(vec![user("hi")]))),
        async {
            let cli = starts.next().await;
            cli.refused();
        }
    );
    let [ProviderEvent::Error(failure)] = events.as_slice() else {
        panic!("the request did not fail: {events:?}");
    };
    assert_eq!(failure.code, Some(ErrorCode::AuthRefreshFailed));
    assert_eq!(
        failure.message,
        "Claude token refresh failed with HTTP 400; sign in to the account again"
    );
    assert_eq!(placement.starts(), 1);
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
