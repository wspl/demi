//! The usage ledger of the control store: one row per vendor call, kept as written
//! and announced as it is, and the totals and a conversation's cost computed when
//! they are read.
//!
//! The counts are the vendor's, so the ledger is a record for reconciliation and
//! not a bill. Prices tiered by a request's length, as some models charge more
//! past 200K tokens, are not modelled: a call is priced at the model's base rates.

use std::collections::{BTreeMap, HashMap};

use demi_conversation_socket_protocol::{AgentCost, CacheMiss, CostCall, CostTotals};
use demi_shared_types::{Cost, Currency, ModelCost, QuotaKind, QuotaScope, QuotaWindow, Timestamp, TokenUsage};
use demi_web_api_protocol::ids::{ConversationId, CredentialId, ProviderId, UserId};
use demi_web_api_protocol::text::EmailAddress;
use demi_web_api_protocol::usage::{ConversationCost, UsageGroup, UserUsage};
use rusqlite::{Connection, Row, params};
use serde::{Deserialize, Serialize};

use super::StorageError;
use super::columns::{decode, instant, json, to_json};
use super::control::ControlService;
use super::tree::AgentLabel;

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageWritten {
    pub user: UserId,
    pub conversation: ConversationId,
    /// The row's id.
    pub id: String,
}

