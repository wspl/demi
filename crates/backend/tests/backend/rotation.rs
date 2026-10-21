//! Account rotation (`crates/backend-providers/src/llm/rotation.rs`).
//!
//! Scenarios:
//!
//! - An entry's rotation and account order over the API.
//! - Each agent's own account is kept across requests and a restart and taken
//!   again after a compaction.
//! - A used-up account's request is sent again on another account.
//! - Every account is used up.
//! - Manual's *Continue with*.
//!
//! The family is scripted: its runtimes answer with their account's ID, and an
//! account the test marks used up fails with a usage limit.

use std::sync::Arc;

use demi_conversation_socket_protocol::{ClientFrame, ServerFrame};
use demi_shared_types::{Block, Clock as _, QuotaKind, QuotaWindow, Timestamp};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::CredentialId;
use demi_web_api_protocol::providers::{Accounts, LoginState, ProviderAnswer, Providers, Rotation};
use demi_web_api_protocol::settings::UserPreferences;
use reqwest::StatusCode;
use serde_json::json;

use crate::accounts::{Scripts, awaited, device_entry, ended, start_login};
use crate::conversations::{FIRST, Runs, SECOND, Socket, choose, create, kinds, send, transcript};
use crate::families::{Directory, LoginScript, QuotaScript, ScriptedSubscription};
use crate::support::{Harness, Session, TestBackend};

/// A backend with a `device` entry of two accounts, `first` and `second`,
/// whose runtimes answer through `runs`, released at once.
struct TwoAccounts {
    harness: Harness,
    backend: TestBackend,
    master: Session,
    entry: String,
    first: CredentialId,
    second: CredentialId,
    runs: Arc<Runs>,
    scripts: Scripts,
}

async fn two_accounts(process_host: bool) -> TwoAccounts {
    let runs = Runs::new();
    runs.released.send_replace(true);
    let directory = Arc::new(Directory::default());
    for _ in 0..16 {
        directory.answer(Ok(crate::families::catalog(&["m"])));
    }
    let login = Arc::new(LoginScript::default());
    let quota = Arc::new(QuotaScript::free(Vec::new()));
    let family = ScriptedSubscription {
        login: login.clone(),
        quota: quota.clone(),
        directory,
        runs: Some(runs.clone()),
        process_host,
        images: None,
        videos: None,
    };
    let scripts = Scripts {
        login,
        quota,
        families: demi_backend::families::builtin().with("device", family),
    };
    let harness = Harness::new().with_families(scripts.families.clone());
    let (backend, master) = harness.start_set_up().await;
    scripts.login.signs_in("first");
    let entry = device_entry(&backend, &master, &scripts).await;
    let path = format!("/api/providers/{}/accounts", entry.id);
    scripts.login.signs_in("second");
    let id = start_login(&backend, &master, &format!("{path}/login"), json!({})).await;
    let LoginState::Completed { credential_id: second, .. } = awaited(&backend, &master, &id, ended).await
    else {
        panic!("the second login did not complete");
    };
    let first = backend
        .get(&path, Some(&master))
        .await
        .json::<Accounts>()
        .active
        .unwrap();
    TwoAccounts {
        harness,
        backend,
        master,
        entry: entry.id.into_string(),
        first,
        second,
        runs,
        scripts,
    }
}

impl TwoAccounts {
    async fn rotate(&self, rotation: &str) {
        let answer = self
            .backend
            .patch(&format!("/api/providers/{}", self.entry), &self.master, json!({ "rotation": rotation }))
            .await;
        assert_eq!(answer.status, StatusCode::OK, "{}", String::from_utf8_lossy(&answer.body));
    }

    async fn order(&self, ids: &[&CredentialId]) -> StatusCode {
        self.backend
            .put(
                &format!("/api/providers/{}/accounts/order", self.entry),
                &self.master,
                json!({ "credentialIds": ids }),
            )
            .await
            .status
    }
}

/// Makes the runs of `accounts` fail with a usage limit, and no other.
fn used_up(runs: &Runs, accounts: &[&CredentialId]) {
    *runs.used_up.lock().unwrap() = accounts.iter().map(|id| id.to_string()).collect();
}

/// The account each run used, by its ID, in order.
fn accounts_run(runs: &Runs) -> Vec<String> {
    runs.calls().into_iter().map(|(account, _)| account).collect()
}

