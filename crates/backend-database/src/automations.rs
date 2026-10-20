//! The users' automations and their runs: each automation's parts as the web
//! contract carries them, its secrets sealed by the caller, a change watch's
//! last output, and every run. The backend's scheduler reads what it needs of
//! these when it starts and when one changes; nothing here polls.

use demi_shared_types::{AutomationId, AutomationRunId, Timestamp, TurnId};
use demi_web_api_protocol::automations::{AutomationTarget, PauseReason, RunNote, RunState, Trigger};
use demi_web_api_protocol::conversations::ModelSettings;
use demi_web_api_protocol::ids::{ConversationId, UserId};
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::StorageError;
use super::columns::{decode, instant, json, to_json};
use super::control::ControlService;

const TABLE: &str = "automations";
const RUNS: &str = "automation_runs";
const CALLS: &str = "automation_calls";

const COLUMNS: &str = "id, user_id, name, trigger, time_zone, max_runs_per_day, target, prompt, model,
     paused, paused_because, schedule_since, secret IS NOT NULL AS has_secret,
     stripe_secret IS NOT NULL AS has_stripe, watch_stdout, watch_digest, watch_exit, watch_checked_at,
     created_at, updated_at";

const RUN_COLUMNS: &str = "id, automation_id, cause, at, state, note, line, conversation_id, turn_id, filed, unread, notified";

/// An automation as it is stored, without its secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAutomation {
    pub id: AutomationId,
    pub user: UserId,
    pub name: String,
    pub trigger: Trigger,
    pub time_zone: String,
    pub max_runs_per_day: Option<u32>,
    pub target: AutomationTarget,
    pub prompt: String,
    pub model: Option<ModelSettings>,
    pub paused: bool,
    pub paused_because: Option<PauseReason>,
    /// When its trigger was last set or it was last turned on: no time of
    /// its schedule before this fires.
    pub schedule_since: Timestamp,
    /// Whether it holds a URL trigger's secret.
    pub has_secret: bool,
    /// Whether it holds a Stripe signing secret.
    pub stripe_signing: bool,
    /// A change watch's last output; none before its first run.
    pub watch: Option<WatchOutput>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// What a change watch's command last gave: its standard output's last
/// 16 KB and its exit status, and when it ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchOutput {
    /// The last 16 KB of its standard output, which a run's message shows.
    pub stdout: Vec<u8>,
    /// The SHA-256 of the whole of it, which the next check compares.
    pub digest: Vec<u8>,
    pub exit: i32,
    pub checked_at: Timestamp,
}

/// A call of a URL to keep: held when it starts nothing, as for a paused
/// automation or one at its daily limit, which keeps only its delivery ID.
#[derive(Debug, Clone)]
pub struct NewCall {
    pub at: Timestamp,
    pub delivery: Option<String>,
    pub held: bool,
    pub method: String,
    /// Each header, as the run's message shows them.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// A call's headers as their column holds them: automation call headers
/// format 1, a JSON array of name and value pairs.
#[derive(serde::Serialize, serde::Deserialize, garde::Validate)]
struct CallHeaders(#[garde(skip)] Vec<(String, String)>);

/// A call waiting for a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingCall {
    pub at: Timestamp,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// What fired a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunCause {
    Time,
    Change,
    Url,
    /// The user's Run Now, or an agent's `demi automation run`.
    Now,
}

impl RunCause {
    fn as_str(self) -> &'static str {
        match self {
            Self::Time => "time",
            Self::Change => "change",
            Self::Url => "url",
            Self::Now => "now",
        }
    }

    fn of(text: &str) -> Result<Self, StorageError> {
        Ok(match text {
            "time" => Self::Time,
            "change" => Self::Change,
            "url" => Self::Url,
            "now" => Self::Now,
            other => {
                return Err(StorageError::Corrupt {
                    table: RUNS,
                    column: "cause",
                    reason: format!("unknown cause {other}"),
                });
            }
        })
    }
}

/// One run as it is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRun {
    pub id: AutomationRunId,
    pub automation: AutomationId,
    pub cause: RunCause,
    pub at: Timestamp,
    pub state: RunState,
    pub note: Option<RunNote>,
    pub line: Option<String>,
    pub conversation: Option<ConversationId>,
    pub turn: Option<TurnId>,
    pub filed: bool,
    pub unread: bool,
    /// Its agent ran `demi notify send` while it worked: it ends Notified.
    pub notified: bool,
}

