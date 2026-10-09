//! The account, its quota and the catalog (`providers.md` § Subscription
//! secrets, `usage-and-quota.md` § Claude Code, `models.md` § Directories).

use std::sync::Arc;

use demi_provider_claude_code::{
    AccountMachine, AccountWork, ClaudeCodeConfig, ClaudeCodeProvider, StartError,
};
use demi_provider_common::credentials::{
    AccountsCapability, CredentialPool, LoginError, LoginKind, MemoryCredentialPool,
};
use demi_provider_common::quota::{MemorySnapshots, QuotaError};
use demi_provider_common::testing::{MockResponse, MockVendor, login_io};
use demi_provider_common::{
    CatalogError, ErrorCode, Provider, ProviderEvent, RuntimeEnv, RuntimeError,
};
use futures_util::future::BoxFuture;
use demi_shared_types::{AuthState, QuotaScope, QuotaSeverity, RuntimeState, SnapshotSource};
use serde_json::json;

use crate::cli::*;

fn json_answer(body: serde_json::Value) -> MockResponse {
    MockResponse::status(200)
        .header("content-type", "application/json")
        .chunk(body.to_string())
}

#[tokio::test(flavor = "local")]
async fn a_sign_in_is_an_account_named_by_its_email_that_signs_in_on_a_machine_and_runs_in_a_placement()
{
    let pool = MemoryCredentialPool::new();
    let staged = ClaudeCodeProvider::new(
        ClaudeCodeConfig::new("entry-1", "Claude", None),
        Arc::new(pool.clone()),
        Arc::new(MemorySnapshots::new()),
        models_dev_client("http://127.0.0.1:9/api.json"),
        reqwest::Client::new(),
        clock(),
    );
    let unauthenticated = AuthState::Unauthenticated {
        message: Some("No Claude Code account is signed in".into()),
    };
    assert_eq!(staged.auth_status().await, unauthenticated);
    // Inference's provider cannot sign in: a sign-in needs the machine the
    // CLI's login runs on.
    let accounts = staged.accounts().unwrap();
    assert_eq!(accounts.capability(), AccountsCapability { login: None });
    assert_eq!(
        accounts.login(login_io(&|_| {})).await.unwrap_err(),
        LoginError::Unsupported
    );
    let signing_in = staged.login_accounts(Arc::new(NoMachine));
    assert_eq!(
        signing_in.capability(),
        AccountsCapability {
            login: Some(LoginKind::PastedCode)
        }
    );

    let account = seed(&pool, sign_in(TOKEN, REFRESH, "2026-09-24T16:00:00Z")).await;
    let provider = provider_of(&pool, &account, &NOWHERE);
    let signed_in = AuthState::Authenticated {
        account_label: Some("zan@example.test".into()),
    };
    assert_eq!(provider.auth_status().await, signed_in);
    assert!(provider.capabilities().process_host);
    // Whether it can run is the Cloud's to show, which the provider does not
    // ask.
    assert!(matches!(
        provider.runtime_state(),
        RuntimeState::Unknown { .. }
    ));
    let env = RuntimeEnv {
        http: reqwest::Client::new(),
    };
    let refused = provider.runtime(env).err().unwrap();
    assert_eq!(
        refused,
        RuntimeError::ProcessHostRequired {
            provider: "Claude".into()
        }
    );
}

/// A machine nothing signs in on.
struct NoMachine;

impl AccountMachine for NoMachine {
    fn run(&self, _: String, _: AccountWork) -> BoxFuture<'static, Result<(), StartError>> {
        unreachable!("no sign-in runs")
    }
}

