//! An update's progress as `GET /api/update` says it: the steps of a server
//! update from its restart on, the last two of which are this backend's own,
//! the databases it migrates or converts before it serves, measured in the
//! bytes of their files.
//!
//! - A backend that serves says so, or, once it heard that the server
//!   restarts for an update, the steps `demi-server` told it.
//! - A backend that starts reads the steps `demi-server` took before it
//!   (`UPDATE_RECORD`), takes its own start as done, and adds how far its
//!   data is: how many conversations of how many, how many bytes, and, once
//!   a rate is known, how long the rest takes. A start without that record,
//!   as one that only migrates, lists only its own two steps.
//! - The update screen that `demi-server` runs while no backend does reads
//!   the record anew at every question.
//!
//! It names no conversation, Cloud or device.

use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use std::path::PathBuf;

use demi_runner_protocol::release::{
    RestartStep, RestartStepKind, SkipReason, StepProgress, StepState, StepSummary, UpdateRecord,
};
use demi_web_api_protocol::state::UpdateState;

use super::{DatabaseKind, SchemaFit, StorageError, schema_fit};

/// How long an update runs before it estimates the time left: the first
/// seconds' rate says little of the rest.
const ESTIMATE_AFTER: Duration = Duration::from_secs(3);

/// What a backend that has not served yet does.
#[derive(Debug, Clone, Copy, PartialEq)]
enum UpdateReport {
    /// It starts, with nothing to migrate.
    Starting,
    /// It migrates or converts its databases.
    Updating(Progress),
    /// It serves.
    Ready,
    /// It serves, and stops next for an update of the server.
    Restarting,
}

/// How far an update is.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Progress {
    /// The control database migrates; the conversations come after it.
    pub control: bool,
    /// The conversations done, of `conversations`.
    pub conversations_done: u64,
    pub conversations: u64,
    /// The bytes done, of `bytes`.
    pub bytes_done: u64,
    pub bytes: u64,
    /// How long the rest takes; none until it can be estimated.
    pub left: Option<Duration>,
}

impl Progress {
    /// The share done, from 0 to 1.
    fn share(&self) -> f64 {
        if self.bytes == 0 { 1.0 } else { (self.bytes_done as f64 / self.bytes as f64).min(1.0) }
    }
}

#[derive(Debug, Default)]
struct State {
    /// What is due, once the update began.
    due: Option<Due>,
    /// The start knows what is due: an update began, or nothing is due.
    checked: bool,
    control_done: bool,
    /// How long the control database took, once it is done.
    control_took: Option<Duration>,
    conversations_done: u64,
    /// The bytes of the databases done.
    done: u64,
    /// The bytes of the database under way that are done.
    current: u64,
    began: Option<Instant>,
    phase: Phase,
    /// The steps `demi-server` took before this start, its own taken as done.
    record: Option<UpdateRecord>,
    /// The update screen's record, read at every question; the last read
    /// answers when the file cannot be read for a moment.
    screen: Option<(PathBuf, Option<UpdateRecord>)>,
}

/// Where the backend is between its start and its stop.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
enum Phase {
    /// It opens its data, updating what is due.
    #[default]
    Opening,
    Serving,
    /// It serves, and stops next for an update of the server, as the
    /// record says.
    Restarting(UpdateRecord),
}

/// An update's progress, shared by the backend's start and what reads it.
#[derive(Debug, Default)]
pub struct Update(Mutex<State>);

impl Update {
    fn locked(&self) -> std::sync::MutexGuard<'_, State> {
        // Nothing panics while it holds the lock.
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Begins an update of what is `due`.
    pub fn begin(&self, due: Due) {
        let mut state = self.locked();
        state.due = Some(due);
        state.checked = true;
        state.control_done = due.control == 0;
        state.began = Some(Instant::now());
    }

    /// The start has nothing to migrate or convert.
    pub fn nothing_due(&self) {
        self.locked().checked = true;
    }

    /// The control database is done.
    pub fn control_finished(&self) {
        let mut state = self.locked();
        if !state.control_done {
            state.control_done = true;
            state.control_took = state.began.map(|began| began.elapsed());
            state.done += state.due.map_or(0, |due| due.control);
        }
    }

    /// The database under way, of `bytes`, is `share` done.
    pub fn within(&self, bytes: u64, share: f64) {
        self.locked().current = (bytes as f64 * share.clamp(0.0, 1.0)) as u64;
    }

    /// A conversation's database of `bytes` is done.
    pub fn finished(&self, bytes: u64) {
        let mut state = self.locked();
        state.done += bytes;
        state.current = 0;
        state.conversations_done += 1;
    }

