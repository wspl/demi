//! Subscriptions: what one client, the backend's connection or a page's direct
//! `watch`, subscribed to of a Host's files.
//!
//! Each subscription is followed through the connection's shared watches
//! (`crate::watches`) and reported by its id alone: the runner never sends a
//! path or content.
//!
//! For example, a page keeps `src/app.ts`'s text and the working tree's changes
//! list. The agent edits the file and runs `cargo check`, which writes 20,000
//! files under the ignored `target/`. Two subscriptions are reported changed,
//! once each, and nothing for `target/`.
//!
//! # How changes are matched
//!
//! - One owner task per client keeps the subscriptions.
//! - The watches' threads match what they receive against an index of the
//!   subscriptions that follow them and note the ids that changed, so a change
//!   nobody subscribed to costs a lookup and sends nothing.
//! - Changes are gathered for `WATCH_GATHER`, and each subscription is sent at
//!   most once per batch, with the highest number it saw. That is why a batch
//!   names no paths and needs no size limit.
//! - Starting and stopping a shared platform watch run on the blocking pool,
//!   never on the control thread, so the last subscriber releases its watch
//!   without waiting for a join. Closing also ends a watch still being started,
//!   so neither a late start nor a direct channel leaves one behind.
//!
//! # What counts as a change
//!
//! - A folder subscription with a suffix counts only the entries whose names
//!   end with it: a `notes.txt` beside `.demi/workflows/review.yaml` changes
//!   nothing of a subscription to that folder's `.yaml` entries.
//! - An ignored path changes no `worktree` subscription (`crate::ignore`), but
//!   a `file` or `folder` subscription inside an ignored folder, such as a log
//!   under `target/` a page shows, changes as any other.
//! - A git directory concerns only its own repository: in a checkout with
//!   nested repositories a commit in `src/v8` changes `src/v8`'s subscriptions
//!   and none of `src`'s.
//! - When the platform says it dropped events, the subscriptions the drop
//!   concerns changed, reported as any change:
//!   - FSEvents' `MustScanSubDirs` names a folder and only the subscriptions
//! under it.
//!   - The other flags and inotify's overflow concern every subscription on
//! that stream.
//!   - A drop outside the working tree changes nothing inside it.

use crate::git::GitDirs;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use demi_command_sdk::paths::resolve;
use demi_runner_protocol::files::SubscriptionKind;
use demi_runner_protocol::wire::{CHANGES_PER_MESSAGE, SUBSCRIPTION_LEASE, WATCH_GATHER};
use tokio::sync::{Notify, mpsc};
use tokio::time::Instant;
use tokio_util::task::AbortOnDropHandle;

use crate::tree_watch::{Change, Event, Reach};
use crate::watches::{Follower, Key, MAX_SINGLE_WATCHES, Status, Watches, lock};

/// What the client hears of its subscriptions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Report {
    /// The subscription is watched: a change at a number above `since` is
    /// reported.
    Live { id: String, since: u64 },
    /// It cannot be watched, for this reason; it is over.
    Failed { id: String, reason: String },
    /// These changed, each with the number of its latest change.
    Changed(Vec<(String, u64)>),
}

/// What the client asks.
enum Request {
    Subscribe {
        id: String,
        kind: SubscriptionKind,
        path: String,
        suffix: Option<String>,
    },
    Unsubscribe(String),
    Renew(Vec<String>),
}

/// The subscriptions of one client; dropping it ends them all.
pub struct Subscriptions {
    requests: mpsc::UnboundedSender<Request>,
    _owner: AbortOnDropHandle<()>,
}

impl Subscriptions {
    /// The subscriptions of a client whose relative paths start in
    /// `default_cwd`, followed through `watches`; their reports come in
    /// order on the receiver.
    pub fn new(watches: Watches, default_cwd: PathBuf) -> (Self, mpsc::Receiver<Report>) {
        let (requests, received) = mpsc::unbounded_channel();
        let (reports, heard) = mpsc::channel(64);
        let (statuses, statused) = mpsc::unbounded_channel();
        let owner = Owner {
            watches,
            default_cwd,
            subscriptions: HashMap::new(),
            bound: HashMap::new(),
            pending: Arc::new(Pending::default()),
            reports,
            statuses,
        };
        let task = AbortOnDropHandle::new(tokio::spawn(owner.run(received, statused)));
        (
            Self {
                requests,
                _owner: task,
            },
            heard,
        )
    }

