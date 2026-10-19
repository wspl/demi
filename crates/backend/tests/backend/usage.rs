//! The usage ledger (`usage-and-quota.md` § Usage ledger): a row per
//! vendor call of a conversation, with its agent, turn, purpose, account and
//! ids, a compaction's summary request among them, written as each call
//! ends, and a write that fails losing the row but no turn; and its totals,
//! one group per entry and model in the order each pair was first used,
//! each user's own, and on a shared instance every account's for an
//! administrator, read from rows written to the ledger directly. The rows of
//! a Claude Code run, of a title request and of a refused call are their
//! modules' (`claude`, `titles`, `rotation`).

use demi_conversation_socket_protocol::{ClientFrame, CostCall, ServerFrame};
use demi_provider_common::testing::MockVendor;
use demi_web_api_protocol::auth::Role;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::settings::InstanceMode;
use demi_web_api_protocol::usage::{ConversationCost, InstanceUsage, UsageTotals};
use reqwest::StatusCode;

use serde_json::json;

use crate::conversations::{
    FIRST, Socket, anthropic, answer, choose, create, kinds, last_text, message, text_block, transcript, usage,
};
use crate::support::Harness;

/// A usage ledger row as the control database keeps it.
#[derive(Debug)]
pub(crate) struct Row {
    pub agent: Option<i64>,
    pub turn_id: Option<String>,
    pub purpose: Option<String>,
    pub provider_id: String,
    pub account_id: Option<String>,
    pub model_id: String,
    pub request_id: Option<String>,
    pub vendor_id: Option<String>,
    /// Input, output, cache read and cache write; each none when the vendor
    /// reported none.
    pub tokens: [Option<i64>; 4],
    pub cache_write_1h_tokens: Option<i64>,
    pub outcome: Option<String>,
    pub quota: Option<String>,
    pub started_at: Option<i64>,
    pub ended_at: i64,
    pub media_seconds: Option<f64>,
    pub cost: Option<f64>,
    pub cost_currency: Option<String>,
}

