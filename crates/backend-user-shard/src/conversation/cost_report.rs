//! What a conversation's calls cost, as `demi conversation cost` and `demi
//! conversation history cost` print it, from the usage ledger
//! (`crates/core-store/src/usage.rs` § A cost report).
//!
//! Prices are the equivalent API price, the one the context card shows: what
//! the calls' tokens would cost on the vendors' APIs, a subscription's calls
//! included, which is no bill. A medium's call is priced at the vendor's own
//! cost when it is in dollars, as the card counts it, and its cost in any
//! currency is shown apart, never converted.
//!
//! ```text
//!  usage_ledger rows ──┬── conversation cost: by agent, model, purpose,
//!                      │   medium, account, miss and outcome
//!                      └── history cost: one agent's rows of a page's
//!                          turns, by turn or by call
//! ```

use std::collections::{BTreeMap, BTreeSet};

use demi_core_store::tree;
use demi_core_store::usage::{LedgerCall, add_call, turn_hit_rate, window_ms};
use demi_backend_host_access::host_commands::conversation_of;
use demi_core_providers::vault::accounts as vault_accounts;
use demi_core_providers::vault::entries::Vault;
use demi_conversation_lane_protocol::{AgentCost, CacheMiss};
use demi_host_interface::{Call, RpcError, RpcPort};
use demi_shared_types::output::{self, Cell, Fields, Table};
use demi_shared_types::{Currency, Timestamp};
use demi_web_api_protocol::conversations::CostWindow;
use demi_web_api_protocol::ids::{ConversationId, CredentialId, ProviderId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::rc::Rc;

use super::organize::failed;
use super::product::entries;
use crate::shard::Shard;

/// The media purposes, whose calls make a file.
const MEDIA: [&str; 3] = ["image", "video", "audio"];

/// The windows a conversation's cost is summed over besides all of it, as
/// the context card offers them.
const WINDOWS: [CostWindow; 2] = [CostWindow::Day, CostWindow::Hour];

/// The input of `demi conversation cost`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct CostArgs {
    /// Another conversation of the user, by its id; this conversation when absent
    #[schemars(extend("patternDescription" = "a conversation's id, as demi conversation list shows it"))]
    conversation: Option<ConversationId>,
}

/// What some calls add up to.
#[derive(Debug, Default, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct Sum {
    /// Every call to a vendor, answered or not, as Settings' usage counts
    /// requests; the context card's calls are the turn calls alone.
    requests: u64,
    #[serde(flatten)]
    tokens: Tokens,
}

/// What some calls' tokens add up to, and their price.
#[derive(Debug, Default, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct Tokens {
    input_tokens: u64,
    cache_write_tokens: u64,
    cache_read_tokens: u64,
    output_tokens: u64,
    /// In dollars at API prices; null when no call had a price.
    price: Option<f64>,
}

/// A sum as it grows, call by call.
#[derive(Default)]
struct Tally {
    requests: u64,
    cost: AgentCost,
}

impl Tally {
    fn add(&mut self, call: &LedgerCall) {
        self.requests += 1;
        add_call(&mut self.cost, call);
    }

    fn sum(&self) -> Sum {
        let totals = &self.cost.totals;
        Sum {
            requests: self.requests,
            tokens: Tokens {
                input_tokens: totals.input_tokens,
                cache_write_tokens: totals.cache_write_tokens,
                cache_read_tokens: totals.cache_read_tokens,
                output_tokens: totals.output_tokens,
                price: totals.price.total,
            },
        }
    }
}

/// The calls of `calls` summed.
fn sum_of<'a>(calls: impl IntoIterator<Item = &'a LedgerCall>) -> Sum {
    let mut tally = Tally::default();
    calls.into_iter().for_each(|call| tally.add(call));
    tally.sum()
}