fn state_str(state: RunState) -> &'static str {
    match state {
        RunState::Running => "running",
        RunState::Done => "done",
        RunState::Notified => "notified",
        RunState::Skipped => "skipped",
        RunState::Failed => "failed",
    }
}

fn state_of(text: &str) -> Result<RunState, StorageError> {
    Ok(match text {
        "running" => RunState::Running,
        "done" => RunState::Done,
        "notified" => RunState::Notified,
        "skipped" => RunState::Skipped,
        "failed" => RunState::Failed,
        other => {
            return Err(StorageError::Corrupt {
                table: RUNS,
                column: "state",
                reason: format!("unknown state {other}"),
            });
        }
    })
}

fn reason_str(reason: PauseReason) -> &'static str {
    match reason {
        PauseReason::ConversationArchived => "conversation_archived",
        PauseReason::ConversationDeleted => "conversation_deleted",
    }
}

fn reason_of(text: &str) -> Result<PauseReason, StorageError> {
    Ok(match text {
        "conversation_archived" => PauseReason::ConversationArchived,
        "conversation_deleted" => PauseReason::ConversationDeleted,
        other => {
            return Err(StorageError::Corrupt {
                table: TABLE,
                column: "paused_because",
                reason: format!("unknown reason {other}"),
            });
        }
    })
}

/// A run's note as its column holds it: automation run note format 1.
fn note_text(note: &Option<RunNote>) -> Option<String> {
    note.as_ref().map(to_json)
}

fn automation_row(row: &Row<'_>) -> Result<StoredAutomation, StorageError> {
    let model: Option<String> = row.get("model")?;
    let because: Option<String> = row.get("paused_because")?;
    let stdout: Option<Vec<u8>> = row.get("watch_stdout")?;
    let digest: Option<Vec<u8>> = row.get("watch_digest")?;
    let exit: Option<i32> = row.get("watch_exit")?;
    let checked: Option<i64> = row.get("watch_checked_at")?;
    let watch = match (stdout, digest, exit, checked) {
        (Some(stdout), Some(digest), Some(exit), Some(checked)) => Some(WatchOutput {
            stdout,
            digest,
            exit,
            checked_at: decode(TABLE, "watch_checked_at", Timestamp::from_millisecond(checked))?,
        }),
        (None, None, None, None) => None,
        _ => {
            return Err(StorageError::Corrupt {
                table: TABLE,
                column: "watch_stdout",
                reason: "a watch's output, digest, exit status and time go together".into(),
            });
        }
    };
    let limit: Option<i64> = row.get("max_runs_per_day")?;
    Ok(StoredAutomation {
        id: decode(TABLE, "id", AutomationId::try_from(row.get::<_, String>("id")?))?,
        user: decode(TABLE, "user_id", UserId::try_from(row.get::<_, String>("user_id")?))?,
        name: row.get("name")?,
        trigger: json(TABLE, "trigger", &row.get::<_, String>("trigger")?)?,
        time_zone: row.get("time_zone")?,
        max_runs_per_day: limit
            .map(|limit| decode(TABLE, "max_runs_per_day", u32::try_from(limit)))
            .transpose()?,
        target: json(TABLE, "target", &row.get::<_, String>("target")?)?,
        prompt: row.get("prompt")?,
        model: model.map(|text| json(TABLE, "model", &text)).transpose()?,
        paused: row.get("paused")?,
        paused_because: because.as_deref().map(reason_of).transpose()?,
        schedule_since: instant(row, TABLE, "schedule_since")?,
        has_secret: row.get("has_secret")?,
        stripe_signing: row.get("has_stripe")?,
        watch,
        created_at: instant(row, TABLE, "created_at")?,
        updated_at: instant(row, TABLE, "updated_at")?,
    })
}

