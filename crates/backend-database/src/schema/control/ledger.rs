//! The usage ledger's passes of the steps from 0.1.37 and from 0.1.5586: each
//! turn call's number, whether a compaction came before it and why it missed
//! the cache, judged against its agent's previous call, and each
//! conversation's kept cost, as the ledger judged and added them when the
//! step from 0.1.5586 was written.

use rusqlite::{Connection, OptionalExtension, Row, params};

/// A cache miss reads less of the cache than its agent's previous call did,
/// by more than this share of what that call read and by at least
/// [`MISS_TOKENS`].
const MISS_SHARE: f64 = 0.05;
const MISS_TOKENS: i64 = 2_000;
/// How long a vendor keeps a prompt after a write, and after a write kept
/// for an hour.
const KEPT_MS: i64 = 5 * 60 * 1000;
const KEPT_HOUR_MS: i64 = 60 * 60 * 1000;

/// What a turn call's row says of the call, which the next one is judged
/// against.
struct CallFacts<'a> {
    /// The rowid of the call: only the rows before it are its agent's
    /// earlier ones.
    before: i64,
    model: &'a str,
    cache_read_tokens: i64,
    started_at: i64,
    system_hash: Option<&'a str>,
    tools_hash: Option<&'a str>,
}

/// What a turn call keeps of its place among its agent's calls.
struct KeptCall {
    number: i64,
    after_compaction: bool,
    miss: Option<&'static str>,
}

/// The token counts of a row, none when the vendor reported none.
#[derive(Clone, Copy, Default)]
struct Counts {
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
}

/// The price of each kind of a row's tokens, in dollars.
#[derive(Clone, Copy)]
struct Prices {
    input: f64,
    cache_write: f64,
    cache_read: f64,
    output: f64,
}

/// Judges every turn call's cache miss again against its agent's previous
/// call; the calls' numbers and compactions stay as they were kept.
pub(super) fn judge_misses(connection: &Connection) -> rusqlite::Result<()> {
    let mut rows = connection.prepare(
        "SELECT rowid, conversation_id, agent, model_id, input_tokens, output_tokens, cache_read_tokens,
           cache_write_tokens, started_at, ended_at, system_hash, tools_hash, miss
         FROM usage_ledger WHERE call_number IS NOT NULL ORDER BY rowid",
    )?;
    let mut rows = rows.query([])?;
    let mut judge = connection.prepare("UPDATE usage_ledger SET miss = ?2 WHERE rowid = ?1")?;
    while let Some(row) = rows.next()? {
        let rowid: i64 = row.get("rowid")?;
        let conversation: String = row.get("conversation_id")?;
        let agent = count(row.get("agent")?, "agent")?;
        let counts = counts(row)?.ok_or_else(|| super::super::corrupt_text("a turn call of the ledger has no token counts"))?;
        let model: String = row.get("model_id")?;
        let system_hash: Option<String> = row.get("system_hash")?;
        let tools_hash: Option<String> = row.get("tools_hash")?;
        let kept: Option<String> = row.get("miss")?;
        let started_at = match row.get::<_, Option<i64>>("started_at")? {
            Some(started_at) => started_at,
            None => row.get("ended_at")?,
        };
        let call = next_call(
            connection,
            &conversation,
            agent,
            &CallFacts {
                before: rowid,
                model: &model,
                cache_read_tokens: counts.cache_read,
                started_at,
                system_hash: system_hash.as_deref(),
                tools_hash: tools_hash.as_deref(),
            },
        )?;
        if call.miss != kept.as_deref() {
            judge.execute(params![rowid, call.miss])?;
        }
    }
    Ok(())
}