    /// Subscribes `id`, replacing what had that id.
    pub fn subscribe(&self, id: String, kind: SubscriptionKind, path: String, suffix: Option<String>) {
        // The owner ends only with this handle.
        let _ = self.requests.send(Request::Subscribe { id, kind, path, suffix });
    }

    pub fn unsubscribe(&self, id: String) {
        let _ = self.requests.send(Request::Unsubscribe(id));
    }

    pub fn renew(&self, ids: Vec<String>) {
        let _ = self.requests.send(Request::Renew(ids));
    }
}

/// The ids the watches' threads saw change, with their latest numbers, and
/// the owner's wake-up once there are any.
#[derive(Default)]
struct Pending {
    changes: Mutex<HashMap<String, u64>>,
    wake: Notify,
}

impl Pending {
    fn note(&self, id: &str, at: u64) {
        let mut changes = lock(&self.changes);
        let first = changes.is_empty();
        let latest = changes.entry(id.to_owned()).or_insert(at);
        *latest = (*latest).max(at);
        drop(changes);
        if first {
            self.wake.notify_one();
        }
    }
}

/// One subscription as its owner keeps it.
struct Subscription {
    subject: Subject,
    /// Which watch it follows.
    key: Key,
    lease: Instant,
    live: bool,
}

/// What a subscription matches, with its canonical path.
#[derive(Debug, Clone)]
enum Subject {
    File(PathBuf),
    Folder { path: PathBuf, entries: Entries },
    Worktree { root: PathBuf, repo: Option<GitDirs> },
    /// Followed through the tree's watch, which also covers its git
    /// directories.
    Head { root: PathBuf, git_dir: Option<GitDirs> },
}

impl Subject {
    fn path(&self) -> Option<&Path> {
        match self {
            Subject::File(path) | Subject::Folder { path, .. } => Some(path),
            Subject::Worktree { .. } | Subject::Head { .. } => None,
        }
    }
}

/// What of a folder's entries a folder subscription counts.
#[derive(Debug, Clone)]
struct Entries {
    /// Their sizes and times too, not only their names.
    details: bool,
    /// Only those whose names end with it; none for all.
    suffix: Option<String>,
}

impl Entries {
    /// Whether a change of the entry `path` counts: `entry` when it came,
    /// went or was renamed.
    fn count(&self, path: &Path, entry: bool) -> bool {
        let named = self.suffix.as_ref().is_none_or(|suffix| {
            path.file_name()
                .is_some_and(|name| name.as_encoded_bytes().ends_with(suffix.as_bytes()))
        });
        (entry || self.details) && named
    }
}

/// The subscriptions that follow one watch, as its thread matches them.
#[derive(Default)]
struct Index {
    files: BTreeMap<PathBuf, BTreeSet<String>>,
    folders: BTreeMap<PathBuf, BTreeMap<String, Entries>>,
    trees: BTreeMap<String, Subject>,
}

impl Index {
    fn insert(&mut self, id: &str, subject: &Subject) {
        match subject {
            Subject::File(path) => {
                self.files.entry(path.clone()).or_default().insert(id.to_owned());
            }
            Subject::Folder { path, entries } => {
                self.folders
                    .entry(path.clone())
                    .or_default()
                    .insert(id.to_owned(), entries.clone());
            }
            Subject::Worktree { .. } | Subject::Head { .. } => {
                self.trees.insert(id.to_owned(), subject.clone());
            }
        }
    }