/// What one row adds to its conversation's cost, as the `cost` frame
/// carries it.
#[derive(Debug, Clone, PartialEq)]
pub struct CostAddition {
    /// The number of the conversation's rows up to this one, in the order
    /// they were written, so a page knows whether it holds the row already
    /// or missed one before it.
    pub rows: u64,
    pub totals: CostTotals,
    /// The row's agent with the row's price; none for a row of no agent or
    /// without a price.
    pub agent: Option<AgentCost>,
    /// The row as a turn call of the main conversation; none for any other.
    pub call: Option<CostCall>,
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
    /// Writes the row, announces it and answers its id.
    pub async fn append_usage(&self, row: UsageRow) -> Result<String, StorageError> {
        let id = uuid::Uuid::new_v4().to_string();
        let written = UsageWritten {
            user: row.user.clone(),
            conversation: row.conversation.clone(),
            id: id.clone(),
        };
        let id = self.call(move |connection, _| {
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
            connection.execute(
                "INSERT INTO usage_ledger (id, user_id, conversation_id, agent, turn_id, purpose,
                   provider_id, account_id, model_id, request_id, vendor_id,
                   input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
                   cache_write_1h_tokens, outcome, quota, started_at, ended_at, media_seconds, cost, cost_currency,
                   api_price_input, api_price_cache_write, api_price_cache_read, api_price_output,
                   system_hash, tools_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                   ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29)",
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
                ],
            )?;
            Ok(id)
        })
        .await?;
        self.announce_usage(written);
        Ok(id)
    }

    /// The number of the conversation's ledger rows.
    pub async fn cost_rows(&self, conversation: ConversationId) -> Result<u64, StorageError> {
        let rows: i64 = self
            .call(move |connection, _| {
                Ok(connection.query_row(
                    "SELECT count(*) FROM usage_ledger WHERE conversation_id = ?1",
                    [conversation.as_str()],
                    |row| row.get(0),
                )?)
            })
            .await?;
        decode("usage_ledger", "rowid", u64::try_from(rows))
    }

    /// The conversation's cost from its ledger rows, its agents named by
    /// `labels`.
    pub async fn conversation_cost(
        &self,
        conversation: ConversationId,
        labels: Vec<AgentLabel>,
    ) -> Result<ConversationCost, StorageError> {
        let rows = self.call(move |connection, _| cost_rows(connection, &conversation)).await?;
        let calls = turn_calls(&rows);
        let mut prices: BTreeMap<u64, f64> = BTreeMap::new();
        for row in &rows {
            if let (Some(agent), Some(price)) = (row.agent, row.price()) {
                *prices.entry(agent).or_default() += price;
            }
        }
        Ok(ConversationCost {
            rows: rows.len() as u64,
            totals: totals_of(&rows),
            agents: prices
                .into_iter()
                .map(|(agent, price)| agent_cost(agent, price, &labels))
                .collect(),
            calls: calls.into_iter().map(|(_, call)| call).collect(),
        })
    }

    /// What the row `id` adds to its conversation's cost, its agent named
    /// by `labels`; none when the conversation holds no such row.
    pub async fn cost_addition(
        &self,
        conversation: ConversationId,
        id: String,
        labels: Vec<AgentLabel>,
    ) -> Result<Option<CostAddition>, StorageError> {
        let row_id = id.clone();
        let (rows, written) = self
            .call(move |connection, _| {
                let transaction = connection.transaction()?;
                let rows = cost_rows(&transaction, &conversation)?;
                let written: i64 = transaction.query_row(
                    "SELECT count(*) FROM usage_ledger
                     WHERE conversation_id = ?1
                       AND rowid <= (SELECT rowid FROM usage_ledger WHERE id = ?2)",
                    params![conversation.as_str(), row_id],
                    |row| row.get(0),
                )?;
                transaction.commit()?;
                Ok((rows, written))
            })
            .await?;
        let Some(row) = rows.iter().find(|row| row.id == id) else {
            return Ok(None);
        };
        let agent = row
            .agent
            .zip(row.price())
            .map(|(agent, price)| agent_cost(agent, price, &labels));
        let call = turn_calls(&rows)
            .into_iter()
            .find_map(|(call_id, call)| (call_id == id).then_some(call));
        Ok(Some(CostAddition {
            rows: decode("usage_ledger", "rowid", u64::try_from(written))?,
            totals: totals_of(std::slice::from_ref(row)),
            agent,
            call,
        }))
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

/// One ledger row as a conversation's cost reads it.
#[derive(Debug, Clone)]
struct CostRow {
    id: String,
    agent: Option<u64>,
    purpose: Option<String>,
    model: String,
    /// None when the vendor reported no counts.
    usage: Option<TokenUsage>,
    cache_write_1h_tokens: u64,
    outcome: Option<String>,
    started_at: Timestamp,
    ended_at: Timestamp,
    parts: Option<ApiPrice>,
    /// The vendor's own cost, when it named it in dollars.
    usd: Option<f64>,
    system_hash: Option<String>,
    tools_hash: Option<String>,
}

impl CostRow {
    /// The call's price: its parts, or for a call whose tokens have no
    /// price the vendor's own cost in dollars.
    fn price(&self) -> Option<f64> {
        self.parts.map(|parts| parts.total()).or(self.usd)
    }

    /// The main conversation's call of a turn, with counts.
    fn main_turn(&self) -> Option<TokenUsage> {
        (self.agent == Some(0) && self.purpose.as_deref() == Some("turn"))
            .then_some(self.usage)
            .flatten()
    }
}

/// The conversation's rows in the order their calls started.
fn cost_rows(connection: &Connection, conversation: &ConversationId) -> Result<Vec<CostRow>, StorageError> {
    let mut statement = connection.prepare_cached(
        "SELECT id, agent, purpose, model_id, input_tokens, output_tokens, cache_read_tokens,
           cache_write_tokens, cache_write_1h_tokens, outcome, started_at, ended_at, cost, cost_currency,
           api_price_input, api_price_cache_write, api_price_cache_read, api_price_output,
           system_hash, tools_hash
         FROM usage_ledger WHERE conversation_id = ?1
         ORDER BY coalesce(started_at, ended_at), rowid",
    )?;
    let mut rows = statement.query([conversation.as_str()])?;
    let mut read = Vec::new();
    while let Some(row) = rows.next()? {
        read.push(cost_row(row)?);
    }
    Ok(read)
}

fn cost_row(row: &Row<'_>) -> Result<CostRow, StorageError> {
    let count = |column: &'static str| -> Result<Option<u64>, StorageError> {
        row.get::<_, Option<i64>>(column)?
            .map(|count| decode("usage_ledger", column, u64::try_from(count)))
            .transpose()
    };
    let counts = [
        count("input_tokens")?,
        count("output_tokens")?,
        count("cache_read_tokens")?,
        count("cache_write_tokens")?,
    ];
    let usage = match counts {
        [Some(input_tokens), Some(output_tokens), Some(cache_read_tokens), Some(cache_write_tokens)] => {
            Some(TokenUsage {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_tokens,
            })
        }
        [None, None, None, None] => None,
        _ => return Err(corrupt("input_tokens", "some of the counts only")),
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
    let amount: Option<f64> = row.get("cost")?;
    let agent: Option<i64> = row.get("agent")?;
    let ended_at = instant(row, "usage_ledger", "ended_at")?;
    let started_at = match row.get::<_, Option<i64>>("started_at")? {
        Some(_) => instant(row, "usage_ledger", "started_at")?,
        // A row written before the ledger kept a call's start.
        None => ended_at,
    };
    Ok(CostRow {
        id: row.get("id")?,
        agent: agent
            .map(|agent| decode("usage_ledger", "agent", u64::try_from(agent)))
            .transpose()?,
        purpose: row.get("purpose")?,
        model: row.get("model_id")?,
        usage,
        cache_write_1h_tokens: count("cache_write_1h_tokens")?.unwrap_or(0),
        outcome: row.get("outcome")?,
        started_at,
        ended_at,
        parts,
        usd: amount.filter(|_| currency == Some(Currency::Usd.to_string())),
        system_hash: row.get("system_hash")?,
        tools_hash: row.get("tools_hash")?,
    })
}

fn corrupt(column: &'static str, problem: &'static str) -> StorageError {
    StorageError::Corrupt {
        table: "usage_ledger",
        column,
        reason: format!("the row sets {problem}"),
    }
}

/// The token and price totals of `rows`; a count the vendor did not report
/// adds nothing, and a price is null when no row has one.
fn totals_of(rows: &[CostRow]) -> CostTotals {
    let mut totals = CostTotals::default();
    let add = |sum: &mut Option<f64>, value: Option<f64>| {
        if let Some(value) = value {
            *sum = Some(sum.unwrap_or(0.0) + value);
        }
    };
    for row in rows {
        if let Some(usage) = row.usage {
            totals.input_tokens += usage.input_tokens;
            totals.cache_write_tokens += usage.cache_write_tokens;
            totals.cache_read_tokens += usage.cache_read_tokens;
            totals.output_tokens += usage.output_tokens;
        }
        add(&mut totals.price.total, row.price());
        add(&mut totals.price.input, row.parts.map(|parts| parts.input));
        add(&mut totals.price.cache_write, row.parts.map(|parts| parts.cache_write));
        add(&mut totals.price.cache_read, row.parts.map(|parts| parts.cache_read));
        add(&mut totals.price.output, row.parts.map(|parts| parts.output));
    }
    totals
}

fn agent_cost(agent: u64, price: f64, labels: &[AgentLabel]) -> AgentCost {
    let label = labels.iter().find(|label| label.number == agent);
    AgentCost {
        agent,
        profile: label.and_then(|label| label.profile.clone()).filter(|_| agent != 0),
        price,
    }
}

/// The main conversation's turn calls among `rows`, each with its row's id,
/// whether it follows a compaction and why it missed the cache.
///
/// A call missed when it wrote again more than [`MISS_SHARE`] of its prompt and at
/// least [`MISS_TOKENS`], except the first call and the first after a compaction,
/// which start a new prompt.
fn turn_calls(rows: &[CostRow]) -> Vec<(String, CostCall)> {
    let mut calls = Vec::new();
    let mut previous: Option<&CostRow> = None;
    let mut compacted = false;
    // Whether each model's latest write was kept for an hour.
    let mut hour_writes: HashMap<&str, bool> = HashMap::new();
    for row in rows {
        if row.agent == Some(0)
            && row.purpose.as_deref() == Some("compaction")
            && row.outcome.as_deref() == Some("answered")
        {
            compacted = true;
            continue;
        }
        let Some(usage) = row.main_turn() else {
            continue;
        };
        let written = usage.input_tokens + usage.cache_write_tokens;
        let prompt = written + usage.cache_read_tokens;
        let missed = written >= MISS_TOKENS && written as f64 > prompt as f64 * MISS_SHARE;
        let miss = match previous {
            Some(previous) if !compacted && missed => {
                let hour_write = hour_writes.get(row.model.as_str()).copied().unwrap_or(false);
                Some(miss_cause(previous, row, hour_write))
            }
            _ => None,
        };
        calls.push((
            row.id.clone(),
            CostCall {
                started_at: row.started_at,
                input_tokens: usage.input_tokens,
                cache_write_tokens: usage.cache_write_tokens,
                cache_read_tokens: usage.cache_read_tokens,
                price: row.price(),
                after_compaction: compacted && previous.is_some(),
                miss,
            },
        ));
        if usage.cache_write_tokens > 0 {
            hour_writes.insert(&row.model, row.cache_write_1h_tokens > 0);
        }
        compacted = false;
        previous = Some(row);
    }
    calls
}

/// Why `row` missed the cache after `previous`, the first cause that
/// applies; a hash only tells a change when both calls have one, and
/// `hour_write` says whether the latest write of `row`'s model was kept for
/// an hour.
fn miss_cause(previous: &CostRow, row: &CostRow, hour_write: bool) -> CacheMiss {
    let changed = |before: &Option<String>, after: &Option<String>| {
        matches!((before, after), (Some(before), Some(after)) if before != after)
    };
    let kept = if hour_write { KEPT_HOUR_MS } else { KEPT_MS };
    if previous.model != row.model {
        CacheMiss::ModelChanged
    } else if changed(&previous.tools_hash, &row.tools_hash) {
        CacheMiss::ToolsChanged
    } else if changed(&previous.system_hash, &row.system_hash) {
        CacheMiss::SystemPromptChanged
    } else if row.started_at.as_millisecond() - previous.ended_at.as_millisecond() > kept {
        CacheMiss::CacheExpired
    } else {
        CacheMiss::NotCached
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use demi_conversation_socket_protocol::{AgentCost, CacheMiss};
    use demi_shared_types::{Cost, Currency, SystemClock, Timestamp, TokenUsage};
    use demi_web_api_protocol::ids::{ConversationId, ProviderId};

    use super::{ApiPrice, UsageRow};
    use crate::control::{ControlService, testing};
    use crate::providers::NoSealedConfigs;
    use crate::tree::AgentLabel;

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

        let cost = control
            .conversation_cost(ConversationId::try_from(CONVERSATION).unwrap(), Vec::new())
            .await
            .unwrap();
        let read: Vec<(bool, Option<CacheMiss>)> =
            cost.calls.iter().map(|call| (call.after_compaction, call.miss)).collect();
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

    /// The totals sum every row of the conversation, every agent's, compactions,
    /// titles and media included.
    ///
    /// They are:
    ///
    /// - the counts, each kind's price, and the price, which for a call whose
    ///   tokens have no price is the vendor's own cost in dollars and never one in
    ///   another currency;
    /// - each agent with a priced call, with its price, named by its profile;
    /// - the calls, which are the main conversation's turns.
    ///
    /// Planted defects: a cost in yuan counted as dollars, a title counted as an
    /// agent, or a subagent's call among the main conversation's.
    ///
    /// Six rows of an in-memory ledger: milliseconds.
    #[tokio::test]
    async fn the_cost_sums_every_row_and_names_each_priced_agent() {
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
        let labels = vec![
            AgentLabel { number: 0, profile: None },
            AgentLabel { number: 1, profile: Some("research".into()) },
            AgentLabel { number: 2, profile: None },
            AgentLabel { number: 3, profile: None },
        ];
        let cost = control
            .conversation_cost(ConversationId::try_from(CONVERSATION).unwrap(), labels)
            .await
            .unwrap();
        let totals = cost.totals;
        assert_eq!(
            (totals.input_tokens, totals.cache_write_tokens, totals.cache_read_tokens, totals.output_tokens),
            (1_016, 2_025, 3_035, 40)
        );
        let price = totals.price;
        assert_eq!(
            (price.total, price.input, price.cache_write, price.cache_read, price.output),
            (Some(14.0), Some(1.75), Some(2.5), Some(3.125), Some(4.625))
        );
        assert_eq!(
            cost.agents,
            [
                AgentCost { agent: 0, profile: None, price: 1.0 },
                AgentCost { agent: 1, profile: Some("research".into()), price: 10.0 },
                AgentCost { agent: 2, profile: None, price: 2.0 },
            ]
        );
        assert_eq!(cost.calls.len(), 1);
        assert_eq!(cost.calls[0].price, Some(1.0));
        control.close().await.unwrap();
    }
}
