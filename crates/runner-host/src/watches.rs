//! The file system watches of one backend connection (`runner.md`
//! § Watching files): one watch per path and reach, shared by everything
//! that follows it. The working tree's changes and the subscriptions of the
//! same tree learn from one watch, which stops when the last of them lets
//! it go.
//!
//! A tree's watch also covers its repository's `.git` when that lies
//! outside it, as the working tree's changes need (`runner.md` § Working
//! tree).

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use tokio::sync::watch;

use crate::ignore::IgnoreFilter;
use crate::tree_watch::{Change, Event, Reach, TreeWatch};

/// The most watches of single files and folders a connection keeps on
/// macOS, each a kqueue or a stream of its own (`runner.md` § Watching
/// files).
pub(crate) const MAX_SINGLE_WATCHES: usize = 64;

/// Where a shared watch is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Status {
    /// Being set up on a blocking thread: on macOS, FSEvents can take
    /// seconds to start a stream when the system is busy.
    Starting,
    /// Running: a change from now on is reported.
    Running,
    /// It could not be set up, or it stopped, for this reason.
    Failed(String),
}

/// What hears a watch's events, on the watch's own thread.
pub(crate) type Listener = Box<dyn FnMut(&Event) + Send>;

/// The watches of one connection.
#[derive(Clone, Default)]
pub struct Watches(Arc<Mutex<HashMap<Key, Slot>>>);

/// Which watch: its canonical path, as the platform reports paths, and how
/// far it reaches.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Key {
    pub(crate) path: PathBuf,
    pub(crate) reach: Reach,
}

struct Slot {
    shared: Arc<Shared>,
    users: usize,
    /// Held for its drop, which stops the watch; none until it started.
    tree: Option<TreeWatch>,
}

/// What a watch's thread and its followers share. It does not own the
/// watch, so the last follower stops the watch on its own thread, never on
/// the watch's thread, which stopping joins.
struct Shared {
    listeners: Mutex<HashMap<u64, Listener>>,
    next: AtomicU64,
    status: watch::Sender<Status>,
    /// The folders the watch leaves out, once it runs.
    left_out: Mutex<Vec<PathBuf>>,
}

impl Shared {
    fn dispatch(&self, event: &Event) {
        if let Change::Failed(reason) = &event.change {
            self.status.send_replace(Status::Failed(reason.clone()));
        }
        for listener in lock(&self.listeners).values_mut() {
            listener(event);
        }
    }
}

/// One follower of a shared watch; dropping it stops following, and the
/// last follower's drop stops the watch.
pub(crate) struct Follower {
    watches: Watches,
    key: Key,
    shared: Arc<Shared>,
    id: u64,
}

impl Watches {
    /// Follows the watch of `key`, starting it unless it runs: `listener`
    /// hears every event from now on.
    pub(crate) fn follow(&self, key: Key, listener: Listener) -> Follower {
        let mut slots = lock(&self.0);
        let slot = slots.entry(key.clone()).or_insert_with(|| {
            let shared = Arc::new(Shared {
                listeners: Mutex::new(HashMap::new()),
                next: AtomicU64::new(0),
                status: watch::Sender::new(Status::Starting),
                left_out: Mutex::new(Vec::new()),
            });
            self.start(key.clone(), shared.clone());
            Slot {
                shared,
                users: 0,
                tree: None,
            }
        });
        slot.users += 1;
        let shared = slot.shared.clone();
        drop(slots);
        let id = shared.next.fetch_add(1, Ordering::Relaxed);
        lock(&shared.listeners).insert(id, listener);
        Follower {
            watches: self.clone(),
            key,
            shared,
            id,
        }
    }

    /// The root of a tree's watch, starting or running, that `path` lies in
    /// and does not leave out, the deepest such root first.
    pub(crate) fn tree_holding(&self, path: &Path) -> Option<PathBuf> {
        let slots = lock(&self.0);
        slots
            .iter()
            .filter(|(key, slot)| {
                key.reach == Reach::Tree
                    && path.starts_with(&key.path)
                    && !matches!(*slot.shared.status.borrow(), Status::Failed(_))
                    && !lock(&slot.shared.left_out).iter().any(|folder| path.starts_with(folder))
            })
            .map(|(key, _)| key.path.clone())
            .max_by_key(|root| root.components().count())
    }