/// Numbers every turn call of the ledger, judges each against its agent's
/// previous one and keeps each conversation's cost, as writing the rows one
/// by one in their order did.
pub(super) fn keep_costs(connection: &Connection) -> rusqlite::Result<()> {
    let mut rows = connection.prepare(
        "SELECT rowid, conversation_id, agent, purpose, model_id, input_tokens, output_tokens,
           cache_read_tokens, cache_write_tokens, started_at, ended_at, cost, cost_currency,
           api_price_input, api_price_cache_write, api_price_cache_read, api_price_output,
           system_hash, tools_hash
         FROM usage_ledger ORDER BY rowid",
    )?;
    let mut rows = rows.query([])?;
    let mut number = connection.prepare(
        "UPDATE usage_ledger SET call_number = ?2, after_compaction = ?3, miss = ?4 WHERE rowid = ?1",
    )?;
    while let Some(row) = rows.next()? {
        let rowid: i64 = row.get("rowid")?;
        let conversation: String = row.get("conversation_id")?;
        let agent = row.get::<_, Option<i64>>("agent")?.map(|agent| count(agent, "agent")).transpose()?;
        let counts = counts(row)?;
        let (prices, usd) = prices(row)?;
        let purpose: Option<String> = row.get("purpose")?;
        let call = match (agent, purpose.as_deref(), counts) {
            (Some(agent), Some("turn"), Some(counts)) => {
                let model: String = row.get("model_id")?;
                let system_hash: Option<String> = row.get("system_hash")?;
                let tools_hash: Option<String> = row.get("tools_hash")?;
                let started_at = match row.get::<_, Option<i64>>("started_at")? {
                    Some(started_at) => started_at,
                    None => row.get("ended_at")?,
                };
                Some(next_call(
                    connection,
                    &conversation,
                    agent,
                    &CallFacts {
                        before: rowid,
                        model: &model,
                        cache_read_tokens: counts.cache_read,
                        started_at,
                        system_hash: system_hash.as_deref(),
                        tools_hash: tools_hash.as_deref(),
                    },
                )?)
            }
            _ => None,
        };
        if let Some(call) = &call {
            number.execute(params![rowid, call.number, call.after_compaction, call.miss])?;
        }
        add_cost(connection, &conversation, agent, counts.unwrap_or_default(), prices, usd, call.is_some())?;
    }
    Ok(())
}

/// The turn call `facts` of the conversation's agent `agent`, judged against
/// the agent's previous call.
fn next_call(connection: &Connection, conversation: &str, agent: i64, facts: &CallFacts<'_>) -> rusqlite::Result<KeptCall> {
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
        return Ok(KeptCall { number: 1, after_compaction: false, miss: None });
    };
    let compacted: bool = connection
        .prepare_cached(
            "SELECT EXISTS (SELECT 1 FROM usage_ledger
               WHERE conversation_id = ?1 AND agent = ?2 AND rowid > ?3 AND rowid < ?4
                 AND purpose = 'compaction' AND outcome = 'answered')",
        )?
        .query_row(params![conversation, agent, rowid, facts.before], |row| row.get(0))?;
    let number = count(number, "call_number")? + 1;
    if compacted {
        return Ok(KeptCall { number, after_compaction: true, miss: None });
    }
    let read = count(read, "cache_read_tokens")?;
    let lost = read.saturating_sub(facts.cache_read_tokens).max(0);
    let miss = if lost >= MISS_TOKENS && lost as f64 > read as f64 * MISS_SHARE {
        let changed = |before: Option<&str>, after: Option<&str>| matches!((before, after), (Some(before), Some(after)) if before != after);
        // Whether the latest write of the call's model was kept for an hour.
        let hour_write: Option<i64> = connection
            .prepare_cached(
                "SELECT cache_write_1h_tokens FROM usage_ledger
                 WHERE conversation_id = ?1 AND agent = ?2 AND call_number IS NOT NULL AND rowid < ?3
                   AND model_id = ?4 AND cache_write_tokens > 0
                 ORDER BY rowid DESC LIMIT 1",
            )?
            .query_row(params![conversation, agent, facts.before, facts.model], |row| row.get(0))
            .optional()?
            .flatten();
        let kept = if hour_write.is_some_and(|tokens| tokens > 0) { KEPT_HOUR_MS } else { KEPT_MS };
        Some(if model != facts.model {
            "model_changed"
        } else if changed(tools_hash.as_deref(), facts.tools_hash) {
            "tools_changed"
        } else if changed(system_hash.as_deref(), facts.system_hash) {
            "system_prompt_changed"
        } else if facts.started_at - ended_at > kept {
            "cache_expired"
        } else {
            "not_cached"
        })
    } else {
        None
    };
    Ok(KeptCall { number, after_compaction: false, miss })
}