/// A conversation's cost.
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct ConversationCost {
    conversation: ConversationId,
    title: Option<String>,
    #[serde(flatten)]
    sum: Sum,
    /// The share of the turn calls' prompt tokens the cache served.
    hit_rate: Option<f64>,
    /// The calls that ended in the last day and the last hour.
    windows: Vec<WindowCost>,
    /// Each agent that made a call, by number, 0 the main agent.
    agents: Vec<AgentRow>,
    models: Vec<ModelRow>,
    purposes: Vec<PurposeRow>,
    media: Vec<MediaRow>,
    /// The subscription accounts that answered calls, in the order they
    /// first did.
    accounts: Vec<AccountRow>,
    /// How many times an agent's turn call was answered by another account
    /// than its previous one.
    account_switches: u64,
    /// The turn calls that missed the cache, by why.
    misses: Vec<MissRow>,
    /// The calls not answered, by how they ended.
    not_answered: Vec<OutcomeRow>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct WindowCost {
    window: CostWindow,
    #[serde(flatten)]
    sum: Sum,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct AgentRow {
    agent: u64,
    /// The subagent's profile; null for the main agent and a subagent of
    /// none.
    profile: Option<String>,
    #[serde(flatten)]
    sum: Sum,
    hit_rate: Option<f64>,
    /// Its share of the conversation's price.
    share: Option<f64>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ModelRow {
    model: String,
    /// The provider entry's name in Settings, or its id once it is gone.
    entry: String,
    #[serde(flatten)]
    sum: Sum,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct PurposeRow {
    /// `turn`, `compaction`, `title`, `suggested_replies`, `image`, `video`
    /// or `audio`.
    purpose: String,
    #[serde(flatten)]
    sum: Sum,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct MediaRow {
    /// `image`, `video` or `audio`.
    kind: String,
    requests: u64,
    answered: u64,
    /// The seconds of video or audio made.
    seconds: Option<f64>,
    /// What the vendor said the calls cost, in `currency`; null for calls
    /// it said nothing of.
    vendor_cost: Option<f64>,
    currency: Option<Currency>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct AccountRow {
    entry: String,
    /// What the user knows the account by; null for the accounts of an
    /// entry that only the user who configures it sees, which count as one.
    account: Option<String>,
    #[serde(flatten)]
    sum: Sum,
    first_call_at: Timestamp,
    last_call_at: Timestamp,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct MissRow {
    miss: CacheMiss,
    calls: u64,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct OutcomeRow {
    /// `cancelled` or the failure code of a refused call.
    outcome: String,
    #[serde(flatten)]
    sum: Sum,
}

/// What the user knows each entry and account by.
///
/// The accounts of an entry another user configures are that user's, as
/// Settings keeps them: a report names none of them and does not tell them
/// apart.
#[derive(Default)]
pub(super) struct Labels {
    entries: BTreeMap<ProviderId, String>,
    accounts: BTreeMap<(ProviderId, CredentialId), String>,
    /// The entries whose accounts the user sees.
    disclosed: BTreeSet<ProviderId>,
}

impl Labels {
    /// The labels of the entries and accounts `calls` name.
    pub(super) async fn of(shard: &Shard, calls: &[LedgerCall]) -> Self {
        let mut labels = Self::default();
        // An entry that cannot be read leaves only its id to show.
        let entries = entries(shard).await.unwrap_or_default();
        for entry in entries {
            let named = calls.iter().any(|call| call.provider == entry.id);
            if !named {
                continue;
            }
            if calls.iter().any(|call| call.provider == entry.id && call.account.is_some())
                && Vault::manages(shard.user(), &entry)
            {
                labels.disclosed.insert(entry.id.clone());
                if let Ok(listed) = vault_accounts::list(&shard.services().vault, &entry, true).await {
                    for account in listed.accounts {
                        if let Ok(id) = CredentialId::try_from(account.id.as_str()) {
                            labels.accounts.insert((entry.id.clone(), id), account.label);
                        }
                    }
                }
            }
            labels.entries.insert(entry.id.clone(), entry.label.clone());
        }
        labels
    }

    fn entry(&self, provider: &ProviderId) -> String {
        self.entries.get(provider).cloned().unwrap_or_else(|| provider.to_string())
    }

    /// The account that answered `call` as the user may tell it apart:
    /// none for an API key's, and the entry with no account for an entry
    /// whose accounts the user does not see.
    fn shown(&self, call: &LedgerCall) -> Option<(ProviderId, Option<CredentialId>)> {
        let account = call.account.as_ref()?;
        let seen = self.disclosed.contains(&call.provider).then(|| account.clone());
        Some((call.provider.clone(), seen))
    }

    fn account(&self, provider: &ProviderId, account: Option<&CredentialId>) -> Option<String> {
        account.map(|account| {
            self.accounts
                .get(&(provider.clone(), account.clone()))
                .cloned()
                .unwrap_or_else(|| account.to_string())
        })
    }

    /// The account that answered `call`, `entry · account`, or the entry
    /// alone when its accounts are not the user's to see; none for an API
    /// key's.
    pub(super) fn account_of(&self, call: &LedgerCall) -> Option<String> {
        self.shown(call).map(|(provider, account)| match self.account(&provider, account.as_ref()) {
            Some(account) => format!("{} · {account}", self.entry(&provider)),
            None => self.entry(&provider),
        })
    }
}

/// `demi conversation cost`.
pub(super) async fn cost(shard: Rc<Shard>, call: Call<CostArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let id = match &call.args.conversation {
        Some(id) => id.clone(),
        None => conversation_of(&call.invocation)?,
    };
    // Another user's conversation is none of the caller's.
    let record = match shard.store().conversation(id.clone()).await.map_err(failed)? {
        Some(record) => record,
        _ => return Err(failed(format!("no conversation {id}"))),
    };
    let (conversation, title) = (record.id, record.title);
    let (calls, now) = shard.store().conversation_calls(conversation.clone()).await.map_err(failed)?;
    let profiles: BTreeMap<u64, Option<String>> = shard.store().databases()
        .read(&conversation, tree::subagents)
        .await
        .map_err(failed)?
        .unwrap_or_default()
        .into_iter()
        .map(|subagent| (subagent.record.number, subagent.record.profile))
        .collect();
    let labels = Labels::of(&shard, &calls).await;
    let report = report(conversation, title, &calls, now, &profiles, &labels);
    if call.invocation.json {
        let text = serde_json::to_string(&report).map_err(failed)?;
        port.stdout(text + "\n").await?;
        return Ok(0);
    }
    // A call the vendor reported counts for, of a model whose catalog has
    // no price for them: a call with no counts costs nothing to leave out,
    // and a medium's is the vendor's own cost.
    let unpriced = |call: &LedgerCall| call.usage.is_some() && call.price.is_none() && !MEDIA.contains(&call.purpose.as_str());
    if report.sum.tokens.price.is_some() && calls.iter().any(unpriced) {
        let note = "the price leaves out the calls of a model with no catalog price";
        port.stderr(output::note("demi conversation cost", note)).await?;
    }
    port.stdout(render(&report)).await?;
    Ok(0)
}

/// The report of `calls`, the conversation's rows, as of `now`.
fn report(
    conversation: ConversationId,
    title: Option<String>,
    calls: &[LedgerCall],
    now: Timestamp,
    profiles: &BTreeMap<u64, Option<String>>,
    labels: &Labels,
) -> ConversationCost {
    let sum = sum_of(calls);
    let windows = WINDOWS
        .iter()
        .map(|&window| WindowCost {
            window,
            sum: sum_of(calls.iter().filter(|call| call.ended_at.as_millisecond() > now.as_millisecond() - window_ms(window))),
        })
        .collect();
    let mut by_agent: BTreeMap<u64, Vec<&LedgerCall>> = BTreeMap::new();
    for call in calls {
        if let Some(agent) = call.agent {
            by_agent.entry(agent).or_default().push(call);
        }
    }
    let agents = by_agent
        .iter()
        .map(|(&agent, rows)| {
            let own = sum_of(rows.iter().copied());
            AgentRow {
                agent,
                profile: profiles.get(&agent).cloned().flatten(),
                share: own
                    .tokens
                    .price
                    .zip(sum.tokens.price)
                    .filter(|&(_, all)| all > 0.0)
                    .map(|(own, all)| own / all),
                hit_rate: turn_hit_rate(rows.iter().copied()),
                sum: own,
            }
        })
        .collect();
    let models = grouped(calls, |call| Some((call.model.clone(), call.provider.clone())))
        .into_iter()
        .map(|((model, provider), sum)| ModelRow {
            model,
            entry: labels.entry(&provider),
            sum,
        })
        .collect();
    let purposes = grouped(calls, |call| Some(call.purpose.clone()))
        .into_iter()
        .map(|(purpose, sum)| PurposeRow { purpose, sum })
        .collect();
    let accounts = grouped(calls, |call| labels.shown(call))
        .into_iter()
        .map(|((provider, account), sum)| {
            let key = Some((provider.clone(), account.clone()));
            let own = || calls.iter().filter(|call| labels.shown(call) == key);
            AccountRow {
                entry: labels.entry(&provider),
                account: labels.account(&provider, account.as_ref()),
                sum,
                first_call_at: own().map(|call| call.started_at.unwrap_or(call.ended_at)).min().unwrap_or(now),
                last_call_at: own().map(|call| call.ended_at).max().unwrap_or(now),
            }
        })
        .collect();
    let account_switches = by_agent
        .values()
        .map(|rows| {
            // Only the accounts the user sees count, as the Accounts table
            // tells only those apart.
            let answered: Vec<CredentialId> = rows
                .iter()
                .filter(|call| call.call_number.is_some())
                .filter_map(|call| labels.shown(call).and_then(|(_, account)| account))
                .collect();
            answered.windows(2).filter(|pair| pair[0] != pair[1]).count() as u64
        })
        .sum();
    let mut misses: Vec<MissRow> = Vec::new();
    for miss in calls.iter().filter_map(|call| call.miss) {
        match misses.iter_mut().find(|row| row.miss == miss) {
            Some(row) => row.calls += 1,
            None => misses.push(MissRow { miss, calls: 1 }),
        }
    }
    misses.sort_by(|a, b| b.calls.cmp(&a.calls));
    let not_answered = grouped(calls, |call| (call.outcome != "answered").then(|| call.outcome.clone()))
        .into_iter()
        .map(|(outcome, sum)| OutcomeRow { outcome, sum })
        .collect();
    ConversationCost {
        conversation,
        title,
        hit_rate: turn_hit_rate(calls),
        windows,
        agents,
        models,
        purposes,
        media: media(calls),
        accounts,
        account_switches,
        misses,
        not_answered,
        sum,
    }
}

/// The calls summed by the key `key` gives each, in the order each key was
/// first met; a call of no key is left out.
fn grouped<K: PartialEq>(calls: &[LedgerCall], key: impl Fn(&LedgerCall) -> Option<K>) -> Vec<(K, Sum)> {
    let mut groups: Vec<(K, Tally)> = Vec::new();
    for call in calls {
        let Some(key) = key(call) else {
            continue;
        };
        match groups.iter_mut().find(|(kept, _)| *kept == key) {
            Some((_, tally)) => tally.add(call),
            None => {
                let mut tally = Tally::default();
                tally.add(call);
                groups.push((key, tally));
            }
        }
    }
    groups.into_iter().map(|(key, tally)| (key, tally.sum())).collect()
}

/// The media calls, by kind and by the currency the vendor named.
fn media(calls: &[LedgerCall]) -> Vec<MediaRow> {
    let mut rows: Vec<MediaRow> = Vec::new();
    for call in calls.iter().filter(|call| MEDIA.contains(&call.purpose.as_str())) {
        let currency = call.cost.map(|cost| cost.currency);
        let row = match rows.iter_mut().position(|row| row.kind == call.purpose && row.currency == currency) {
            Some(at) => &mut rows[at],
            None => {
                rows.push(MediaRow {
                    kind: call.purpose.clone(),
                    requests: 0,
                    answered: 0,
                    seconds: None,
                    vendor_cost: None,
                    currency,
                });
                rows.last_mut().expect("a row was pushed")
            }
        };
        row.requests += 1;
        row.answered += u64::from(call.outcome == "answered");
        if let Some(seconds) = call.media_seconds {
            row.seconds = Some(row.seconds.unwrap_or(0.0) + seconds);
        }
        if let Some(cost) = call.cost {
            row.vendor_cost = Some(row.vendor_cost.unwrap_or(0.0) + cost.amount);
        }
    }
    rows
}

/// A price in dollars as a cell: to a hundredth of a cent, which a call
/// costs at the least.
pub(super) fn dollars(price: Option<f64>) -> Cell {
    price.map(|price| format!("{price:.4}")).into()
}

/// A share as a percentage cell, to a tenth.
pub(super) fn percent(share: Option<f64>) -> Cell {
    share.map(|share| format!("{:.1}", share * 100.0)).into()
}

/// The columns of a sum, after the row's own.
const SUM_COLUMNS: [&str; 6] = ["requests", "input", "cache write", "cache read", "output", "price ($)"];

fn sum_cells(sum: &Sum) -> [Cell; 6] {
    let tokens = &sum.tokens;
    [
        sum.requests.into(),
        tokens.input_tokens.into(),
        tokens.cache_write_tokens.into(),
        tokens.cache_read_tokens.into(),
        tokens.output_tokens.into(),
        dollars(tokens.price),
    ]
}

fn table<const N: usize>(own: [&str; N], after: &[&str]) -> Table {
    Table::new(own.iter().chain(SUM_COLUMNS.iter()).chain(after.iter()))
}

fn render(report: &ConversationCost) -> String {
    let mut fields = Fields::new();
    fields
        .field("Conversation", report.conversation.as_str())
        .field("Title", report.title.as_deref())
        .field("Requests", report.sum.requests)
        .field("Price ($)", dollars(report.sum.tokens.price))
        .field("Cache hit (%)", percent(report.hit_rate));
    let mut windows = table(["window"], &[]);
    windows.row([Cell::from("all")].into_iter().chain(sum_cells(&report.sum)));
    for window in &report.windows {
        let name = match window.window {
            CostWindow::Day => "day",
            CostWindow::Hour => "hour",
        };
        windows.row([Cell::from(name)].into_iter().chain(sum_cells(&window.sum)));
    }
    fields.table("Windows", &windows);
    if !report.agents.is_empty() {
        let mut agents = table(["agent", "profile"], &["hit (%)", "share (%)"]);
        for agent in &report.agents {
            agents.row(
                [Cell::from(agent.agent), agent.profile.as_deref().into()]
                    .into_iter()
                    .chain(sum_cells(&agent.sum))
                    .chain([percent(agent.hit_rate), percent(agent.share)]),
            );
        }
        fields.table("Agents", &agents);
    }
    if !report.models.is_empty() {
        let mut models = table(["model", "entry"], &[]);
        for model in &report.models {
            models.row([Cell::from(&model.model), Cell::from(&model.entry)].into_iter().chain(sum_cells(&model.sum)));
        }
        fields.table("Models", &models);
    }
    if !report.purposes.is_empty() {
        let mut purposes = table(["purpose"], &[]);
        for purpose in &report.purposes {
            purposes.row([Cell::from(&purpose.purpose)].into_iter().chain(sum_cells(&purpose.sum)));
        }
        fields.table("Purposes", &purposes);
    }
    if !report.media.is_empty() {
        let mut media = Table::new(["kind", "requests", "answered", "made (s)", "vendor cost", "currency"]);
        for row in &report.media {
            media.row::<Cell>([
                (&row.kind).into(),
                row.requests.into(),
                row.answered.into(),
                row.seconds.map(|seconds| format!("{seconds:.1}")).into(),
                row.vendor_cost.map(|cost| format!("{cost:.4}")).into(),
                row.currency.map(|currency| currency.to_string()).into(),
            ]);
        }
        fields.table("Media", &media);
    }
    if !report.accounts.is_empty() {
        let mut accounts = table(["entry", "account"], &["first call", "last call"]);
        for row in &report.accounts {
            accounts.row(
                [Cell::from(&row.entry), row.account.as_deref().into()]
                    .into_iter()
                    .chain(sum_cells(&row.sum))
                    .chain([row.first_call_at.into(), row.last_call_at.into()]),
            );
        }
        fields.field("Account switches", report.account_switches).table("Accounts", &accounts);
    }
    if !report.misses.is_empty() {
        let mut misses = Table::new(["miss", "calls"]);
        for row in &report.misses {
            misses.row::<Cell>([row.miss.to_string().into(), row.calls.into()]);
        }
        fields.table("Cache misses", &misses);
    }
    if !report.not_answered.is_empty() {
        let mut outcomes = table(["outcome"], &[]);
        for row in &report.not_answered {
            outcomes.row([Cell::from(&row.outcome)].into_iter().chain(sum_cells(&row.sum)));
        }
        fields.table("Not answered", &outcomes);
    }
    fields.render()
}

/// What one turn's calls cost.
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct TurnCost {
    /// The index of the turn's first block.
    index: usize,
    started_at: Timestamp,
    #[serde(flatten)]
    sum: Sum,
    /// The prompt tokens of the turn's first turn call and of its last.
    context_first: Option<u64>,
    context_last: Option<u64>,
    hit_rate: Option<f64>,
    /// The turn calls that missed the cache, by why, in order.
    misses: Vec<CacheMiss>,
    /// A compaction ran in the turn.
    compacted: bool,
    /// How each call not answered ended, in order.
    not_answered: Vec<String>,
    /// The kind of each medium the turn made, in order.
    media: Vec<String>,
}

/// One call of a turn.
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct CallCost {
    /// The index of its turn's first block.
    turn: usize,
    /// Its number among its agent's turn calls; null for every other call.
    number: Option<u64>,
    purpose: String,
    model: String,
    /// `entry · account` of a subscription's call; null for an API key's.
    account: Option<String>,
    prompt_tokens: Option<u64>,
    #[serde(flatten)]
    tokens: Tokens,
    outcome: String,
    miss: Option<CacheMiss>,
    after_compaction: bool,
    started_at: Option<Timestamp>,
    ended_at: Timestamp,
}

/// What each turn of `turns`, by its id, first block's index and start,
/// cost, from `calls`, its agent's rows of those turns.
pub(super) fn turn_costs(turns: &[(String, usize, Timestamp)], calls: &[LedgerCall]) -> Vec<TurnCost> {
    turns
        .iter()
        .map(|(id, index, started_at)| {
            let own: Vec<&LedgerCall> = calls.iter().filter(|call| call.turn.as_deref() == Some(id)).collect();
            let prompts: Vec<u64> = own
                .iter()
                .filter(|call| call.call_number.is_some())
                .filter_map(|call| call.prompt_tokens())
                .collect();
            TurnCost {
                index: *index,
                started_at: *started_at,
                sum: sum_of(own.iter().copied()),
                context_first: prompts.first().copied(),
                context_last: prompts.last().copied(),
                hit_rate: turn_hit_rate(own.iter().copied()),
                misses: own.iter().filter_map(|call| call.miss).collect(),
                compacted: own.iter().any(|call| call.purpose == "compaction"),
                not_answered: own
                    .iter()
                    .filter(|call| call.outcome != "answered")
                    .map(|call| call.outcome.clone())
                    .collect(),
                media: own
                    .iter()
                    .filter(|call| MEDIA.contains(&call.purpose.as_str()))
                    .map(|call| call.purpose.clone())
                    .collect(),
            }
        })
        .collect()
}

/// Each call of `calls`, under the turn of `turns` it belongs to.
pub(super) fn call_costs(turns: &[(String, usize, Timestamp)], calls: &[LedgerCall], labels: &Labels) -> Vec<CallCost> {
    turns
        .iter()
        .flat_map(|(id, index, _)| {
            calls.iter().filter(move |call| call.turn.as_deref() == Some(id)).map(move |call| CallCost {
                turn: *index,
                number: call.call_number,
                purpose: call.purpose.clone(),
                model: call.model.clone(),
                account: labels.account_of(call),
                prompt_tokens: call.prompt_tokens(),
                tokens: sum_of([call]).tokens,
                outcome: call.outcome.clone(),
                miss: call.miss,
                after_compaction: call.after_compaction,
                started_at: call.started_at,
                ended_at: call.ended_at,
            })
        })
        .collect()
}

/// The turns as a table.
pub(super) fn render_turns(turns: &[TurnCost]) -> String {
    let mut table = Table::new([
        "turn",
        "started",
        "requests",
        "context first",
        "context last",
        "input",
        "cache write",
        "cache read",
        "output",
        "hit (%)",
        "price ($)",
        "notes",
    ]);
    for turn in turns {
        let mut notes = Vec::new();
        if !turn.misses.is_empty() {
            let misses: Vec<String> = turn.misses.iter().map(ToString::to_string).collect();
            notes.push(format!("missed: {}", misses.join(", ")));
        }
        if turn.compacted {
            notes.push("compacted".to_owned());
        }
        if !turn.not_answered.is_empty() {
            notes.push(format!("not answered: {}", turn.not_answered.join(", ")));
        }
        if !turn.media.is_empty() {
            notes.push(format!("made: {}", turn.media.join(", ")));
        }
        table.row::<Cell>([
            turn.index.into(),
            turn.started_at.into(),
            turn.sum.requests.into(),
            turn.context_first.into(),
            turn.context_last.into(),
            turn.sum.tokens.input_tokens.into(),
            turn.sum.tokens.cache_write_tokens.into(),
            turn.sum.tokens.cache_read_tokens.into(),
            turn.sum.tokens.output_tokens.into(),
            percent(turn.hit_rate),
            dollars(turn.sum.tokens.price),
            notes.join("; ").into(),
        ]);
    }
    table.render()
}

/// What `history cost` prints: one agent's turns, or with `--calls` their
/// calls.
#[derive(Serialize, JsonSchema)]
#[serde(untagged)]
pub(super) enum CostRows {
    Turns(Vec<TurnCost>),
    Calls(Vec<CallCost>),
}

/// The calls as a table.
pub(super) fn render_calls(calls: &[CallCost]) -> String {
    let mut table = Table::new([
        "turn",
        "call",
        "purpose",
        "model",
        "account",
        "prompt",
        "input",
        "cache write",
        "cache read",
        "output",
        "price ($)",
        "outcome",
        "miss",
        "ended",
    ]);
    for call in calls {
        let miss = match (call.miss, call.after_compaction) {
            (Some(miss), _) => Some(miss.to_string()),
            (None, true) => Some("after compaction".to_owned()),
            (None, false) => None,
        };
        table.row::<Cell>([
            call.turn.into(),
            call.number.into(),
            (&call.purpose).into(),
            (&call.model).into(),
            call.account.as_deref().into(),
            call.prompt_tokens.into(),
            call.tokens.input_tokens.into(),
            call.tokens.cache_write_tokens.into(),
            call.tokens.cache_read_tokens.into(),
            call.tokens.output_tokens.into(),
            dollars(call.tokens.price),
            (&call.outcome).into(),
            miss.into(),
            call.ended_at.into(),
        ]);
    }
    table.render()
}
