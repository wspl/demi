//! The usage ledger of the control store: one row per vendor call, kept as written
//! and announced as it is, the user's totals computed when they are read, and each
//! conversation's cost kept as its rows are written.
//!
//! # A conversation's cost
//!
//! Each conversation keeps, per agent and for the rows of no agent, the totals
//! of its rows and the number of its turn calls (`usage_costs`), which the row's
//! insert adds to in the same transaction. A turn call is numbered among its
//! agent's and keeps why it missed the cache, judged from the agent's previous
//! call as it is written. So no read of a cost reads the ledger, and a page
//! reads an agent's totals with its metadata.
//!
//! # A cache chart
//!
//! Each agent's cache hit rate and the chart of it are counted here from its
//! latest turn calls, at most 400:
//! as a turn call's row is written, in its transaction, and for each agent as a
//! conversation's tree opens. The window's calls are found by the agent's index
//! from its latest row back, so a count reads the window's rows and no others.
//!
//! ```text
//!  calls  1 ··· 600 [601 ··· 610] [611 ··· 620] ··· [991 ··· 1000]
//!                    └ point 1 ┘   └ point 2 ┘        └ point 40 ┘
//!  size = ⌈min(last, 400) / 40⌉, the window starts at a point's first
//!  call, so a new call changes only the last point
//! ```
//!
//! # The last hour and the last day
//!
//! The cost of the last hour or the last day is not kept: it is summed from the
//! ledger's rows that ended in the last 60 minutes or 24 hours, per agent, when
//! a page asks. The rows of one conversation are found by
//! `usage_ledger_conversation`, ordered by their end, so a read reads that
//! window's rows of that conversation and no others.
//!
//! The counts are the vendor's, so the ledger is a record for reconciliation and
//! not a bill. Prices tiered by a request's length, as some models charge more
//! past 200K tokens, are not modelled: a call is priced at the model's base rates.

use demi_conversation_lane_protocol::{AgentCost, CacheChart, CacheMiss, ChartPoint, CostPrice, CostTotals};
use demi_shared_types::{Cost, Currency, ModelCost, QuotaKind, QuotaScope, QuotaWindow, Timestamp, TokenUsage};
use demi_web_api_protocol::conversations::{CostWindow, RecentAgent};
use demi_web_api_protocol::ids::{ConversationId, CredentialId, ProviderId, UserId};
use demi_web_api_protocol::text::EmailAddress;
use demi_web_api_protocol::usage::{UsageGroup, UserUsage};
use std::collections::BTreeMap;

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::StorageError;
use super::columns::{decode, instant, json, to_json};
use super::control::ControlService;

/// How far back the cost of a window counts.
const fn window_ms(window: CostWindow) -> i64 {
    match window {
        CostWindow::Hour => 60 * 60 * 1000,
        CostWindow::Day => 24 * 60 * 60 * 1000,
    }
}

/// The most recent turn calls of an agent a cache chart counts.
///
/// Bounded because a chart reads a row for each call, and a conversation has no
/// end of them.
const CHART_WINDOW: u64 = 400;

/// The points a cache chart draws at most: past them a point stands for
/// several calls, so a long conversation's line stays readable.
const CHART_POINTS: u64 = 40;

/// The turn calls of an agent from which its chart has points: with fewer, a
/// line says nothing a user can read and the rate alone says it.
const CHART_CALLS: usize = 4;

/// One call to a vendor: whose it was, what it was for, with which entry,
/// account and model, the vendor's counts for it alone, how it ended and
/// the account's quota observed during it.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageRow {
    pub user: UserId,
    pub conversation: ConversationId,
    /// The session's agent number, 0 for the root; none outside a session.
    pub agent: Option<u64>,
    /// None outside a turn.
    pub turn: Option<String>,
    /// `turn`, `compaction`, `title`, `image`, `video` or `audio`.
    pub purpose: &'static str,
    pub provider: ProviderId,
    /// The subscription account; none for an API key.
    pub account: Option<CredentialId>,
    pub model: String,
    /// Demi's id of the request, under which the server log holds its
    /// course.
    pub request: String,
    pub vendor_id: Option<String>,
    /// The vendor's counts for the call; none when it reported none, and
    /// then every count, the hour-long cache write included, is null.
    pub usage: Option<TokenUsage>,
    pub cache_write_1h_tokens: u64,
    /// `answered`, `cancelled` or the failure code of a refused call.
    pub outcome: String,
    /// The windows the call's own response reported; none when it
    /// reported none.
    pub quota: Option<Vec<ObservedWindow>>,
    pub started_at: Timestamp,
    pub ended_at: Timestamp,
    /// The seconds of video or audio the call made, as its file states
    /// them; none for any other call or when nothing was made.
    pub media_seconds: Option<f64>,
    /// What the vendor said the call cost; none when it said nothing in a
    /// documented unit.
    pub cost: Option<Cost>,
    /// What the call's tokens would cost on the vendor's API; none when the
    /// model has no price for a kind the call used, or the call reported no
    /// counts.
    pub price: Option<ApiPrice>,
    /// Hashes of the system prompt and of the tool definitions the request
    /// sent; none for a request outside a session.
    pub system_hash: Option<String>,
    pub tools_hash: Option<String>,
}