/// The counts of a row of the ledger: none when the vendor reported none.
fn counts(row: &Row<'_>) -> rusqlite::Result<Option<Counts>> {
    match [
        row.get::<_, Option<i64>>("input_tokens")?,
        row.get("output_tokens")?,
        row.get("cache_read_tokens")?,
        row.get("cache_write_tokens")?,
    ] {
        [Some(input), Some(output), Some(cache_read), Some(cache_write)] => Ok(Some(Counts {
            input: count(input, "input_tokens")?,
            output: count(output, "output_tokens")?,
            cache_read: count(cache_read, "cache_read_tokens")?,
            cache_write: count(cache_write, "cache_write_tokens")?,
        })),
        [None, None, None, None] => Ok(None),
        _ => Err(super::super::corrupt_text("a row of the ledger has some of the token counts only")),
    }
}

/// The price of each kind of a row's tokens, and the vendor's own cost when
/// that is in dollars.
fn prices(row: &Row<'_>) -> rusqlite::Result<(Option<Prices>, Option<f64>)> {
    let prices = match [
        row.get::<_, Option<f64>>("api_price_input")?,
        row.get("api_price_cache_write")?,
        row.get("api_price_cache_read")?,
        row.get("api_price_output")?,
    ] {
        [Some(input), Some(cache_write), Some(cache_read), Some(output)] => Some(Prices { input, cache_write, cache_read, output }),
        [None, None, None, None] => None,
        _ => return Err(super::super::corrupt_text("a row of the ledger has some of the price's parts only")),
    };
    let currency: Option<String> = row.get("cost_currency")?;
    let usd = row.get::<_, Option<f64>>("cost")?.filter(|_| currency.as_deref() == Some("usd"));
    Ok((prices, usd))
}

/// Adds a row to the kept cost of the conversation's agent `agent`, or of
/// its rows of no agent, with one more turn call when `call`. A price that
/// is null in the cost or in the row adds to the other: null only while no
/// row had one.
fn add_cost(
    connection: &Connection,
    conversation: &str,
    agent: Option<i64>,
    counts: Counts,
    prices: Option<Prices>,
    usd: Option<f64>,
    call: bool,
) -> rusqlite::Result<()> {
    let kept = connection
        .prepare_cached(
            "SELECT input_tokens, cache_write_tokens, cache_read_tokens, output_tokens, price_total,
               price_input, price_cache_write, price_cache_read, price_output, calls
             FROM usage_costs WHERE conversation_id = ?1 AND agent IS ?2",
        )?
        .query_row(params![conversation, agent], |row| {
            Ok((
                [row.get::<_, i64>(0)?, row.get(1)?, row.get(2)?, row.get(3)?],
                [row.get::<_, Option<f64>>(4)?, row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?],
                row.get::<_, i64>(9)?,
            ))
        })
        .optional()?;
    let (mut tokens, mut price, mut calls) = kept.unwrap_or(([0; 4], [None; 5], 0));
    for (sum, part) in tokens.iter_mut().zip([counts.input, counts.cache_write, counts.cache_read, counts.output]) {
        *sum = sum.saturating_add(part);
    }
    let total = prices.map(|prices| prices.input + prices.cache_write + prices.cache_read + prices.output).or(usd);
    let parts = [
        total,
        prices.map(|prices| prices.input),
        prices.map(|prices| prices.cache_write),
        prices.map(|prices| prices.cache_read),
        prices.map(|prices| prices.output),
    ];
    for (sum, part) in price.iter_mut().zip(parts) {
        if let Some(part) = part {
            *sum = Some(sum.unwrap_or(0.0) + part);
        }
    }
    calls += i64::from(call);
    let values = params![
        conversation, agent, tokens[0], tokens[1], tokens[2], tokens[3], price[0], price[1], price[2], price[3], price[4], calls
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

/// A count of the ledger, which is never below zero: one that is fails the
/// step, so the database stays as it was.
fn count(value: i64, column: &str) -> rusqlite::Result<i64> {
    if value < 0 {
        return Err(super::super::corrupt_text(&format!("a row of the ledger has a {column} below zero")));
    }
    Ok(value)
}
