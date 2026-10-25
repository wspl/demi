//! An update's progress: the databases a backend migrates or converts
//! before it serves, measured in the bytes of their files.
//!
//! The backend starts it when it finds a database of an earlier form, before
//! it opens any, and every page and `demi-server` read it until the backend
//! is ready, and says when the backend stops next for an administrator's
//! update of the server. It says which step runs, how many conversations are done of
//! how many, how many bytes, and, once a rate is known, how long the rest
//! takes. It names no conversation.

use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use demi_web_api_protocol::state::{UpdateState, UpdateStep};

use super::{DatabaseKind, SchemaFit, StorageError, schema_fit};

/// How long an update runs before it estimates the time left: the first
/// seconds' rate says little of the rest.
const ESTIMATE_AFTER: Duration = Duration::from_secs(3);

/// What a backend that has not served yet does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UpdateReport {
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
pub struct Progress {
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
    pub fn share(&self) -> f64 {
        if self.bytes == 0 { 1.0 } else { (self.bytes_done as f64 / self.bytes as f64).min(1.0) }
    }
}

#[derive(Debug, Default)]
struct State {
    /// What is due, once the update began.
    due: Option<Due>,
    control_done: bool,
    conversations_done: u64,
    /// The bytes of the databases done.
    done: u64,
    /// The bytes of the database under way that are done.
    current: u64,
    began: Option<Instant>,
    phase: Phase,
}

/// Where the backend is between its start and its stop.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// It opens its data, updating what is due.
    #[default]
    Opening,
    Serving,
    /// It serves, and stops next for an update of the server.
    Restarting,
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
        state.control_done = due.control == 0;
        state.began = Some(Instant::now());
    }

    /// The control database is done.
    pub fn control_finished(&self) {
        let mut state = self.locked();
        if !state.control_done {
            state.control_done = true;
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

    /// The backend serves.
    pub fn ready(&self) {
        self.locked().phase = Phase::Serving;
    }

    /// The backend stops next for an update of the server, or, with
    /// `false`, no longer does, as when the update failed before the stop.
    pub fn restarting(&self, restarting: bool) {
        let mut state = self.locked();
        if state.phase != Phase::Opening {
            state.phase = if restarting { Phase::Restarting } else { Phase::Serving };
        }
    }

    pub fn report(&self) -> UpdateReport {
        let state = self.locked();
        match state.phase {
            Phase::Serving => return UpdateReport::Ready,
            Phase::Restarting => return UpdateReport::Restarting,
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
        match self.report() {
            UpdateReport::Starting | UpdateReport::Ready => UpdateState::None,
            UpdateReport::Restarting => UpdateState::Restarting,
            UpdateReport::Updating(progress) => UpdateState::Updating {
                step: if progress.control { UpdateStep::Control } else { UpdateStep::Conversations },
                conversations_done: u32::try_from(progress.conversations_done).unwrap_or(u32::MAX),
                conversations: u32::try_from(progress.conversations).unwrap_or(u32::MAX),
                bytes_done: progress.bytes_done,
                bytes: progress.bytes,
                left_seconds: progress.left.map(|left| u32::try_from(left.as_secs()).unwrap_or(u32::MAX)),
            },
        }
    }
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