/// The account the last `context_usage` frame of the root among `frames`
/// names.
fn usage_account(frames: &[ServerFrame]) -> Option<String> {
    frames
        .iter()
        .rev()
        .find_map(|frame| demi_agent_server::testing::root_context(frame).map(|usage| usage.account.clone()))
        .flatten()
}

// A backend, two logins and a few API calls: about half a second.
#[tokio::test]
async fn rotation_and_order_are_the_entrys_and_refused_where_they_cannot_apply() {
    let setup = two_accounts(false).await;
    let TwoAccounts { backend, master, entry, first, second, .. } = &setup;
    let path = format!("/api/providers/{entry}");

    // An order names every account once; the list then shows it.
    assert_eq!(setup.order(&[second]).await, StatusCode::BAD_REQUEST);
    assert_eq!(setup.order(&[second, second]).await, StatusCode::BAD_REQUEST);
    assert_eq!(setup.order(&[second, first]).await, StatusCode::NO_CONTENT);
    let listed = backend.get(&format!("{path}/accounts"), Some(master)).await.json::<Accounts>();
    let ids: Vec<_> = listed.accounts.iter().map(|account| account.id.clone()).collect();
    assert_eq!(ids, [second.as_str(), first.as_str()]);

    let rotated = backend.patch(&path, master, json!({ "rotation": "balanced" })).await;
    assert_eq!(rotated.json::<ProviderAnswer>().provider.rotation, Rotation::Balanced);

    // With one account left the entry is Manual again, as it reads at
    // once, and no rotation but Manual is taken.
    backend.delete(&format!("{path}/accounts/{first}"), master).await;
    let listed = backend.get("/api/providers", Some(master)).await.json::<Providers>();
    let left = listed.providers.iter().find(|provider| provider.id.as_str() == entry).unwrap();
    assert_eq!(left.rotation, Rotation::Manual);
    let refused = backend.patch(&path, master, json!({ "rotation": "in_order" })).await;
    assert_eq!(refused.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::RotationNeedsAccounts));
    let manual = backend.patch(&path, master, json!({ "rotation": "manual" })).await;
    assert_eq!(manual.json::<ProviderAnswer>().provider.rotation, Rotation::Manual);

    // The warning about several accounts is told once, for every entry.
    let preferences = "/api/settings/preferences";
    let told = backend.patch(preferences, master, json!({ "multiAccountWarned": true })).await;
    assert_eq!(told.status, StatusCode::OK);
    let read = backend.get(preferences, Some(master)).await.json::<UserPreferences>();
    assert_eq!(read.preferences.multi_account_warned, Some(true));
    setup.backend.close().await;
}