    /// The update screen, which reads its steps from the record at `path`
    /// at every question.
    pub fn screen(path: PathBuf) -> Self {
        let update = Self::default();
        update.locked().screen = Some((path, None));
        update
    }

    /// The steps `demi-server` took before this start, which took until now.
    pub fn recorded(&self, mut record: UpdateRecord) {
        let start = record.restart_step(RestartStepKind::StartDemi);
        *start = start.done(StepSummary::None);
        self.locked().record = Some(record);
    }

    /// The backend serves.
    pub fn ready(&self) {
        self.locked().phase = Phase::Serving;
    }

    /// The backend stops next for an update of the server, as `record` says,
    /// or, with none, no longer does, as when the update failed before the
    /// stop.
    pub fn restarting(&self, record: Option<UpdateRecord>) {
        let mut state = self.locked();
        if state.phase != Phase::Opening {
            state.phase = match record {
                Some(record) => Phase::Restarting(record),
                None => Phase::Serving,
            };
        }
    }

    fn report(&self) -> UpdateReport {
        let state = self.locked();
        match state.phase {
            Phase::Serving => return UpdateReport::Ready,
            Phase::Restarting(_) => return UpdateReport::Restarting,
            Phase::Opening => {}
        }
        let (Some(due), Some(began)) = (state.due, state.began) else {
            return UpdateReport::Starting;
        };
        let mut progress = Progress {
            control: !state.control_done,
            conversations_done: state.conversations_done.min(due.conversations),
            conversations: due.conversations,
            bytes_done: (state.done + state.current).min(due.all),
            bytes: due.all,
            left: None,
        };
        let share = progress.share();
        let elapsed = began.elapsed();
        progress.left = (elapsed >= ESTIMATE_AFTER && share > 0.0)
            .then(|| elapsed.mul_f64((1.0 - share) / share));
        UpdateReport::Updating(progress)
    }

    /// What `GET /api/update` answers, and what the state channel pushes
    /// when the backend stops for an update.
    pub fn state(&self) -> UpdateState {
        if let Some(record) = self.screen_record() {
            return updating(record, record_steps);
        }
        let report = self.report();
        let state = self.locked();
        match (&state.phase, report) {
            (Phase::Restarting(record), _) => updating(record.clone(), record_steps),
            (_, UpdateReport::Ready | UpdateReport::Restarting) => UpdateState::None,
            (_, UpdateReport::Starting) => match &state.record {
                Some(record) => updating(record.clone(), |record| {
                    let mut steps = record.restart.clone();
                    if state.checked {
                        for step in &mut steps {
                            if matches!(step.kind, RestartStepKind::UpdateControl | RestartStepKind::UpdateConversations) {
                                step.state = StepState::Skipped { why: SkipReason::NoChanges };
                            }
                        }
                    }
                    steps
                }),
                None => UpdateState::None,
            },
            (_, UpdateReport::Updating(progress)) => {
                // A start that only migrates moves to its own version.
                let record = state
                    .record
                    .clone()
                    .unwrap_or_else(|| UpdateRecord::new("", demi_shared_artifacts::BUILD_VERSION));
                let only_own = state.record.is_none();
                let began = state.began.map_or_else(now, |began| ago(began.elapsed()));
                updating(record, |record| {
                    let steps = own_steps(record, progress, state.due, began, state.control_took);
                    if only_own {
                        steps
                            .into_iter()
                            .filter(|step| matches!(step.kind, RestartStepKind::UpdateControl | RestartStepKind::UpdateConversations))
                            .collect()
                    } else {
                        steps
                    }
                })
            }
        }
    }

    /// The update screen's record as its file says now, or as it said last.
    fn screen_record(&self) -> Option<UpdateRecord> {
        let mut state = self.locked();
        let (path, last) = state.screen.as_mut()?;
        if let Some(read) = std::fs::read(path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()) {
            *last = Some(read);
        }
        last.clone()
    }
}

/// The state of an update that `record` says, its steps as `steps` gives them.
fn updating(record: UpdateRecord, steps: impl FnOnce(&UpdateRecord) -> Vec<RestartStep>) -> UpdateState {
    UpdateState::Updating {
        steps: steps(&record),
        from: (!record.from.is_empty()).then(|| record.from.clone()),
        returning: record.returning,
        to: record.to,
    }
}

fn record_steps(record: &UpdateRecord) -> Vec<RestartStep> {
    record.restart.clone()
}