    fn remove(&mut self, id: &str, subject: &Subject) {
        match subject {
            Subject::File(path) => {
                if let Some(ids) = self.files.get_mut(path) {
                    ids.remove(id);
                    if ids.is_empty() {
                        self.files.remove(path);
                    }
                }
            }
            Subject::Folder { path, .. } => {
                if let Some(ids) = self.folders.get_mut(path) {
                    ids.remove(id);
                    if ids.is_empty() {
                        self.folders.remove(path);
                    }
                }
            }
            Subject::Worktree { .. } | Subject::Head { .. } => {
                self.trees.remove(id);
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.files.is_empty() && self.folders.is_empty() && self.trees.is_empty()
    }

    /// Notes in `pending` each subscription `event` changes.
    fn matched(&mut self, event: &Event, pending: &Pending) {
        match &event.change {
            Change::Changed {
                path,
                metadata,
                entry,
                ignored,
            } => {
                if let Some(ids) = self.files.get(path) {
                    ids.iter().for_each(|id| pending.note(id, event.at));
                }
                if *entry && let Some(ids) = self.folders.get(path) {
                    ids.keys().for_each(|id| pending.note(id, event.at));
                }
                if let Some(parent) = path.parent()
                    && let Some(ids) = self.folders.get(parent)
                {
                    for (id, entries) in ids {
                        if entries.count(path, *entry) {
                            pending.note(id, event.at);
                        }
                    }
                }
                if *entry {
                    // What lies below a folder that came, went or was renamed.
                    self.below(path, |id| pending.note(id, event.at));
                }
                for (id, subject) in &mut self.trees {
                    if rebound(subject, path) || tree_changed(subject, path, *metadata, *entry, *ignored) {
                        pending.note(id, event.at);
                    }
                }
            }
            Change::Lost { under: None, .. } => {
                let all = self
                    .files
                    .values()
                    .flatten()
                    .chain(self.folders.values().flat_map(BTreeMap::keys))
                    .chain(self.trees.keys());
                all.for_each(|id| pending.note(id, event.at));
            }
            Change::Lost {
                under: Some(under),
                ignored,
            } => {
                if let Some(ids) = self.files.get(under) {
                    ids.iter().for_each(|id| pending.note(id, event.at));
                }
                if let Some(ids) = self.folders.get(under) {
                    ids.keys().for_each(|id| pending.note(id, event.at));
                }
                self.below(under, |id| pending.note(id, event.at));
                for (id, subject) in &self.trees {
                    if !ignored && tree_lost(subject, under) {
                        pending.note(id, event.at);
                    }
                }
            }
            Change::Failed(_) => {}
        }
    }

    /// Each file and folder subscription strictly below `folder`.
    fn below(&self, folder: &Path, mut each: impl FnMut(&str)) {
        let below = |path: &PathBuf| path.starts_with(folder) && path != folder;
        let files = self.files.range(folder.to_path_buf()..);
        for (_, ids) in files.take_while(|(path, _)| path.starts_with(folder)).filter(|(path, _)| below(path)) {
            ids.iter().for_each(|id| each(id));
        }
        let folders = self.folders.range(folder.to_path_buf()..);
        for (_, ids) in folders.take_while(|(path, _)| path.starts_with(folder)).filter(|(path, _)| below(path)) {
            ids.keys().for_each(|id| each(id));
        }
    }
}

/// Whether a working tree that was in no repository is in one now that
/// `path` changed in its `.git`, as `git init` makes it: its subject then
/// follows the new repository's git directory, and has changed.
fn rebound(subject: &mut Subject, path: &Path) -> bool {
    let (root, dirs) = match subject {
        Subject::Worktree { root, repo: dirs @ None } | Subject::Head { root, git_dir: dirs @ None } => (root, dirs),
        _ => return false,
    };
    if !path.starts_with(root.join(".git")) {
        return false;
    }
    *dirs = crate::git::git_dirs(root);
    dirs.is_some()
}

/// Whether a change at `path` changes a working tree's list or its last
/// commit.
fn tree_changed(subject: &Subject, path: &Path, metadata: bool, entry: bool, ignored: bool) -> bool {
    match subject {
        Subject::Worktree { root, repo: Some(repo) } => {
            if let Ok(inside) = path.strip_prefix(&repo.git_dir) {
                return !metadata && git_listing(inside);
            }
            if repo.common_dir.as_deref().is_some_and(|common| path.starts_with(common)) {
                return !metadata && repo.shared_ref(path);
            }
            let Ok(relative) = path.strip_prefix(root) else {
                return false;
            };
            if rules_file(path) {
                return true;
            }
            // A nested repository's git directory concerns only its own.
            if relative.components().any(|part| part.as_os_str() == ".git") {
                return false;
            }
            !ignored
        }
        Subject::Worktree { root, repo: None } => {
            entry && path.starts_with(root) && path.file_name() == Some(".git".as_ref())
        }
        Subject::Head { git_dir: Some(dirs), .. } => {
            !metadata
                && (path.strip_prefix(&dirs.git_dir).is_ok_and(head_path) || dirs.shared_ref(path))
        }
        Subject::Head { git_dir: None, .. } | Subject::File(_) | Subject::Folder { .. } => false,
    }
}

/// Whether events lost below `under` may have changed a working tree's
/// list or its last commit.
fn tree_lost(subject: &Subject, under: &Path) -> bool {
    let overlaps = |path: &Path| path.starts_with(under) || under.starts_with(path);
    match subject {
        Subject::Worktree { root, repo } => {
            overlaps(root) || repo.as_ref().is_some_and(|repo| repo.overlaps(under))
        }
        Subject::Head { git_dir: Some(dirs), .. } => dirs.overlaps(under),
        Subject::Head { git_dir: None, .. } | Subject::File(_) | Subject::Folder { .. } => false,
    }
}

/// What in the git directory, relative to it, decides what a working
/// tree's list holds: the index, `HEAD`, a ref, `packed-refs`, `config`,
/// `info/exclude`.
fn git_listing(inside: &Path) -> bool {
    let mut parts = inside.components().map(|part| part.as_os_str().to_string_lossy());
    match parts.next().as_deref() {
        Some("index") => parts.next().is_none(),
        Some("HEAD" | "packed-refs" | "config" | "refs") => true,
        Some("info") => parts.next().as_deref() == Some("exclude"),
        _ => false,
    }
}

/// What in the git directory moves the last commit: `HEAD`, a ref,
/// `packed-refs`.
fn head_path(inside: &Path) -> bool {
    let first = inside.components().next().map(|part| part.as_os_str().to_string_lossy());
    matches!(first.as_deref(), Some("HEAD" | "packed-refs" | "refs"))
}

/// Whether git reads `path` to decide what it lists for other paths.
fn rules_file(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some(".gitignore" | ".gitattributes")
    )
}

/// A watch the client's subscriptions follow, with their index on its
/// thread.
struct Bound {
    follower: Follower,
    index: Arc<Mutex<Index>>,
    /// Hands the owner the watch's status as it changes.
    _status: AbortOnDropHandle<()>,
}

struct Owner {
    watches: Watches,
    default_cwd: PathBuf,
    subscriptions: HashMap<String, Subscription>,
    bound: HashMap<Key, Bound>,
    pending: Arc<Pending>,
    reports: mpsc::Sender<Report>,
    statuses: mpsc::UnboundedSender<(Key, Status)>,
}

impl Owner {
    async fn run(
        mut self,
        mut requests: mpsc::UnboundedReceiver<Request>,
        mut statuses: mpsc::UnboundedReceiver<(Key, Status)>,
    ) {
        let mut flush: Option<Instant> = None;
        loop {
            let lease_ends = self.subscriptions.values().map(|subscription| subscription.lease).min();
            let pending = self.pending.clone();
            let sent = tokio::select! {
                request = requests.recv() => match request {
                    Some(request) => self.request(request).await,
                    None => return,
                },
                Some((key, status)) = statuses.recv() => self.status(&key, status).await,
                () = pending.wake.notified(), if flush.is_none() => {
                    flush = Some(Instant::now() + WATCH_GATHER);
                    true
                }
                () = sleep_until(flush), if flush.is_some() => {
                    flush = None;
                    self.flush().await
                }
                () = sleep_until(lease_ends), if lease_ends.is_some() => {
                    self.expire();
                    true
                }
            };
            if !sent {
                return;
            }
        }
    }