/// What each kind of a call's tokens would cost on the vendor's API, in
/// dollars, fixed as its row is written.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ApiPrice {
    pub input: f64,
    /// The cache write's, the part kept for an hour included.
    pub cache_write: f64,
    pub cache_read: f64,
    pub output: f64,
}

impl ApiPrice {
    /// The call's tokens at the model's catalog prices, `cost` per million tokens.
    ///
    /// - Input, cache read and cache write are priced at their rates.
    /// - The part of the write kept for an hour is priced at twice the input rate,
    ///   as the Messages API prices it.
    /// - Output is priced at its rate.
    ///
    /// None when the model has no price for a kind the call used.
    pub fn of(cost: &ModelCost, usage: &TokenUsage, cache_write_1h_tokens: u64) -> Option<Self> {
        let part = |tokens: u64, rate: Option<f64>| -> Option<f64> {
            if tokens == 0 {
                return Some(0.0);
            }
            // A count is far below 2^53, where an f64 stops being exact.
            rate.map(|rate| tokens as f64 * rate / 1_000_000.0)
        };
        let hour = cache_write_1h_tokens.min(usage.cache_write_tokens);
        let cache_write = part(usage.cache_write_tokens - hour, cost.cache_write)?
            + part(hour, cost.input.map(|rate| rate * 2.0))?;
        Some(Self {
            input: part(usage.input_tokens, cost.input)?,
            cache_write,
            cache_read: part(usage.cache_read_tokens, cost.cache_read)?,
            output: part(usage.output_tokens, cost.output)?,
        })
    }

    pub fn total(&self) -> f64 {
        self.input + self.cache_write + self.cache_read + self.output
    }
}

/// A row written to the ledger, as the control store announces it to
/// whoever shows its conversation's cost.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageWritten {
    pub user: UserId,
    pub conversation: ConversationId,
    /// The row's agent; none for a row of no agent, as a title's.
    pub agent: Option<u64>,
    /// The kept cost of that agent, or of the rows of no agent, with the
    /// row.
    pub cost: AgentCost,
    /// The agent's cache chart with the row, for a turn call; none for any
    /// other row, which leaves it as it was.
    pub chart: Option<CacheChart>,
    /// The row is the conversation's first, whichever agent wrote it.
    pub first: bool,
}

/// A window of the account's quota as a call's response reported it, as
/// the row's `quota` keeps it: named as the snapshot names a window, by its
/// kind, period and scope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObservedWindow {
    #[garde(skip)]
    pub kind: QuotaKind,
    #[serde(deserialize_with = "Option::deserialize")]
    #[garde(skip)]
    pub period_seconds: Option<u64>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[garde(skip)]
    pub scope: Option<QuotaScope>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[garde(range(min = 0.0, max = 100.0))]
    pub used_percent: Option<f64>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[garde(skip)]
    pub resets_at: Option<Timestamp>,
}

impl From<&QuotaWindow> for ObservedWindow {
    fn from(window: &QuotaWindow) -> Self {
        Self {
            kind: window.kind,
            period_seconds: window.period_seconds,
            scope: window.scope.clone(),
            used_percent: window.used_percent,
            resets_at: window.resets_at,
        }
    }
}

/// Adds `windows` to `kept`, each replacing a window of the same name, in
/// the order they were reported.
pub fn merge_windows(kept: &mut Vec<ObservedWindow>, windows: &[QuotaWindow]) {
    for window in windows {
        let window = ObservedWindow::from(window);
        let named = |kept: &ObservedWindow| {
            (kept.kind, kept.period_seconds, &kept.scope) == (window.kind, window.period_seconds, &window.scope)
        };
        match kept.iter_mut().find(|kept| named(kept)) {
            Some(kept) => *kept = window,
            None => kept.push(window),
        }
    }
}

/// One user's totals: a group per entry and model, in the order each pair
/// was first used.
/// A count the vendor did not report adds nothing.
const TOTALS: &str = "SELECT provider_id, model_id, COUNT(*) AS requests,
       COALESCE(SUM(input_tokens), 0) AS input_tokens, COALESCE(SUM(output_tokens), 0) AS output_tokens,
       COALESCE(SUM(cache_read_tokens), 0) AS cache_read_tokens,
       COALESCE(SUM(cache_write_tokens), 0) AS cache_write_tokens
     FROM usage_ledger WHERE user_id = ?1
     GROUP BY provider_id, model_id
     ORDER BY MIN(ended_at), MIN(rowid)";

/// A count as the column keeps it: a count beyond i64 is no count a vendor
/// reports, and the column keeps it as the largest it holds rather than
/// failing the row.
fn column_count(count: u64) -> i64 {
    i64::try_from(count).unwrap_or(i64::MAX)
}