/// The record's steps with this backend's own, as far as `progress` says:
/// the control database from `began`, then the conversations, from when the
/// control database was done.
fn own_steps(
    record: &UpdateRecord,
    progress: Progress,
    due: Option<Due>,
    began: demi_shared_types::Timestamp,
    control_took: Option<Duration>,
) -> Vec<RestartStep> {
    let mut steps = record.restart.clone();
    let control_bytes = due.map_or(0, |due| due.control);
    let took = control_took.unwrap_or_default();
    for step in &mut steps {
        step.state = match step.kind {
            RestartStepKind::UpdateControl if control_bytes == 0 => StepState::Skipped {
                why: SkipReason::NoChanges,
            },
            RestartStepKind::UpdateControl if progress.control => StepState::Running {
                started: began,
                progress: Some(StepProgress::Bytes {
                    done: progress.bytes_done.min(control_bytes),
                    total: control_bytes,
                    per_second: None,
                    left_seconds: None,
                }),
            },
            RestartStepKind::UpdateControl => StepState::Done {
                took_ms: u64::try_from(took.as_millis()).unwrap_or(u64::MAX),
                summary: StepSummary::Bytes { bytes: control_bytes },
            },
            RestartStepKind::UpdateConversations if progress.conversations == 0 => StepState::Skipped {
                why: SkipReason::NoChanges,
            },
            RestartStepKind::UpdateConversations if progress.control => StepState::Waiting,
            RestartStepKind::UpdateConversations => StepState::Running {
                started: demi_shared_types::Timestamp::truncate(
                    jiff::Timestamp::from(began) + jiff::SignedDuration::try_from(took).unwrap_or_default(),
                ),
                progress: Some(StepProgress::Conversations {
                    done: u32::try_from(progress.conversations_done).unwrap_or(u32::MAX),
                    total: u32::try_from(progress.conversations).unwrap_or(u32::MAX),
                    // The conversations' own bytes, after the control
                    // database's.
                    bytes_done: progress.bytes_done.saturating_sub(control_bytes),
                    bytes: progress.bytes.saturating_sub(control_bytes),
                    left_seconds: progress.left.map(|left| u32::try_from(left.as_secs()).unwrap_or(u32::MAX)),
                }),
            },
            _ => continue,
        };
    }
    steps
}

/// The time `elapsed` before now.
fn ago(elapsed: Duration) -> demi_shared_types::Timestamp {
    let elapsed = jiff::SignedDuration::try_from(elapsed).unwrap_or_default();
    demi_shared_types::Timestamp::truncate(jiff::Timestamp::now() - elapsed)
}

fn now() -> demi_shared_types::Timestamp {
    demi_shared_types::Timestamp::truncate(jiff::Timestamp::now())
}

/// The databases a start migrates or converts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Due {
    /// The control database's bytes, or 0 when it has the current form.
    pub control: u64,
    /// How many conversation databases.
    pub conversations: u64,
    /// The bytes of all of them, the control database's included.
    pub all: u64,
}

/// What the control database at `control` and the conversation databases
/// in `conversations` hold that a start migrates or converts; none when
/// every one has the current form. An empty file, which a start gives its
/// schema at once, is none of them.
pub fn due(control: &Path, conversations: &Path) -> Result<Option<Due>, StorageError> {
    let control_bytes = due_bytes(control, DatabaseKind::Control)?;
    let mut all = control_bytes;
    let mut count = 0;
    match std::fs::read_dir(conversations) {
        Ok(entries) => {
            for entry in entries {
                let path = entry?.path();
                if path.extension().is_some_and(|extension| extension == "sqlite") {
                    let bytes = due_bytes(&path, DatabaseKind::Conversation)?;
                    all += bytes;
                    count += u64::from(bytes > 0);
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok((all > 0).then_some(Due {
        control: control_bytes,
        conversations: count,
        all,
    }))
}

/// The size of the database at `path` when a start migrates or converts it,
/// else 0. A conversation's database that cannot be read is left as it is,
/// as its open fails on its own and the others still serve; control's fails
/// the start.
pub(crate) fn due_bytes(path: &Path, kind: DatabaseKind) -> Result<u64, StorageError> {
    let size = match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => metadata.len(),
        Ok(_) => return Ok(0),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.into()),
    };
    if size == 0 {
        return Ok(0);
    }
    let fit = match schema_fit(path, kind) {
        Ok(fit) => fit,
        Err(error) if kind == DatabaseKind::Conversation => {
            tracing::warn!(
                event = "update.unreadable",
                database = path.file_name().map(|name| name.to_string_lossy().into_owned()),
                error = &error as &dyn std::error::Error,
                "a conversation's database cannot be read, so the update leaves it as it is"
            );
            return Ok(0);
        }
        Err(error) => return Err(error),
    };
    Ok(match fit {
        SchemaFit::Migrates | SchemaFit::Converts => size,
        SchemaFit::Current | SchemaFit::Other => 0,
    })
}