fn run_row(row: &Row<'_>) -> Result<StoredRun, StorageError> {
    let note: Option<String> = row.get("note")?;
    let conversation: Option<String> = row.get("conversation_id")?;
    let turn: Option<String> = row.get("turn_id")?;
    Ok(StoredRun {
        id: decode(RUNS, "id", AutomationRunId::try_from(row.get::<_, String>("id")?))?,
        automation: decode(
            RUNS,
            "automation_id",
            AutomationId::try_from(row.get::<_, String>("automation_id")?),
        )?,
        cause: RunCause::of(&row.get::<_, String>("cause")?)?,
        at: instant(row, RUNS, "at")?,
        state: state_of(&row.get::<_, String>("state")?)?,
        note: note
            .map(|text| decode(RUNS, "note", serde_json::from_str::<RunNote>(&text)))
            .transpose()?,
        line: row.get("line")?,
        conversation: conversation
            .map(|id| decode(RUNS, "conversation_id", ConversationId::try_from(id)))
            .transpose()?,
        turn: turn.map(|id| decode(RUNS, "turn_id", TurnId::try_from(id))).transpose()?,
        filed: row.get("filed")?,
        unread: row.get("unread")?,
        notified: row.get("notified")?,
    })
}

fn existing_conversation(target: &AutomationTarget) -> Option<&str> {
    match target {
        AutomationTarget::Existing { conversation_id } => Some(conversation_id.as_str()),
        AutomationTarget::New { .. } => None,
    }
}

fn select_automation(
    connection: &Connection,
    id: &AutomationId,
) -> Result<Option<StoredAutomation>, StorageError> {
    connection
        .prepare_cached(&format!("SELECT {COLUMNS} FROM automations WHERE id = ?1"))?
        .query_row([id.as_str()], |row| Ok(automation_row(row)))
        .optional()?
        .transpose()
}

impl ControlService {
    /// The user's automations, newest first.
    pub async fn automations(&self, user: UserId) -> Result<Vec<StoredAutomation>, StorageError> {
        self.call(move |connection, _| {
            connection
                .prepare_cached(&format!(
                    "SELECT {COLUMNS} FROM automations WHERE user_id = ?1 ORDER BY created_at DESC, id"
                ))?
                .query_map([user.as_str()], |row| Ok(automation_row(row)))?
                .map(|row| row?)
                .collect()
        })
        .await
    }

    /// The user's automation `id`.
    pub async fn automation(
        &self,
        user: UserId,
        id: AutomationId,
    ) -> Result<Option<StoredAutomation>, StorageError> {
        self.call(move |connection, _| {
            Ok(select_automation(connection, &id)?.filter(|automation| automation.user == user))
        })
        .await
    }

    /// The owner of the automation `id`, whoever it is, for its URL's calls,
    /// which carry no session.
    pub async fn automation_owner(&self, id: AutomationId) -> Result<Option<UserId>, StorageError> {
        self.call(move |connection, _| {
            connection
                .query_row("SELECT user_id FROM automations WHERE id = ?1", [id.as_str()], |row| {
                    row.get::<_, String>(0)
                })
                .optional()?
                .map(|user| decode(TABLE, "user_id", UserId::try_from(user)))
                .transpose()
        })
        .await
    }

    /// Every user who has an automation, whose scheduler the start opens.
    pub async fn users_with_automations(&self) -> Result<Vec<UserId>, StorageError> {
        self.call(move |connection, _| {
            connection
                .prepare("SELECT DISTINCT user_id FROM automations ORDER BY user_id")?
                .query_map([], |row| row.get::<_, String>(0))?
                .map(|user| decode(TABLE, "user_id", UserId::try_from(user?)))
                .collect()
        })
        .await
    }