// A backend, a restart, eight turns and a compaction: about two seconds,
// since an agent's account is kept across all of them in one conversation.
#[tokio::test]
async fn an_agent_keeps_its_account_until_a_compaction_or_a_used_up_account_moves_it() {
    let setup = two_accounts(false).await;
    let (first, second) = (setup.first.clone(), setup.second.clone());
    assert_eq!(setup.order(&[&second, &first]).await, StatusCode::NO_CONTENT);
    setup.rotate("in_order").await;
    create(&setup.backend, &setup.master, FIRST).await;
    choose(&setup.backend, &setup.master, FIRST, &setup.entry, "m").await;
    let mut socket = Socket::connect(&setup.backend, &setup.master, FIRST).await;
    socket.open().await;

    // In Order takes the first account of the list, and the agent keeps it
    // while the order changes.
    let turn = socket.chat("m1", "one").await;
    assert_eq!(usage_account(&turn), Some(second.to_string()));
    assert_eq!(setup.order(&[&first, &second]).await, StatusCode::NO_CONTENT);
    socket.chat("m2", "two").await;
    assert_eq!(accounts_run(&setup.runs), [second.as_str(), second.as_str()]);

    // A restart keeps it with the session's state.
    drop(socket);
    setup.backend.close().await;
    let backend = setup.harness.start().await;
    let mut socket = Socket::connect(&backend, &setup.master, FIRST).await;
    socket.open().await;
    // Its answer fills most of the window, so the user may compact.
    setup.runs.input_tokens.store(60_000, std::sync::atomic::Ordering::SeqCst);
    socket.chat("m3", "three").await;
    assert_eq!(accounts_run(&setup.runs).last().map(String::as_str), Some(second.as_str()));

    // After a compaction, In Order goes back to the first account.
    setup.runs.input_tokens.store(1, std::sync::atomic::Ordering::SeqCst);
    socket.send(&ClientFrame::Compact { node: None }).await;
    socket.until_idle().await;
    let turn = socket.chat("m4", "four").await;
    assert_eq!(usage_account(&turn), Some(first.to_string()));
    assert_eq!(accounts_run(&setup.runs).last().map(String::as_str), Some(first.as_str()));

    // A used-up account: the same request goes at once to the other, which
    // the transcript says in a neutral line, and no error is recorded.
    used_up(&setup.runs, &[&first]);
    let runs_before = setup.runs.calls().len();
    let turn = socket.chat("m5", "five").await;
    assert_eq!(usage_account(&turn), Some(second.to_string()));
    let tail: Vec<_> = accounts_run(&setup.runs)[runs_before..].to_vec();
    assert_eq!(tail, [first.as_str(), second.as_str()]);
    // The refused call keeps its row, with its outcome and its account, and
    // no counts, since the vendor reported none; the totals count it and
    // add nothing.
    let rows = crate::usage::rows(&setup.harness);
    let calls: Vec<_> = rows[rows.len() - 2..]
        .iter()
        .map(|row| (row.account_id.as_deref(), row.outcome.as_deref(), row.tokens[0].is_some()))
        .collect();
    assert_eq!(
        calls,
        [(Some(first.as_str()), Some("usage_limit"), false), (Some(second.as_str()), Some("answered"), true)]
    );
    let totals = crate::conversations::usage(&backend, &setup.master).await.totals;
    let counted: i64 = rows.iter().filter_map(|row| row.tokens[0]).sum();
    assert_eq!(
        (totals[0].requests, totals[0].input_tokens),
        (rows.len() as u64, counted as u64)
    );
    assert!(
        !turn.iter().any(|frame| matches!(frame, ServerFrame::RetryScheduled { .. })),
        "{turn:?}"
    );
    let blocks = transcript(&backend, &setup.master, FIRST).await.blocks;
    let tail = kinds(&blocks);
    assert_eq!(tail[tail.len() - 4..], ["user", "account_switch", "text", "response"]);
    let switch = blocks.iter().rev().find_map(|block| match block {
        Block::AccountSwitch(switch) => Some(switch),
        _ => None,
    });
    let switch = switch.expect("an account switch is recorded");
    assert_eq!(
        (switch.from.as_str(), switch.to.as_str()),
        ("first@example.test", "second@example.test")
    );

    // Every account used up: the turn ends with the usage limit, its
    // moment the earliest among the accounts, half an hour from the first
    // one's failure since neither names one.
    used_up(&setup.runs, &[&first, &second]);
    let failed_at = setup.harness.clock.now();
    socket.chat("m6", "six").await;
    let cold = transcript(&backend, &setup.master, FIRST).await;
    let Some(Block::Error(error)) = cold.blocks.last() else {
        panic!("{:?}", kinds(&cold.blocks));
    };
    assert_eq!(error.code.as_deref(), Some("usage_limit"));
    assert_eq!(error.account.as_deref(), Some(second.as_str()));
    let moment = cold.failures.as_ref().and_then(|facts| facts.get(&error.id)?.retry_at);
    let moment = moment.expect("the failure names when an account can be used again");
    assert_eq!(moment.as_millisecond() - failed_at.as_millisecond(), 30 * 60 * 1000);
    backend.close().await;
}

// A backend, two turns: about half a second.
#[tokio::test]
async fn a_claude_code_like_agent_that_changes_account_closes_its_process_for_one_of_the_other() {
    let setup = two_accounts(true).await;
    let (first, second) = (setup.first.clone(), setup.second.clone());
    setup.rotate("in_order").await;
    create(&setup.backend, &setup.master, FIRST).await;
    choose(&setup.backend, &setup.master, FIRST, &setup.entry, "m").await;
    let mut socket = Socket::connect(&setup.backend, &setup.master, FIRST).await;
    socket.open().await;
    socket.chat("m1", "one").await;
    used_up(&setup.runs, &[&first]);
    socket.chat("m2", "two").await;
    assert_eq!(accounts_run(&setup.runs), [first.as_str(), first.as_str(), second.as_str()]);
    // The first account's process closed when the agent moved; the second
    // account's serves it.
    assert!(setup.runs.closed.lock().unwrap().contains(&first.to_string()));
    assert!(!setup.runs.closed.lock().unwrap().contains(&second.to_string()));
    setup.backend.close().await;
}