    /// Carries out a request; false once nobody hears the reports.
    async fn request(&mut self, request: Request) -> bool {
        match request {
            Request::Subscribe { id, kind, path, suffix } => {
                self.end(&id);
                self.subscribe(id, kind, path, suffix).await
            }
            Request::Unsubscribe(id) => {
                self.end(&id);
                true
            }
            Request::Renew(ids) => {
                let lease = Instant::now() + SUBSCRIPTION_LEASE;
                for id in ids {
                    if let Some(subscription) = self.subscriptions.get_mut(&id) {
                        subscription.lease = lease;
                    }
                }
                true
            }
        }
    }

    async fn subscribe(&mut self, id: String, kind: SubscriptionKind, path: String, suffix: Option<String>) -> bool {
        let lease = Instant::now() + SUBSCRIPTION_LEASE;
        let resolved = resolve(&self.default_cwd, &path);
        let subject = match resolved {
            Ok(path) => {
                let found = tokio::task::spawn_blocking(move || subject_of(kind, &path, suffix)).await;
                match found {
                    Ok(found) => found,
                    Err(error) => Err(error.to_string()),
                }
            }
            Err(error) => Err(error.to_string()),
        };
        let subject = match subject {
            Ok(subject) => subject,
            Err(reason) => return self.report(Report::Failed { id, reason }).await,
        };
        let Some(key) = self.key_of(&subject, &[]) else {
            let reason = too_many_watches();
            return self.report(Report::Failed { id, reason }).await;
        };
        self.bind(&id, &subject, &key);
        let subscription = Subscription {
            subject,
            key: key.clone(),
            lease,
            live: false,
        };
        self.subscriptions.insert(id.clone(), subscription);
        let status = self.bound[&key].follower.status();
        self.settle(&id, status).await
    }

