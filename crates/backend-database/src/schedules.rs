//! The conversations' schedules and their runs.
//!
//! The module stores each schedule's parts as the web contract carries them,
//! its URL secret sealed by the caller, a change watch's last output, and every
//! run. A schedule belongs to one conversation, whose owner is the schedule's:
//! the schedule is deleted with the conversation. The backend's scheduler reads
//! what it needs of these when it starts and when one changes; nothing here
//! polls.

use demi_shared_types::{ScheduleId, ScheduleRunId, Timestamp, TurnId};
use demi_web_api_protocol::ids::{ConversationId, UserId};
use demi_web_api_protocol::schedules::{IfBusy, RunNote, RunState, Trigger};
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::StorageError;
use super::columns::{decode, instant, json, to_json};
use super::control::ControlService;

const TABLE: &str = "schedules";
const RUNS: &str = "schedule_runs";
const CALLS: &str = "schedule_calls";

const COLUMNS: &str = "id, conversation_id, name, trigger, time_zone, max_runs_per_day, if_busy, prompt,
     paused, since, secret IS NOT NULL AS has_secret,
     (SELECT archived FROM conversations WHERE id = schedules.conversation_id) AS conversation_archived,
     watch_stdout, watch_digest, watch_exit, watch_checked_at, created_at, updated_at";

const RUN_COLUMNS: &str = "id, schedule_id, cause, at, state, note, line, turn_id, notified, task_id";

/// A schedule as it is stored, without its secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSchedule {
    pub id: ScheduleId,
    /// The conversation it wakes, and nothing else.
    pub conversation: ConversationId,
    pub name: String,
    pub trigger: Trigger,
    pub time_zone: String,
    pub max_runs_per_day: Option<u32>,
    pub if_busy: IfBusy,
    pub prompt: String,
    pub paused: bool,
    /// Whether its conversation is archived, which keeps it from waking the
    /// conversation while it is. Read with the schedule, never kept with it.
    pub conversation_archived: bool,
    /// When its trigger was last set or it was last turned on: no time of
    /// its timing before this fires.
    pub since: Timestamp,
    /// Whether it holds a URL trigger's secret.
    pub has_secret: bool,
    /// A change watch's last output; none before its first run.
    pub watch: Option<WatchOutput>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl StoredSchedule {
    /// Whether it wakes its conversation: neither paused, by the user, nor
    /// held back while its conversation is archived.
    pub fn on(&self) -> bool {
        !self.paused && !self.conversation_archived
    }
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
/// schedule or one at its daily limit, which keeps only its delivery ID.
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

/// A call's headers as their column holds them: schedule call headers
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
    /// An agent's `demi schedule run`.
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
    pub id: ScheduleRunId,
    pub schedule: ScheduleId,
    pub cause: RunCause,
    pub at: Timestamp,
    /// The repeat's task that started it; none for a single trigger.
    pub task: Option<String>,
    pub state: RunState,
    pub note: Option<RunNote>,
    pub line: Option<String>,
    /// The turn its message started; none for a run skipped before it.
    pub turn: Option<TurnId>,
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
fn busy_str(if_busy: IfBusy) -> &'static str {
    match if_busy {
        IfBusy::Queue => "queue",
        IfBusy::Steer => "steer",
    }
}

fn busy_of(text: &str) -> Result<IfBusy, StorageError> {
    Ok(match text {
        "queue" => IfBusy::Queue,
        "steer" => IfBusy::Steer,
        other => {
            return Err(StorageError::Corrupt {
                table: TABLE,
                column: "if_busy",
                reason: format!("unknown choice {other}"),
            });
        }
    })
}

/// A run's note as its column holds it: schedule run note format 1.
fn note_text(note: &Option<RunNote>) -> Option<String> {
    note.as_ref().map(to_json)
}
fn schedule_row(row: &Row<'_>) -> Result<StoredSchedule, StorageError> {
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
    Ok(StoredSchedule {
        id: decode(TABLE, "id", ScheduleId::try_from(row.get::<_, String>("id")?))?,
        conversation: decode(TABLE, "conversation_id", ConversationId::try_from(row.get::<_, String>("conversation_id")?))?,
        name: row.get("name")?,
        trigger: json(TABLE, "trigger", &row.get::<_, String>("trigger")?)?,
        time_zone: row.get("time_zone")?,
        max_runs_per_day: limit
            .map(|limit| decode(TABLE, "max_runs_per_day", u32::try_from(limit)))
            .transpose()?,
        if_busy: busy_of(&row.get::<_, String>("if_busy")?)?,
        prompt: row.get("prompt")?,
        paused: row.get("paused")?,
        conversation_archived: row.get("conversation_archived")?,
        since: instant(row, TABLE, "since")?,
        has_secret: row.get("has_secret")?,
        watch,
        created_at: instant(row, TABLE, "created_at")?,
        updated_at: instant(row, TABLE, "updated_at")?,
    })
}

