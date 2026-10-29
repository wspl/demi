//! From 0.1.37's control schema to 0.1.44's.
//!
//! - An automation's trigger is format 2: a daily schedule is a repeat with
//!   one task at its time, carrying the automation's prompt, Every Day when
//!   it ran every day and Every Week on its days otherwise; its prompt
//!   column is then empty, as a repeat's is.
//! - Its target is format 2: an existing conversation's message waits in its
//!   queue while it is busy, as every run's did.
//! - A run records the repeat's task that started it: each run of a daily
//!   schedule's, its one task.
//! - Each conversation keeps its cost as its ledger rows are written, and each
//!   turn call its number and its cache miss (`crates/backend-database/src/usage.rs`
//!   § A conversation's cost): the rows written before are numbered, judged and
//!   added up in the order they were written, as writing them now would.
//! - The backend keeps each Host directory's workflows as it reads them
//!   (`host_workflows`); nothing was kept before, so the next read fills it.

use rusqlite::Transaction;

use super::super::{Migration, ReadBlob, corrupt};
use super::shared::{named_timing};

pub(super) const MIGRATION: Migration = Migration::Code(control_from_0_1_37);

fn control_from_0_1_37(transaction: &Transaction<'_>, _: &dyn ReadBlob) -> rusqlite::Result<()> {
    transaction.execute_batch("ALTER TABLE automation_runs ADD COLUMN task_id TEXT;")?;
    transaction.execute_batch(CONTROL_FROM_0_1_37)?;
    super::ledger::keep_costs(transaction)?;
    let rows: Vec<(String, String, String, String)> = transaction
        .prepare("SELECT id, trigger, target, prompt FROM automations")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, trigger, target, prompt) in rows {
        let (trigger, prompt, task) = automation_trigger_2(&trigger, prompt)?;
        let target = automation_target_2(&target)?;
        transaction.execute(
            "UPDATE automations SET trigger = ?2, target = ?3, prompt = ?4 WHERE id = ?1",
            rusqlite::params![id, trigger.to_string(), target.to_string(), prompt],
        )?;
        if let Some(task) = task {
            transaction.execute(
                "UPDATE automation_runs SET task_id = ?2 WHERE automation_id = ?1",
                rusqlite::params![id, task],
            )?;
        }
    }
    Ok(())
}

const CONTROL_FROM_0_1_37: &str = "
ALTER TABLE usage_ledger ADD COLUMN call_number INTEGER CHECK (call_number >= 1);
ALTER TABLE usage_ledger ADD COLUMN after_compaction INTEGER CHECK (after_compaction IN (0, 1));
ALTER TABLE usage_ledger ADD COLUMN miss TEXT CHECK (miss IN ('model_changed', 'tools_changed', 'system_prompt_changed', 'cache_expired', 'not_cached'));
CREATE INDEX usage_ledger_agent ON usage_ledger (conversation_id, agent);
CREATE INDEX usage_ledger_cache_writes ON usage_ledger (conversation_id, agent, model_id)
  WHERE call_number IS NOT NULL AND cache_write_tokens > 0;
CREATE TABLE usage_costs (
  conversation_id    TEXT NOT NULL,
  agent              INTEGER CHECK (agent >= 0),
  input_tokens       INTEGER NOT NULL CHECK (input_tokens >= 0),
  cache_write_tokens INTEGER NOT NULL CHECK (cache_write_tokens >= 0),
  cache_read_tokens  INTEGER NOT NULL CHECK (cache_read_tokens >= 0),
  output_tokens      INTEGER NOT NULL CHECK (output_tokens >= 0),
  price_total        REAL CHECK (price_total >= 0),
  price_input        REAL CHECK (price_input >= 0),
  price_cache_write  REAL CHECK (price_cache_write >= 0),
  price_cache_read   REAL CHECK (price_cache_read >= 0),
  price_output       REAL CHECK (price_output >= 0),
  calls              INTEGER NOT NULL CHECK (calls >= 0)
) STRICT;
CREATE UNIQUE INDEX usage_costs_by_agent ON usage_costs (conversation_id, ifnull(agent, -1));
CREATE TABLE host_workflows (
  device_id TEXT NOT NULL,
  directory TEXT NOT NULL,
  list      TEXT NOT NULL,
  read_at   INTEGER NOT NULL,
  revision  INTEGER NOT NULL CHECK (revision >= 1),
  PRIMARY KEY (device_id, directory)
) STRICT;
";

/// The id of the one task a daily schedule becomes.
const DAILY_TASK: &str = "daily";

/// A target of format 1 in format 2: an existing conversation queues.
fn automation_target_2(text: &str) -> rusqlite::Result<serde_json::Value> {
    let mut value: serde_json::Value = serde_json::from_str(text).map_err(corrupt)?;
    if value.get("kind").and_then(serde_json::Value::as_str) == Some("existing")
        && let Some(object) = value.as_object_mut()
    {
        object.insert("ifBusy".into(), serde_json::Value::String("queue".into()));
    }
    Ok(value)
}

/// A trigger of format 1 in format 2, with the prompt the automation keeps
/// and the id of the task a daily schedule became.
///
/// A time trigger's time is named `timing`, as the step from 0.1.65 reads
/// it. A daily schedule becomes a repeat of one task, every day when it
/// named all seven, else every week on its days.
fn automation_trigger_2(text: &str, prompt: String) -> rusqlite::Result<(serde_json::Value, String, Option<&'static str>)> {
    /// What format 1 had that format 2 does not.
    #[derive(serde::Deserialize)]
    #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
    enum Daily {
        Daily { time: String, days: Vec<String> },
    }
    let mut value: serde_json::Value = serde_json::from_str(text).map_err(corrupt)?;
    named_timing(&mut value);
    let daily = value
        .get_mut("timing")
        .filter(|timing| timing.get("kind").and_then(serde_json::Value::as_str) == Some("daily"))
        .map(serde_json::Value::take);
    let Some(daily) = daily else {
        return Ok((value, prompt, None));
    };
    let Daily::Daily { time, days } = serde_json::from_value(daily).map_err(corrupt)?;
    let unit = if days.len() == 7 { "days" } else { "weeks" };
    let trigger = serde_json::json!({
        "kind": "time",
        "timing": {
            "kind": "repeat",
            "repeat": { "every": 1, "unit": unit, "start": null },
            "tasks": [{
                "id": DAILY_TASK,
                "time": time,
                "days": days,
                "day": { "kind": "date", "date": 1 },
                "month": 1,
                "holidays": null,
                "prompt": prompt,
            }],
        },
    });
    Ok((trigger, String::new(), Some(DAILY_TASK)))
}
