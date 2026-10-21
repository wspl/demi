//! What the backend knows of each source's default branch, for every user
//! who added the source.
//!
//! A source's id is its URL's, so two users who added the same repository
//! share its head: the backend checks a source once per [`CHECK_INTERVAL`]
//! however many users added it, and a commit one user's check or fetch
//! finds moves the head for all of them. Each instance whose user read a
//! page state hears of every move and tells its pages when the source is one
//! of its user's.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use demi_shared_types::Timestamp;
use tokio::sync::broadcast;

/// How long a check of a source's default branch counts before the backend
/// checks it again: a commit pushed to a source shows within this time,
/// at the cost of one read of the remote's refs per source per interval.
pub(crate) const CHECK_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// The moves a slow instance may be behind before it hears that it missed
/// some, which makes it tell its pages anyway.
const MOVES: usize = 256;

/// What the backend knows of the commit a source's default branch points
/// to: when it last asked, and what it last heard, if any check or fetch
/// answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Head {
    pub(crate) checked_at: Timestamp,
    pub(crate) commit: Option<String>,
}

/// Every source's head, by id, shared by the instances of one factory.
pub(crate) struct Heads {
    /// A `std` mutex: the instances run on several shard threads, and no
    /// section awaits.
    heads: Mutex<HashMap<String, Head>>,
    /// The id of each source whose newest commit changed.
    moved: broadcast::Sender<String>,
}

impl Default for Heads {
    fn default() -> Self {
        Self {
            heads: Mutex::default(),
            moved: broadcast::channel(MOVES).0,
        }
    }
}

impl Heads {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, Head>> {
        // A section only reads or replaces heads.
        self.heads.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The heads of the sources `ids` names that the backend knows.
    pub(crate) fn of<'a>(&self, ids: impl IntoIterator<Item = &'a String>) -> BTreeMap<String, Head> {
        let heads = self.lock();
        ids.into_iter()
            .filter_map(|id| heads.get(id).map(|head| (id.clone(), head.clone())))
            .collect()
    }

    /// Takes the check of source `id` at `now`, unless one ran or a fetch
    /// ended within [`CHECK_INTERVAL`]; the commit last heard stays until
    /// the check answers.
    pub(crate) fn claim(&self, id: &str, now: Timestamp) -> bool {
        let interval = i64::try_from(CHECK_INTERVAL.as_millis()).expect("minutes fit");
        let mut heads = self.lock();
        let recent = heads
            .get(id)
            .is_some_and(|head| now.as_millisecond() - head.checked_at.as_millisecond() < interval);
        if recent {
            return false;
        }
        let commit = heads.get(id).and_then(|head| head.commit.clone());
        heads.insert(id.to_owned(), Head { checked_at: now, commit });
        true
    }

    /// Records what the check claimed at `claimed` heard of source `id`,
    /// unless a fetch that ended since knows better.
    pub(crate) fn checked(&self, id: &str, claimed: Timestamp, commit: String) {
        let mut heads = self.lock();
        let Some(head) = heads.get_mut(id).filter(|head| head.checked_at == claimed) else {
            return;
        };
        if head.commit.as_ref() != Some(&commit) {
            head.commit = Some(commit);
            self.announce(id);
        }
    }

    /// Records that a fetch of source `id` ended at `at` on its newest
    /// commit, which counts as a check.
    pub(crate) fn fetched(&self, id: &str, at: Timestamp, commit: String) {
        let previous = self.lock().insert(
            id.to_owned(),
            Head {
                checked_at: at,
                commit: Some(commit.clone()),
            },
        );
        if previous.and_then(|head| head.commit) != Some(commit) {
            self.announce(id);
        }
    }

    /// Every move from now on.
    pub(crate) fn moves(&self) -> broadcast::Receiver<String> {
        self.moved.subscribe()
    }

    fn announce(&self, id: &str) {
        // No instance listens before its user's first page state; that state
        // reads the head.
        let _ = self.moved.send(id.to_owned());
    }
}