fn run_row(row: &Row<'_>) -> Result<StoredRun, StorageError> {
    let note: Option<String> = row.get("note")?;
    let turn: Option<String> = row.get("turn_id")?;
    Ok(StoredRun {
        id: decode(RUNS, "id", ScheduleRunId::try_from(row.get::<_, String>("id")?))?,
        schedule: decode(RUNS, "schedule_id", ScheduleId::try_from(row.get::<_, String>("schedule_id")?))?,
        cause: RunCause::of(&row.get::<_, String>("cause")?)?,
        at: instant(row, RUNS, "at")?,
        task: row.get("task_id")?,
        state: state_of(&row.get::<_, String>("state")?)?,
        note: note
            .map(|text| decode(RUNS, "note", serde_json::from_str::<RunNote>(&text)))
            .transpose()?,
        line: row.get("line")?,
        turn: turn.map(|id| decode(RUNS, "turn_id", TurnId::try_from(id))).transpose()?,
        notified: row.get("notified")?,
    })
}

fn select_schedule(connection: &Connection, id: &ScheduleId) -> Result<Option<StoredSchedule>, StorageError> {
    connection
        .prepare_cached(&format!("SELECT {COLUMNS} FROM schedules WHERE id = ?1"))?
        .query_row([id.as_str()], |row| Ok(schedule_row(row)))
        .optional()?
        .transpose()
}

impl ControlService {
    /// The user's schedules, newest first: every conversation's.
    pub async fn schedules(&self, user: UserId) -> Result<Vec<StoredSchedule>, StorageError> {
        self.call(move |connection, _| {
            connection
                .prepare_cached(&format!(
                    "SELECT {COLUMNS} FROM schedules
                     WHERE conversation_id IN (SELECT id FROM conversations WHERE user_id = ?1)
                     ORDER BY created_at DESC, id"
                ))?
                .query_map([user.as_str()], |row| Ok(schedule_row(row)))?
                .map(|row| row?)
                .collect()
        })
        .await
    }

    /// The conversation's schedules, newest first.
    pub async fn conversation_schedules(&self, conversation: ConversationId) -> Result<Vec<StoredSchedule>, StorageError> {
        self.call(move |connection, _| {
            connection
                .prepare_cached(&format!(
                    "SELECT {COLUMNS} FROM schedules WHERE conversation_id = ?1 ORDER BY created_at DESC, id"
                ))?
                .query_map([conversation.as_str()], |row| Ok(schedule_row(row)))?
                .map(|row| row?)
                .collect()
        })
        .await
    }

    /// The schedule `id` of the conversation `conversation`, while `user` owns
    /// that conversation: what the user's page and the conversation's agent
    /// may read and change. The conversation is matched as its record spells
    /// it.
    pub async fn owned_schedule(
        &self,
        user: UserId,
        conversation: ConversationId,
        id: ScheduleId,
    ) -> Result<Option<StoredSchedule>, StorageError> {
        self.call(move |connection, _| {
            connection
                .prepare_cached(&format!(
                    "SELECT {COLUMNS} FROM schedules
                     WHERE id = ?1
                       AND conversation_id = (SELECT id FROM conversations WHERE id = ?2 AND user_id = ?3)"
                ))?
                .query_row(params![id.as_str(), conversation.as_str(), user.as_str()], |row| Ok(schedule_row(row)))
                .optional()?
                .transpose()
        })
        .await
    }

    /// The schedule `id`, of whichever conversation.
    pub async fn schedule(&self, id: ScheduleId) -> Result<Option<StoredSchedule>, StorageError> {
        self.call(move |connection, _| select_schedule(connection, &id)).await
    }

