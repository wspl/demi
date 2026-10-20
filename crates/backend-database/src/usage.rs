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
//! call as it is written. So no read of a cost, and no new row, reads more than
//! a few rows of the ledger: a page reads an agent's totals with its metadata,
//! and the recent calls behind its cache chart only when the chart opens.
//!
//! The counts are the vendor's, so the ledger is a record for reconciliation and
//! not a bill. Prices tiered by a request's length, as some models charge more
//! past 200K tokens, are not modelled: a call is priced at the model's base rates.

use demi_conversation_socket_protocol::{AgentCost, CacheMiss, CostCall, CostPrice, CostTotals};
use demi_shared_types::{Cost, Currency, ModelCost, QuotaKind, QuotaScope, QuotaWindow, Timestamp, TokenUsage};
use demi_web_api_protocol::ids::{ConversationId, CredentialId, ProviderId, UserId};
use demi_web_api_protocol::text::EmailAddress;
use demi_web_api_protocol::usage::{UsageGroup, UserUsage};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::StorageError;
use super::columns::{decode, instant, json, to_json};
use super::control::ControlService;

/// How many of an agent's latest turn calls a cache chart reads at most.
pub const CHART_CALLS: u64 = 400;

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
        let (id, totals) = self.call(move |connection, _| {
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
            let added = RowCost {
                usage: row.usage,
                parts: row.price,
                usd: row.cost.filter(|cost| cost.currency == Currency::Usd).map(|cost| cost.amount),
            };
            add_cost(&transaction, row.conversation.as_str(), row.agent, &added, call.is_some())?;
            let totals = kept_cost(&transaction, row.conversation.as_str(), row.agent)?;
            transaction.commit()?;
            Ok((id, totals))
        })
        .await?;
        self.announce_usage(UsageWritten {
            user,
            conversation,
            agent,
            cost: totals,
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

    /// The turn calls of the conversation's agent `agent` numbered after
    /// `after`, oldest first: at most the latest `limit` of them, which the
    /// cache chart draws.
    pub async fn agent_calls(
        &self,
        conversation: ConversationId,
        agent: u64,
        after: u64,
        limit: u64,
    ) -> Result<Vec<CostCall>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(
                "SELECT call_number, started_at, ended_at, input_tokens, cache_write_tokens, cache_read_tokens,
                   api_price_input, api_price_cache_write, api_price_cache_read, api_price_output,
                   cost, cost_currency, after_compaction, miss
                 FROM usage_ledger
                 WHERE conversation_id = ?1 AND agent = ?2 AND call_number > ?3
                 ORDER BY rowid DESC LIMIT ?4",
            )?;
            let mut rows = statement.query(params![
                conversation.as_str(),
                column_count(agent),
                column_count(after),
                column_count(limit)
            ])?;
            let mut calls = Vec::new();
            while let Some(row) = rows.next()? {
                calls.push(cost_call(row)?);
            }
            calls.reverse();
            Ok(calls)
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

/// A cache miss writes again more than this share of the call's prompt.
const MISS_SHARE: f64 = 0.05;
/// And at least this many tokens, as Claude Code's own cache diagnosis
/// counts one.
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
/// A call missed when it wrote again more than [`MISS_SHARE`] of its prompt and
/// at least [`MISS_TOKENS`], except the first call and the first after a
/// compaction, which start a new prompt.
fn next_call(
    connection: &Connection,
    conversation: &str,
    agent: u64,
    facts: &CallFacts<'_>,
) -> Result<KeptCall, StorageError> {
    let agent = column_count(agent);
    let previous = connection
        .prepare_cached(
            "SELECT rowid, call_number, model_id, system_hash, tools_hash, ended_at FROM usage_ledger
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
            ))
        })
        .optional()?;
    let Some((rowid, number, model, system_hash, tools_hash, ended_at)) = previous else {
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
    let usage = facts.usage;
    let written = usage.input_tokens + usage.cache_write_tokens;
    let prompt = written + usage.cache_read_tokens;
    let missed = written >= MISS_TOKENS && written as f64 > prompt as f64 * MISS_SHARE;
    let miss = if missed {
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

/// Adds `row` to the kept cost of the conversation's agent `agent`, or of its
/// rows of no agent, with one more turn call when `call`.
fn add_cost(
    connection: &Connection,
    conversation: &str,
    agent: Option<u64>,
    row: &RowCost,
    call: bool,
) -> Result<(), StorageError> {
    let usage = row.usage.unwrap_or_default();
    let price = row.parts.map(|parts| parts.total()).or(row.usd);
    let values = params![
        conversation,
        agent.map(column_count),
        column_count(usage.input_tokens),
        column_count(usage.cache_write_tokens),
        column_count(usage.cache_read_tokens),
        column_count(usage.output_tokens),
        price,
        row.parts.map(|parts| parts.input),
        row.parts.map(|parts| parts.cache_write),
        row.parts.map(|parts| parts.cache_read),
        row.parts.map(|parts| parts.output),
        i64::from(call),
    ];
    // A price null in the kept cost or in the row adds to the other: null
    // only while no row had one.
    let updated = connection.prepare_cached(
        "UPDATE usage_costs SET
           input_tokens = input_tokens + ?3, cache_write_tokens = cache_write_tokens + ?4,
           cache_read_tokens = cache_read_tokens + ?5, output_tokens = output_tokens + ?6,
           price_total = CASE WHEN ?7 IS NULL THEN price_total ELSE coalesce(price_total, 0) + ?7 END,
           price_input = CASE WHEN ?8 IS NULL THEN price_input ELSE coalesce(price_input, 0) + ?8 END,
           price_cache_write = CASE WHEN ?9 IS NULL THEN price_cache_write ELSE coalesce(price_cache_write, 0) + ?9 END,
           price_cache_read = CASE WHEN ?10 IS NULL THEN price_cache_read ELSE coalesce(price_cache_read, 0) + ?10 END,
           price_output = CASE WHEN ?11 IS NULL THEN price_output ELSE coalesce(price_output, 0) + ?11 END,
           calls = calls + ?12
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

/// A turn call's row as the cache chart plots it.
fn cost_call(row: &Row<'_>) -> Result<CostCall, StorageError> {
    let count = |column: &'static str| -> Result<u64, StorageError> {
        decode("usage_ledger", column, u64::try_from(row.get::<_, i64>(column)?))
    };
    let parts = match [
        row.get::<_, Option<f64>>("api_price_input")?,
        row.get("api_price_cache_write")?,
        row.get("api_price_cache_read")?,
        row.get("api_price_output")?,
    ] {
        [Some(input), Some(cache_write), Some(cache_read), Some(output)] => Some(input + cache_write + cache_read + output),
        [None, None, None, None] => None,
        _ => return Err(corrupt("api_price_input", "some of the price's parts only")),
    };
    let currency: Option<String> = row.get("cost_currency")?;
    let usd = row
        .get::<_, Option<f64>>("cost")?
        .filter(|_| currency == Some(Currency::Usd.to_string()));
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
    Ok(CostCall {
        number: count("call_number")?,
        started_at,
        input_tokens: count("input_tokens")?,
        cache_write_tokens: count("cache_write_tokens")?,
        cache_read_tokens: count("cache_read_tokens")?,
        price: parts.or(usd),
        after_compaction: row.get("after_compaction")?,
        miss,
    })
}

fn corrupt(column: &'static str, problem: &'static str) -> StorageError {
    StorageError::Corrupt {
        table: "usage_ledger",
        column,
        reason: format!("the row sets {problem}"),
    }
}

/// Numbers every turn call of the ledger, judges each against its agent's
/// previous one and keeps each conversation's cost, as writing the rows one
/// by one in their order would: the migration to the ledger that keeps
/// them (`crates/backend-database/src/schema.rs`).
pub(crate) fn keep_costs(connection: &Connection) -> Result<(), StorageError> {
    let mut rows = connection.prepare(
        "SELECT rowid, conversation_id, agent, purpose, model_id, input_tokens, output_tokens,
           cache_read_tokens, cache_write_tokens, cache_write_1h_tokens, started_at, ended_at,
           cost, cost_currency, api_price_input, api_price_cache_write, api_price_cache_read,
           api_price_output, system_hash, tools_hash
         FROM usage_ledger ORDER BY rowid",
    )?;
    let mut rows = rows.query([])?;
    let mut number = connection.prepare(
        "UPDATE usage_ledger SET call_number = ?2, after_compaction = ?3, miss = ?4 WHERE rowid = ?1",
    )?;
    while let Some(row) = rows.next()? {
        let rowid: i64 = row.get("rowid")?;
        // The id is a key here, which the ledger of any release holds as
        // text.
        let conversation: String = row.get("conversation_id")?;
        let agent = row
            .get::<_, Option<i64>>("agent")?
            .map(|agent| decode("usage_ledger", "agent", u64::try_from(agent)))
            .transpose()?;
        let count = |column: &'static str| -> Result<Option<u64>, StorageError> {
            row.get::<_, Option<i64>>(column)?
                .map(|count| decode("usage_ledger", column, u64::try_from(count)))
                .transpose()
        };
        let usage = match [
            count("input_tokens")?,
            count("output_tokens")?,
            count("cache_read_tokens")?,
            count("cache_write_tokens")?,
        ] {
            [Some(input_tokens), Some(output_tokens), Some(cache_read_tokens), Some(cache_write_tokens)] => Some(TokenUsage {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_tokens,
            }),
            // A call the vendor reported no counts for.
            [None, None, None, None] => None,
            _ => return Err(corrupt("input_tokens", "some of the token counts only")),
        };
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
        let purpose: Option<String> = row.get("purpose")?;
        let call = match (agent, purpose.as_deref(), usage) {
            (Some(agent), Some("turn"), Some(usage)) => {
                let model: String = row.get("model_id")?;
                let system_hash: Option<String> = row.get("system_hash")?;
                let tools_hash: Option<String> = row.get("tools_hash")?;
                let ended_at = instant(row, "usage_ledger", "ended_at")?;
                let started_at = match row.get::<_, Option<i64>>("started_at")? {
                    Some(_) => instant(row, "usage_ledger", "started_at")?,
                    None => ended_at,
                };
                Some(next_call(
                    connection,
                    &conversation,
                    agent,
                    &CallFacts {
                        before: rowid,
                        model: &model,
                        usage,
                        started_at,
                        system_hash: system_hash.as_deref(),
                        tools_hash: tools_hash.as_deref(),
                    },
                )?)
            }
            _ => None,
        };
        if let Some(call) = call {
            number.execute(params![
                rowid,
                column_count(call.number),
                call.after_compaction,
                call.miss.map(|miss| miss.to_string())
            ])?;
        }
        add_cost(connection, &conversation, agent, &RowCost { usage, parts, usd }, call.is_some())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use demi_conversation_socket_protocol::CacheMiss;
    use demi_shared_types::{Cost, Currency, SystemClock, Timestamp, TokenUsage};
    use demi_web_api_protocol::ids::{ConversationId, ProviderId};

    use super::{ApiPrice, UsageRow};
    use crate::control::{ControlService, testing};
    use crate::providers::NoSealedConfigs;

    const MINUTE: i64 = 60 * 1000;

    /// A call of the main conversation's turn: `input` uncached, `write`
    /// written to the cache, `hour` of it for an hour, `read` read from it,
    /// with the model and prompt hashes named, from `start` to a second
    /// later, in milliseconds.
    struct Call {
        agent: Option<u64>,
        purpose: &'static str,
        model: &'static str,
        system: &'static str,
        tools: &'static str,
        usage: Option<TokenUsage>,
        hour: u64,
        start: i64,
        price: Option<ApiPrice>,
        cost: Option<Cost>,
    }

    fn turn(input: u64, write: u64, read: u64, start: i64) -> Call {
        Call {
            agent: Some(0),
            purpose: "turn",
            model: "model-1",
            system: "system-1",
            tools: "tools-1",
            usage: Some(TokenUsage {
                input_tokens: input,
                output_tokens: 10,
                cache_read_tokens: read,
                cache_write_tokens: write,
            }),
            hour: 0,
            start,
            price: None,
            cost: None,
        }
    }

    async fn write(control: &ControlService, user: &demi_web_api_protocol::ids::UserId, calls: Vec<Call>) {
        for call in calls {
            let at = |ms: i64| Timestamp::from_millisecond(1_790_000_000_000 + ms).unwrap();
            control
                .append_usage(UsageRow {
                    user: user.clone(),
                    conversation: ConversationId::try_from(CONVERSATION).unwrap(),
                    agent: call.agent,
                    turn: Some("turn-1".into()),
                    purpose: call.purpose,
                    provider: ProviderId::try_from("entry-1").unwrap(),
                    account: None,
                    model: call.model.into(),
                    request: "request-1".into(),
                    vendor_id: None,
                    usage: call.usage,
                    cache_write_1h_tokens: call.hour,
                    outcome: if call.usage.is_some() { "answered" } else { "cancelled" }.into(),
                    quota: None,
                    started_at: at(call.start),
                    ended_at: at(call.start + 1_000),
                    media_seconds: None,
                    cost: call.cost,
                    price: call.price,
                    system_hash: Some(call.system.into()),
                    tools_hash: Some(call.tools.into()),
                })
                .await
                .unwrap();
        }
    }

    const CONVERSATION: &str = "0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b";

    async fn ledger() -> (tempfile::TempDir, ControlService, demi_web_api_protocol::ids::UserId) {
        let data = tempfile::tempdir().unwrap();
        let control = ControlService::open(
            &data.path().join("control.sqlite"),
            Arc::new(SystemClock),
            Arc::new(NoSealedConfigs),
        )
        .await
        .unwrap();
        let master = testing::master(&control).await;
        (data, control, master.id)
    }

    /// A call of the main conversation that wrote again more than 5% of its prompt
    /// and at least 2,000 tokens is a miss, with the first cause that applies.
    ///
    /// The causes, in order:
    ///
    /// 1. The model.
    /// 2. The tools.
    /// 3. The system prompt.
    /// 4. The time since the previous call past the vendor's five minutes or, after
    ///    the same model's latest write was kept for an hour, its hour, across a
    ///    switch of model and a compaction.
    /// 5. Otherwise the provider.
    ///
    /// The first call and the first after an answered compaction are never misses,
    /// and a call the vendor reported no counts for is no call.
    ///
    /// Planted defects: a threshold of share or tokens alone, the causes in another
    /// order, the hour-long write ignored or taken from another model, or a miss
    /// after a compaction.
    ///
    /// Twenty rows of an in-memory ledger: milliseconds.
    #[tokio::test]
    async fn misses_are_the_rewrites_past_the_threshold_each_with_its_first_cause() {
        let (_data, control, user) = ledger().await;
        let mut calls = vec![
            // The first call writes the whole prompt.
            turn(5_000, 40_000, 0, 0),
            turn(100, 500, 45_000, MINUTE),
            Call { model: "model-2", ..turn(100, 46_000, 0, 2 * MINUTE) },
            Call { model: "model-2", tools: "tools-2", ..turn(100, 46_000, 0, 3 * MINUTE) },
            Call { model: "model-2", tools: "tools-2", system: "system-2", ..turn(100, 46_000, 0, 4 * MINUTE) },
            // Six minutes after the previous call's end.
            Call { model: "model-2", tools: "tools-2", system: "system-2", ..turn(100, 46_000, 0, 10 * MINUTE + 1_000) },
            Call { model: "model-2", tools: "tools-2", system: "system-2", ..turn(100, 46_000, 0, 11 * MINUTE) },
        ];
        let same = |call: Call| Call { model: "model-2", tools: "tools-2", system: "system-2", ..call };
        calls.extend([
            // Wrote the whole prompt, but fewer than 2,000 tokens.
            same(turn(1_999, 0, 0, 12 * MINUTE)),
            // Wrote 3,000, under 5% of the prompt.
            same(turn(1_000, 2_000, 100_000, 13 * MINUTE)),
            // Cancelled before the vendor counted anything.
            Call { usage: None, ..same(turn(0, 0, 0, 14 * MINUTE)) },
            Call { purpose: "compaction", ..same(turn(50_000, 0, 0, 15 * MINUTE)) },
            same(turn(100, 50_000, 0, 16 * MINUTE)),
            Call { hour: 3_000, ..same(turn(0, 3_000, 100_000, 17 * MINUTE)) },
            // Thirty minutes later the hour-long write is still kept.
            same(turn(100, 50_000, 0, 48 * MINUTE)),
            // Another model writes for an hour, and a third, which reports
            // no cache writes, as OpenAI's, comes six minutes after its own
            // previous call: the hour is the other model's.
            Call { model: "model-3", hour: 3_000, ..same(turn(100, 50_000, 0, 50 * MINUTE)) },
            Call { model: "model-4", ..same(turn(50_000, 0, 0, 51 * MINUTE)) },
            Call { model: "model-4", ..same(turn(50_000, 0, 0, 57 * MINUTE + 2_000)) },
            // After a compaction the model's latest write is the five-minute
            // one the new prompt made, not the hour-long one before it.
            Call { purpose: "compaction", model: "model-3", ..same(turn(50_000, 0, 0, 58 * MINUTE)) },
            Call { model: "model-3", ..same(turn(100, 50_000, 0, 59 * MINUTE)) },
            Call { model: "model-3", ..same(turn(100, 50_000, 0, 65 * MINUTE + 2_000)) },
        ]);
        write(&control, &user, calls).await;

        let calls = control
            .agent_calls(ConversationId::try_from(CONVERSATION).unwrap(), 0, 0, 100)
            .await
            .unwrap();
        let read: Vec<(bool, Option<CacheMiss>)> =
            calls.iter().map(|call| (call.after_compaction, call.miss)).collect();
        assert_eq!(
            read,
            [
                (false, None),
                (false, None),
                (false, Some(CacheMiss::ModelChanged)),
                (false, Some(CacheMiss::ToolsChanged)),
                (false, Some(CacheMiss::SystemPromptChanged)),
                (false, Some(CacheMiss::CacheExpired)),
                (false, Some(CacheMiss::NotCached)),
                (false, None),
                (false, None),
                (true, None),
                (false, None),
                (false, Some(CacheMiss::NotCached)),
                (false, Some(CacheMiss::ModelChanged)),
                (false, Some(CacheMiss::ModelChanged)),
                (false, Some(CacheMiss::CacheExpired)),
                (true, None),
                (false, Some(CacheMiss::CacheExpired)),
            ]
        );
        control.close().await.unwrap();
    }

    /// The kept cost of each agent sums its rows as they are written,
    /// compactions and media included, with the rows of no agent, such as a
    /// title's, kept apart.
    ///
    /// Each holds the counts, each kind's price, and the price, which for a
    /// call whose tokens have no price is the vendor's own cost in dollars and
    /// never one in another currency, and how many turn calls it made; an
    /// agent's calls are its turns alone.
    ///
    /// Planted defects: a cost in yuan counted as dollars, a title counted as
    /// an agent, or a subagent's call among the main conversation's.
    ///
    /// Six rows of an in-memory ledger: milliseconds.
    #[tokio::test]
    async fn each_agent_keeps_the_cost_of_its_rows_as_they_are_written() {
        let (_data, control, user) = ledger().await;
        let price = |input, cache_write, cache_read, output| {
            Some(ApiPrice {
                input,
                cache_write,
                cache_read,
                output,
            })
        };
        write(
            &control,
            &user,
            vec![
                Call { price: price(0.25, 0.5, 0.125, 0.125), ..turn(1_000, 2_000, 3_000, 0) },
                Call { agent: Some(1), price: price(1.0, 2.0, 3.0, 4.0), ..turn(10, 20, 30, MINUTE) },
                Call { agent: None, purpose: "title", price: price(0.5, 0.0, 0.0, 0.5), ..turn(1, 0, 0, MINUTE) },
                Call {
                    agent: Some(2),
                    purpose: "image",
                    usage: None,
                    cost: Some(Cost { amount: 2.0, currency: Currency::Usd }),
                    ..turn(0, 0, 0, MINUTE)
                },
                Call {
                    agent: Some(2),
                    purpose: "video",
                    usage: None,
                    cost: Some(Cost { amount: 9.0, currency: Currency::Cny }),
                    ..turn(0, 0, 0, MINUTE)
                },
                // A subagent whose model has no price.
                Call { agent: Some(3), ..turn(5, 5, 5, MINUTE) },
            ],
        )
        .await;
        let conversation = ConversationId::try_from(CONVERSATION).unwrap();
        let costs = control.conversation_costs(conversation.clone()).await.unwrap();
        let kept: Vec<(Option<u64>, u64, u64, Option<f64>, u64)> = costs
            .iter()
            .map(|kept| {
                let totals = &kept.cost.totals;
                (kept.agent, totals.input_tokens, totals.output_tokens, totals.price.total, kept.cost.calls)
            })
            .collect();
        assert_eq!(
            kept,
            [
                (None, 1, 10, Some(1.0), 0),
                (Some(0), 1_000, 10, Some(1.0), 1),
                (Some(1), 10, 10, Some(10.0), 1),
                (Some(2), 0, 0, Some(2.0), 0),
                (Some(3), 5, 10, None, 1),
            ]
        );
        let sum = |part: fn(&super::KeptCost) -> Option<f64>| -> f64 { costs.iter().filter_map(part).sum() };
        assert_eq!(
            (
                sum(|kept| kept.cost.totals.price.input),
                sum(|kept| kept.cost.totals.price.cache_write),
                sum(|kept| kept.cost.totals.price.cache_read),
                sum(|kept| kept.cost.totals.price.output)
            ),
            (1.75, 2.5, 3.125, 4.625)
        );

        // One agent's calls: its turn call alone.
        let calls = control.agent_calls(conversation, 1, 0, 100).await.unwrap();
        assert_eq!(calls.iter().map(|call| call.price).collect::<Vec<_>>(), [Some(10.0)]);
        control.close().await.unwrap();
    }

    /// A new call finds its agent's latest cache write by its model through
    /// an index, never by reading the agent's earlier rows one by one: a
    /// call and each row of the 0.1.37 migration judge their miss by it.
    #[test]
    fn the_latest_cache_write_is_found_through_its_index() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("control.sqlite");
        let mut connection = rusqlite::Connection::open(&path).unwrap();
        crate::schema::CONTROL.apply(&mut connection, &path, &crate::schema::NoBlobs).unwrap();
        let plan: Vec<String> = connection
            .prepare(&format!("EXPLAIN QUERY PLAN {}", super::LATEST_CACHE_WRITE))
            .unwrap()
            .query_map(rusqlite::params!["c1", 1, 100, "m1"], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(
            plan.iter().any(|step| step.contains("USING INDEX usage_ledger_cache_writes (conversation_id=? AND agent=? AND model_id=? AND rowid<?)")),
            "{plan:?}"
        );
    }
}