impl ControlService {
    /// Writes the row, adds it to its conversation's kept cost, announces it
    /// and answers its id.
    pub async fn append_usage(&self, row: UsageRow) -> Result<String, StorageError> {
        let id = uuid::Uuid::new_v4().to_string();
        let user = row.user.clone();
        let conversation = row.conversation.clone();
        let agent = row.agent;
        let (id, cost, chart, first) = self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let call = match (row.agent, row.purpose, row.usage) {
                (Some(agent), "turn", Some(usage)) => Some(next_call(
                    &transaction,
                    row.conversation.as_str(),
                    agent,
                    &CallFacts {
                        before: i64::MAX,
                        model: &row.model,
                        usage,
                        started_at: row.started_at,
                        system_hash: row.system_hash.as_deref(),
                        tools_hash: row.tools_hash.as_deref(),
                    },
                )?),
                _ => None,
            };
            let quota = row.quota.as_ref().map(to_json);
            let counts = row.usage.map(|usage| {
                [
                    usage.input_tokens,
                    usage.output_tokens,
                    usage.cache_read_tokens,
                    usage.cache_write_tokens,
                    row.cache_write_1h_tokens,
                ]
                .map(column_count)
            });
            let count = |index: usize| counts.map(|counts| counts[index]);
            transaction.execute(
                "INSERT INTO usage_ledger (id, user_id, conversation_id, agent, turn_id, purpose,
                   provider_id, account_id, model_id, request_id, vendor_id,
                   input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
                   cache_write_1h_tokens, outcome, quota, started_at, ended_at, media_seconds, cost, cost_currency,
                   api_price_input, api_price_cache_write, api_price_cache_read, api_price_output,
                   system_hash, tools_hash, call_number, after_compaction, miss)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                   ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32)",
                params![
                    id,
                    row.user.as_str(),
                    row.conversation.as_str(),
                    row.agent.map(column_count),
                    row.turn,
                    row.purpose,
                    row.provider.as_str(),
                    row.account.as_ref().map(CredentialId::as_str),
                    row.model,
                    row.request,
                    row.vendor_id,
                    count(0),
                    count(1),
                    count(2),
                    count(3),
                    count(4),
                    row.outcome,
                    quota,
                    row.started_at.as_millisecond(),
                    row.ended_at.as_millisecond(),
                    row.media_seconds,
                    row.cost.map(|cost| cost.amount),
                    row.cost.map(|cost| cost.currency.to_string()),
                    row.price.map(|price| price.input),
                    row.price.map(|price| price.cache_write),
                    row.price.map(|price| price.cache_read),
                    row.price.map(|price| price.output),
                    row.system_hash,
                    row.tools_hash,
                    call.map(|call| column_count(call.number)),
                    call.map(|call| call.after_compaction),
                    call.and_then(|call| call.miss).map(|miss| miss.to_string()),
                ],
            )?;
            // Asked after the insert: a transaction that has read cannot wait for
            // another connection's write lock, which SQLite refuses at once
            // with BUSY, and a row that is no turn call reads nothing before
            // its insert.
            let first: bool = transaction
                .prepare_cached(
                    "SELECT NOT EXISTS (SELECT 1 FROM usage_ledger WHERE conversation_id = ?1 AND id <> ?2)",
                )?
                .query_row(params![row.conversation.as_str(), id], |row| row.get(0))?;
            let added = RowCost {
                usage: row.usage,
                parts: row.price,
                usd: row.cost.filter(|cost| cost.currency == Currency::Usd).map(|cost| cost.amount),
            };
            add_cost(&transaction, row.conversation.as_str(), row.agent, &added, call.is_some())?;
            let totals = kept_cost(&transaction, row.conversation.as_str(), row.agent)?;
            let chart = match (row.agent, call) {
                (Some(agent), Some(call)) => Some(agent_chart(&transaction, row.conversation.as_str(), agent, call.number)?),
                _ => None,
            };
            transaction.commit()?;
            Ok((id, totals, chart, first))
        })
        .await?;
        self.announce_usage(UsageWritten {
            user,
            conversation,
            agent,
            cost,
            chart,
            first,
        });
        Ok(id)
    }

    /// The conversation's kept cost: each agent's own, by number, and the
    /// rows' of no agent.
    pub async fn conversation_costs(&self, conversation: ConversationId) -> Result<Vec<KeptCost>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(
                "SELECT agent, input_tokens, cache_write_tokens, cache_read_tokens, output_tokens,
                   price_total, price_input, price_cache_write, price_cache_read, price_output, calls
                 FROM usage_costs WHERE conversation_id = ?1 ORDER BY agent",
            )?;
            let mut rows = statement.query([conversation.as_str()])?;
            let mut costs = Vec::new();
            while let Some(row) = rows.next()? {
                costs.push(agent_cost(row)?);
            }
            Ok(costs)
        })
        .await
    }

    /// When the conversation's first call ended; none before it made one.
    pub async fn conversation_first_call(&self, conversation: ConversationId) -> Result<Option<Timestamp>, StorageError> {
        self.call(move |connection, _| {
            let first: Option<i64> = connection
                .prepare_cached("SELECT MIN(ended_at) FROM usage_ledger WHERE conversation_id = ?1")?
                .query_row([conversation.as_str()], |row| row.get(0))?;
            first
                .map(|first| decode("usage_ledger", "ended_at", Timestamp::from_millisecond(first)))
                .transpose()
        })
        .await
    }

    /// What the conversation's calls that ended in `window` cost, by agent,
    /// and the share of each agent's turn calls' prompt tokens that the cache
    /// served, as of the control store's clock.
    pub async fn conversation_recent(&self, conversation: ConversationId, window: CostWindow) -> Result<Vec<RecentAgent>, StorageError> {
        self.call(move |connection, now| {
            recent(connection, conversation.as_str(), now.as_millisecond() - window_ms(window))
        })
        .await
    }

    /// The cache chart of each agent of the conversation that made a turn
    /// call, by its number.
    pub async fn conversation_charts(&self, conversation: ConversationId) -> Result<Vec<(u64, CacheChart)>, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let agents = {
                let mut statement = transaction.prepare_cached(
                    "SELECT agent, calls FROM usage_costs
                     WHERE conversation_id = ?1 AND agent IS NOT NULL AND calls > 0 ORDER BY agent",
                )?;
                let mut rows = statement.query([conversation.as_str()])?;
                let mut agents = Vec::new();
                while let Some(row) = rows.next()? {
                    let count = |column: &'static str| -> Result<u64, StorageError> {
                        decode("usage_costs", column, u64::try_from(row.get::<_, i64>(column)?))
                    };
                    agents.push((count("agent")?, count("calls")?));
                }
                agents
            };
            let mut charts = Vec::with_capacity(agents.len());
            for (agent, last) in agents {
                charts.push((agent, agent_chart(&transaction, conversation.as_str(), agent, last)?));
            }
            transaction.commit()?;
            Ok(charts)
        })
        .await
    }

    /// Adds `windows`, which the vendor reported after the call of the row
    /// `id` ended and before the next one started, to the row's quota.
    pub async fn add_usage_quota(&self, id: String, windows: Vec<QuotaWindow>) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let kept: Option<String> =
                transaction.query_row("SELECT quota FROM usage_ledger WHERE id = ?1", [&id], |row| row.get(0))?;
            let mut quota: Vec<ObservedWindow> = match kept {
                Some(text) => json("usage_ledger", "quota", &text)?,
                None => Vec::new(),
            };
            merge_windows(&mut quota, &windows);
            transaction.execute(
                "UPDATE usage_ledger SET quota = ?2 WHERE id = ?1",
                params![id, to_json(&quota)],
            )?;
            transaction.commit()?;
            Ok(())
        })
        .await
    }

    /// The user's totals.
    pub async fn usage_totals(&self, user: UserId) -> Result<Vec<UsageGroup>, StorageError> {
        self.call(move |connection, _| totals(connection, &user))
            .await
    }

    /// Every account's totals, the accounts in the order they were created,
    /// read in one transaction.
    pub async fn instance_usage(&self) -> Result<Vec<UserUsage>, StorageError> {
        self.call(|connection, _| {
            let transaction = connection.transaction()?;
            let users = {
                let mut statement = transaction
                    .prepare_cached("SELECT id, email FROM users ORDER BY created_at, rowid")?;
                let mut rows = statement.query([])?;
                let mut users = Vec::new();
                while let Some(row) = rows.next()? {
                    let id = decode("users", "id", UserId::try_from(row.get::<_, String>("id")?))?;
                    let email = decode(
                        "users",
                        "email",
                        EmailAddress::try_from(row.get::<_, String>("email")?),
                    )?;
                    users.push((id, email));
                }
                users
            };
            let mut usage = Vec::with_capacity(users.len());
            for (user_id, email) in users {
                let totals = totals(&transaction, &user_id)?;
                usage.push(UserUsage {
                    user_id,
                    email,
                    totals,
                });
            }
            transaction.commit()?;
            Ok(usage)
        })
        .await
    }
}