/// Every row of the ledger, in the order they were written.
pub(crate) fn rows(harness: &Harness) -> Vec<Row> {
    let database = harness.control_database();
    let mut statement = database
        .prepare(
            "SELECT agent, turn_id, purpose, provider_id, account_id, model_id, request_id, vendor_id,
               input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, cache_write_1h_tokens,
               outcome, quota, started_at, ended_at, media_seconds, cost, cost_currency
             FROM usage_ledger ORDER BY rowid",
        )
        .unwrap();
    statement
        .query_map([], |row| {
            Ok(Row {
                agent: row.get(0)?,
                turn_id: row.get(1)?,
                purpose: row.get(2)?,
                provider_id: row.get(3)?,
                account_id: row.get(4)?,
                model_id: row.get(5)?,
                request_id: row.get(6)?,
                vendor_id: row.get(7)?,
                tokens: [row.get(8)?, row.get(9)?, row.get(10)?, row.get(11)?],
                cache_write_1h_tokens: row.get(12)?,
                outcome: row.get(13)?,
                quota: row.get(14)?,
                started_at: row.get(15)?,
                ended_at: row.get(16)?,
                media_seconds: row.get(17)?,
                cost: row.get(18)?,
                cost_currency: row.get(19)?,
            })
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// Every line the server log of `harness` kept, oldest first, by its
/// message and its fields; read once its writer has closed.
pub(crate) fn logged(harness: &Harness) -> Vec<(String, serde_json::Value)> {
    let database = rusqlite::Connection::open(harness.data_dir().join("log.sqlite")).unwrap();
    let mut statement = database.prepare("SELECT message, fields FROM events ORDER BY id").unwrap();
    statement
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .unwrap()
        .map(|line| {
            let (message, fields) = line.unwrap();
            (message, serde_json::from_str(&fields).unwrap())
        })
        .collect()
}

/// A ledger row of `email`'s, `order` milliseconds into the day.
fn row(harness: &Harness, email: &str, provider: &str, model: &str, input: i64, order: i64) {
    harness
        .control_database()
        .execute(
            "INSERT INTO usage_ledger (id, user_id, conversation_id, provider_id, model_id,
               input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, ended_at)
             SELECT ?1, id, '0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b', ?2, ?3, ?4, 10, 2, 1, ?5 FROM users WHERE email = ?6",
            rusqlite::params![uuid::Uuid::new_v4().to_string(), provider, model, input, 1_790_000_000_000_i64 + order, email],
        )
        .unwrap();
}

#[tokio::test]
async fn totals_group_by_entry_and_model_in_first_use_order_and_the_instance_view_is_for_administrators()
 {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    harness.add_user("admin@example.test", "admin-pass-1", Role::Admin);
    harness.add_user("bob@example.test", "bob-pass-1", Role::User);
    row(
        &harness,
        "master@example.test",
        "entry-b",
        "model-2",
        100,
        1,
    );
    row(
        &harness,
        "master@example.test",
        "entry-a",
        "model-1",
        200,
        2,
    );
    row(
        &harness,
        "master@example.test",
        "entry-b",
        "model-2",
        300,
        3,
    );
    row(&harness, "bob@example.test", "entry-a", "model-1", 50, 4);

    let own = backend
        .get("/api/usage", Some(&master))
        .await
        .json::<UsageTotals>();
    let groups: Vec<(&str, &str, u64, u64, u64)> = own
        .totals
        .iter()
        .map(|group| {
            (
                group.provider_id.as_str(),
                group.model_id.as_str(),
                group.requests,
                group.input_tokens,
                group.output_tokens,
            )
        })
        .collect();
    assert_eq!(
        groups,
        [
            ("entry-b", "model-2", 2, 400, 20),
            ("entry-a", "model-1", 1, 200, 10)
        ]
    );
    assert_eq!(
        (
            own.totals[0].cache_read_tokens,
            own.totals[0].cache_write_tokens
        ),
        (4, 2)
    );

    let bob = backend.login("bob@example.test", "bob-pass-1").await;
    assert_eq!(
        backend
            .get("/api/usage", Some(&bob))
            .await
            .json::<UsageTotals>()
            .totals
            .len(),
        1
    );
    let refused = backend.get("/api/usage/instance", Some(&bob)).await;
    assert_eq!(
        refused.refusal(),
        (StatusCode::FORBIDDEN, ErrorCode::Forbidden)
    );
    let admin = backend.login("admin@example.test", "admin-pass-1").await;
    let instance = backend
        .get("/api/usage/instance", Some(&admin))
        .await
        .json::<InstanceUsage>();
    let requests: Vec<(&str, u64)> = instance
        .users
        .iter()
        .map(|user| {
            (
                user.email.as_str(),
                user.totals.iter().map(|group| group.requests).sum(),
            )
        })
        .collect();
    assert_eq!(
        requests,
        [
            ("master@example.test", 3),
            ("admin@example.test", 0),
            ("bob@example.test", 1)
        ]
    );
    backend.close().await;

    // An isolated instance has no instance view.
    let backend = harness.start_in_mode(InstanceMode::Isolated).await;
    let master = backend
        .login(
            crate::support::MASTER_EMAIL,
            crate::support::MASTER_PASSWORD,
        )
        .await;
    let refused = backend.get("/api/usage/instance", Some(&master)).await;
    assert_eq!(
        refused.refusal(),
        (StatusCode::FORBIDDEN, ErrorCode::Forbidden)
    );
    backend.close().await;
}

/// Each call of a conversation on an API key is a row as it ends, with the
/// root's agent, its turn and no account, the message's id and its counts,
/// the hour-long part of its cache write among them; a compaction's
/// summary request is a row that says so. A ledger that cannot be written
/// loses the row, and the turn goes on.
// A backend and three answers of a scripted vendor: about half a second.
#[tokio::test]
async fn each_call_is_a_row_a_compaction_says_so_and_a_ledger_that_fails_fails_no_turn() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let vendor = MockVendor::start().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    let started = json!({
        "input_tokens": 600_000, "output_tokens": 0, "cache_read_input_tokens": 0,
        "cache_creation_input_tokens": 50,
        "cache_creation": { "ephemeral_5m_input_tokens": 10, "ephemeral_1h_input_tokens": 40 },
    });
    // The answer fills most of the window, so the user may compact.
    vendor.respond(message(vec![text_block(0, &["First answer."])], "end_turn", started, 3));
    socket.chat("m1", "First question").await;
    vendor.respond(answer(&["The user asked one question."], 10, 5));
    socket.send(&ClientFrame::Compact { node: None }).await;
    socket.until_idle().await;

    let rows = rows(&harness);
    let [turn, compaction] = rows.as_slice() else {
        panic!("{rows:?}");
    };
    assert_eq!(
        (
            turn.agent,
            turn.turn_id.as_deref(),
            turn.purpose.as_deref(),
            turn.account_id.as_deref(),
            turn.provider_id.as_str(),
            turn.model_id.as_str(),
        ),
        (Some(0), Some("m1"), Some("turn"), None, provider.as_str(), "claude-opus-4-8")
    );
    assert!(turn.vendor_id.as_deref().is_some_and(|id| id.starts_with("msg_")), "{turn:?}");
    assert_eq!((turn.tokens, turn.cache_write_1h_tokens), ([Some(600_000), Some(3), Some(0), Some(50)], Some(40)));
    assert_eq!((turn.outcome.as_deref(), turn.quota.as_deref()), (Some("answered"), None));
    assert!(turn.started_at.is_some_and(|at| at <= turn.ended_at), "{turn:?}");
    assert!(turn.request_id.is_some(), "{turn:?}");
    assert_eq!(
        (
            compaction.agent,
            compaction.purpose.as_deref(),
            compaction.account_id.as_deref(),
            compaction.tokens,
            compaction.outcome.as_deref(),
        ),
        (Some(0), Some("compaction"), None, [Some(10), Some(5), Some(0), Some(0)], Some("answered"))
    );
    assert_ne!(compaction.request_id, turn.request_id);
    // The totals read the rows: one group of both calls.
    let totals = usage(&backend, &master).await.totals;
    assert_eq!((totals.len(), totals[0].requests, totals[0].input_tokens), (1, 2, 600_010));

    // The ledger refuses every row: the turn still answers.
    harness
        .control_database()
        .execute_batch(
            "CREATE TRIGGER refused BEFORE INSERT ON usage_ledger
             BEGIN SELECT RAISE(ABORT, 'the ledger is full'); END;",
        )
        .unwrap();
    vendor.respond(answer(&["Second answer."], 8, 2));
    socket.chat("m2", "Second question").await;
    let blocks = transcript(&backend, &master, FIRST).await.blocks;
    assert_eq!(last_text(&blocks), "Second answer.");
    assert!(!kinds(&blocks).iter().any(|kind| kind == "error"), "{:?}", kinds(&blocks));
    backend.close().await;
}

/// Each call of a conversation reaches its open page as a `cost` frame of
/// what its row adds, the call itself for a turn call of the main
/// conversation, and `GET /api/conversations/:id/cost` sums the rows, its
/// second call a miss once it wrote the prompt again (`usage-and-quota.md`
/// § Reading it). Each frame and the answer count the rows they include, so
/// the page can tell a frame it holds from one past a gap, and a page that
/// opens the conversation again receives the count with the handshake,
/// which tells it of a row whose frame it missed. An entry whose
/// models have no price leaves every price null and lists no agent. Planted
/// defect: no frame, which the socket waits for until the test's timeout.
// A backend and two answers of a scripted vendor: about half a second.
#[tokio::test]
async fn each_call_reaches_the_open_page_as_a_cost_frame_and_the_cost_sums_the_rows() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let vendor = MockVendor::start().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    let usage = |input: u64, write: u64, read: u64| {
        json!({
            "input_tokens": input, "output_tokens": 0, "cache_read_input_tokens": read,
            "cache_creation_input_tokens": write,
        })
    };
    let cost_frame = |frame: &ServerFrame| matches!(frame, ServerFrame::Cost { .. });
    let mut calls: Vec<CostCall> = Vec::new();
    let mut counts = Vec::new();
    for (id, (input, write, read)) in [("m1", (1_000, 20_000, 0)), ("m2", (10, 30_000, 0))] {
        vendor.respond(message(vec![text_block(0, &["Answer."])], "end_turn", usage(input, write, read), 7));
        let mut frames = socket.chat(id, "Question").await;
        // The frame follows the row's write, which may come after the turn's end.
        if !frames.iter().any(cost_frame) {
            frames = socket.until(cost_frame).await;
        }
        let Some(ServerFrame::Cost { rows, totals, agent, call }) = frames.into_iter().find(|frame| cost_frame(frame))
        else {
            unreachable!()
        };
        counts.push(rows);
        assert_eq!(
            (totals.input_tokens, totals.cache_write_tokens, totals.output_tokens, totals.price.total, agent),
            (input, write, 7, None, None)
        );
        calls.push(call.expect("a turn call of the main conversation"));
    }
    assert_eq!(
        calls.iter().map(|call| (call.cache_write_tokens, call.after_compaction, call.miss)).collect::<Vec<_>>(),
        [(20_000, false, None), (30_000, false, Some(demi_conversation_socket_protocol::CacheMiss::NotCached))]
    );

    let cost = backend
        .get(&format!("/api/conversations/{FIRST}/cost"), Some(&master))
        .await
        .json::<ConversationCost>();
    assert_eq!(
        (cost.totals.input_tokens, cost.totals.cache_write_tokens, cost.totals.output_tokens),
        (1_010, 50_000, 14)
    );
    assert_eq!((cost.calls, cost.agents.len(), cost.totals.price.total), (calls, 0, None));
    assert_eq!((counts, cost.rows), (vec![1, 2], 2));
    let mut again = Socket::connect(&backend, &master, FIRST).await;
    again.open().await;
    let opened = again.until(|frame| matches!(frame, ServerFrame::CostRows { .. })).await;
    assert_eq!(opened.last(), Some(&ServerFrame::CostRows { rows: 2 }));
    backend.close().await;
}