    /// The watch `subject` follows: the tree's watch of a working tree or
    /// its last commit, a running tree's that holds the path outside what
    /// it leaves out, or one of the path's own; none past the limit of
    /// those.
    fn key_of(&self, subject: &Subject, left_out: &[PathBuf]) -> Option<Key> {
        let tree = |path: &Path| Key {
            path: path.to_path_buf(),
            reach: Reach::Tree,
        };
        match subject {
            Subject::Worktree { root, .. } | Subject::Head { root, .. } => Some(tree(root)),
            Subject::File(path) | Subject::Folder { path, .. } => {
                let held = self
                    .watches
                    .tree_holding(path)
                    .filter(|_| !left_out.iter().any(|folder| path.starts_with(folder)));
                if let Some(root) = held {
                    return Some(tree(&root));
                }
                // A kqueue of a folder hears that some name changed, not
                // which, so a suffix needs the entries' stream too.
                let reach = match subject {
                    Subject::Folder { entries, .. }
                        if cfg!(target_os = "macos") && (entries.details || entries.suffix.is_some()) =>
                    {
                        Reach::Entries
                    }
                    _ => Reach::Node,
                };
                let key = Key {
                    path: path.clone(),
                    reach,
                };
                let known = self.bound.contains_key(&key);
                if !known && cfg!(target_os = "macos") && self.watches.singles() >= MAX_SINGLE_WATCHES {
                    return None;
                }
                Some(key)
            }
        }
    }

    /// Adds `id` to the index of the watch of `key`, following the watch
    /// unless the client does already.
    fn bind(&mut self, id: &str, subject: &Subject, key: &Key) {
        let bound = self.bound.entry(key.clone()).or_insert_with(|| {
            let index = Arc::new(Mutex::new(Index::default()));
            let listener = {
                let index = index.clone();
                let pending = self.pending.clone();
                Box::new(move |event: &Event| lock(&index).matched(event, &pending))
            };
            let follower = self.watches.follow(key.clone(), listener);
            let mut statuses = follower.statuses();
            let told = self.statuses.clone();
            let watched = key.clone();
            let status = AbortOnDropHandle::new(tokio::spawn(async move {
                // Each change of the watch's status after the first look.
                while statuses.changed().await.is_ok() {
                    let status = statuses.borrow_and_update().clone();
                    if told.send((watched.clone(), status)).is_err() {
                        return;
                    }
                }
            }));
            Bound {
                follower,
                index,
                _status: status,
            }
        });
        lock(&bound.index).insert(id, subject);
    }

    /// Takes `id` out of its watch's index, and lets the watch go once no
    /// subscription of the client follows it.
    fn unbind(&mut self, id: &str, subject: &Subject, key: &Key) {
        let Some(bound) = self.bound.get(key) else {
            return;
        };
        let mut index = lock(&bound.index);
        index.remove(id, subject);
        let empty = index.is_empty();
        drop(index);
        if empty {
            self.bound.remove(key);
        }
    }

    /// Ends `id`, if it runs.
    fn end(&mut self, id: &str) {
        if let Some(subscription) = self.subscriptions.remove(id) {
            self.unbind(id, &subscription.subject, &subscription.key);
        }
    }

    /// A watch's status changed: its subscriptions that wait for it are
    /// live, or failed.
    async fn status(&mut self, key: &Key, status: Status) -> bool {
        let waiting: Vec<String> = self
            .subscriptions
            .iter()
            .filter(|(_, subscription)| subscription.key == *key && (!subscription.live || matches!(status, Status::Failed(_))))
            .map(|(id, _)| id.clone())
            .collect();
        for id in waiting {
            if !self.settle(&id, status.clone()).await {
                return false;
            }
        }
        true
    }