fn totals(connection: &Connection, user: &UserId) -> Result<Vec<UsageGroup>, StorageError> {
    let mut statement = connection.prepare_cached(TOTALS)?;
    let mut rows = statement.query([user.as_str()])?;
    let mut groups = Vec::new();
    while let Some(row) = rows.next()? {
        groups.push(group(row)?);
    }
    Ok(groups)
}

fn group(row: &Row<'_>) -> Result<UsageGroup, StorageError> {
    let count = |column: &'static str| -> Result<u64, StorageError> {
        decode(
            "usage_ledger",
            column,
            u64::try_from(row.get::<_, i64>(column)?),
        )
    };
    Ok(UsageGroup {
        provider_id: row.get("provider_id")?,
        model_id: row.get("model_id")?,
        requests: count("requests")?,
        input_tokens: count("input_tokens")?,
        output_tokens: count("output_tokens")?,
        cache_read_tokens: count("cache_read_tokens")?,
        cache_write_tokens: count("cache_write_tokens")?,
    })
}

/// A cache miss reads less of the cache than its agent's previous call did,
/// by more than this share of what that call read and by at least
/// [`MISS_TOKENS`], as Claude Code's own diagnosis of a cache break counts one.
/// A call that only adds to the prompt reads all its previous call read and
/// writes what it adds, so what it writes is no measure of a miss.
const MISS_SHARE: f64 = 0.05;
const MISS_TOKENS: u64 = 2_000;
/// How long a vendor keeps a prompt after a write, and after a write kept
/// for an hour.
const KEPT_MS: i64 = 5 * 60 * 1000;
const KEPT_HOUR_MS: i64 = 60 * 60 * 1000;

