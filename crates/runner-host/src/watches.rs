//! The file system watches of one backend connection (`runner.md`
//! § Watching files): one watch per path and depth, shared by everything
//! that follows it. The working tree's changes and the backend's
//! `fs_watch` of the same tree learn from one watch, which stops when the
//! last of them lets it go.
//!
//! A watch below a path also covers the path's repository's `.git` when that
//! lies outside it, as the working tree's changes need
//! (`runner.md` § Working tree).

use std::{
    collections::{HashMap, HashSet},
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use demi_command_sdk::paths::resolve;
use demi_runner_protocol::wire::{self, Inbound};
use tokio::sync::{mpsc, watch};
use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};

use crate::tree_watch::{Depth, TreeWatch, WatchEvent};

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
pub(crate) type Listener = Box<dyn FnMut(&WatchEvent) + Send>;

/// The watches of one connection.
#[derive(Clone, Default)]
pub struct Watches(Arc<Mutex<HashMap<Key, Slot>>>);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Key {
    /// Canonical, as the platform reports paths.
    path: PathBuf,
    depth: Depth,
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
}

impl Shared {
    fn dispatch(&self, event: &WatchEvent) {
        if let WatchEvent::Failed(reason) = event {
            self.status.send_replace(Status::Failed(reason.clone()));
        }
        for listener in lock(&self.listeners).values_mut() {
            listener(event);
        }
    }
}

/// One follower of a shared watch; dropping it stops following, and the
/// last follower's drop stops the watch.
pub(crate) struct Subscription {
    watches: Watches,
    key: Key,
    shared: Arc<Shared>,
    id: u64,
}

