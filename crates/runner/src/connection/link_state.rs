//! What the runner's link keeps across its connections
//! (`crates/runner-protocol/src/wire/session.rs`): the numbers of the
//! messages it wrote and received, the ones the backend did not confirm yet,
//! and the backlog the messages of a call wait in.
//!
//! The backend answers the runner's hello with the number of the session it
//! carries. The number the runner holds resumes the session: the backend says
//! how many of the runner's messages it received, and the transport writes
//! again what follows. Another number is a session the backend started anew,
//! as it does when it restarted or another runner took the device meanwhile:
//! the session begins again, and the calls that ran in the old one end with the
//! reason.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use demi_runner_protocol::wire::{Backlog, Resume, Session};
use tokio::sync::Notify;

/// Why a call ends when the backend answers with a session of another number.
pub const SESSION_RESTARTED: &str = "the backend started a new session for the link";

/// The link's session, shared by each connection's transport and the work
/// that sends messages across connections. Cloning it is cheap.
#[derive(Clone, Default)]
pub struct LinkState(Arc<Shared>);

#[derive(Default)]
struct Shared {
    session: Mutex<Session>,
    /// The number of the session, as the backend named it; none until a hello
    /// was answered.
    number: Mutex<Option<u64>>,
    /// Why the session that ran before ended, until the owner takes it.
    ended: Mutex<Option<String>>,
    backlog: Backlog,
    /// Wakes the writer when messages arrived that wait for an
    /// acknowledgement.
    owed: Notify,
}

impl LinkState {
    /// The session's numbers, locked for as short as a frame takes.
    pub fn session(&self) -> MutexGuard<'_, Session> {
        self.0.session.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The queue the messages of calls wait in for a connection.
    pub fn backlog(&self) -> &Backlog {
        &self.0.backlog
    }

    /// What the next hello says of the session it carries on.
    pub fn resume(&self) -> Option<Resume> {
        let session = (*self.0.number.lock().unwrap_or_else(PoisonError::into_inner))?;
        Some(Resume {
            session,
            received: self.session().received(),
        })
    }

    /// The backend answered the hello with the number of its session and the
    /// count of the runner's messages it received. Resumes the session when
    /// the number is the one it held, and otherwise starts it anew.
    pub fn hello_ok(&self, number: u64, received: u64) {
        let mut known = self.0.number.lock().unwrap_or_else(PoisonError::into_inner);
        let mut session = self.session();
        let resumed = *known == Some(number) && session.accepts(received) && session.resume(received).is_ok();
        if !resumed {
            let reason = match *known {
                None => None,
                Some(previous) if previous == number => Some("the link could not resume its session"),
                Some(_) => Some(SESSION_RESTARTED),
            };
            self.restart(&mut session, reason);
        }
        *known = Some(number);
    }

    /// The backend paired the runner: its session is a new one, of the number
    /// `number`.
    pub fn claimed(&self, number: u64) {
        let mut known = self.0.number.lock().unwrap_or_else(PoisonError::into_inner);
        let mut session = self.session();
        self.restart(&mut session, None);
        *known = Some(number);
    }

    fn restart(&self, session: &mut Session, reason: Option<&str>) {
        session.restart();
        self.0.backlog.clear();
        if let Some(reason) = reason {
            tracing::warn!(event = "link.session_ended", cause = reason, "the link's session ended");
            *self.0.ended.lock().unwrap_or_else(PoisonError::into_inner) = Some(reason.to_owned());
        }
    }

    /// Why the session that ran before ended, once.
    pub fn take_ended(&self) -> Option<String> {
        self.0.ended.lock().unwrap_or_else(PoisonError::into_inner).take()
    }

    /// A message of the backend's was handled, or skipped in its turn: counts
    /// it, and wakes the writer, which acknowledges it.
    pub fn handled(&self) {
        self.session().handled();
        self.0.owed.notify_one();
    }

    /// Messages arrived that wait for an acknowledgement.
    pub fn owe(&self) {
        self.0.owed.notify_one();
    }

    /// Resolves when the reader says messages wait for one.
    pub async fn owed(&self) {
        self.0.owed.notified().await;
    }
}