    /// How many watches of single files and folders run or start.
    pub(crate) fn singles(&self) -> usize {
        lock(&self.0).keys().filter(|key| key.reach != Reach::Tree).count()
    }

    /// Sets the watch up on a blocking thread, which no follower waits for.
    /// A watch nobody follows any more by then is stopped at once.
    fn start(&self, key: Key, shared: Arc<Shared>) {
        let watches = self.clone();
        tokio::task::spawn_blocking(move || {
            let trees = trees_of(&key);
            let filter = match key.reach {
                Reach::Tree => IgnoreFilter::of(&key.path),
                Reach::Node | Reach::Entries => None,
            };
            let report = {
                let shared = shared.clone();
                Box::new(move |event: &Event| shared.dispatch(event))
            };
            let started = TreeWatch::start(&trees, key.reach, filter, report);
            let mut slots = lock(&watches.0);
            let slot = slots
                .get_mut(&key)
                .filter(|slot| Arc::ptr_eq(&slot.shared, &shared));
            match (started, slot) {
                (Ok(tree), Some(slot)) => {
                    *lock(&shared.left_out) = tree.left_out().to_vec();
                    slot.tree = Some(tree);
                    drop(slots);
                    shared.status.send_if_modified(|status| {
                        // A failure reported while it started stays.
                        let starting = *status == Status::Starting;
                        if starting {
                            *status = Status::Running;
                        }
                        starting
                    });
                }
                (Ok(tree), None) => {
                    drop(slots);
                    drop(tree);
                }
                (Err(reason), _) => {
                    drop(slots);
                    shared.status.send_replace(Status::Failed(reason));
                }
            }
        });
    }

    /// What FSEvents would say of `path` on the stream watching `watched`,
    /// with `flags`, such as that it dropped events, which nothing else can
    /// make it say; false while no stream of that path runs.
    #[cfg(all(feature = "testing", target_os = "macos"))]
    pub fn said(&self, watched: &Path, path: &Path, flags: u32) -> bool {
        let watched = std::fs::canonicalize(watched).unwrap_or_else(|_| watched.to_path_buf());
        let slots = lock(&self.0);
        slots
            .iter()
            .filter(|(key, _)| key.path == watched)
            .filter_map(|(_, slot)| slot.tree.as_ref())
            .any(|tree| tree.said(path.to_path_buf(), flags))
    }
}

impl Follower {
    pub(crate) fn status(&self) -> Status {
        self.shared.status.borrow().clone()
    }

    /// The watch's status from now on.
    pub(crate) fn statuses(&self) -> watch::Receiver<Status> {
        self.shared.status.subscribe()
    }

    /// Whether anything else follows the same watch.
    pub(crate) fn shared(&self) -> bool {
        lock(&self.watches.0)
            .get(&self.key)
            .is_some_and(|slot| Arc::ptr_eq(&slot.shared, &self.shared) && slot.users > 1)
    }

    /// The folders the watch leaves out, known once it runs.
    pub(crate) fn left_out(&self) -> Vec<PathBuf> {
        lock(&self.shared.left_out).clone()
    }
}

impl Drop for Follower {
    fn drop(&mut self) {
        lock(&self.shared.listeners).remove(&self.id);
        let mut slots = lock(&self.watches.0);
        let Some(slot) = slots
            .get_mut(&self.key)
            .filter(|slot| Arc::ptr_eq(&slot.shared, &self.shared))
        else {
            return;
        };
        slot.users -= 1;
        if slot.users > 0 {
            return;
        }
        let slot = slots.remove(&self.key);
        drop(slots);
        // Stopping joins the watch's thread, which takes only the listeners'
        // lock, released above.
        drop(slot);
    }
}

/// The trees a watch covers: the path, and for a tree also its
/// repository's `.git` when that lies outside it.
fn trees_of(key: &Key) -> Vec<PathBuf> {
    let mut trees = vec![key.path.clone()];
    if key.reach == Reach::Tree
        && let Some(git_dir) = crate::git::git_dir(&key.path)
        && !git_dir.starts_with(&key.path)
    {
        trees.push(git_dir);
    }
    trees
}

/// A poisoned lock means a panic mid-update, which nothing may paper over.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().expect("the watches are intact")
}