/// One conversation's kept cost of one agent, or of its rows of no agent.
#[derive(Debug, Clone, PartialEq)]
pub struct KeptCost {
    /// None for the rows of no agent, as a title's.
    pub agent: Option<u64>,
    pub cost: AgentCost,
}

/// What a turn call's row says of the call, which the next one is judged
/// against.
struct CallFacts<'a> {
    /// The rowid the call is written at, or past every row for a new one:
    /// only the rows before it are its agent's earlier ones.
    before: i64,
    model: &'a str,
    usage: TokenUsage,
    started_at: Timestamp,
    system_hash: Option<&'a str>,
    tools_hash: Option<&'a str>,
}

/// What a turn call keeps of its place among its agent's calls.
#[derive(Debug, Clone, Copy, PartialEq)]
struct KeptCall {
    /// Its number among its agent's turn calls, from 1.
    number: u64,
    after_compaction: bool,
    miss: Option<CacheMiss>,
}

/// The turn call `facts` of the conversation's agent `agent`, judged against
/// the agent's previous call: its number, whether an answered compaction of
/// the agent came between them and why it missed the cache.
///
/// A call missed when it read less of the cache than the previous call by
/// [`MISS_SHARE`] and [`MISS_TOKENS`], except the first call and the first
/// after a compaction, which start a new prompt.
fn next_call(
    connection: &Connection,
    conversation: &str,
    agent: u64,
    facts: &CallFacts<'_>,
) -> Result<KeptCall, StorageError> {
    let agent = column_count(agent);
    let previous = connection
        .prepare_cached(
            "SELECT rowid, call_number, model_id, system_hash, tools_hash, ended_at, cache_read_tokens FROM usage_ledger
             WHERE conversation_id = ?1 AND agent = ?2 AND call_number IS NOT NULL AND rowid < ?3
             ORDER BY rowid DESC LIMIT 1",
        )?
        .query_row(params![conversation, agent, facts.before], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })
        .optional()?;
    let Some((rowid, number, model, system_hash, tools_hash, ended_at, read)) = previous else {
        return Ok(KeptCall {
            number: 1,
            after_compaction: false,
            miss: None,
        });
    };
    let compacted: bool = connection
        .prepare_cached(
            "SELECT EXISTS (SELECT 1 FROM usage_ledger
               WHERE conversation_id = ?1 AND agent = ?2 AND rowid > ?3 AND rowid < ?4
                 AND purpose = 'compaction' AND outcome = 'answered')",
        )?
        .query_row(params![conversation, agent, rowid, facts.before], |row| row.get(0))?;
    let number = decode("usage_ledger", "call_number", u64::try_from(number))? + 1;
    if compacted {
        return Ok(KeptCall {
            number,
            after_compaction: true,
            miss: None,
        });
    }
    let read = decode("usage_ledger", "cache_read_tokens", u64::try_from(read))?;
    let miss = if missed(read, facts.usage.cache_read_tokens) {
        let changed = |before: Option<&str>, after: Option<&str>| matches!((before, after), (Some(before), Some(after)) if before != after);
        // Whether the latest write of the call's model was kept for an hour.
        let hour_write: Option<i64> = connection
            .prepare_cached(LATEST_CACHE_WRITE)?
            .query_row(params![conversation, agent, facts.before, facts.model], |row| row.get(0))
            .optional()?
            .flatten();
        let kept = if hour_write.is_some_and(|tokens| tokens > 0) { KEPT_HOUR_MS } else { KEPT_MS };
        Some(if model != facts.model {
            CacheMiss::ModelChanged
        } else if changed(tools_hash.as_deref(), facts.tools_hash) {
            CacheMiss::ToolsChanged
        } else if changed(system_hash.as_deref(), facts.system_hash) {
            CacheMiss::SystemPromptChanged
        } else if facts.started_at.as_millisecond() - ended_at > kept {
            CacheMiss::CacheExpired
        } else {
            CacheMiss::NotCached
        })
    } else {
        None
    };
    Ok(KeptCall {
        number,
        after_compaction: false,
        miss,
    })
}

/// Whether a call that read `read` tokens from the cache missed it, when its
/// agent's previous call read `previous`.
fn missed(previous: u64, read: u64) -> bool {
    let lost = previous.saturating_sub(read);
    lost >= MISS_TOKENS && lost as f64 > previous as f64 * MISS_SHARE
}

/// The hour-long part of the latest cache write of agent `?2` of
/// conversation `?1` by model `?4`, before rowid `?3`: found by
/// `usage_ledger_cache_writes` directly, however many rows the agent has.
const LATEST_CACHE_WRITE: &str = "SELECT cache_write_1h_tokens FROM usage_ledger
     WHERE conversation_id = ?1 AND agent = ?2 AND call_number IS NOT NULL AND rowid < ?3
       AND model_id = ?4 AND cache_write_tokens > 0
     ORDER BY rowid DESC LIMIT 1";