#[tokio::test(flavor = "local")]
async fn an_account_of_a_setup_token_fails_with_a_message_to_sign_in_again_and_starts_no_process() {
    let pool = MemoryCredentialPool::new();
    let account = seed(
        &pool,
        json!({ "accessToken": "sk-ant-oat01-old-setup-token" }).to_string(),
    )
    .await;
    let provider = provider_of(&pool, &account, &NOWHERE);
    let message = "The Claude Code account cannot be read: its secret document is malformed \
                   at .: a field is missing, unknown or of the wrong type. Remove \
                   the account and sign in again";
    // The document is refused, never repaired, and its token never shown.
    assert_eq!(
        provider.auth_status().await,
        AuthState::Error {
            message: message.into()
        }
    );
    let (placement, _starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let events = all_events(runtime.run(request(vec![user("hi")]))).await;
    let [ProviderEvent::Error(failure)] = events.as_slice() else {
        panic!("the request did not fail: {events:?}");
    };
    assert_eq!(
        (failure.message.as_str(), failure.code.clone()),
        (message, Some(ErrorCode::AuthInvalid))
    );
    assert_eq!(placement.starts(), 0);
    let stored = pool.document(&account).read().await.unwrap().unwrap();
    assert!(stored.text.contains("sk-ant-oat01-old-setup-token"));
}

#[tokio::test(flavor = "local")]
async fn the_quota_is_probed_with_the_accounts_token_and_observed_on_the_clis_lines() {
    let vendor = MockVendor::start().await;
    let usage = vendor.url("/api/oauth/usage");
    let urls = Urls {
        usage: &usage,
        ..NOWHERE
    };
    let (provider, _) = provider_with(TOKEN, &urls).await;
    vendor.respond(json_answer(json!({
        "five_hour": { "utilization": 12, "resets_at": "2026-09-24T10:00:00.000Z" },
        "seven_day": { "utilization": "lots" },
        "seven_day_opus": null,
        "limits": [
            { "kind": "session", "percent": 12 },
            { "percent": 50 },
            { "kind": "weekly_scoped", "percent": 100, "severity": "critical",
              "resets_at": "2026-09-28T08:00:00Z", "scope": { "model": { "display_name": "Fable" } } },
            { "kind": "monthly_credits", "percent": 85, "severity": "sideways" },
        ],
    })));
    let quota = provider.quota().unwrap();
    let snapshot = quota.probe().await.unwrap();
    let request = &vendor.requests()[0];
    assert_eq!(
        request.header("authorization"),
        Some(format!("Bearer {TOKEN}").as_str())
    );
    assert_eq!(request.header("anthropic-beta"), Some("oauth-2025-04-20"));
    assert_eq!(snapshot.source, SnapshotSource::Probe);
    assert_eq!(snapshot.plan, None);
    let ids: Vec<&str> = snapshot
        .windows
        .iter()
        .map(|window| window.id.as_str())
        .collect();
    assert_eq!(
        ids,
        [
            "five_hour",
            "limit:weekly_scoped:Fable",
            "limit:monthly_credits"
        ]
    );
    let five_hour = &snapshot.windows[0];
    assert_eq!(five_hour.label, "5h session");
    assert_eq!(five_hour.used_percent, Some(12.0));
    assert_eq!(
        five_hour.resets_at,
        Some("2026-09-24T10:00:00Z".parse().unwrap())
    );
    let scoped = &snapshot.windows[1];
    assert_eq!(scoped.label, "7d Fable");
    assert_eq!(scoped.severity, Some(QuotaSeverity::Critical));
    assert_eq!(
        scoped.scope,
        Some(QuotaScope {
            kind: "model".into(),
            label: Some("Fable".into())
        })
    );
    // A severity Demi does not know falls back to the share's.
    assert_eq!(snapshot.windows[2].severity, Some(QuotaSeverity::Warning));

    // An answer that is not a usage object is refused, never read as empty.
    vendor.respond(json_answer(json!(["five_hour"])));
    let unreadable = quota.probe().await.unwrap_err();
    assert!(
        matches!(unreadable, QuotaError::Invalid(_)),
        "{unreadable:?}"
    );

    // A refusal says why.
    vendor.respond(MockResponse::status(401).chunk("token expired"));
    let refused = quota.probe().await.unwrap_err();
    assert_eq!(
        refused,
        QuotaError::Unavailable("Claude usage request failed (401): token expired".into())
    );

    // A `rate_limit_event` line, as Claude Code 2.1.286 prints it from the
    // vendor's headers, updates the windows it names and keeps the others.
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let (_, ()) = tokio::join!(
        all_events(runtime.run(request_without_tools(vec![user("hi")]))),
        async {
            let cli = starts.next().await;
            cli.say(json!({
                "type": "rate_limit_event",
                "rate_limit_info": {
                    "status": "allowed_warning",
                    "resetsAt": 1790855225,
                    "rateLimitType": "seven_day_opus",
                    "utilization": 0.97,
                    "isUsingOverage": false,
                    "surpassedThreshold": 0.95,
                    "unifiedWindows": {
                        "five_hour": { "utilization": 0.33, "resetsAt": 1790855225 },
                        "seven_day": { "utilization": 0.5, "resetsAt": "soon" },
                        "seven_day_overage_included": { "utilization": 0.1, "resetsAt": 1790855225 },
                    },
                },
                "uuid": "f76bacdd-1275-43c0-bff9-c6405a86183a",
                "session_id": "s",
            }));
            cli.result(1, 1);
        }
    );
    let observed = quota.latest().unwrap();
    assert_eq!(observed.source, SnapshotSource::Observation);
    let window = |id: &str| {
        observed
            .windows
            .iter()
            .find(|window| window.id == id)
            .unwrap()
    };
    // A utilization is a fraction and a reset time is Unix seconds.
    assert_eq!(window("five_hour").used_percent, Some(33.0));
    assert_eq!(
        window("five_hour").resets_at,
        Some("2026-10-01T11:47:05Z".parse().unwrap())
    );
    assert_eq!(window("seven_day").used_percent, Some(50.0));
    assert_eq!(window("seven_day").resets_at, None);
    // The binding window counts when the reported windows do not hold it.
    assert_eq!(window("seven_day_opus").used_percent, Some(97.0));
    assert_eq!(
        window("seven_day_opus").severity,
        Some(QuotaSeverity::Critical)
    );
    // The probe's limits stay; a window Demi does not name is left out.
    assert_eq!(observed.windows.len(), 5);
}

/// A limit of the probe is named by its period and its model, never by the
/// vendor's kind (`usage-and-quota.md` § Claude Code).
#[tokio::test(flavor = "local")]
async fn a_probed_limit_is_named_by_its_period_and_model() {
    let vendor = MockVendor::start().await;
    let usage = vendor.url("/api/oauth/usage");
    let urls = Urls {
        usage: &usage,
        ..NOWHERE
    };
    let (provider, _) = provider_with(TOKEN, &urls).await;
    let on = |model: &str| json!({ "model": { "display_name": model } });
    vendor.respond(json_answer(json!({
        "limits": [
            { "kind": "weekly_scoped", "percent": 10, "scope": on("Fable") },
            { "kind": "session_scoped", "percent": 20, "scope": on("Fable") },
            { "kind": "weekly_extra", "percent": 30 },
            { "kind": "monthly_credits", "percent": 40, "scope": on("Opus") },
            { "kind": "monthly_credits", "percent": 50 },
        ],
    })));
    let snapshot = provider.quota().unwrap().probe().await.unwrap();
    let labels: Vec<&str> = snapshot
        .windows
        .iter()
        .map(|window| window.label.as_str())
        .collect();
    assert_eq!(labels, ["7d Fable", "5h Fable", "7d", "Opus", "Other limit"]);
}

#[tokio::test(flavor = "local")]
async fn the_catalog_is_models_devs_claude_models_from_4_6_flagship_first_and_thinking_stays_on() {
    let vendor = MockVendor::start().await;
    let model = |name: &str| {
        json!({
            "name": name,
            "attachment": true,
            "reasoning": true,
            "tool_call": true,
            "reasoning_options": [{ "type": "effort", "values": ["low", "medium", "high"] }],
            "limit": { "context": 1_000_000, "output": 128_000 },
            "cost": { "input": 5, "output": 25, "cache_read": 0.5, "cache_write": 6.25 },
        })
    };
    vendor.respond(json_answer(json!({
        "anthropic": {
            "id": "anthropic",
            "name": "Anthropic",
            "npm": "@ai-sdk/anthropic",
            "models": {
                "claude-haiku-4-5": model("Claude Haiku 4.5"),
                "claude-sonnet-4-6": model("Claude Sonnet 4.6"),
                "claude-3-5-sonnet-20241022": model("Claude Sonnet 3.5"),
                "claude-newfamily-5": { "name": "Claude Newfamily 5" },
                "claude-opus-4-8": model("Claude Opus 4.8"),
                "claude-opus-4-6": model("Claude Opus 4.6"),
                "claude-sonnet-4-20250514": model("Claude Sonnet 4"),
                "claude-mystery": model("Claude Mystery"),
                "gpt-4o": model("Not Claude"),
            },
        },
        "openai": { "id": "openai", "name": "OpenAI", "models": {} },
    })));
    let models_dev = vendor.url("/api.json");
    let urls = Urls {
        models_dev: &models_dev,
        ..NOWHERE
    };
    let (provider, _) = provider_with(TOKEN, &urls).await;
    let catalog = provider.list_models().await.unwrap();
    let ids: Vec<&str> = catalog
        .models
        .iter()
        .map(|model| model.id.as_str())
        .collect();
    assert_eq!(
        ids,
        [
            "claude-opus-4-8",
            "claude-opus-4-6",
            "claude-sonnet-4-6",
            "claude-newfamily-5"
        ]
    );
    assert_eq!(
        catalog.warnings,
        ["Skipped Claude model with unparseable version: claude-mystery"]
    );
    let opus = &catalog.models[0];
    assert_eq!(opus.display_name, "Claude Opus 4.8");
    assert_eq!(opus.context_window, Some(1_000_000));
    assert_eq!(opus.can_disable_thinking, Some(false));
    let unknown = &catalog.models[3];
    assert_eq!(unknown.supports_tools, None);
    assert_eq!(unknown.context_window, None);

    // A document without the vendor is one Demi cannot read.
    vendor.respond(json_answer(
        json!({ "openai": { "id": "openai", "name": "OpenAI", "models": {} } }),
    ));
    let unreadable = provider.list_models().await.unwrap_err();
    assert!(
        matches!(unreadable, CatalogError::Invalid(_)),
        "{unreadable:?}"
    );
}
