//! The account, its sign-in, its quota and the catalog
//! (`crates/provider-claude-code/src/account.rs`,
//! `crates/provider-claude-code/src/quota.rs`,
//! `crates/backend-providers/src/llm/catalog.rs`).

use std::sync::Arc;

use demi_host_interface::{ProcessEnd, Signal, SpawnEnv};
use demi_provider_claude_code::{ClaudeCodeConfig, ClaudeCodeProvider, CliWork};
use demi_provider_common::credentials::{
    AccountLabel, AccountsCapability, CredentialPool as _, LoginError, LoginIo, LoginKind,
    MemoryCredentialPool,
};
use demi_provider_common::quota::{MemorySnapshots, QuotaError};
use demi_provider_common::testing::{ManualClock, MockResponse, MockVendor, login_io};
use demi_provider_common::{CatalogError, Provider, RuntimeEnv, RuntimeError, Secret};
use demi_shared_types::{
    AuthState, LoginStage, QuotaKind, QuotaPlan, QuotaScope, QuotaSeverity, RuntimeState,
    SnapshotSource,
};
use serde_json::{Value, json};

use crate::cli::*;

fn json_answer(body: Value) -> MockResponse {
    MockResponse::status(200)
        .header("content-type", "application/json")
        .chunk(body.to_string())
}

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

#[tokio::test(flavor = "local")]
async fn an_account_is_named_by_its_record_and_runs_only_in_a_placement() {
    let pool = MemoryCredentialPool::new();
    let staged = staged(&pool);
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

    let account = seed(&pool, None).await;
    let provider = provider_of(&pool, &account);
    let signed_in = AuthState::Authenticated {
        account_label: Some("alex@example.test".into()),
    };
    assert_eq!(provider.auth_status().await, signed_in);
    assert!(provider.capabilities().process_host);
    // Whether it can run is its machine's to show, which the provider does
    // not ask.
    assert!(matches!(
        provider.runtime_state(),
        RuntimeState::Unknown { .. }
    ));
    let env = RuntimeEnv {
        http: demi_provider_common::ProviderHttp::new(reqwest::Client::new()),
    };
    let refused = provider.runtime(env).err().unwrap();
    assert_eq!(
        refused,
        RuntimeError::ProcessHostRequired {
            provider: "Claude".into()
        }
    );
}