    /// Writes `automation`, new or changed, with its secret and Stripe
    /// secret as sealed bytes when they change: `Some(None)` removes one.
    pub async fn put_automation(
        &self,
        automation: StoredAutomation,
        secret: Option<Option<Vec<u8>>>,
        stripe_secret: Option<Option<Vec<u8>>>,
    ) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let a = &automation;
            transaction.execute(
                "INSERT INTO automations (id, user_id, name, trigger, time_zone, max_runs_per_day, target,
                   conversation_id, prompt, model, paused, paused_because, schedule_since, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
                 ON CONFLICT (id) DO UPDATE SET name = ?3, trigger = ?4, time_zone = ?5,
                   max_runs_per_day = ?6, target = ?7, conversation_id = ?8, prompt = ?9, model = ?10,
                   paused = ?11, paused_because = ?12, schedule_since = ?13, updated_at = ?15
                 WHERE user_id = ?2",
                params![
                    a.id.as_str(),
                    a.user.as_str(),
                    a.name,
                    to_json(&a.trigger),
                    a.time_zone,
                    a.max_runs_per_day,
                    to_json(&a.target),
                    existing_conversation(&a.target),
                    a.prompt,
                    a.model.as_ref().map(to_json),
                    a.paused,
                    a.paused_because.map(reason_str),
                    a.schedule_since.as_millisecond(),
                    a.created_at.as_millisecond(),
                    a.updated_at.as_millisecond(),
                ],
            )?;
            if let Some(secret) = secret {
                transaction.execute(
                    "UPDATE automations SET secret = ?2 WHERE id = ?1",
                    params![a.id.as_str(), secret],
                )?;
            }
            if let Some(secret) = stripe_secret {
                transaction.execute(
                    "UPDATE automations SET stripe_secret = ?2 WHERE id = ?1",
                    params![a.id.as_str(), secret],
                )?;
            }
            // A trigger that is no longer a change watch keeps no output.
            if !matches!(a.trigger, Trigger::Change { .. }) {
                transaction.execute(
                    "UPDATE automations SET watch_stdout = NULL, watch_digest = NULL, watch_exit = NULL,
                       watch_checked_at = NULL
                     WHERE id = ?1",
                    [a.id.as_str()],
                )?;
            }
            transaction.commit()?;
            Ok(())
        })
        .await
    }

    /// The sealed secret and Stripe secret of the automation `id`.
    pub async fn automation_secrets(
        &self,
        id: AutomationId,
    ) -> Result<(Option<Vec<u8>>, Option<Vec<u8>>), StorageError> {
        self.call(move |connection, _| {
            Ok(connection
                .query_row(
                    "SELECT secret, stripe_secret FROM automations WHERE id = ?1",
                    [id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?
                .unwrap_or((None, None)))
        })
        .await
    }

    /// Records what a change watch's command gave, or forgets it with none,
    /// so the next run only records again.
    pub async fn set_watch_output(
        &self,
        id: AutomationId,
        output: Option<WatchOutput>,
    ) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            let (stdout, digest, exit, at) = match &output {
                Some(output) => (
                    Some(output.stdout.as_slice()),
                    Some(output.digest.as_slice()),
                    Some(output.exit),
                    Some(output.checked_at.as_millisecond()),
                ),
                None => (None, None, None, None),
            };
            connection.execute(
                "UPDATE automations SET watch_stdout = ?2, watch_digest = ?3, watch_exit = ?4, watch_checked_at = ?5
                 WHERE id = ?1",
                params![id.as_str(), stdout, digest, exit, at],
            )?;
            Ok(())
        })
        .await
    }

    /// Deletes the user's automation `id` with its runs' records; their
    /// conversations stay and join the sidebar. Answers those conversations,
    /// none when the user has no such automation.
    pub async fn delete_automation(
        &self,
        user: UserId,
        id: AutomationId,
    ) -> Result<Option<Vec<ConversationId>>, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let owned: Option<String> = transaction
                .query_row(
                    "SELECT id FROM automations WHERE id = ?1 AND user_id = ?2",
                    params![id.as_str(), user.as_str()],
                    |row| row.get(0),
                )
                .optional()?;
            if owned.is_none() {
                return Ok(None);
            }
            let filed = filed_conversations(&transaction, "automation_id = ?1", id.as_str())?;
            transaction.execute("DELETE FROM automation_runs WHERE automation_id = ?1", [id.as_str()])?;
            transaction.execute("DELETE FROM automations WHERE id = ?1", [id.as_str()])?;
            transaction.commit()?;
            Ok(Some(filed))
        })
        .await
    }

    /// The latest runs of the automation `id`, newest first.
    pub async fn automation_runs(
        &self,
        id: AutomationId,
        limit: usize,
    ) -> Result<Vec<StoredRun>, StorageError> {
        self.call(move |connection, _| {
            connection
                .prepare_cached(&format!(
                    "SELECT {RUN_COLUMNS} FROM automation_runs WHERE automation_id = ?1
                     ORDER BY at DESC, rowid DESC LIMIT ?2"
                ))?
                .query_map(params![id.as_str(), limit as i64], |row| Ok(run_row(row)))?
                .map(|row| row?)
                .collect()
        })
        .await
    }

    /// The run `id`.
    pub async fn automation_run(&self, id: AutomationRunId) -> Result<Option<StoredRun>, StorageError> {
        self.call(move |connection, _| {
            connection
                .prepare_cached(&format!("SELECT {RUN_COLUMNS} FROM automation_runs WHERE id = ?1"))?
                .query_row([id.as_str()], |row| Ok(run_row(row)))
                .optional()?
                .transpose()
        })
        .await
    }

    /// The agent working for the run `id` ran `demi notify send`: the run
    /// notified, which leaves it unread until the user opens its
    /// conversation; one that already ended Done ends Notified. Answers
    /// whether the run is there.
    pub async fn notify_run(&self, id: AutomationRunId) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let changed = connection.execute(
                "UPDATE automation_runs SET notified = 1, unread = 1,
                   state = CASE state WHEN 'done' THEN 'notified' ELSE state END
                 WHERE id = ?1",
                [id.as_str()],
            )?;
            Ok(changed > 0)
        })
        .await
    }

    /// How many of the user's runs notified and their conversations were
    /// not opened since, whatever their age: the sidebar's count.
    pub async fn unread_automation_runs(&self, user: UserId) -> Result<u32, StorageError> {
        self.call(move |connection, _| {
            let count: i64 = connection.query_row(
                "SELECT count(*) FROM automation_runs r JOIN automations a ON a.id = r.automation_id
                 WHERE a.user_id = ?1 AND r.unread",
                [user.as_str()],
                |row| row.get(0),
            )?;
            Ok(u32::try_from(count).unwrap_or(u32::MAX))
        })
        .await
    }

    /// The latest run whose message went to the conversation `id`, of any
    /// age.
    pub async fn run_of_conversation(&self, id: ConversationId) -> Result<Option<StoredRun>, StorageError> {
        self.call(move |connection, _| {
            connection
                .prepare_cached(&format!(
                    "SELECT {RUN_COLUMNS} FROM automation_runs WHERE conversation_id = ?1 ORDER BY at DESC, rowid DESC
                     LIMIT 1"
                ))?
                .query_row([id.as_str()], |row| Ok(run_row(row)))
                .optional()?
                .transpose()
        })
        .await
    }

    /// Records a run, as it starts or as it is skipped.
    pub async fn insert_automation_run(&self, run: StoredRun) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                &format!(
                    "INSERT INTO automation_runs ({RUN_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"
                ),
                params![
                    run.id.as_str(),
                    run.automation.as_str(),
                    run.cause.as_str(),
                    run.at.as_millisecond(),
                    state_str(run.state),
                    note_text(&run.note),
                    run.line,
                    run.conversation.as_ref().map(|id| id.as_str()),
                    run.turn.as_ref().map(|id| id.as_str()),
                    run.filed,
                    run.unread,
                    run.notified,
                ],
            )?;
            Ok(())
        })
        .await
    }

    /// Writes a run's state, note, line, conversation and turn as `run`
    /// holds them.
    pub async fn update_automation_run(&self, run: StoredRun) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "UPDATE automation_runs SET state = ?2, note = ?3, line = ?4, conversation_id = ?5,
                   turn_id = ?6, filed = ?7, unread = ?8, notified = ?9 WHERE id = ?1",
                params![
                    run.id.as_str(),
                    state_str(run.state),
                    note_text(&run.note),
                    run.line,
                    run.conversation.as_ref().map(|id| id.as_str()),
                    run.turn.as_ref().map(|id| id.as_str()),
                    run.filed,
                    run.unread,
                    run.notified,
                ],
            )?;
            Ok(())
        })
        .await
    }

    /// Keeps a call of the automation `id`'s URL as `state` holds it, and
    /// answers whether it was taken: false, and nothing kept, when a call
    /// carried its delivery ID since `since`. The calls no run waits for
    /// that are older than `since` are forgotten, in the same transaction.
    pub async fn accept_call(&self, id: AutomationId, call: NewCall, since: Timestamp) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            transaction.execute(
                "DELETE FROM automation_calls WHERE automation_id = ?1 AND state <> 'pending' AND at < ?2",
                params![id.as_str(), since.as_millisecond()],
            )?;
            if let Some(delivery) = &call.delivery {
                let seen: bool = transaction.query_row(
                    "SELECT EXISTS (SELECT 1 FROM automation_calls
                                    WHERE automation_id = ?1 AND delivery_id = ?2 AND at >= ?3)",
                    params![id.as_str(), delivery, since.as_millisecond()],
                    |row| row.get(0),
                )?;
                if seen {
                    return Ok(false);
                }
            }
            let (state, method, headers, body) = match call.held {
                true => ("held", None, None, None),
                false => (
                    "pending",
                    Some(call.method),
                    Some(to_json(&CallHeaders(call.headers))),
                    Some(call.body),
                ),
            };
            transaction.execute(
                "INSERT INTO automation_calls (automation_id, at, delivery_id, state, method, headers, body)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![id.as_str(), call.at.as_millisecond(), call.delivery, state, method, headers, body],
            )?;
            transaction.commit()?;
            Ok(true)
        })
        .await
    }

    /// Takes the automation `id`'s pending calls for a run, oldest first:
    /// they are no longer pending, and keep only their delivery IDs.
    pub async fn take_pending_calls(&self, id: AutomationId) -> Result<Vec<PendingCall>, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let calls = transaction
                .prepare(
                    "SELECT at, method, headers, body FROM automation_calls
                     WHERE automation_id = ?1 AND state = 'pending' ORDER BY id",
                )?
                .query_map([id.as_str()], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                    ))
                })?
                .map(|row| {
                    let (at, method, headers, body) = row?;
                    Ok(PendingCall {
                        at: decode(CALLS, "at", Timestamp::from_millisecond(at))?,
                        method,
                        headers: json::<CallHeaders>(CALLS, "headers", &headers)?.0,
                        body,
                    })
                })
                .collect::<Result<Vec<_>, StorageError>>()?;
            transaction.execute(
                "UPDATE automation_calls SET state = 'taken', method = NULL, headers = NULL, body = NULL
                 WHERE automation_id = ?1 AND state = 'pending'",
                [id.as_str()],
            )?;
            transaction.commit()?;
            Ok(calls)
        })
        .await
    }

    /// Drops the automation `id`'s pending calls, as a pause or an edit of
    /// its trigger does: they start nothing.
    pub async fn drop_pending_calls(&self, id: AutomationId) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "UPDATE automation_calls SET state = 'held', method = NULL, headers = NULL, body = NULL
                 WHERE automation_id = ?1 AND state = 'pending'",
                [id.as_str()],
            )?;
            Ok(())
        })
        .await
    }

    /// The user's automations that have calls waiting for a run.
    pub async fn automations_with_pending_calls(&self, user: UserId) -> Result<Vec<AutomationId>, StorageError> {
        self.call(move |connection, _| {
            connection
                .prepare(
                    "SELECT DISTINCT c.automation_id FROM automation_calls c
                     JOIN automations a ON a.id = c.automation_id
                     WHERE a.user_id = ?1 AND c.state = 'pending'",
                )?
                .query_map([user.as_str()], |row| row.get::<_, String>(0))?
                .map(|id| decode(CALLS, "automation_id", AutomationId::try_from(id?)))
                .collect()
        })
        .await
    }

    /// Whether the automation `id` has calls waiting for a run.
    pub async fn has_pending_calls(&self, id: AutomationId) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            Ok(connection.query_row(
                "SELECT EXISTS (SELECT 1 FROM automation_calls WHERE automation_id = ?1 AND state = 'pending')",
                [id.as_str()],
                |row| row.get(0),
            )?)
        })
        .await
    }

    /// The runs of the automation `id` since `since` that started work,
    /// which its daily limit counts: every run but a skipped one.
    pub async fn automation_runs_since(
        &self,
        id: AutomationId,
        since: Timestamp,
    ) -> Result<u32, StorageError> {
        self.call(move |connection, _| {
            let count: i64 = connection.query_row(
                "SELECT count(*) FROM automation_runs WHERE automation_id = ?1 AND at >= ?2 AND state != 'skipped'",
                params![id.as_str(), since.as_millisecond()],
                |row| row.get(0),
            )?;
            decode(RUNS, "id", u32::try_from(count))
        })
        .await
    }

    /// The skipped run since `since` that says the automation's limit was
    /// reached, whose count of calls grows.
    pub async fn automation_limit_run(
        &self,
        id: AutomationId,
        since: Timestamp,
    ) -> Result<Option<StoredRun>, StorageError> {
        self.call(move |connection, _| {
            connection
                .prepare_cached(&format!(
                    "SELECT {RUN_COLUMNS} FROM automation_runs WHERE automation_id = ?1 AND at >= ?2
                       AND state = 'skipped' AND json_extract(note, '$.kind') = 'limit'
                     ORDER BY at DESC LIMIT 1"
                ))?
                .query_row(params![id.as_str(), since.as_millisecond()], |row| Ok(run_row(row)))
                .optional()?
                .transpose()
        })
        .await
    }

    /// When a time trigger of the automation `id` last fired.
    pub async fn automation_last_fired(&self, id: AutomationId) -> Result<Option<Timestamp>, StorageError> {
        self.call(move |connection, _| {
            let at: Option<i64> = connection.query_row(
                "SELECT max(at) FROM automation_runs WHERE automation_id = ?1 AND cause = 'time'",
                [id.as_str()],
                |row| row.get(0),
            )?;
            at.map(|at| decode(RUNS, "at", Timestamp::from_millisecond(at))).transpose()
        })
        .await
    }

    /// The running runs whose message went to the conversation `id`.
    pub async fn running_runs_in(&self, id: ConversationId) -> Result<Vec<StoredRun>, StorageError> {
        self.call(move |connection, _| {
            connection
                .prepare_cached(&format!(
                    "SELECT {RUN_COLUMNS} FROM automation_runs WHERE conversation_id = ?1 AND state = 'running'
                     ORDER BY at"
                ))?
                .query_map([id.as_str()], |row| Ok(run_row(row)))?
                .map(|row| row?)
                .collect()
        })
        .await
    }

    /// The running runs of the user's automations, which a start settles.
    pub async fn running_runs(&self, user: UserId) -> Result<Vec<StoredRun>, StorageError> {
        self.call(move |connection, _| {
            connection
                .prepare(&format!(
                    "SELECT {} FROM automation_runs r JOIN automations a ON a.id = r.automation_id
                     WHERE a.user_id = ?1 AND r.state = 'running' ORDER BY r.at",
                    RUN_COLUMNS
                        .split(", ")
                        .map(|column| format!("r.{column}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ))?
                .query_map([user.as_str()], |row| Ok(run_row(row)))?
                .map(|row| row?)
                .collect()
        })
        .await
    }

    /// Lets the conversation `id` out of its automation into the sidebar,
    /// as the user's message or Move to Conversations does. Answers whether
    /// it was filed.
    pub async fn unfile_conversation(&self, id: ConversationId) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let changed = connection.execute(
                "UPDATE automation_runs SET filed = 0 WHERE conversation_id = ?1 AND filed",
                [id.as_str()],
            )?;
            Ok(changed > 0)
        })
        .await
    }

    /// Marks the runs of the conversation `id` read, as opening it does.
    /// Answers whether one was unread.
    pub async fn read_automation_runs(&self, id: ConversationId) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let changed = connection.execute(
                "UPDATE automation_runs SET unread = 0 WHERE conversation_id = ?1 AND unread",
                [id.as_str()],
            )?;
            Ok(changed > 0)
        })
        .await
    }

    /// The conversation `id` was deleted: its runs keep no conversation, and
    /// the automations that woke it pause, saying why. Answers whether any
    /// record changed.
    pub async fn conversation_gone_from_automations(
        &self,
        id: ConversationId,
        reason: PauseReason,
    ) -> Result<bool, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let mut changed = 0;
            if reason == PauseReason::ConversationDeleted {
                changed += transaction.execute(
                    "UPDATE automation_runs SET conversation_id = NULL, filed = 0, unread = 0
                     WHERE conversation_id = ?1",
                    [id.as_str()],
                )?;
            }
            changed += transaction.execute(
                "UPDATE automations SET paused = 1, paused_because = ?2, updated_at = ?3
                 WHERE conversation_id = ?1 AND NOT (paused AND paused_because IS ?2)",
                params![id.as_str(), reason_str(reason), now.as_millisecond()],
            )?;
            transaction.commit()?;
            Ok(changed > 0)
        })
        .await
    }
}

/// The conversations of the runs `filter` selects that are filed, with
/// `value` as its one parameter.
fn filed_conversations(
    connection: &Connection,
    filter: &str,
    value: &str,
) -> Result<Vec<ConversationId>, StorageError> {
    connection
        .prepare(&format!(
            "SELECT conversation_id FROM automation_runs WHERE {filter} AND filed AND conversation_id IS NOT NULL"
        ))?
        .query_map([value], |row| row.get::<_, String>(0))?
        .map(|id| decode(RUNS, "conversation_id", ConversationId::try_from(id?)))
        .collect()
}
