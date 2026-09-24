//! The account, its quota and the catalog (`providers.md` § Subscription
//! secrets, `usage-and-quota.md` § Claude Code, `models.md` § Directories).

use std::sync::Arc;

use demi_core::{AuthState, QuotaScope, QuotaSeverity, RuntimeState, SnapshotSource};
use demi_provider::credentials::{
    AccountsCapability, AddAccount, CredentialPool, LoginError, MemoryCredentialPool,
};
use demi_provider::quota::{MemorySnapshots, QuotaError};
use demi_provider::testing::{MockResponse, MockVendor};
use demi_provider::{CatalogError, Provider, RuntimeEnv, RuntimeError, Secret};
use demi_provider_claude_code::{ClaudeCodeConfig, ClaudeCodeProvider};
use serde_json::json;

use crate::cli::*;

fn json_answer(body: serde_json::Value) -> MockResponse {
    MockResponse::status(200)
        .header("content-type", "application/json")
        .chunk(body.to_string())
}

#[tokio::test(flavor = "local")]
async fn a_setup_token_is_an_account_named_by_its_digest_and_the_provider_needs_a_placement() {
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
    let accounts = staged.accounts().unwrap();
    let setup_token = AccountsCapability {
        login: false,
        add: true,
    };
    assert_eq!(accounts.capability(), setup_token);
    assert_eq!(
        accounts.login(&|_| {}).await.unwrap_err(),
        LoginError::Unsupported
    );
    let token = || AddAccount::SetupToken(Secret::try_from(TOKEN.to_owned()).unwrap());
    let added = accounts.add(token()).await.unwrap();
    assert!(added.label.starts_with("claude-"), "{}", added.label);
    assert_eq!(added.label.len(), "claude-".len() + 8);
    assert_eq!(added.detail, None);
    // The same token again is the same account.
    let again = accounts.add(token()).await.unwrap();
    assert_eq!(again.id, added.id);
    assert_eq!(pool.list().await.unwrap().len(), 1);
    assert_eq!(pool.active().await.unwrap(), Some(added.id.clone()));
    let document = pool.document(&added.id).read().await.unwrap().unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&document.text).unwrap(),
        json!({ "accessToken": TOKEN })
    );

    let provider = ClaudeCodeProvider::new(
        ClaudeCodeConfig::new("entry-1", "Claude", Some(added.id.clone())),
        Arc::new(pool.clone()),
        Arc::new(MemorySnapshots::new()),
        models_dev_client("http://127.0.0.1:9/api.json"),
        reqwest::Client::new(),
        clock(),
    );
    let signed_in = AuthState::Authenticated {
        account_label: Some(added.label.clone()),
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

    // A corrupt document is refused, never repaired.
    pool.document(&added.id)
        .replace(
            "{\"accessToken\":\"x\",\"refreshToken\":\"y\"}".into(),
            document.version,
        )
        .await
        .unwrap();
    let corrupt = provider.auth_status().await;
    assert!(matches!(corrupt, AuthState::Error { .. }), "{corrupt:?}");
}

#[tokio::test(flavor = "local")]
async fn the_quota_is_probed_with_the_accounts_token_and_observed_on_the_clis_lines() {
    let vendor = MockVendor::start().await;
    let (provider, _) = provider_with(
        TOKEN,
        "http://127.0.0.1:9/api.json",
        &vendor.url("/api/oauth/usage"),
    )
    .await;
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
    assert_eq!(scoped.label, "weekly_scoped (Fable)");
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

    // A line of the CLI's output that carries rate limits updates the
    // windows it names and keeps the others.
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let (_, ()) = tokio::join!(
        all_events(runtime.run(request_without_tools(vec![user("hi")]))),
        async {
            let mut cli = starts.next().await;
            cli.read().await;
            cli.say(json!({
                "type": "result",
                "usage": { "input_tokens": 1, "output_tokens": 1 },
                "rate_limits": {
                    "five_hour": { "used_percentage": 33, "resets_at": "2026-09-24T10:00:00Z" },
                    "seven_day": { "utilization": 50, "resets_at": 1790000000 },
                },
            }));
        }
    );
    let observed = quota.latest().unwrap();
    assert_eq!(observed.source, SnapshotSource::Observation);
    let five_hour = observed
        .windows
        .iter()
        .find(|window| window.id == "five_hour")
        .unwrap();
    assert_eq!(five_hour.used_percent, Some(33.0));
    let seven_day = observed
        .windows
        .iter()
        .find(|window| window.id == "seven_day")
        .unwrap();
    assert_eq!(seven_day.used_percent, Some(50.0));
    // The unit of a reset time is RFC 3339 text; a number is an unknown
    // reset time.
    assert_eq!(seven_day.resets_at, None);
    assert_eq!(observed.windows.len(), 4);
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
    let (provider, _) =
        provider_with(TOKEN, &vendor.url("/api.json"), "http://127.0.0.1:9/usage").await;
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