// A few milliseconds: the scripted CLI signs in at once.
#[tokio::test(flavor = "local")]
async fn a_sign_in_runs_the_clis_login_in_a_new_directory_which_becomes_the_accounts() {
    let pool = MemoryCredentialPool::new();
    let (placement, mut starts) = ScriptedPlacement::new();
    let accounts = staged(&pool).login_accounts(machine(&placement));
    assert_eq!(
        accounts.capability(),
        AccountsCapability {
            login: Some(LoginKind::PastedCode)
        }
    );
    let reached = std::sync::Mutex::new(Vec::new());
    let stage = |stage: LoginStage| reached.lock().unwrap().push(stage);
    let shown = std::sync::Mutex::new(None);
    let pending = |pending: demi_shared_types::LoginPending| {
        *shown.lock().unwrap() = Some(pending.verification_url);
    };
    let (codes, received) = tokio::sync::mpsc::unbounded_channel();
    let io = LoginIo {
        stage: &stage,
        codes: received,
        ..login_io(&pending)
    };
    let link = "https://claude.com/cai/oauth/authorize?code=true&state=scripted";
    let (signed_in, (cli, holding)) = tokio::join!(accounts.login(io), async {
        let mut cli = starts.next().await;
        // The sign-in holds the machine while it runs.
        let holding = placement.leases();
        cli.say_text(&format!("Opening browser to sign in…\nIf the browser didn't open, visit: {link}\n"));
        codes.send(Secret::try_from("code#state".to_owned()).unwrap()).unwrap();
        // The code goes to the CLI with its newline, as the user typed it.
        assert_eq!(cli.read().await, Value::String("code#state".into()));
        cli.write_file(
            ".claude.json",
            r#"{"numStartups":1,"oauthAccount":{"accountUuid":"uuid-1","emailAddress":"alex@example.test","organizationUuid":"org-1"}}"#,
        );
        cli.write_file(".credentials.json", r#"{"claudeAiOauth":{"accessToken":"never-read"}}"#);
        cli.exit(ProcessEnd::Exited(0));
        (cli, holding)
    });
    let info = signed_in.unwrap();
    assert_eq!(holding, 1);
    assert_eq!(placement.leases(), 0);
    assert_eq!(shown.lock().unwrap().as_deref(), Some(link));
    assert_eq!(
        *reached.lock().unwrap(),
        [LoginStage::WaitingForMachine, LoginStage::Starting]
    );
    // The CLI's own login, in a new directory of its own.
    assert_eq!(cli.work, CliWork::Login);
    assert_eq!(cli.spawn.args, ["auth", "login", "--claudeai"]);
    let SpawnEnv::Overlay(env) = &cli.spawn.env else {
        panic!("the login's environment is an overlay");
    };
    assert_eq!(env["CLAUDE_CONFIG_DIR"].as_deref(), Some(cli.config_dir()));
    // The account is the one the CLI signed in, named by its email; its
    // record holds no token, and its directory is the sign-in's.
    let key = AccountLabel {
        label: "alex@example.test".into(),
        detail: None,
        identity_key: Some("account:uuid-1".into()),
    };
    assert_eq!(info.label, key.label);
    assert_eq!(placement.adopted(), [info.id.clone()]);
    assert_eq!(pool.document(&info.id).read().await.unwrap(), None);
    let stored = pool.meta(&info.id).await.unwrap().unwrap();
    assert_eq!(stored.identity_key, key.identity_key);
    assert!(!stored.signed_out);
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

// Instant: the start fails as soon as the machine is reached.
#[tokio::test(flavor = "local")]
async fn a_sign_in_that_fails_right_after_a_stage_reports_the_stage_before_its_failure() {
    let (placement, _starts) = ScriptedPlacement::new();
    placement.fail_next("The Claude Code machine did not reconnect");
    let accounts = staged(&MemoryCredentialPool::new()).login_accounts(machine(&placement));
    let reached = std::sync::Mutex::new(Vec::new());
    let stage = |stage: LoginStage| reached.lock().unwrap().push(stage);
    let io = LoginIo {
        stage: &stage,
        ..login_io(&|_| {})
    };
    let failed = accounts.login(io).await.unwrap_err();
    assert_eq!(
        failed,
        LoginError::Failed("The Claude Code machine did not reconnect".into())
    );
    assert_eq!(*reached.lock().unwrap(), [LoginStage::WaitingForMachine]);
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

#[tokio::test(flavor = "local")]
async fn a_rate_limit_line_carries_the_start_of_the_call_it_follows() {
    let pool = MemoryCredentialPool::new();
    let account = seed(&pool, None).await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let clock = Arc::new(ManualClock::new(NOW.parse().unwrap()));
    let provider = ClaudeCodeProvider::new(
        ClaudeCodeConfig::new("entry-1", "Claude", Some(account), Some(machine(&placement))),
        Arc::new(pool.clone()),
        Arc::new(MemorySnapshots::new()),
        models_dev_client(NOWHERE),
        clock.clone(),
    );
    let rate_limit = |window: &str, utilization: f64| {
        json!({
            "type": "rate_limit_event",
            "rate_limit_info": {
                "status": "allowed",
                "unifiedWindows": { window: { "utilization": utilization, "resetsAt": 1790855225 } },
            },
        })
    };
    let later = "2026-09-24T08:00:05Z";
    let mut runtime = runtime_of(&provider, &placement);
    let (_, ()) = tokio::join!(
        all_events(runtime.run(request_without_tools(vec![user("hi")]))),
        async {
            let cli = starts.next().await;
            // The run started at NOW; the CLI starts a second call of the
            // turn on its own at `later`.
            clock.set(later.parse().unwrap());
            cli.message_start();
            cli.message_stop();
            cli.say(rate_limit("five_hour", 0.5));
            cli.message_start();
            cli.message_stop();
            cli.say(rate_limit("seven_day", 0.25));
            cli.result(1, 1);
        }
    );
    let snapshot = provider.quota().unwrap().latest().unwrap();
    let requested_at = |id: &str| {
        snapshot
            .windows
            .iter()
            .find(|window| window.id == id)
            .unwrap()
            .requested_at
    };
    // The run's first call started with the run, before its first line;
    // the second when its first line came.
    assert_eq!(requested_at("five_hour"), Some(NOW.parse().unwrap()));
    assert_eq!(requested_at("seven_day"), Some(later.parse().unwrap()));
}

// Thirty seconds on a paused clock, then a few milliseconds.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_probe_that_times_out_or_is_given_up_closes_its_process_sigterm_first() {
    let pool = MemoryCredentialPool::new();
    let account = seed(&pool, None).await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let provider = provider_on(&pool, &account, Some(machine(&placement)));
    // A CLI that reads the usage request and never answers it.
    async fn unanswered(starts: &mut Starts) -> Cli {
        let mut cli = starts.next().await;
        cli.read().await;
        cli.read().await;
        cli
    }
    let (timed_out, cli) = tokio::join!(provider.quota().unwrap().probe(), unanswered(&mut starts));
    assert_eq!(
        timed_out.unwrap_err(),
        QuotaError::Unavailable("Claude Code did not answer its usage request within 30 seconds".into())
    );
    // Closed, never killed outright: a kill while the CLI renews the
    // account's sign-in stops the account for a minute.
    assert_eq!(cli.signals(), [Signal::Terminate]);
    assert!(!cli.dropped());
    assert!(cli.config_removed());

    // Nobody waits for the probe any more: its process is closed the same
    // way.
    let mut probe = Box::pin(provider.quota().unwrap().probe());
    let cli = tokio::select! {
        probed = &mut probe => panic!("the probe ended unanswered: {probed:?}"),
        cli = unanswered(&mut starts) => cli,
    };
    drop(probe);
    cli.ended().await;
    assert_eq!(cli.signals(), [Signal::Terminate]);
    assert!(!cli.dropped());
    assert_eq!(placement.leases(), 0);
}

/// A limit of the probe has its period and its model, never the vendor's
/// kind, which the page names it from (`crates/provider-claude-code/src/quota.rs`).
#[tokio::test(flavor = "local")]
async fn a_probed_limit_has_its_period_and_model() {
    let pool = MemoryCredentialPool::new();
    let account = seed(&pool, None).await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let provider = provider_on(&pool, &account, Some(machine(&placement)));
    let on = |model: &str| json!({ "model": { "display_name": model } });
    let (snapshot, _) = probed(&provider, &mut starts, json!({
        "limits": [
            { "kind": "weekly_scoped", "percent": 10, "scope": on("Fable") },
            { "kind": "session_scoped", "percent": 20, "scope": on("Fable") },
            { "kind": "weekly_extra", "percent": 30 },
            { "kind": "monthly_credits", "percent": 40, "scope": on("Opus") },
            { "kind": "monthly_credits", "percent": 50 },
        ],
    }))
    .await;
    let snapshot = snapshot.unwrap();
    let names: Vec<(Option<u64>, Option<&str>)> = snapshot
        .windows
        .iter()
        .map(|window| {
            assert_eq!(window.kind, QuotaKind::Limit);
            let model = window.scope.as_ref().and_then(|scope| scope.label.as_deref());
            (window.period_seconds, model)
        })
        .collect();
    let (week, five_hours) = (Some(7 * 86_400), Some(5 * 3600));
    assert_eq!(
        names,
        [(week, Some("Fable")), (five_hours, Some("Fable")), (week, None), (None, Some("Opus")), (None, None)]
    );
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
    let pool = MemoryCredentialPool::new();
    let account = seed(&pool, None).await;
    let provider = ClaudeCodeProvider::new(
        ClaudeCodeConfig::new("entry-1", "Claude", Some(account), None),
        Arc::new(pool),
        Arc::new(MemorySnapshots::new()),
        models_dev_client(&vendor.url("/api.json")),
        clock(),
    );
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