impl Watches {
    /// Follows the watch of the canonical `path` to `depth`, starting it
    /// unless it runs: `listener` hears every event from now on.
    pub(crate) fn subscribe(&self, path: PathBuf, depth: Depth, listener: Listener) -> Subscription {
        let key = Key { path, depth };
        let mut slots = lock(&self.0);
        let slot = slots.entry(key.clone()).or_insert_with(|| {
            let shared = Arc::new(Shared {
                listeners: Mutex::new(HashMap::new()),
                next: AtomicU64::new(0),
                status: watch::Sender::new(Status::Starting),
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
        Subscription {
            watches: self.clone(),
            key,
            shared,
            id,
        }
    }

    /// Sets the watch up on a blocking thread, which no follower waits for.
    /// A watch nobody follows any more by then is stopped at once.
    fn start(&self, key: Key, shared: Arc<Shared>) {
        let watches = self.clone();
        tokio::task::spawn_blocking(move || {
            let trees = trees_of(&key);
            let trees: Vec<&Path> = trees.iter().map(PathBuf::as_path).collect();
            let report = {
                let shared = shared.clone();
                move |event: WatchEvent| shared.dispatch(&event)
            };
            let started = TreeWatch::start(&trees, key.depth, report);
            let mut slots = lock(&watches.0);
            let slot = slots
                .get_mut(&key)
                .filter(|slot| Arc::ptr_eq(&slot.shared, &shared));
            match (started, slot) {
                (Ok(tree), Some(slot)) => {
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
}

impl Subscription {
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
}

impl Drop for Subscription {
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

/// The trees a watch covers: the path, and below it also its repository's
/// `.git` when that lies outside it.
fn trees_of(key: &Key) -> Vec<PathBuf> {
    let mut trees = vec![key.path.clone()];
    if key.depth == Depth::Recursive
        && let Some(git_dir) = crate::git::git_dir(&key.path)
        && !git_dir.starts_with(&key.path)
    {
        trees.push(git_dir);
    }
    trees
}

/// A poisoned lock means a panic mid-update, which nothing may paper over.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().expect("the watches are intact")
}

/// The backend's watches of one connection (`runner.md` § Watching files):
/// each `fs_watch` follows the connection's watch of its path and reports,
/// as messages of its id, `ready` once that runs, the paths that changed,
/// gathered for 100 ms and each sent once, `lost` and `failed`.
pub struct WatchRequests {
    watches: Watches,
    output: mpsc::Sender<wire::Frame>,
    default_cwd: PathBuf,
    /// Each running watch's task, by its id; dropping one ends it.
    running: Mutex<HashMap<String, AbortOnDropHandle<()>>>,
    /// The connection's end, which ends every watch.
    cancel: CancellationToken,
}

impl WatchRequests {
    pub fn new(
        watches: Watches,
        output: mpsc::Sender<wire::Frame>,
        default_cwd: PathBuf,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            watches,
            output,
            default_cwd,
            running: Mutex::new(HashMap::new()),
            cancel,
        }
    }

    /// Starts or ends a watch; a watch started again under its id replaces
    /// the one before.
    pub fn handle(&self, message: Inbound) -> io::Result<()> {
        match message {
            Inbound::FsWatch {
                id,
                path,
                recursive,
            } => {
                let depth = if recursive {
                    Depth::Recursive
                } else {
                    Depth::Entries
                };
                let follow = Follow {
                    id: id.clone(),
                    requested: resolve(&self.default_cwd, &path),
                    depth,
                    watches: self.watches.clone(),
                    output: self.output.clone(),
                    cancel: self.cancel.clone(),
                };
                let task = AbortOnDropHandle::new(tokio::spawn(follow.run()));
                let replaced = lock(&self.running).insert(id, task);
                drop(replaced);
                Ok(())
            }
            Inbound::FsUnwatch { id } => {
                let ended = lock(&self.running).remove(&id);
                drop(ended);
                Ok(())
            }
            _ => Err(io::Error::other("not a watch request")),
        }
    }

    /// Ends every watch.
    pub fn close(&self) {
        let ended = std::mem::take(&mut *lock(&self.running));
        drop(ended);
    }
}

/// One `fs_watch`, as its task runs it.
struct Follow {
    id: String,
    requested: io::Result<PathBuf>,
    depth: Depth,
    watches: Watches,
    output: mpsc::Sender<wire::Frame>,
    cancel: CancellationToken,
}

impl Follow {
    async fn run(self) {
        let requested = match &self.requested {
            Ok(path) => path.clone(),
            Err(error) => return self.failed(error.to_string()).await,
        };
        let canonical = match tokio::fs::canonicalize(&requested).await {
            Ok(path) => path,
            Err(error) => return self.failed(error.to_string()).await,
        };
        let (events, mut received) = mpsc::unbounded_channel();
        let listener: Listener = Box::new(move |event| {
            // The task's end drops the receiver and then the subscription.
            let _ = events.send(event.clone());
        });
        let subscription = self.watches.subscribe(canonical.clone(), self.depth, listener);
        let mut statuses = subscription.statuses();
        let mut ready = false;
        let mut gathered: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut due: Option<tokio::time::Instant> = None;
        loop {
            let status = statuses.borrow_and_update().clone();
            match status {
                Status::Running if !ready => {
                    ready = true;
                    if !self.send(wire::Outbound::FsWatchReady { id: self.id.clone() }).await {
                        return;
                    }
                }
                Status::Failed(reason) => return self.failed(reason).await,
                Status::Starting | Status::Running => {}
            }
            let deadline = due.unwrap_or_else(tokio::time::Instant::now);
            tokio::select! {
                () = self.cancel.cancelled() => return,
                changed = statuses.changed() => {
                    // The sender lives as long as the subscription.
                    let _ = changed;
                }
                event = received.recv() => match event {
                    Some(WatchEvent::Changed { path, metadata }) => {
                        if metadata && under_git(&path) {
                            continue;
                        }
                        let path = respelled(&path, &canonical, &requested);
                        if seen.insert(path.clone()) {
                            gathered.push(path);
                        }
                        due.get_or_insert_with(|| tokio::time::Instant::now() + wire::WATCH_GATHER);
                    }
                    Some(WatchEvent::Lost) => {
                        gathered.clear();
                        seen.clear();
                        due = None;
                        if !self.send(wire::Outbound::FsWatchLost { id: self.id.clone() }).await {
                            return;
                        }
                    }
                    Some(WatchEvent::Failed(reason)) => return self.failed(reason).await,
                    None => return,
                },
                () = tokio::time::sleep_until(deadline), if due.is_some() => {
                    due = None;
                    seen.clear();
                    let paths = std::mem::take(&mut gathered);
                    if !self.report(paths).await {
                        return;
                    }
                }
            }
        }
    }

    /// Sends the paths gathered, or `lost` for more than a message holds.
    async fn report(&self, paths: Vec<String>) -> bool {
        let lost = || wire::Outbound::FsWatchLost { id: self.id.clone() };
        if paths.len() > wire::MAX_WATCH_PATHS {
            return self.send(lost()).await;
        }
        let changed = wire::Outbound::FsWatchChanged {
            id: self.id.clone(),
            paths,
        };
        let frame = wire::encode(&changed)
            .and_then(|frame| wire::within_limit(frame, |_| wire::encode(&lost())));
        self.send_frame(frame).await
    }

    async fn failed(&self, reason: String) {
        self.send(wire::Outbound::FsWatchFailed {
            id: self.id.clone(),
            reason,
        })
        .await;
    }

    async fn send(&self, message: wire::Outbound) -> bool {
        self.send_frame(wire::encode(&message)).await
    }

    /// Whether the message went out; a connection that closed takes none.
    async fn send_frame(&self, frame: Result<wire::Frame, wire::WireError>) -> bool {
        let frame = match frame {
            Ok(frame) => frame,
            Err(error) => {
                tracing::warn!("a watch message could not be encoded: {error}");
                return false;
            }
        };
        tokio::select! {
            () = self.cancel.cancelled() => false,
            sent = self.output.send(frame) => sent.is_ok(),
        }
    }
}

/// Whether `path` lies in a repository's `.git`, whose metadata alone
/// changes nothing a page shows (`runner.md` § Working tree).
fn under_git(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == ".git")
}

/// A reported path under the spelling the request used: the platform reports
/// real paths, such as `/private/var/...` for `/var/...`.
fn respelled(path: &Path, canonical: &Path, requested: &Path) -> String {
    let path = match path.strip_prefix(canonical) {
        Ok(rest) if rest.as_os_str().is_empty() => requested.to_path_buf(),
        Ok(rest) => requested.join(rest),
        Err(_) => path.to_path_buf(),
    };
    path.to_string_lossy().into_owned()
}