    /// Where `id` is now that its watch has `status`: live once it runs,
    /// unless the watch leaves its path out and it follows one of its own;
    /// over once the watch failed.
    async fn settle(&mut self, id: &str, status: Status) -> bool {
        match status {
            Status::Starting => true,
            Status::Failed(reason) => {
                self.end(id);
                tracing::info!("a subscription cannot be watched: {reason}");
                self.report(Report::Failed {
                    id: id.to_owned(),
                    reason,
                })
                .await
            }
            Status::Running => {
                let Some(subscription) = self.subscriptions.get(id) else {
                    return true;
                };
                if subscription.live {
                    return true;
                }
                let left_out = self.bound[&subscription.key].follower.left_out();
                let inside = subscription
                    .subject
                    .path()
                    .is_some_and(|path| left_out.iter().any(|folder| path.starts_with(folder)));
                if inside {
                    // A folder git ignores that the tree's watch leaves out:
                    // the subscription follows a watch of its own.
                    let subject = subscription.subject.clone();
                    let key = subscription.key.clone();
                    self.unbind(id, &subject, &key);
                    let Some(own) = self.key_of(&subject, &left_out) else {
                        self.subscriptions.remove(id);
                        let reason = too_many_watches();
                        return self.report(Report::Failed { id: id.to_owned(), reason }).await;
                    };
                    self.bind(id, &subject, &own);
                    if let Some(subscription) = self.subscriptions.get_mut(id) {
                        subscription.key = own.clone();
                    }
                    let status = self.bound[&own].follower.status();
                    return Box::pin(self.settle(id, status)).await;
                }
                // Taken once the index holds it: any event from here on has
                // a higher number and reaches it.
                let since = crate::numbers::now();
                if let Some(subscription) = self.subscriptions.get_mut(id) {
                    subscription.live = true;
                }
                tracing::debug!(id, since, "a subscription is live");
                self.report(Report::Live {
                    id: id.to_owned(),
                    since,
                })
                .await
            }
        }
    }

    /// Reports the subscriptions that changed since the last batch, each
    /// once; those that are not live, or whose lease ran out, are not.
    async fn flush(&mut self) -> bool {
        self.expire();
        let changes = std::mem::take(&mut *lock(&self.pending.changes));
        let mut changes: Vec<(String, u64)> = changes
            .into_iter()
            .filter(|(id, _)| self.subscriptions.get(id).is_some_and(|subscription| subscription.live))
            .collect();
        changes.sort();
        if !changes.is_empty() {
            tracing::debug!(count = changes.len(), "subscriptions changed");
        }
        while !changes.is_empty() {
            let rest = changes.split_off(changes.len().min(CHANGES_PER_MESSAGE));
            if !self.report(Report::Changed(changes)).await {
                return false;
            }
            changes = rest;
        }
        true
    }

    /// Ends the subscriptions whose leases ran out, releasing what they
    /// held.
    fn expire(&mut self) {
        let now = Instant::now();
        let ended: Vec<String> = self
            .subscriptions
            .iter()
            .filter(|(_, subscription)| subscription.lease <= now)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ended {
            self.end(&id);
        }
    }

    async fn report(&self, report: Report) -> bool {
        self.reports.send(report).await.is_ok()
    }
}

/// Why a subscription past the limit of single watches cannot be watched.
fn too_many_watches() -> String {
    format!("the Host watches at most {MAX_SINGLE_WATCHES} files and folders outside a working tree")
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

/// What `kind` of the absolute `path` matches, of a folder's entries only
/// those whose names end with `suffix`, with its path canonical as the
/// platform reports paths; a path that is not there yet is canonical under
/// the nearest folder above it that is.
fn subject_of(kind: SubscriptionKind, path: &Path, suffix: Option<String>) -> Result<Subject, String> {
    if suffix.is_some() && !matches!(kind, SubscriptionKind::FolderNames | SubscriptionKind::FolderDetails) {
        return Err("only a folder's subscription counts entries by the ending of their names".to_owned());
    }
    let folder = |details| Entries { details, suffix: suffix.clone() };
    let canonical = canonical(path)?;
    Ok(match kind {
        SubscriptionKind::File => Subject::File(canonical),
        SubscriptionKind::FolderNames => Subject::Folder {
            path: canonical,
            entries: folder(false),
        },
        SubscriptionKind::FolderDetails => Subject::Folder {
            path: canonical,
            entries: folder(true),
        },
        SubscriptionKind::Worktree => Subject::Worktree {
            repo: crate::git::git_dirs(&canonical),
            root: canonical,
        },
        SubscriptionKind::Head => Subject::Head {
            git_dir: crate::git::git_dirs(&canonical),
            root: canonical,
        },
    })
}

fn canonical(path: &Path) -> Result<PathBuf, String> {
    match std::fs::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
                return Err(error.to_string());
            };
            Ok(canonical(parent)?.join(name))
        }
        Err(error) => Err(error.to_string()),
    }
}