/// What one row adds to a cost: its counts, and its price by kind or, for a
/// call whose tokens have no price, the vendor's own cost in dollars.
struct RowCost {
    usage: Option<TokenUsage>,
    parts: Option<ApiPrice>,
    usd: Option<f64>,
}

/// The tokens of a call's prompt: those sent uncached, written to the cache
/// and read from it.
fn prompt_tokens(usage: &TokenUsage) -> u64 {
    usage.input_tokens + usage.cache_write_tokens + usage.cache_read_tokens
}

/// What a row costs: its tokens' price or, for a call whose tokens have no
/// price, the vendor's own cost in dollars; none when neither is known.
fn row_price(parts: Option<ApiPrice>, usd: Option<f64>) -> Option<f64> {
    parts.map(|parts| parts.total()).or(usd)
}

/// Adds `row` to `cost`, with one more turn call when `call`. A price that is
/// null in `cost` or in the row adds to the other: null only while no row had
/// one.
fn add_row(cost: &mut AgentCost, row: &RowCost, call: bool) {
    let usage = row.usage.unwrap_or_default();
    let totals = &mut cost.totals;
    totals.input_tokens += usage.input_tokens;
    totals.cache_write_tokens += usage.cache_write_tokens;
    totals.cache_read_tokens += usage.cache_read_tokens;
    totals.output_tokens += usage.output_tokens;
    let add = |sum: &mut Option<f64>, part: Option<f64>| {
        if let Some(part) = part {
            *sum = Some(sum.unwrap_or(0.0) + part);
        }
    };
    add(&mut totals.price.total, row_price(row.parts, row.usd));
    add(&mut totals.price.input, row.parts.map(|parts| parts.input));
    add(&mut totals.price.cache_write, row.parts.map(|parts| parts.cache_write));
    add(&mut totals.price.cache_read, row.parts.map(|parts| parts.cache_read));
    add(&mut totals.price.output, row.parts.map(|parts| parts.output));
    cost.calls += u64::from(call);
}

/// Adds `row` to the kept cost of the conversation's agent `agent`, or of its
/// rows of no agent, with one more turn call when `call`.
fn add_cost(
    connection: &Connection,
    conversation: &str,
    agent: Option<u64>,
    row: &RowCost,
    call: bool,
) -> Result<(), StorageError> {
    let mut cost = kept_cost(connection, conversation, agent)?;
    add_row(&mut cost, row, call);
    let totals = &cost.totals;
    let values = params![
        conversation,
        agent.map(column_count),
        column_count(totals.input_tokens),
        column_count(totals.cache_write_tokens),
        column_count(totals.cache_read_tokens),
        column_count(totals.output_tokens),
        totals.price.total,
        totals.price.input,
        totals.price.cache_write,
        totals.price.cache_read,
        totals.price.output,
        column_count(cost.calls),
    ];
    let updated = connection
        .prepare_cached(
            "UPDATE usage_costs SET
               input_tokens = ?3, cache_write_tokens = ?4, cache_read_tokens = ?5, output_tokens = ?6,
               price_total = ?7, price_input = ?8, price_cache_write = ?9, price_cache_read = ?10,
               price_output = ?11, calls = ?12
             WHERE conversation_id = ?1 AND agent IS ?2",
        )?
        .execute(values)?;
    if updated == 0 {
        connection
            .prepare_cached(
                "INSERT INTO usage_costs (conversation_id, agent, input_tokens, cache_write_tokens,
                   cache_read_tokens, output_tokens, price_total, price_input, price_cache_write,
                   price_cache_read, price_output, calls)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            )?
            .execute(values)?;
    }
    Ok(())
}

/// The rows of conversation `?1` that ended after `?2`, with what `row_usage`,
/// `row_prices` and the cost's call count read of a row.
const RECENT: &str = "SELECT agent, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
       cost, cost_currency, api_price_input, api_price_cache_write, api_price_cache_read, api_price_output,
       call_number
     FROM usage_ledger WHERE conversation_id = ?1 AND ended_at > ?2";

/// What the rows that ended after `since` add up to, per agent and for the
/// rows of no agent, the rows of no agent first: each row is added as the kept
/// cost adds it (`add_row`), and the share of the prompts the cache served is
/// counted over the turn calls alone.
fn recent(connection: &Connection, conversation: &str, since: i64) -> Result<Vec<RecentAgent>, StorageError> {
    // The cost, and the prompt tokens and the part the cache read.
    let mut agents: BTreeMap<Option<u64>, (AgentCost, u64, u64)> = BTreeMap::new();
    let mut statement = connection.prepare_cached(RECENT)?;
    let mut rows = statement.query(params![conversation, since])?;
    while let Some(row) = rows.next()? {
        let agent = row
            .get::<_, Option<i64>>("agent")?
            .map(|agent| decode("usage_ledger", "agent", u64::try_from(agent)))
            .transpose()?;
        let usage = row_usage(row)?;
        let (parts, usd) = row_prices(row)?;
        let call = row.get::<_, Option<i64>>("call_number")?.is_some();
        let (cost, prompt, read) = agents.entry(agent).or_default();
        add_row(cost, &RowCost { usage, parts, usd }, call);
        if let (true, Some(usage)) = (call, usage) {
            *prompt += prompt_tokens(&usage);
            *read += usage.cache_read_tokens;
        }
    }
    Ok(agents
        .into_iter()
        .map(|(agent, (cost, prompt, read))| RecentAgent { agent, cost, rate: read_share(read, prompt) })
        .collect())
}