// A backend, one turn: about half a second.
#[tokio::test]
async fn rotation_passes_over_an_account_its_vendor_signed_out_which_the_list_shows() {
    let setup = two_accounts(false).await;
    let (first, second) = (setup.first.clone(), setup.second.clone());
    setup.rotate("in_order").await;
    // The provider marks an account whose sign-in its vendor refused, as
    // Claude Code's does (`crates/provider-claude-code/src/account.rs`).
    let marked = setup
        .harness
        .control_database()
        .execute(
            "UPDATE provider_credentials SET signed_out = 1 WHERE id = ?1",
            [first.as_str()],
        )
        .unwrap();
    assert_eq!(marked, 1);
    let listed = setup
        .backend
        .get(&format!("/api/providers/{}/accounts", setup.entry), Some(&setup.master))
        .await;
    let body: serde_json::Value = listed.json();
    let signed_out: Vec<(String, bool)> = body["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|account| {
            let id = account["id"].as_str().unwrap().to_owned();
            (id, account.get("signedOut").and_then(serde_json::Value::as_bool).unwrap_or(false))
        })
        .collect();
    assert_eq!(signed_out, [(first.to_string(), true), (second.to_string(), false)]);
    create(&setup.backend, &setup.master, FIRST).await;
    choose(&setup.backend, &setup.master, FIRST, &setup.entry, "m").await;
    let mut socket = Socket::connect(&setup.backend, &setup.master, FIRST).await;
    socket.open().await;
    socket.chat("m1", "one").await;
    assert_eq!(accounts_run(&setup.runs), [second.as_str()]);
    setup.backend.close().await;
}

/// A window `used` percent used that resets `hours` hours after `now`, of
/// a period of `period` hours.
fn window(now: Timestamp, used: f64, hours: i64, period: u64) -> QuotaWindow {
    let resets_at = Timestamp::from_millisecond(now.as_millisecond() + hours * 3_600_000).unwrap();
    QuotaWindow {
        id: "five_hour".into(),
        kind: QuotaKind::Limit,
        used_percent: Some(used),
        used: None,
        limit: None,
        unit: None,
        resets_at: Some(resets_at),
        period_seconds: Some(period * 3600),
        requested_at: None,
        severity: None,
        scope: None,
    }
}