    /// The owner of the schedule `id`'s conversation, whoever it is, for its
    /// URL's calls, which carry no session.
    pub async fn schedule_owner(&self, id: ScheduleId) -> Result<Option<UserId>, StorageError> {
        self.call(move |connection, _| {
            connection
                .query_row(
                    "SELECT c.user_id FROM schedules s JOIN conversations c ON c.id = s.conversation_id
                     WHERE s.id = ?1",
                    [id.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
                .map(|user| decode(TABLE, "user_id", UserId::try_from(user)))
                .transpose()
        })
        .await
    }

    /// Every user whose conversations have a schedule, whose scheduler the
    /// start opens.
    pub async fn users_with_schedules(&self) -> Result<Vec<UserId>, StorageError> {
        self.call(move |connection, _| {
            connection
                .prepare(
                    "SELECT DISTINCT c.user_id FROM schedules s JOIN conversations c ON c.id = s.conversation_id
                     ORDER BY c.user_id",
                )?
                .query_map([], |row| row.get::<_, String>(0))?
                .map(|user| decode(TABLE, "user_id", UserId::try_from(user?)))
                .collect()
        })
        .await
    }

    /// Writes `schedule`, new or changed, with its URL secret as sealed bytes
    /// when it changes: `Some(None)` removes it.
    pub async fn put_schedule(
        &self,
        schedule: StoredSchedule,
        secret: Option<Option<Vec<u8>>>,
    ) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let a = &schedule;
            transaction.execute(
                "INSERT INTO schedules (id, conversation_id, name, trigger, time_zone, max_runs_per_day, if_busy,
                   prompt, paused, since, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT (id) DO UPDATE SET name = ?3, trigger = ?4, time_zone = ?5,
                   max_runs_per_day = ?6, if_busy = ?7, prompt = ?8, paused = ?9,
                   since = ?10, updated_at = ?12
                 WHERE conversation_id = ?2",
                params![
                    a.id.as_str(),
                    a.conversation.as_str(),
                    a.name,
                    to_json(&a.trigger),
                    a.time_zone,
                    a.max_runs_per_day,
                    busy_str(a.if_busy),
                    a.prompt,
                    a.paused,
                    a.since.as_millisecond(),
                    a.created_at.as_millisecond(),
                    a.updated_at.as_millisecond(),
                ],
            )?;
            if let Some(secret) = secret {
                transaction.execute(
                    "UPDATE schedules SET secret = ?2 WHERE id = ?1",
                    params![a.id.as_str(), secret],
                )?;
            }
            // A trigger that is no longer a change watch keeps no output.
            if !matches!(a.trigger, Trigger::Change { .. }) {
                transaction.execute(
                    "UPDATE schedules SET watch_stdout = NULL, watch_digest = NULL, watch_exit = NULL,
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

    /// The sealed URL secret of the schedule `id`.
    pub async fn schedule_secret(&self, id: ScheduleId) -> Result<Option<Vec<u8>>, StorageError> {
        self.call(move |connection, _| {
            Ok(connection
                .query_row("SELECT secret FROM schedules WHERE id = ?1", [id.as_str()], |row| row.get(0))
                .optional()?
                .flatten())
        })
        .await
    }

    /// Records what a change watch's command gave, or forgets it with none,
    /// so the next run only records again.
    pub async fn set_watch_output(
        &self,
        id: ScheduleId,
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
                "UPDATE schedules SET watch_stdout = ?2, watch_digest = ?3, watch_exit = ?4, watch_checked_at = ?5
                 WHERE id = ?1",
                params![id.as_str(), stdout, digest, exit, at],
            )?;
            Ok(())
        })
        .await
    }

    /// Deletes the schedule `id` with its runs' records and its calls.
    /// Answers whether there was one.
    pub async fn delete_schedule(&self, id: ScheduleId) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            Ok(connection.execute("DELETE FROM schedules WHERE id = ?1", [id.as_str()])? > 0)
        })
        .await
    }

    /// The latest runs of the schedule `id`, newest first.
    pub async fn schedule_runs(&self, id: ScheduleId, limit: usize) -> Result<Vec<StoredRun>, StorageError> {
        self.call(move |connection, _| {
            connection
                .prepare_cached(&format!(
                    "SELECT {RUN_COLUMNS} FROM schedule_runs WHERE schedule_id = ?1
                     ORDER BY at DESC, rowid DESC LIMIT ?2"
                ))?
                .query_map(params![id.as_str(), limit as i64], |row| Ok(run_row(row)))?
                .map(|row| row?)
                .collect()
        })
        .await
    }

    /// The run `id`.
    pub async fn schedule_run(&self, id: ScheduleRunId) -> Result<Option<StoredRun>, StorageError> {
        self.call(move |connection, _| {
            connection
                .prepare_cached(&format!("SELECT {RUN_COLUMNS} FROM schedule_runs WHERE id = ?1"))?
                .query_row([id.as_str()], |row| Ok(run_row(row)))
                .optional()?
                .transpose()
        })
        .await
    }

    /// The agent working for the run `id` ran `demi notify send`: the run
    /// notified; one that already ended Done ends Notified. Answers whether
    /// the run is there.
    pub async fn notify_run(&self, id: ScheduleRunId) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let changed = connection.execute(
                "UPDATE schedule_runs SET notified = 1,
                   state = CASE state WHEN 'done' THEN 'notified' ELSE state END
                 WHERE id = ?1",
                [id.as_str()],
            )?;
            Ok(changed > 0)
        })
        .await
    }

    /// Records a run, as it starts or as it is skipped.
    pub async fn insert_schedule_run(&self, run: StoredRun) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                &format!("INSERT INTO schedule_runs ({RUN_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"),
                params![
                    run.id.as_str(),
                    run.schedule.as_str(),
                    run.cause.as_str(),
                    run.at.as_millisecond(),
                    state_str(run.state),
                    note_text(&run.note),
                    run.line,
                    run.turn.as_ref().map(|id| id.as_str()),
                    run.notified,
                    run.task,
                ],
            )?;
            Ok(())
        })
        .await
    }

    /// Writes a run's state, note, line and turn as `run` holds them.
    pub async fn update_schedule_run(&self, run: StoredRun) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "UPDATE schedule_runs SET state = ?2, note = ?3, line = ?4, turn_id = ?5, notified = ?6
                 WHERE id = ?1",
                params![
                    run.id.as_str(),
                    state_str(run.state),
                    note_text(&run.note),
                    run.line,
                    run.turn.as_ref().map(|id| id.as_str()),
                    run.notified,
                ],
            )?;
            Ok(())
        })
        .await
    }

    /// Keeps a call of the schedule `id`'s URL as `state` holds it, and
    /// answers whether it was taken: false, and nothing kept, when a call
    /// carried its delivery ID since `since`. The calls no run waits for
    /// that are older than `since` are forgotten, in the same transaction.
    pub async fn accept_call(&self, id: ScheduleId, call: NewCall, since: Timestamp) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            transaction.execute(
                "DELETE FROM schedule_calls WHERE schedule_id = ?1 AND state <> 'pending' AND at < ?2",
                params![id.as_str(), since.as_millisecond()],
            )?;
            if let Some(delivery) = &call.delivery {
                let seen: bool = transaction.query_row(
                    "SELECT EXISTS (SELECT 1 FROM schedule_calls
                                    WHERE schedule_id = ?1 AND delivery_id = ?2 AND at >= ?3)",
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
                "INSERT INTO schedule_calls (schedule_id, at, delivery_id, state, method, headers, body)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![id.as_str(), call.at.as_millisecond(), call.delivery, state, method, headers, body],
            )?;
            transaction.commit()?;
            Ok(true)
        })
        .await
    }

    /// Takes the schedule `id`'s pending calls for a run, oldest first:
    /// they are no longer pending, and keep only their delivery IDs.
    pub async fn take_pending_calls(&self, id: ScheduleId) -> Result<Vec<PendingCall>, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let calls = transaction
                .prepare(
                    "SELECT at, method, headers, body FROM schedule_calls
                     WHERE schedule_id = ?1 AND state = 'pending' ORDER BY id",
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
                "UPDATE schedule_calls SET state = 'taken', method = NULL, headers = NULL, body = NULL
                 WHERE schedule_id = ?1 AND state = 'pending'",
                [id.as_str()],
            )?;
            transaction.commit()?;
            Ok(calls)
        })
        .await
    }

    /// Drops the schedule `id`'s pending calls, as a pause or an edit of
    /// its trigger does: they start nothing.
    pub async fn drop_pending_calls(&self, id: ScheduleId) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "UPDATE schedule_calls SET state = 'held', method = NULL, headers = NULL, body = NULL
                 WHERE schedule_id = ?1 AND state = 'pending'",
                [id.as_str()],
            )?;
            Ok(())
        })
        .await
    }

    /// The user's schedules that have calls waiting for a run.
    pub async fn schedules_with_pending_calls(&self, user: UserId) -> Result<Vec<ScheduleId>, StorageError> {
        self.call(move |connection, _| {
            connection
                .prepare(
                    "SELECT DISTINCT c.schedule_id FROM schedule_calls c
                     JOIN schedules a ON a.id = c.schedule_id
                     JOIN conversations v ON v.id = a.conversation_id
                     WHERE v.user_id = ?1 AND c.state = 'pending'",
                )?
                .query_map([user.as_str()], |row| row.get::<_, String>(0))?
                .map(|id| decode(CALLS, "schedule_id", ScheduleId::try_from(id?)))
                .collect()
        })
        .await
    }

    /// Whether the schedule `id` has calls waiting for a run.
    pub async fn has_pending_calls(&self, id: ScheduleId) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            Ok(connection.query_row(
                "SELECT EXISTS (SELECT 1 FROM schedule_calls WHERE schedule_id = ?1 AND state = 'pending')",
                [id.as_str()],
                |row| row.get(0),
            )?)
        })
        .await
    }

    /// The runs of the schedule `id` since `since` that started work,
    /// which its daily limit counts: every run but a skipped one.
    pub async fn schedule_runs_since(
        &self,
        id: ScheduleId,
        since: Timestamp,
    ) -> Result<u32, StorageError> {
        self.call(move |connection, _| {
            let count: i64 = connection.query_row(
                "SELECT count(*) FROM schedule_runs WHERE schedule_id = ?1 AND at >= ?2 AND state != 'skipped'",
                params![id.as_str(), since.as_millisecond()],
                |row| row.get(0),
            )?;
            decode(RUNS, "id", u32::try_from(count))
        })
        .await
    }

    /// The skipped run since `since` that says the schedule's limit was
    /// reached, whose count of calls grows.
    pub async fn schedule_limit_run(
        &self,
        id: ScheduleId,
        since: Timestamp,
    ) -> Result<Option<StoredRun>, StorageError> {
        self.call(move |connection, _| {
            connection
                .prepare_cached(&format!(
                    "SELECT {RUN_COLUMNS} FROM schedule_runs WHERE schedule_id = ?1 AND at >= ?2
                       AND state = 'skipped' AND json_extract(note, '$.kind') = 'limit'
                     ORDER BY at DESC LIMIT 1"
                ))?
                .query_row(params![id.as_str(), since.as_millisecond()], |row| Ok(run_row(row)))
                .optional()?
                .transpose()
        })
        .await
    }

    /// When a time trigger of the schedule `id` last fired.
    pub async fn schedule_last_fired(&self, id: ScheduleId) -> Result<Option<Timestamp>, StorageError> {
        self.call(move |connection, _| {
            let at: Option<i64> = connection.query_row(
                "SELECT max(at) FROM schedule_runs WHERE schedule_id = ?1 AND cause = 'time'",
                [id.as_str()],
                |row| row.get(0),
            )?;
            at.map(|at| decode(RUNS, "at", Timestamp::from_millisecond(at))).transpose()
        })
        .await
    }

    /// The running runs of the conversation `id`'s schedules, whose message
    /// went to it.
    pub async fn running_runs_in(&self, id: ConversationId) -> Result<Vec<StoredRun>, StorageError> {
        self.call(move |connection, _| {
            connection
                .prepare_cached(&format!(
                    "SELECT {RUN_COLUMNS} FROM schedule_runs
                     WHERE state = 'running'
                       AND schedule_id IN (SELECT id FROM schedules WHERE conversation_id = ?1)
                     ORDER BY at"
                ))?
                .query_map([id.as_str()], |row| Ok(run_row(row)))?
                .map(|row| row?)
                .collect()
        })
        .await
    }

    /// The running runs of the user's schedules, with the conversation each
    /// woke, which a start settles.
    pub async fn running_runs(&self, user: UserId) -> Result<Vec<(StoredRun, ConversationId)>, StorageError> {
        self.call(move |connection, _| {
            connection
                .prepare(
                    "SELECT r.id, r.schedule_id, r.cause, r.at, r.state, r.note, r.line, r.turn_id, r.notified,
                            r.task_id, s.conversation_id
                     FROM schedule_runs r JOIN schedules s ON s.id = r.schedule_id
                     JOIN conversations c ON c.id = s.conversation_id
                     WHERE c.user_id = ?1 AND r.state = 'running' ORDER BY r.at",
                )?
                .query_map([user.as_str()], |row| {
                    Ok((run_row(row), row.get::<_, String>("conversation_id")?))
                })?
                .map(|row| {
                    let (run, conversation) = row?;
                    Ok((run?, decode(RUNS, "conversation_id", ConversationId::try_from(conversation))?))
                })
                .collect()
        })
        .await
    }
}