/// The counts of a row of the ledger: none when the vendor reported none.
fn row_usage(row: &Row<'_>) -> Result<Option<TokenUsage>, StorageError> {
    let count = |column: &'static str| -> Result<Option<u64>, StorageError> {
        row.get::<_, Option<i64>>(column)?
            .map(|count| decode("usage_ledger", column, u64::try_from(count)))
            .transpose()
    };
    match [
        count("input_tokens")?,
        count("output_tokens")?,
        count("cache_read_tokens")?,
        count("cache_write_tokens")?,
    ] {
        [Some(input_tokens), Some(output_tokens), Some(cache_read_tokens), Some(cache_write_tokens)] => Ok(Some(TokenUsage {
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
        })),
        // A call the vendor reported no counts for.
        [None, None, None, None] => Ok(None),
        _ => Err(corrupt("input_tokens", "some of the token counts only")),
    }
}

/// What a row of the ledger says it would cost: the price of each kind of its
/// tokens, and the vendor's own cost when that is in dollars.
fn row_prices(row: &Row<'_>) -> Result<(Option<ApiPrice>, Option<f64>), StorageError> {
    let parts = match [
        row.get::<_, Option<f64>>("api_price_input")?,
        row.get("api_price_cache_write")?,
        row.get("api_price_cache_read")?,
        row.get("api_price_output")?,
    ] {
        [Some(input), Some(cache_write), Some(cache_read), Some(output)] => Some(ApiPrice {
            input,
            cache_write,
            cache_read,
            output,
        }),
        [None, None, None, None] => None,
        _ => return Err(corrupt("api_price_input", "some of the price's parts only")),
    };
    let currency: Option<String> = row.get("cost_currency")?;
    let usd = row
        .get::<_, Option<f64>>("cost")?
        .filter(|_| currency == Some(Currency::Usd.to_string()));
    Ok((parts, usd))
}

/// The kept cost of the conversation's agent `agent`, or of its rows of no
/// agent; nothing before its first row.
fn kept_cost(connection: &Connection, conversation: &str, agent: Option<u64>) -> Result<AgentCost, StorageError> {
    let kept = connection
        .prepare_cached(
            "SELECT agent, input_tokens, cache_write_tokens, cache_read_tokens, output_tokens,
               price_total, price_input, price_cache_write, price_cache_read, price_output, calls
             FROM usage_costs WHERE conversation_id = ?1 AND agent IS ?2",
        )?
        .query_row(params![conversation, agent.map(column_count)], |row| Ok(agent_cost(row)))
        .optional()?;
    Ok(match kept {
        Some(kept) => kept?.cost,
        None => AgentCost::default(),
    })
}

fn agent_cost(row: &Row<'_>) -> Result<KeptCost, StorageError> {
    let count = |column: &'static str| -> Result<u64, StorageError> {
        decode("usage_costs", column, u64::try_from(row.get::<_, i64>(column)?))
    };
    let agent: Option<i64> = row.get("agent")?;
    Ok(KeptCost {
        agent: agent
            .map(|agent| decode("usage_costs", "agent", u64::try_from(agent)))
            .transpose()?,
        cost: AgentCost {
            totals: CostTotals {
                input_tokens: count("input_tokens")?,
                cache_write_tokens: count("cache_write_tokens")?,
                cache_read_tokens: count("cache_read_tokens")?,
                output_tokens: count("output_tokens")?,
                price: CostPrice {
                    total: row.get("price_total")?,
                    input: row.get("price_input")?,
                    cache_write: row.get("price_cache_write")?,
                    cache_read: row.get("price_cache_read")?,
                    output: row.get("price_output")?,
                },
            },
            calls: count("calls")?,
        },
    })
}

/// A turn call's row as a cache chart counts it.
#[derive(Debug, Clone, PartialEq)]
struct ChartCall {
    /// Its number among its agent's turn calls, from 1.
    number: u64,
    started_at: Timestamp,
    usage: TokenUsage,
    /// Its tokens' price, or the vendor's own cost in dollars; none when
    /// neither is known.
    price: Option<f64>,
    after_compaction: bool,
    miss: Option<CacheMiss>,
    /// The tokens its agent's previous call read from the cache and it did
    /// not; 0 for the agent's first.
    lost: u64,
}

/// The calls each point stands for and the number of the first call a
/// cache chart counts, when its agent's latest turn call is `last`.
fn chart_window(last: u64) -> (u64, u64) {
    let size = last.min(CHART_WINDOW).div_ceil(CHART_POINTS).max(1);
    let first = last.div_ceil(size).saturating_sub(CHART_POINTS) * size + 1;
    (size, first)
}

