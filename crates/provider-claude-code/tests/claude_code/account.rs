//! The account, its sign-in, its quota and the catalog
//! (`crates/provider-claude-code/src/account.rs`,
//! `crates/provider-claude-code/src/quota.rs`,
//! `crates/backend-providers/src/llm/catalog.rs`).

use std::sync::Arc;

use demi_host_interface::{ProcessEnd, SpawnEnv};
use demi_provider_claude_code::{ClaudeCodeConfig, ClaudeCodeProvider, CliWork};
use demi_provider_common::credentials::{
    CredentialPool as _, LoginError, LoginIo,
    MemoryCredentialPool,
};
use demi_provider_common::quota::{MemorySnapshots, QuotaError};
use demi_provider_common::testing::login_io;
use demi_provider_common::{Provider, Secret};
use demi_shared_types::{
    QuotaKind, QuotaPlan, QuotaScope, QuotaSeverity,
    SnapshotSource,
};
use serde_json::{Value, json};

use crate::cli::*;

/// A provider of an entry with no account yet, as a first sign-in builds.
fn staged(pool: &MemoryCredentialPool) -> ClaudeCodeProvider {
    ClaudeCodeProvider::new(
        ClaudeCodeConfig::new("entry-1", "Claude", None, None),
        Arc::new(pool.clone()),
        Arc::new(MemorySnapshots::new()),
        models_dev_client(NOWHERE),
        clock(),
    )
}

// A few milliseconds: the scripted CLI signs in at once.
#[tokio::test(flavor = "local")]
async fn a_sign_in_stopped_while_it_waits_for_the_accounts_processes_ends_at_once() {
    let pool = MemoryCredentialPool::new();
    let (placement, mut starts) = ScriptedPlacement::new();
    let accounts = staged(&pool).login_accounts(machine(&placement));
    let adopting = placement.hold_adoptions();
    let (codes, received) = tokio::sync::mpsc::unbounded_channel();
    let stop = tokio_util::sync::CancellationToken::new();
    let io = LoginIo {
        codes: received,
        stop: stop.clone(),
        ..login_io(&|_| {})
    };
    let login = demi_provider_common::testing::guarded("the stopped sign-in's end", accounts.login(io));
    let (stopped, ()) = tokio::join!(login, async {
        let mut cli = starts.next().await;
        cli.say_text("If the browser didn't open, visit: https://claude.com/cai/oauth/authorize?state=s\n");
        codes.send(Secret::try_from("code#state".to_owned()).unwrap()).unwrap();
        cli.read().await;
        cli.write_file(".claude.json", r#"{"oauthAccount":{"accountUuid":"uuid-1","emailAddress":"alex@example.test"}}"#);
        cli.exit(ProcessEnd::Exited(0));
        // The account's processes never end; the user cancels meanwhile.
        adopting.await;
        stop.cancel();
    });
    // Cancel ends the sign-in, which stores nothing and lets go of the
    // machine.
    assert_eq!(stopped.unwrap_err(), LoginError::Stopped);
    assert!(placement.adopted().is_empty());
    assert!(pool.list().await.unwrap().is_empty());
    assert_eq!(placement.leases(), 0);
}

/// What Claude Code 2.1.301 answers `get_usage` with, around `rate_limits`.
fn usage_answer(request_id: &Value, rate_limits: Value) -> Value {
    json!({ "type": "control_response", "response": {
        "subtype": "success",
        "request_id": request_id,
        "response": {
            "session": { "total_cost_usd": 0, "model_usage": {} },
            "subscription_type": "max",
            "rate_limits_available": true,
            "rate_limits": rate_limits,
            "behaviors": null,
        },
    } })
}

/// Probes `provider`'s usage on `placement`, whose process answers
/// `get_usage` with `rate_limits`, and answers the probe's result with the
/// process.
async fn probed(
    provider: &ClaudeCodeProvider,
    starts: &mut Starts,
    rate_limits: Value,
) -> (Result<Arc<demi_shared_types::QuotaSnapshot>, QuotaError>, Cli) {
    tokio::join!(provider.quota().unwrap().probe(), async {
        let mut cli = starts.next().await;
        let initialize = cli.read().await;
        assert_eq!(initialize["request"], json!({ "subtype": "initialize" }));
        let usage = cli.read().await;
        assert_eq!(
            usage["request"],
            json!({ "subtype": "get_usage", "skip_behaviors": true })
        );
        cli.say(json!({ "type": "system", "subtype": "init" }));
        cli.say(json!({ "type": "control_response", "response": {
            "subtype": "success", "request_id": initialize["request_id"] } }));
        cli.say(usage_answer(&usage["request_id"], rate_limits));
        cli
    })
}

#[tokio::test(flavor = "local")]
async fn the_quota_is_the_clis_usage_answer_in_the_accounts_directory_and_its_lines() {
    let pool = MemoryCredentialPool::new();
    let account = seed(&pool, None).await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let provider = provider_on(&pool, &account, Some(machine(&placement)));
    let (snapshot, cli) = probed(
        &provider,
        &mut starts,
        json!({
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
        }),
    )
    .await;
    let snapshot = snapshot.unwrap();
    // A process of the account's directory that sends no message, whose
    // quota reading stays on, closed once it answered.
    assert_eq!(
        cli.work,
        CliWork::Account {
            account,
            handover: None
        }
    );
    let SpawnEnv::Overlay(env) = &cli.spawn.env else {
        panic!("the probe's environment is an overlay");
    };
    assert_eq!(env["CLAUDE_CONFIG_DIR"].as_deref(), Some(cli.config_dir()));
    assert_eq!(env.get("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC"), Some(&None));
    assert!(!cli.spawn.args.iter().any(|arg| arg == "--resume"));
    assert!(cli.end().is_some());
    assert_eq!(placement.leases(), 0);
    assert_eq!(snapshot.source, SnapshotSource::Probe);
    assert_eq!(
        snapshot.plan,
        Some(QuotaPlan {
            id: "max".into(),
            label: "Max".into()
        })
    );
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
    assert_eq!((five_hour.kind, five_hour.period_seconds), (QuotaKind::Limit, Some(5 * 3600)));
    assert_eq!(five_hour.used_percent, Some(12.0));
    assert_eq!(
        five_hour.resets_at,
        Some("2026-09-24T10:00:00Z".parse().unwrap())
    );
    let scoped = &snapshot.windows[1];
    assert_eq!((scoped.kind, scoped.period_seconds), (QuotaKind::Limit, Some(7 * 86_400)));
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

    // Rate limits that are not a usage object are refused, never read as
    // empty, and so are none, which the CLI answers when its quota reading
    // is off.
    for rate_limits in [json!(["five_hour"]), Value::Null] {
        let (unreadable, _) = probed(&provider, &mut starts, rate_limits).await;
        let unreadable = unreadable.unwrap_err();
        assert!(matches!(unreadable, QuotaError::Invalid(_)), "{unreadable:?}");
    }

    // A probe whose process ends before it answers says why.
    let (ended, ()) = tokio::join!(provider.quota().unwrap().probe(), async {
        let cli = starts.next().await;
        cli.stderr("Not logged in\n");
        cli.exit(ProcessEnd::Exited(1));
    });
    assert_eq!(ended.unwrap_err(), QuotaError::Unavailable("Not logged in".into()));

    // A `rate_limit_event` line, as Claude Code 2.1.286 prints it from the
    // vendor's headers, updates the windows it names and keeps the others.
    let mut runtime = runtime_of(&provider, &placement);
    let quota = provider.quota().unwrap();
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