// A backend, two probes, two turns: about half a second.
#[tokio::test]
async fn balanced_reads_stale_snapshots_and_takes_the_account_with_the_most_room() {
    let setup = two_accounts(false).await;
    let (first, second) = (setup.first.clone(), setup.second.clone());
    let now = setup.harness.clock.now();
    {
        let mut windows = setup.scripts.quota.windows_of.lock().unwrap();
        // 0.2 of the window left with 0.6 of it to go: 0.33.
        windows.insert(first.to_string(), vec![window(now, 80.0, 3, 5)]);
        // 0.9 left with 0.8 to go: 1.1.
        windows.insert(second.to_string(), vec![window(now, 10.0, 4, 5)]);
    }
    setup.rotate("balanced").await;
    create(&setup.backend, &setup.master, FIRST).await;
    choose(&setup.backend, &setup.master, FIRST, &setup.entry, "m").await;
    let mut socket = Socket::connect(&setup.backend, &setup.master, FIRST).await;
    socket.open().await;
    socket.chat("m1", "one").await;
    // Neither account had a snapshot, so both were read before the choice.
    assert_eq!(setup.scripts.quota.probes.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(accounts_run(&setup.runs), [second.as_str()]);
    setup.backend.close().await;
}

// A backend, two turns, an activation and a Resume: about half a second.
#[tokio::test]
async fn without_rotation_a_usage_limit_offers_to_continue_with_the_other_account() {
    let setup = two_accounts(false).await;
    let (first, second) = (setup.first.clone(), setup.second.clone());
    create(&setup.backend, &setup.master, FIRST).await;
    choose(&setup.backend, &setup.master, FIRST, &setup.entry, "m").await;
    let mut socket = Socket::connect(&setup.backend, &setup.master, FIRST).await;
    socket.open().await;
    used_up(&setup.runs, &[&first]);
    socket.chat("m1", "one").await;
    // Manual moves nobody: the turn ends with the limit.
    assert_eq!(accounts_run(&setup.runs), [first.as_str()]);
    let cold = transcript(&setup.backend, &setup.master, FIRST).await;
    let Some(Block::Error(error)) = cold.blocks.last() else {
        panic!("{:?}", kinds(&cold.blocks));
    };
    assert_eq!((error.code.as_deref(), error.account.as_deref()), (Some("usage_limit"), None));
    let offered = cold
        .failures
        .as_ref()
        .and_then(|facts| facts.get(&error.id)?.continue_with.clone())
        .expect("the failure offers the other account");
    assert_eq!(
        (offered.credential_id.as_str(), offered.label.as_str()),
        (second.as_str(), "second@example.test")
    );

    // Continue with it: the page makes it active and resumes.
    let activated = setup
        .backend
        .put(
            &format!("/api/providers/{}/accounts/active", setup.entry),
            &setup.master,
            json!({ "credentialId": offered.credential_id }),
        )
        .await;
    assert_eq!(activated.status, StatusCode::OK);
    socket.send(&ClientFrame::Resume {}).await;
    socket.until_idle().await;
    assert_eq!(accounts_run(&setup.runs), [first.as_str(), second.as_str()]);
    setup.backend.close().await;
}

// A backend, three turns: about half a second.
#[tokio::test]
async fn a_limit_after_only_the_vendors_record_mirrored_moves_the_agent_and_leaves_nothing_of_the_attempt() {
    let setup = two_accounts(true).await;
    let (first, second) = (setup.first.clone(), setup.second.clone());
    // Two accounts of equal room: the agent takes the first.
    setup.rotate("balanced").await;
    create(&setup.backend, &setup.master, FIRST).await;
    choose(&setup.backend, &setup.master, FIRST, &setup.entry, "m").await;
    let mut socket = Socket::connect(&setup.backend, &setup.master, FIRST).await;
    socket.open().await;
    socket.chat("m1", "one").await;

    // Its account runs out after Claude Code mirrored the last tool's
    // result, before the model wrote anything: the same request goes on
    // with the other account, and no error is recorded.
    setup.runs.mirrors.store(true, std::sync::atomic::Ordering::SeqCst);
    used_up(&setup.runs, &[&first]);
    let turn = socket.chat("m2", "two").await;
    assert_eq!(usage_account(&turn), Some(second.to_string()));
    assert_eq!(accounts_run(&setup.runs), [first.as_str(), first.as_str(), second.as_str()]);
    let blocks = transcript(&setup.backend, &setup.master, FIRST).await.blocks;
    let tail = kinds(&blocks);
    assert_eq!(tail[tail.len() - 4..], ["user", "account_switch", "text", "response"]);
    assert!(!blocks.iter().any(|block| matches!(block, Block::Error(_))), "{tail:?}");

    // The attempt given up left nothing: the next request's blocks, which
    // a new process's session file is written from, hold the entry the
    // attempt mirrored once, as the second account's run mirrored it, and
    // not the CLI's record of the failure.
    socket.chat("m3", "three").await;
    let entries = setup.runs.entries.lock().unwrap().last().cloned().unwrap();
    let uuids: Vec<&str> = entries
        .iter()
        .flatten()
        .filter_map(|entry| entry["uuid"].as_str())
        .collect();
    assert!(
        matches!(uuids.as_slice(), [one] if one.starts_with("item-")),
        "{uuids:?}"
    );
    setup.backend.close().await;
}

// A backend, a probe, one turn: about a quarter of a second.
#[tokio::test]
async fn a_limit_reached_mid_answer_ends_the_turn_with_the_moment_of_the_agents_own_account() {
    let setup = two_accounts(false).await;
    let (first, second) = (setup.first.clone(), setup.second.clone());
    let now = setup.harness.clock.now();
    let path = format!("/api/providers/{}", setup.entry);
    // The agent takes the first account while the second is active, so
    // the active account's quota cannot stand in for the agent's.
    setup.rotate("in_order").await;
    let activated = setup
        .backend
        .put(&format!("{path}/accounts/active"), &setup.master, json!({ "credentialId": second }))
        .await;
    assert_eq!(activated.status, StatusCode::OK);
    create(&setup.backend, &setup.master, FIRST).await;
    choose(&setup.backend, &setup.master, FIRST, &setup.entry, "m").await;
    let mut socket = Socket::connect(&setup.backend, &setup.master, FIRST).await;
    socket.open().await;
    socket.chat("m1", "one").await;
    // Its account's window is used up by the time of the next turn.
    let used_up_window = QuotaWindow {
        used_percent: Some(100.0),
        ..window(now, 0.0, 2, 5)
    };
    setup
        .scripts
        .quota
        .windows_of
        .lock()
        .unwrap()
        .insert(first.to_string(), vec![used_up_window]);
    let probed = setup
        .backend
        .post(&format!("{path}/quota"), Some(&setup.master), json!({ "credentialId": first }))
        .await;
    assert_eq!(probed.status, StatusCode::OK);

    setup.runs.fails_late.store(true, std::sync::atomic::Ordering::SeqCst);
    used_up(&setup.runs, &[&first]);
    socket.chat("m2", "two").await;
    // Something of the answer had shown: nothing is sent again.
    assert_eq!(accounts_run(&setup.runs), [first.as_str(), first.as_str()]);
    let cold = transcript(&setup.backend, &setup.master, FIRST).await;
    let Some(Block::Error(error)) = cold.blocks.last() else {
        panic!("{:?}", kinds(&cold.blocks));
    };
    assert_eq!(error.account.as_deref(), Some(first.as_str()));
    let moment = cold.failures.as_ref().and_then(|facts| facts.get(&error.id)?.retry_at);
    assert_eq!(moment, Some(Timestamp::from_millisecond(now.as_millisecond() + 2 * 3_600_000).unwrap()));
    // But the account is used up: neither another agent nor this one's
    // next request takes it until its moment.
    create(&setup.backend, &setup.master, SECOND).await;
    choose(&setup.backend, &setup.master, SECOND, &setup.entry, "m").await;
    let mut other = Socket::connect(&setup.backend, &setup.master, SECOND).await;
    other.open().await;
    other.chat("o1", "other").await;
    socket.chat("m3", "three").await;
    assert_eq!(
        accounts_run(&setup.runs),
        [first.as_str(), first.as_str(), second.as_str(), second.as_str()]
    );
    setup.backend.close().await;
}

// A backend and four conversations' turns: about half a second.
#[tokio::test]
async fn balanced_spreads_the_agents_whose_turns_run_and_not_the_idle_ones() {
    let setup = two_accounts(false).await;
    let (first, second) = (setup.first.clone(), setup.second.clone());
    // Two accounts of equal room.
    setup.rotate("balanced").await;
    let ids = [FIRST, SECOND, THIRD, FOURTH];
    let mut sockets = Vec::new();
    for id in ids {
        create(&setup.backend, &setup.master, id).await;
        choose(&setup.backend, &setup.master, id, &setup.entry, "m").await;
        let mut socket = Socket::connect(&setup.backend, &setup.master, id).await;
        socket.open().await;
        sockets.push(socket);
    }
    // One after the other: each turn has ended before the next starts, so
    // the idle agent keeping the first account counts on it no more, and
    // the next agent takes the first account too.
    sockets[0].chat("a", "one").await;
    sockets[1].chat("b", "two").await;
    // Together: the third agent's turn runs on the first account while the
    // fourth chooses, which takes the second.
    setup.runs.held.send_replace(true);
    sockets[2].send(&send("c", "three")).await;
    let mut started = setup.runs.started.subscribe();
    let _ = started.wait_for(|count| *count == 3).await;
    sockets[3].send(&send("d", "four")).await;
    let _ = started.wait_for(|count| *count == 4).await;
    setup.runs.held.send_replace(false);
    sockets[2].until_idle().await;
    sockets[3].until_idle().await;
    assert_eq!(
        accounts_run(&setup.runs),
        [first.as_str(), first.as_str(), first.as_str(), second.as_str()]
    );
    setup.backend.close().await;
}

const THIRD: &str = "5e2d3c4b-1a0f-4e9d-8c7b-6a5f4e3d2c1b";
const FOURTH: &str = "9a8b7c6d-5e4f-4a3b-8c2d-1e0f9a8b7c6d";