/// The cache chart of the conversation's agent `agent`, whose latest turn
/// call is `last`.
fn agent_chart(connection: &Connection, conversation: &str, agent: u64, last: u64) -> Result<CacheChart, StorageError> {
    let (size, first) = chart_window(last);
    // The window's calls are the agent's latest turn calls, so the scan
    // back from its latest row stops at the window's first, or at the call
    // before it, which says what the first one lost.
    let from = first.saturating_sub(1).max(1);
    let mut statement = connection.prepare_cached(
        "SELECT call_number, started_at, ended_at, input_tokens, output_tokens, cache_write_tokens, cache_read_tokens,
           api_price_input, api_price_cache_write, api_price_cache_read, api_price_output,
           cost, cost_currency, after_compaction, miss
         FROM usage_ledger
         WHERE conversation_id = ?1 AND agent = ?2 AND call_number >= ?3
         ORDER BY rowid DESC LIMIT ?4",
    )?;
    let mut rows = statement.query(params![
        conversation,
        column_count(agent),
        column_count(from),
        column_count(last.saturating_sub(from) + 1)
    ])?;
    let mut calls = Vec::new();
    while let Some(row) = rows.next()? {
        calls.push(chart_call(row)?);
    }
    calls.reverse();
    for index in 1..calls.len() {
        calls[index].lost = calls[index - 1].usage.cache_read_tokens.saturating_sub(calls[index].usage.cache_read_tokens);
    }
    if calls.first().is_some_and(|call| call.number < first) {
        calls.remove(0);
    }
    Ok(count_chart(size, &calls))
}

/// The share of `calls`' prompt tokens read from the cache; none when they
/// sent no prompt.
fn hit_rate(calls: &[ChartCall]) -> Option<f64> {
    let prompt: u64 = calls.iter().map(|call| prompt_tokens(&call.usage)).sum();
    let read: u64 = calls.iter().map(|call| call.usage.cache_read_tokens).sum();
    read_share(read, prompt)
}

/// The share of `prompt` tokens that `read` of them were read from the
/// cache; none when no prompt was sent.
fn read_share(read: u64, prompt: u64) -> Option<f64> {
    // A count is far below 2^53, where an f64 stops being exact.
    (prompt > 0).then(|| read as f64 / prompt as f64)
}

/// The chart of a window's `calls`, oldest first, each point of `size`
/// consecutive calls counted from the agent's call 1.
fn count_chart(size: u64, calls: &[ChartCall]) -> CacheChart {
    let start = |call: &ChartCall| (call.number - 1) / size * size + 1;
    let points = if calls.len() < CHART_CALLS {
        Vec::new()
    } else {
        calls
            .chunk_by(|a, b| start(a) == start(b))
            .map(|group| {
                let (head, tail) = (&group[0], &group[group.len() - 1]);
                let mut misses = Vec::new();
                for miss in group.iter().filter_map(|call| call.miss) {
                    if !misses.contains(&miss) {
                        misses.push(miss);
                    }
                }
                let prices: Vec<f64> = group.iter().filter_map(|call| call.price).collect();
                ChartPoint {
                    first: start(head),
                    last: tail.number,
                    rate: hit_rate(group).unwrap_or(0.0),
                    from: head.started_at,
                    to: tail.started_at,
                    misses,
                    after_compaction: group.iter().any(|call| call.after_compaction),
                    price: (!prices.is_empty()).then(|| prices.iter().sum()),
                    rewritten: (group.len() == 1 && head.miss.is_some()).then_some(head.lost),
                }
            })
            .collect()
    };
    CacheChart {
        rate: hit_rate(calls),
        size,
        points,
    }
}

/// A turn call's row as a cache chart counts it.
fn chart_call(row: &Row<'_>) -> Result<ChartCall, StorageError> {
    let count = |column: &'static str| -> Result<u64, StorageError> {
        decode("usage_ledger", column, u64::try_from(row.get::<_, i64>(column)?))
    };
    let usage = row_usage(row)?.ok_or_else(|| corrupt("input_tokens", "no token counts for a turn call"))?;
    let (parts, usd) = row_prices(row)?;
    let ended_at = instant(row, "usage_ledger", "ended_at")?;
    let started_at = match row.get::<_, Option<i64>>("started_at")? {
        Some(_) => instant(row, "usage_ledger", "started_at")?,
        // A row written before the ledger kept a call's start.
        None => ended_at,
    };
    let miss = row
        .get::<_, Option<String>>("miss")?
        .map(|miss| decode("usage_ledger", "miss", miss.parse::<CacheMiss>()))
        .transpose()?;
    Ok(ChartCall {
        number: count("call_number")?,
        started_at,
        usage,
        price: row_price(parts, usd),
        after_compaction: row.get("after_compaction")?,
        miss,
        lost: 0,
    })
}

fn corrupt(column: &'static str, problem: &'static str) -> StorageError {
    StorageError::Corrupt {
        table: "usage_ledger",
        column,
        reason: format!("the row sets {problem}"),
    }
}
