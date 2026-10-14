//! Working-tree requests answered in process (`runner.md` § Working tree):
//! the uncommitted changes under a directory, and a file as the last commit
//! has it. Computation runs on blocking threads with gitoxide; a filesystem
//! watch per directory turns later requests incremental.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

use gix::bstr::{BStr, BString, ByteSlice};
use tokio::{
    sync::{Semaphore, mpsc, oneshot},
    task::JoinSet,
};
use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};

use demi_command_sdk::paths::resolve;
use demi_command_protocol::line_counts;
use demi_runner_protocol::wire::{self, ChangeKind, Frame, Inbound};

use crate::tree_watch::{Change as Seen, Event, Reach};
use crate::watches::{Follower, Key, Status, Watches};

/// The list stops here and reports `truncated`.
pub const MAX_FILES: usize = 5_000;
/// Files beyond this size count no lines; `git_show` refuses them.
pub const MAX_BLOB_BYTES: u64 = 8 * 1024 * 1024;
/// Beyond this many recorded paths a whole walk is cheaper: a walk over some
/// paths checks every index entry against each of them.
const MAX_TOUCHED: usize = 100;
const MAX_ROOTS: usize = 8;
const IDLE: Duration = Duration::from_secs(15 * 60);
const TIMEOUT: Duration = Duration::from_secs(30);
const CONCURRENT_COMPUTATIONS: usize = 2;

/// How the working tree differs from HEAD at a path.
/// One path `git status` lists, relative to the requested directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub path: String,
    /// git's two status letters for the path, as `git status --porcelain` prints them.
    pub status: String,
    pub kind: ChangeKind,
    pub from: Option<String>,
    pub added: u64,
    pub removed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changes {
    pub repository: bool,
    /// The absolute path of the repository's git directory.
    pub git_dir: Option<PathBuf>,
    pub head: Option<String>,
    pub files: Vec<Change>,
    pub truncated: bool,
    pub watched: bool,
}

impl Changes {
    fn outside_repository() -> Self {
        Changes {
            repository: false,
            git_dir: None,
            head: None,
            files: Vec::new(),
            truncated: false,
            watched: false,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("not inside a git repository")]
    NotRepository,
    #[error("the working-tree request timed out")]
    Timeout,
    #[error("the file is too large")]
    TooLarge,
    #[error("the abbreviation names more than one commit")]
    Ambiguous,
    #[error("the working-tree request was cancelled")]
    Cancelled,
    #[error("{0}")]
    Internal(String),
    #[error("{0}")]
    Io(#[from] io::Error),
}

impl GitError {
    pub fn code(&self) -> &str {
        match self {
            GitError::NotRepository => "not_repository",
            GitError::Timeout => "timeout",
            GitError::TooLarge => "too_large",
            GitError::Ambiguous => "ambiguous",
            GitError::Cancelled => "cancelled",
            GitError::Internal(_) => "internal",
            GitError::Io(error) => crate::fs::error_code(error).unwrap_or("EIO"),
        }
    }

    pub fn message(&self) -> String {
        self.to_string()
    }
}

/// Requests that share one computation each get its failure. A copied IO
/// error keeps its kind, its operating-system code and its words, which is
/// all a reply and the open-file wait read.
impl Clone for GitError {
    fn clone(&self) -> Self {
        match self {
            GitError::NotRepository => GitError::NotRepository,
            GitError::Timeout => GitError::Timeout,
            GitError::TooLarge => GitError::TooLarge,
            GitError::Ambiguous => GitError::Ambiguous,
            GitError::Cancelled => GitError::Cancelled,
            GitError::Internal(message) => GitError::Internal(message.clone()),
            // Out of open files stays so, whatever the error wraps: the
            // request then waits (`runner.md` § Load).
            GitError::Io(error) if demi_command_sdk::descriptors::exhausted(error) => {
                GitError::Io(demi_command_sdk::descriptors::exhaustion())
            }
            GitError::Io(error) => GitError::Io(match error.raw_os_error() {
                Some(code) => io::Error::from_raw_os_error(code),
                None => io::Error::new(error.kind(), error.to_string()),
            }),
        }
    }
}

pub(crate) fn internal<E: std::error::Error + Send + Sync + 'static>(error: E) -> GitError {
    // Running out of open files stays an IO error, so the request waits for
    // one (`runner.md` § Load).
    if demi_command_sdk::descriptors::exhausted(&error) {
        return GitError::Io(io::Error::other(error));
    }
    GitError::Internal(error.to_string())
}

/// The working-tree service of one backend connection (`runner.md`
/// § Working tree). One owner task keeps each requested directory's baseline
/// and watch, at most eight, and drops one after fifteen minutes without a
/// request. Requests for a directory that arrive while it is being computed
/// wait for the next computation, which they all share.
#[derive(Clone)]
pub struct GitService {
    requests: mpsc::Sender<ChangesRequest>,
    /// A commit's file list stops here, as the working tree's does.
    max_files: usize,
    /// How many whole walks the service made.
    walks: Arc<AtomicU64>,
    /// The owner ends with the last handle.
    _owner: Arc<AbortOnDropHandle<()>>,
}

struct ChangesRequest {
    root: PathBuf,
    reply: oneshot::Sender<Result<Changes, GitError>>,
    cancel: CancellationToken,
}

/// Requests waiting for the owner; a sender waits for room.
const REQUESTS: usize = 64;

struct Owner {
    roots: HashMap<PathBuf, Root>,
    watches: Watches,
    computations: Arc<Semaphore>,
    max_files: usize,
    walks: Arc<AtomicU64>,
    /// Each computation hands back its root's state with the answer.
    running: JoinSet<(PathBuf, RootState, Result<Changes, GitError>)>,
}

/// One requested directory: its state, the paths its watch recorded, and
/// the requests waiting for it.
struct Root {
    /// Absent while a computation holds it.
    state: Option<RootState>,
    /// The watch's own thread records here, so the section is short and
    /// shared across threads; nothing awaits while holding it.
    touched: Arc<Mutex<Touched>>,
    last_used: Instant,
    /// Waiting for the computation that runs now.
    current: Vec<Waiter>,
    /// Arrived during it; they share the next one.
    queued: Vec<Waiter>,
}

struct Waiter {
    reply: oneshot::Sender<Result<Changes, GitError>>,
    cancel: CancellationToken,
}

struct RootState {
    baseline: Option<Baseline>,
    watch: Watch,
}

/// A root's watch, from its start to its end: the connection's watch of the
/// tree, which the subscriptions of the same tree share (`runner.md`
/// § Watching files).
enum Watch {
    /// Not started: the directory has not been found in a repository yet.
    Unstarted,
    /// Followed; `adopted` once a computation found it running. No request
    /// waits for it to start: on macOS, FSEvents can take seconds to start a
    /// stream when the system is busy.
    Followed {
        follower: Follower,
        adopted: bool,
    },
    /// It could not be set up, or it failed: every request walks the whole
    /// tree.
    Unavailable,
}

impl Watch {
    /// Whether something besides this root follows its watch.
    fn shared(&self) -> bool {
        match self {
            Watch::Followed { follower, .. } => follower.shared(),
            Watch::Unstarted | Watch::Unavailable => false,
        }
    }
}

struct Baseline {
    head: Option<String>,
    /// The index's entries as this baseline found them (`index_entries`): an
    /// index rewritten with the same entries changes nothing listed.
    index: u64,
    files: BTreeMap<String, Change>,
    /// Each staged rename under the root, new path to old, whether listed or
    /// not. Renames change only with the index, and any change under `.git`
    /// recomputes the whole, so a walk over some paths keeps these.
    renames: BTreeMap<String, String>,
    /// The rules files above the root as this walk found them.
    rules_above: Vec<RulesStamp>,
    truncated: bool,
}

/// A `.gitignore` or `.gitattributes` as a walk found it: its size and when
/// it was last written, or `None` when it was not there.
type RulesStamp = Option<(u64, SystemTime)>;

/// What the watch recorded: paths to re-examine, or the need for a whole walk.
#[derive(Default)]
struct Touched {
    paths: HashSet<PathBuf>,
    whole: bool,
    /// The index was written: its entries decide whether that changed
    /// anything, as `git status` rewrites it with fresh file times.
    index: bool,
    /// The watch reported an error; it is dropped and the next walk is whole.
    broken: bool,
}

/// Where a directory sits in its repository.
#[derive(Clone)]
pub(crate) struct Located {
    workdir: PathBuf,
    git_dir: PathBuf,
    /// In a linked worktree, the main repository's git directory, which
    /// holds the refs and `packed-refs`; none when it is `git_dir`.
    common_dir: Option<PathBuf>,
    /// The directory relative to the work tree, with `/` separators; empty at the work tree itself.
    pub(crate) prefix: String,
}

enum Scope {
    Whole,
    /// Pathspecs relative to the work tree, and the same paths relative to the root.
    Paths {
        pathspecs: Vec<BString>,
        root_relative: Vec<String>,
    },
}

struct Computed {
    head: Option<String>,
    index: u64,
    files: BTreeMap<String, Change>,
    renames: BTreeMap<String, String>,
    truncated: bool,
}

impl Default for GitService {
    fn default() -> Self {
        Self::with_limits(MAX_FILES)
    }
}

impl GitService {
    pub fn with_limits(max_files: usize) -> Self {
        Self::new(Watches::default(), max_files)
    }

    /// The service of a connection whose watches are `watches`.
    pub fn new(watches: Watches, max_files: usize) -> Self {
        let (requests, received) = mpsc::channel(REQUESTS);
        let walks = Arc::new(AtomicU64::new(0));
        let owner = Owner {
            roots: HashMap::new(),
            watches,
            computations: Arc::new(Semaphore::new(CONCURRENT_COMPUTATIONS)),
            max_files,
            walks: walks.clone(),
            running: JoinSet::new(),
        };
        GitService {
            requests,
            max_files,
            walks,
            _owner: Arc::new(AbortOnDropHandle::new(tokio::spawn(owner.run(received)))),
        }
    }

    /// How many times the service walked a whole tree: what a build's
    /// thousands of ignored files must not cost (`runner.md` § Working
    /// tree).
    pub fn whole_walks(&self) -> u64 {
        self.walks.load(Ordering::Relaxed)
    }

    /// The uncommitted changes under `root`.
    pub async fn changes(
        &self,
        root: &Path,
        cancel: &CancellationToken,
    ) -> Result<Changes, GitError> {
        if cancel.is_cancelled() {
            return Err(GitError::Cancelled);
        }
        let root = tokio::fs::canonicalize(root).await?;
        let (reply, answer) = oneshot::channel();
        let request = ChangesRequest {
            root,
            reply,
            cancel: cancel.clone(),
        };
        let ended = || GitError::Internal("the working-tree service ended".into());
        tokio::select! {
            _ = cancel.cancelled() => return Err(GitError::Cancelled),
            sent = self.requests.send(request) => sent.map_err(|_| ended())?,
        }
        tokio::select! {
            _ = cancel.cancelled() => Err(GitError::Cancelled),
            answer = answer => answer.map_err(|_| ended())?,
        }
    }

    /// The file at `path` under `root` as the last commit has it, or as
    /// `commit` has it when one is named.
    pub async fn show(
        &self,
        root: &Path,
        path: &str,
        commit: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>, GitError> {
        if cancel.is_cancelled() {
            return Err(GitError::Cancelled);
        }
        let root = tokio::fs::canonicalize(root).await?;
        let path = path.to_owned();
        let commit = commit.map(str::to_owned);
        blocking(cancel, move || {
            let located = locate(&root)?.ok_or(GitError::NotRepository)?;
            let repo = discover(&root)?.ok_or(GitError::NotRepository)?;
            let tree = match &commit {
                Some(commit) => crate::history::tree_at(&repo, commit)?,
                None => head_tree(&repo)?,
            };
            let rela = join_prefix(&located.prefix, &path);
            let entry = tree
                .lookup_entry_by_path(gix::path::from_bstr(rela.as_bstr()))
                .map_err(internal)?
                .ok_or_else(|| GitError::Io(io::Error::from(io::ErrorKind::NotFound)))?;
            if !entry.mode().is_blob_or_symlink() {
                return Err(GitError::Io(io::Error::from(io::ErrorKind::IsADirectory)));
            }
            let header = repo.find_header(entry.oid()).map_err(internal)?;
            if header.size() > MAX_BLOB_BYTES {
                return Err(GitError::TooLarge);
            }
            Ok(repo
                .find_object(entry.oid())
                .map_err(internal)?
                .data
                .clone())
        })
        .await
    }

    /// The local branches of `root`'s repository and the one checked out.
    pub async fn branches(
        &self,
        root: &Path,
        cancel: &CancellationToken,
    ) -> Result<wire::GitBranches, GitError> {
        let root = canonical(root, cancel).await?;
        history(cancel, move |_| crate::history::branches(&root)).await
    }

    /// One page of `branch`'s first-parent history, HEAD's without one,
    /// after the commit `after` names.
    pub async fn log(
        &self,
        root: &Path,
        branch: Option<&str>,
        after: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<wire::GitLog, GitError> {
        let root = canonical(root, cancel).await?;
        let branch = branch.map(str::to_owned);
        let after = after.map(str::to_owned);
        history(cancel, move |interrupt| {
            crate::history::log(&root, branch.as_deref(), after.as_deref(), &interrupt)
        })
        .await
    }

    /// The commit `commit` names, its files under `root` against its first
    /// parent and the local branches it is on.
    pub async fn commit(
        &self,
        root: &Path,
        commit: &str,
        cancel: &CancellationToken,
    ) -> Result<wire::GitCommit, GitError> {
        let root = canonical(root, cancel).await?;
        let commit = commit.to_owned();
        let max_files = self.max_files;
        history(cancel, move |interrupt| {
            crate::history::commit(&root, &commit, max_files, &interrupt)
        })
        .await
    }
}

/// `root` as the history requests read it, unless the request was
/// cancelled first.
async fn canonical(root: &Path, cancel: &CancellationToken) -> Result<PathBuf, GitError> {
    if cancel.is_cancelled() {
        return Err(GitError::Cancelled);
    }
    Ok(tokio::fs::canonicalize(root).await?)
}

/// A history request's work, stopped as a computation is when the
/// connection closes or after thirty seconds (`runner.md` § Working tree).
async fn history<T: Send + 'static>(
    cancel: &CancellationToken,
    work: impl FnOnce(Arc<AtomicBool>) -> Result<T, GitError> + Send + 'static,
) -> Result<T, GitError> {
    interruptible(cancel, Arc::new(AtomicBool::new(false)), work).await
}

impl Owner {
    async fn run(mut self, mut requests: mpsc::Receiver<ChangesRequest>) {
        let mut idle = tokio::time::interval(Duration::from_secs(60));
        idle.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                request = requests.recv() => match request {
                    Some(request) => self.request(request),
                    None => return,
                },
                Some(computed) = self.running.join_next() => {
                    let (path, state, result) =
                        computed.expect("working-tree computations do not panic");
                    self.computed(path, state, result);
                }
                _ = idle.tick() => self.expire(),
            }
        }
    }

    fn request(&mut self, request: ChangesRequest) {
        self.expire();
        let now = Instant::now();
        if !self.roots.contains_key(&request.root) && self.roots.len() >= MAX_ROOTS {
            // The oldest directory no request is waiting for makes room.
            let oldest = self
                .roots
                .iter()
                .filter(|(_, root)| root.state.is_some())
                .min_by_key(|(_, root)| root.last_used)
                .map(|(path, _)| path.clone());
            if let Some(oldest) = oldest {
                self.roots.remove(&oldest);
            }
        }
        let root = self
            .roots
            .entry(request.root.clone())
            .or_insert_with(|| Root {
                state: Some(RootState {
                    baseline: None,
                    watch: Watch::Unstarted,
                }),
                touched: Arc::new(Mutex::new(Touched::default())),
                last_used: now,
                current: Vec::new(),
                queued: Vec::new(),
            });
        root.last_used = now;
        root.queued.push(Waiter {
            reply: request.reply,
            cancel: request.cancel,
        });
        self.start(request.root);
    }

    /// Starts the computation the queued requests of `path` share, unless one
    /// runs.
    fn start(&mut self, path: PathBuf) {
        let Some(root) = self.roots.get_mut(&path) else {
            return;
        };
        root.queued.retain(|waiter| !waiter.reply.is_closed());
        if root.queued.is_empty() {
            return;
        }
        let Some(state) = root.state.take() else {
            return;
        };
        root.current = std::mem::take(&mut root.queued);
        // Requests share their connection's cancellation.
        let cancel = root.current[0].cancel.clone();
        let touched = root.touched.clone();
        let computations = self.computations.clone();
        let max_files = self.max_files;
        let watches = self.watches.clone();
        let walks = self.walks.clone();
        self.running.spawn(async move {
            let mut state = state;
            let result = changes(
                &path,
                &mut state,
                &touched,
                &watches,
                &computations,
                max_files,
                &walks,
                &cancel,
            )
            .await;
            if result.is_err() {
                // The failed computation may have taken what the watch
                // recorded, or adopted the watch, without a baseline to
                // show for it: the next one walks the whole tree.
                lock(&touched).whole = true;
            }
            (path, state, result)
        });
    }

    fn computed(&mut self, path: PathBuf, state: RootState, result: Result<Changes, GitError>) {
        let Some(root) = self.roots.get_mut(&path) else {
            return;
        };
        root.state = Some(state);
        let mut waiters = root.current.drain(..);
        let first = waiters.next();
        // A request that gave up no longer waits for the answer.
        for waiter in waiters {
            let _gone = waiter.reply.send(result.clone());
        }
        if let Some(waiter) = first {
            let _gone = waiter.reply.send(result);
        }
        self.start(path);
    }

    /// Drops directories without a request for fifteen minutes, unless a
    /// subscription still uses their watch.
    fn expire(&mut self) {
        let now = Instant::now();
        self.roots.retain(|_, root| {
            root.state.as_ref().is_none_or(|state| state.watch.shared())
                || !root.queued.is_empty()
                || now.duration_since(root.last_used) < IDLE
        });
    }
}

/// The changes under `path` from its state: incremental when the watch
/// recorded few paths since the baseline, a whole walk otherwise.
async fn changes(
    path: &Path,
    state: &mut RootState,
    touched_paths: &Arc<Mutex<Touched>>,
    watches: &Watches,
    computations: &Arc<Semaphore>,
    max_files: usize,
    walks: &AtomicU64,
    cancel: &CancellationToken,
) -> Result<Changes, GitError> {
    let root_path = path.to_owned();
    let located = blocking(cancel, move || {
        Ok(locate(&root_path)?.map(|located| {
            let rules = rules_above(&root_path, &located);
            (located, rules)
        }))
    })
    .await?;
    let Some((located, rules)) = located else {
        // A directory that stops being a repository loses its baseline and
        // watch; should it become one again, its next request starts another.
        state.baseline = None;
        state.watch = Watch::Unstarted;
        return Ok(Changes::outside_repository());
    };
    // The watch this computation finds running for the first time began
    // after the baseline's walk: it cannot tell what changed before it ran.
    let mut adopted = false;
    match &mut state.watch {
        Watch::Unstarted => {
            let dirs = GitDirs::of(&located);
            let touched = touched_paths.clone();
            let listener = Box::new(move |event: &Event| lock(&touched).record(event, &dirs));
            let key = Key {
                path: path.to_owned(),
                reach: Reach::Tree,
            };
            state.watch = Watch::Followed {
                follower: watches.follow(key, listener),
                adopted: false,
            };
        }
        Watch::Followed {
            follower,
            adopted: found,
        } => match follower.status() {
            Status::Running if !*found => {
                *found = true;
                adopted = true;
            }
            Status::Failed(_) => state.watch = Watch::Unavailable,
            Status::Starting | Status::Running => {}
        },
        Watch::Unavailable => {}
    }
    let touched = std::mem::take(&mut *lock(touched_paths));
    if touched.broken {
        state.watch = Watch::Unavailable;
    }
    let watched = matches!(state.watch, Watch::Followed { adopted: true, .. });
    // A written index changed what is listed only when its entries did.
    let index_moved = match &state.baseline {
        Some(baseline) if touched.index && !touched.whole => {
            let workdir = located.workdir.clone();
            let index = blocking(cancel, move || {
                let repo = discover(&workdir)?.ok_or(GitError::NotRepository)?;
                index_entries(&repo)
            })
            .await?;
            index != baseline.index
        }
        _ => false,
    };
    let scope = match &state.baseline {
        Some(baseline)
            if watched && !adopted && !touched.whole && !index_moved && baseline.rules_above == rules =>
        {
            let scope = paths_scope(&touched.paths, &located, baseline);
            match scope {
                Some(scope) => scope,
                None => return Ok(baseline.to_changes(true, &located.git_dir)),
            }
        }
        _ => Scope::Whole,
    };
    if matches!(scope, Scope::Whole) {
        walks.fetch_add(1, Ordering::Relaxed);
    }
    // Past the computation limit a request waits its turn (`runner.md` § Load).
    let _permit = tokio::select! {
        permit = computations.clone().acquire_owned() => {
            permit.expect("working-tree computations are never closed")
        }
        _ = cancel.cancelled() => return Err(GitError::Cancelled),
    };
    let interrupt = Arc::new(AtomicBool::new(false));
    let computed = {
        let located = located.clone();
        let interrupt = interrupt.clone();
        let scope_pathspecs = match &scope {
            Scope::Whole => None,
            Scope::Paths { pathspecs, .. } => Some(pathspecs.clone()),
        };
        interruptible(cancel, interrupt, move |flag| {
            compute(&located, scope_pathspecs, flag, max_files)
        })
        .await?
    };
    let mut baseline = match scope {
        Scope::Whole => Baseline {
            head: computed.head,
            index: computed.index,
            files: computed.files,
            renames: computed.renames,
            rules_above: rules,
            truncated: computed.truncated,
        },
        Scope::Paths { root_relative, .. } => {
            let mut baseline = state
                .baseline
                .take()
                .expect("a paths scope needs a baseline");
            baseline.merge(computed, &root_relative);
            baseline
        }
    };
    baseline.truncate(max_files);
    let changes = baseline.to_changes(watched, &located.git_dir);
    state.baseline = Some(baseline);
    Ok(changes)
}

impl Baseline {
    /// Replaces what was known under each re-examined path with the fresh result.
    fn merge(&mut self, computed: Computed, root_relative: &[String]) {
        self.files.retain(|path, _| !within(path, root_relative));
        self.files.extend(computed.files);
        self.head = computed.head;
        self.index = computed.index;
        self.truncated |= computed.truncated;
    }

    /// The other path of each staged rename that has only one of its paths
    /// within `paths`: git pairs a rename only in a walk that takes in both.
    fn rename_partners(&self, paths: &[String]) -> Vec<String> {
        let mut partners = Vec::new();
        for (path, from) in &self.renames {
            let path_within = within(path, paths);
            let from_within = within(from, paths);
            if path_within && !from_within {
                partners.push(from.clone());
            } else if from_within && !path_within {
                partners.push(path.clone());
            }
        }
        partners
    }

    fn truncate(&mut self, max_files: usize) {
        if self.files.len() > max_files {
            let keep: Vec<String> = self.files.keys().take(max_files).cloned().collect();
            self.files.retain(|path, _| keep.contains(path));
            self.truncated = true;
        }
    }

    fn to_changes(&self, watched: bool, git_dir: &Path) -> Changes {
        Changes {
            repository: true,
            git_dir: Some(git_dir.to_path_buf()),
            head: self.head.clone(),
            files: self.files.values().cloned().collect(),
            truncated: self.truncated,
            watched,
        }
    }
}

/// A poisoned lock means a panic mid-update, which nothing may paper over.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().expect("the watch's record is intact")
}

/// Runs `work` on a blocking thread; a panic there answers `internal`. Only
/// short work goes here (finding the repository, reading one blob); a
/// computation, which can take long, goes through `interruptible`. A
/// cancelled request waits for the thread before it answers, so the
/// request's admission covers the thread (`runner.md` § Load).
async fn blocking<T: Send + 'static>(
    cancel: &CancellationToken,
    work: impl FnOnce() -> Result<T, GitError> + Send + 'static,
) -> Result<T, GitError> {
    let handle = tokio::task::spawn_blocking(work);
    tokio::pin!(handle);
    let panicked = || GitError::Internal("working-tree work panicked".into());
    tokio::select! {
        outcome = &mut handle => outcome.map_err(|_| panicked())?,
        _ = cancel.cancelled() => {
            // What the work found no longer matters.
            let _unused = handle.await;
            Err(GitError::Cancelled)
        }
    }
}

/// Runs `work` on a blocking thread with an interrupt flag it checks; the
/// flag is raised when the connection closes or the deadline passes, and
/// the thread is awaited before the failure is reported.
async fn interruptible<T: Send + 'static>(
    cancel: &CancellationToken,
    interrupt: Arc<AtomicBool>,
    work: impl FnOnce(Arc<AtomicBool>) -> Result<T, GitError> + Send + 'static,
) -> Result<T, GitError> {
    let flag = interrupt.clone();
    let handle = tokio::task::spawn_blocking(move || work(flag));
    tokio::pin!(handle);
    let failure = tokio::select! {
        outcome = &mut handle => {
            return outcome.map_err(|_| GitError::Internal("working-tree work panicked".into()))?;
        }
        _ = cancel.cancelled() => GitError::Cancelled,
        _ = tokio::time::sleep(TIMEOUT) => GitError::Timeout,
    };
    interrupt.store(true, Ordering::Relaxed);
    let _ = handle.await;
    Err(failure)
}

/// Finds the repository around `root`; `None` when there is none.
/// The repository `root` lies in, if any. Running out of open files is no
/// answer about the directory (`runner.md` § Load), but gix gives the answer
/// "no repository" when it could not open a candidate's files as well (on
/// Linux, without the cause), so that answer stands only once the runner can
/// open the directory itself.
pub(crate) fn discover(root: &Path) -> Result<Option<gix::Repository>, GitError> {
    match gix::discover(root) {
        Ok(repo) => Ok(Some(repo)),
        Err(error) if demi_command_sdk::descriptors::exhausted(&error) => Err(internal(error)),
        Err(gix::discover::Error::Discover(_)) => match std::fs::File::open(root) {
            Err(error) if demi_command_sdk::descriptors::exhausted(&error) => {
                Err(GitError::Io(error))
            }
            _ => Ok(None),
        },
        Err(error) => Err(internal(error)),
    }
}

pub(crate) fn locate(root: &Path) -> Result<Option<Located>, GitError> {
    let Some(repo) = discover(root)? else {
        return Ok(None);
    };
    let Some(workdir) = repo.workdir() else {
        return Ok(None);
    };
    let workdir = std::fs::canonicalize(workdir).map_err(GitError::Io)?;
    let git_dir = std::fs::canonicalize(repo.git_dir()).map_err(GitError::Io)?;
    let common_dir = std::fs::canonicalize(repo.common_dir())
        .map_err(GitError::Io)?;
    let common_dir = (common_dir != git_dir).then_some(common_dir);
    let prefix = match root.strip_prefix(&workdir) {
        Ok(relative) => unix_path(relative),
        Err(_) => return Ok(None),
    };
    Ok(Some(Located {
        workdir,
        git_dir,
        common_dir,
        prefix,
    }))
}

fn unix_path(path: &Path) -> String {
    gix::path::to_unix_separators_on_windows(gix::path::into_bstr(path))
        .to_str_lossy()
        .into_owned()
}

fn join_prefix(prefix: &str, path: &str) -> BString {
    if prefix.is_empty() {
        BString::from(path)
    } else {
        BString::from(format!("{prefix}/{path}"))
    }
}

/// A pathspec naming exactly this path and what lies under it.
fn literal_pathspec(path: &str) -> BString {
    BString::from(format!(":(literal){path}"))
}

/// The touched paths as a scope, with the other path of each staged rename
/// that has one path among them: `None` when nothing under the root was
/// touched, so the baseline stands.
fn paths_scope(
    touched: &HashSet<PathBuf>,
    located: &Located,
    baseline: &Baseline,
) -> Option<Scope> {
    let mut root_relative = Vec::new();
    for path in touched {
        let Ok(relative) = path.strip_prefix(&located.workdir) else {
            continue;
        };
        let relative = unix_path(relative);
        let Some(under_root) = root_relative_path(&relative, &located.prefix) else {
            continue;
        };
        if under_root.is_empty() {
            // The root itself: nothing narrower than a whole walk fits.
            return Some(Scope::Whole);
        }
        root_relative.push(under_root.to_owned());
    }
    if root_relative.is_empty() {
        return None;
    }
    let partners = baseline.rename_partners(&root_relative);
    root_relative.extend(partners);
    let pathspecs = root_relative
        .iter()
        .map(|path| literal_pathspec(&join_prefix(&located.prefix, path).to_str_lossy()))
        .collect();
    Some(Scope::Paths {
        pathspecs,
        root_relative,
    })
}

/// Whether `path` is one of `paths` or lies under one of them.
fn within(path: &str, paths: &[String]) -> bool {
    paths
        .iter()
        .any(|examined| path == examined || path.starts_with(&format!("{examined}/")))
}

/// `rela` (relative to the work tree) relative to the root, or `None` outside it.
pub(crate) fn root_relative_path<'a>(rela: &'a str, prefix: &str) -> Option<&'a str> {
    if prefix.is_empty() {
        return Some(rela);
    }
    if rela == prefix {
        return Some("");
    }
    rela.strip_prefix(prefix)
        .and_then(|rest| rest.strip_prefix('/'))
}

impl Touched {
    /// Notes what a watch saw. Under the git directory, a moved `HEAD` or
    /// ref, or a change to what git ignores or how it reads the repository,
    /// can change what git lists anywhere, so the next walk is whole; a
    /// written index may, which its entries tell; anything else there, such
    /// as an object, a log or a lock, changes nothing listed. A `.gitignore`
    /// or `.gitattributes` makes the next walk whole too. A path git ignores
    /// changes nothing listed and is not noted, so a build that writes
    /// thousands of files under `target/` leaves nothing to re-examine;
    /// events lost below a folder have it re-examined.
    fn record(&mut self, event: &Event, dirs: &GitDirs) {
        let git_dir = &dirs.git_dir;
        let (path, metadata) = match &event.change {
            Seen::Changed { ignored: true, .. } | Seen::Lost { ignored: true, .. } => return,
            Seen::Changed { path, metadata, .. } => (path, *metadata),
            Seen::Lost { under: None, .. } => {
                self.whole = true;
                return;
            }
            Seen::Lost { under: Some(under), .. } => {
                if dirs.overlaps(under) {
                    self.whole = true;
                } else {
                    self.touch(under);
                }
                return;
            }
            Seen::Failed(_) => {
                self.broken = true;
                return;
            }
        };
        if let Ok(inside) = path.strip_prefix(git_dir) {
            if !metadata {
                match git_path(inside) {
                    GitPath::Index => self.index = true,
                    GitPath::Listing => self.whole = true,
                    GitPath::Other => {}
                }
            }
        } else if let Some(common) = dirs.common_dir.as_deref()
            && path.starts_with(common)
        {
            // The refs a linked worktree shares; nothing else there is listed.
            if !metadata && dirs.shared_ref(path) {
                self.whole = true;
            }
        } else if !metadata && rules_file(path) {
            self.whole = true;
        } else {
            self.touch(path);
        }
    }

    /// Notes a path to re-examine; past [`MAX_TOUCHED`] of them the next
    /// walk is whole instead.
    fn touch(&mut self, path: &Path) {
        if self.whole {
            return;
        }
        self.paths.insert(path.to_path_buf());
        if self.paths.len() > MAX_TOUCHED {
            self.whole = true;
            self.paths.clear();
        }
    }
}

/// What a path in the git directory, relative to it, is to the list.
enum GitPath {
    /// The index.
    Index,
    /// What moves `HEAD` or decides what git ignores or how it reads the
    /// repository: `HEAD`, a ref, `packed-refs`, `info/exclude`, `config`.
    Listing,
    /// Anything else: objects, logs, locks, `FETCH_HEAD` and the like.
    Other,
}

fn git_path(inside: &Path) -> GitPath {
    let mut parts = inside.components().map(|part| part.as_os_str().to_string_lossy());
    match parts.next().as_deref() {
        Some("index") if parts.next().is_none() => GitPath::Index,
        Some("HEAD" | "packed-refs" | "config" | "refs") => GitPath::Listing,
        Some("info") if parts.next().as_deref() == Some("exclude") => GitPath::Listing,
        _ => GitPath::Other,
    }
}

/// The index's entries, each its path, object, mode and flags, as one
/// number: two indexes with the same entries list the same, whatever file
/// times they hold.
fn index_entries(repo: &gix::Repository) -> Result<u64, GitError> {
    use std::hash::{Hash as _, Hasher as _};
    let index = repo.index_or_empty().map_err(internal)?;
    let mut hasher = std::hash::DefaultHasher::new();
    for entry in index.entries() {
        entry.path(&index).hash(&mut hasher);
        entry.id.hash(&mut hasher);
        entry.mode.bits().hash(&mut hasher);
        entry.flags.bits().hash(&mut hasher);
    }
    Ok(hasher.finish())
}

/// Whether git reads `path` to decide what it lists and how it compares
/// files: a `.gitignore` or a `.gitattributes`.
fn rules_file(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some(".gitignore" | ".gitattributes")
    )
}

/// Where git keeps what a working tree's repository reads: its own git
/// directory, and in a linked worktree the main one's, which holds the refs
/// and `packed-refs` every worktree shares (`runner.md` § Watching files).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitDirs {
    pub(crate) git_dir: PathBuf,
    pub(crate) common_dir: Option<PathBuf>,
}

impl GitDirs {
    fn of(located: &Located) -> Self {
        GitDirs {
            git_dir: located.git_dir.clone(),
            common_dir: located.common_dir.clone(),
        }
    }

    /// The folders a watch of the work tree covers besides it: the git
    /// directory, or in a linked worktree the common one that holds it.
    pub(crate) fn watched(&self) -> &Path {
        match &self.common_dir {
            Some(common) if self.git_dir.starts_with(common) => common,
            _ => &self.git_dir,
        }
    }

    /// Whether `path` is a ref or `packed-refs` of the common git directory.
    pub(crate) fn shared_ref(&self, path: &Path) -> bool {
        let Some(inside) = self.common_dir.as_deref().and_then(|common| path.strip_prefix(common).ok()) else {
            return false;
        };
        let mut parts = inside.components().map(|part| part.as_os_str().to_string_lossy());
        match parts.next().as_deref() {
            Some("refs") => true,
            Some("packed-refs") => parts.next().is_none(),
            _ => false,
        }
    }

    /// Whether events lost below `under` may have missed one of them.
    pub(crate) fn overlaps(&self, under: &Path) -> bool {
        let overlaps = |dir: &Path| under.starts_with(dir) || dir.starts_with(under);
        overlaps(&self.git_dir) || self.common_dir.as_deref().is_some_and(overlaps)
    }
}

/// The git directories of the repository the canonical `root` lies in, if
/// any, which a watch below the root also covers.
pub(crate) fn git_dirs(root: &Path) -> Option<GitDirs> {
    locate(root).ok().flatten().map(|located| GitDirs::of(&located))
}

/// The `.gitignore` and `.gitattributes` of each directory above the root,
/// up to the work tree, as they stand. They decide what git lists under the
/// root too, but the watch covers only the root, so a request compares them
/// with what the last walk found.
fn rules_above(root: &Path, located: &Located) -> Vec<RulesStamp> {
    root.ancestors()
        .skip(1)
        .take_while(|directory| directory.starts_with(&located.workdir))
        .flat_map(|directory| [".gitignore", ".gitattributes"].map(|name| directory.join(name)))
        .map(|path| {
            let metadata = std::fs::metadata(path).ok()?;
            Some((metadata.len(), metadata.modified().ok()?))
        })
        .collect()
}

fn head_tree(repo: &gix::Repository) -> Result<gix::Tree<'_>, GitError> {
    let id = repo.head_tree_id_or_empty().map_err(internal)?;
    Ok(id.object().map_err(internal)?.into_tree())
}

/// What `git status` says of a path, before it is judged against HEAD and the
/// disk: its letter for the index and for the working tree, whether it is
/// untracked or in conflict, and a staged rename's source.
#[derive(Default)]
struct Signal {
    /// The index against HEAD: `M`, `T`, `A`, `D`, `R` or `C`.
    index: Option<char>,
    /// The working tree against the index: `M`, `T`, `D`, or `A` for an
    /// entry added with intent.
    worktree: Option<char>,
    untracked: bool,
    /// The pair git prints for a conflict, standing for both letters.
    conflict: Option<&'static str>,
    rename_from: Option<BString>,
}

impl Signal {
    /// The two letters `git status --porcelain` prints for the path; `None`
    /// when it lists nothing.
    fn status(&self) -> Option<String> {
        if let Some(pair) = self.conflict {
            return Some(pair.to_owned());
        }
        // git lists a deletion staged under an untracked file twice; the
        // untracked entry stands for both.
        if self.untracked {
            return Some("??".to_owned());
        }
        if self.index.is_none() && self.worktree.is_none() {
            return None;
        }
        Some(format!(
            "{}{}",
            self.index.unwrap_or(' '),
            self.worktree.unwrap_or(' ')
        ))
    }
}

/// The pair `git status --porcelain` prints for a conflict.
fn conflict_pair(conflict: gix::status::plumbing::index_as_worktree::Conflict) -> &'static str {
    use gix::status::plumbing::index_as_worktree::Conflict;
    match conflict {
        Conflict::BothDeleted => "DD",
        Conflict::AddedByUs => "AU",
        Conflict::DeletedByThem => "UD",
        Conflict::AddedByThem => "UA",
        Conflict::DeletedByUs => "DU",
        Conflict::BothAdded => "AA",
        Conflict::BothModified => "UU",
    }
}

/// The working tree's letter for an index entry that is not in conflict:
/// `None` for one that did not change, only its stats to refresh, and for a
/// submodule, which is not examined.
fn worktree_letter<T, U>(
    status: &gix::status::plumbing::index_as_worktree::EntryStatus<T, U>,
) -> Option<char> {
    use gix::status::plumbing::index_as_worktree::{Change, EntryStatus};
    match status {
        EntryStatus::Change(Change::Removed) => Some('D'),
        EntryStatus::Change(Change::Type { .. }) => Some('T'),
        EntryStatus::Change(Change::Modification {
            executable_bit_changed,
            content_change,
            ..
        }) if *executable_bit_changed || content_change.is_some() => Some('M'),
        EntryStatus::IntentToAdd => Some('A'),
        _ => None,
    }
}

/// Whether an index entry changed what it is, among a file, a symlink and a
/// submodule; an executable bit alone is no change of type.
fn type_changed(before: gix::index::entry::Mode, after: gix::index::entry::Mode) -> bool {
    use gix::index::entry::Mode;
    let kind = |mode: Mode| {
        if mode.is_submodule() {
            2
        } else if mode == Mode::SYMLINK {
            1
        } else {
            0
        }
    };
    kind(before) != kind(after)
}

/// Lists the paths `git status` lists under the located directory, or under
/// `pathspecs` when given, each judged against HEAD and the disk.
fn compute(
    located: &Located,
    pathspecs: Option<Vec<BString>>,
    interrupt: Arc<AtomicBool>,
    max_files: usize,
) -> Result<Computed, GitError> {
    use gix::status::index_worktree::{Item as WorktreeItem, RewriteSource};
    use gix::status::plumbing::index_as_worktree::EntryStatus;

    let repo = discover(&located.workdir)?.ok_or(GitError::NotRepository)?;
    let head = repo.head().map_err(internal)?.id().map(|id| id.to_string());
    let tree = head_tree(&repo)?;
    let pathspecs = match pathspecs {
        Some(pathspecs) => pathspecs,
        None if located.prefix.is_empty() => Vec::new(),
        None => vec![literal_pathspec(&located.prefix)],
    };
    let platform = repo
        .status(gix::progress::Discard)
        .map_err(internal)?
        .untracked_files(gix::status::UntrackedFiles::Files)
        // git pairs no deletion with an untracked file: a move nobody staged
        // is a deletion and an untracked file. Staged renames are found below.
        .index_worktree_rewrites(None)
        .tree_index_track_renames(gix::status::tree_index::TrackRenames::Given(
            gix::diff::Rewrites::default(),
        ))
        .index_worktree_submodules(Option::<gix::status::Submodule>::None)
        .should_interrupt_owned(interrupt.clone())
        .dirwalk_options(|options| {
            options
                .emit_ignored(None)
                .emit_empty_directories(false)
                .emit_pruned(false)
                .recurse_repositories(false)
        });
    let iter = platform.into_iter(pathspecs).map_err(internal)?;

    let mut signals: BTreeMap<BString, Signal> = BTreeMap::new();
    let mut truncated = false;
    for item in iter {
        let item = item.map_err(internal)?;
        match item {
            gix::status::Item::TreeIndex(change) => match change {
                gix::diff::index::Change::Addition { location, .. } => {
                    signals.entry(location.into_owned()).or_default().index = Some('A');
                }
                gix::diff::index::Change::Deletion { location, .. } => {
                    signals.entry(location.into_owned()).or_default().index = Some('D');
                }
                gix::diff::index::Change::Modification {
                    location,
                    previous_entry_mode,
                    entry_mode,
                    ..
                } => {
                    let letter = if type_changed(previous_entry_mode, entry_mode) {
                        'T'
                    } else {
                        'M'
                    };
                    signals.entry(location.into_owned()).or_default().index = Some(letter);
                }
                gix::diff::index::Change::Rewrite {
                    source_location,
                    location,
                    copy,
                    ..
                } => {
                    let signal = signals.entry(location.into_owned()).or_default();
                    if copy {
                        signal.index = Some('C');
                    } else {
                        signal.index = Some('R');
                        signal.rename_from = Some(source_location.into_owned());
                    }
                }
            },
            gix::status::Item::IndexWorktree(item) => match item {
                WorktreeItem::Modification {
                    rela_path, status, ..
                } => {
                    if let EntryStatus::Conflict { summary, .. } = status {
                        signals.entry(rela_path).or_default().conflict =
                            Some(conflict_pair(summary));
                    } else if let Some(letter) = worktree_letter(&status) {
                        signals.entry(rela_path).or_default().worktree = Some(letter);
                    }
                }
                WorktreeItem::DirectoryContents { entry, .. } => {
                    let untracked = entry.status == gix::dir::entry::Status::Untracked;
                    let file = matches!(
                        entry.disk_kind,
                        Some(gix::dir::entry::Kind::File | gix::dir::entry::Kind::Symlink)
                    );
                    if untracked && file {
                        signals.entry(entry.rela_path).or_default().untracked = true;
                    }
                }
                // Not asked for (`index_worktree_rewrites(None)`); should one come, it
                // reads as git would put it: the source deleted, the destination untracked.
                WorktreeItem::Rewrite {
                    source,
                    dirwalk_entry,
                    ..
                } => {
                    if let RewriteSource::RewriteFromIndex {
                        source_rela_path, ..
                    } = source
                    {
                        signals.entry(source_rela_path).or_default().worktree = Some('D');
                    }
                    signals
                        .entry(dirwalk_entry.rela_path)
                        .or_default()
                        .untracked = true;
                }
            },
        }
        if signals.len() > max_files {
            truncated = true;
            interrupt.store(true, Ordering::Relaxed);
            break;
        }
    }
    if interrupt.load(Ordering::Relaxed) && !truncated {
        return Err(GitError::Cancelled);
    }

    let renames: BTreeMap<String, String> = signals
        .iter()
        .filter_map(|(rela, signal)| {
            let from = signal.rename_from.as_ref()?;
            let path = root_relative_path(&rela.to_str_lossy(), &located.prefix)?.to_owned();
            let from = root_relative_path(&from.to_str_lossy(), &located.prefix)?.to_owned();
            Some((path, from))
        })
        .collect();
    let mut files: BTreeMap<String, Change> = BTreeMap::new();
    for (rela, signal) in signals {
        if interrupt.load(Ordering::Relaxed) && !truncated {
            return Err(GitError::Cancelled);
        }
        let rela_text = rela.to_str_lossy();
        let Some(path) = root_relative_path(&rela_text, &located.prefix) else {
            continue;
        };
        if path.is_empty() {
            continue;
        }
        let Some(status) = signal.status() else {
            continue;
        };
        let rename_from = renames.get(path).map(String::as_str);
        let Some(change) = judge(
            &repo,
            &tree,
            located,
            rela.as_bstr(),
            path,
            status,
            rename_from,
        )?
        else {
            continue;
        };
        files.insert(path.to_owned(), change);
        if files.len() >= max_files {
            truncated = true;
            break;
        }
    }
    Ok(Computed {
        head,
        index: index_entries(&repo)?,
        files,
        renames,
        truncated,
    })
}

/// A file's content on one side, or why there is none to count.
enum Content {
    Absent,
    Bytes(Vec<u8>),
    /// Present but not read: beyond the size limit.
    Large,
}

impl Content {
    fn present(&self) -> bool {
        !matches!(self, Content::Absent)
    }

    fn bytes(&self) -> Option<&[u8]> {
        match self {
            Content::Bytes(bytes) => Some(bytes),
            _ => None,
        }
    }
}

/// Decides what a path git lists is against HEAD and the disk, given git's
/// letters for it and, for a staged rename, its old path relative to the
/// root: nothing when neither side has it (added to the index, then deleted
/// from the disk).
fn judge(
    repo: &gix::Repository,
    tree: &gix::Tree<'_>,
    located: &Located,
    rela: &BStr,
    path: &str,
    status: String,
    rename_from: Option<&str>,
) -> Result<Option<Change>, GitError> {
    let disk = read_disk(&located.workdir.join(gix::path::from_bstr(rela)))?;
    let head = read_head(repo, tree, rela)?;
    let (kind, before) = match (head.present(), disk.present(), rename_from) {
        (false, true, Some(source)) => {
            // A rename whose source the last commit knows counts against it.
            let source_head =
                read_head(repo, tree, join_prefix(&located.prefix, source).as_bstr())?;
            if source_head.present() {
                (ChangeKind::Renamed, source_head)
            } else {
                (ChangeKind::Added, Content::Absent)
            }
        }
        (false, true, None) => (ChangeKind::Added, Content::Absent),
        (true, false, _) => (ChangeKind::Deleted, head),
        (true, true, _) => (ChangeKind::Modified, head),
        (false, false, _) => return Ok(None),
    };
    // A side too large to read counts no lines, the way a binary one does.
    let unread = matches!(before, Content::Large) || matches!(disk, Content::Large);
    let (added, removed) = if unread {
        (0, 0)
    } else {
        line_counts(before.bytes(), disk.bytes())
    };
    let from = if kind == ChangeKind::Renamed {
        rename_from.map(str::to_owned)
    } else {
        None
    };
    Ok(Some(Change {
        path: path.to_owned(),
        status,
        kind,
        from,
        added,
        removed,
    }))
}

fn read_disk(path: &Path) -> Result<Content, GitError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Content::Absent),
        Err(error) => return Err(GitError::Io(error)),
    };
    if metadata.is_symlink() {
        let target = std::fs::read_link(path).map_err(GitError::Io)?;
        return Ok(Content::Bytes(unix_path(&target).into_bytes()));
    }
    if !metadata.is_file() {
        return Ok(Content::Absent);
    }
    if metadata.len() > MAX_BLOB_BYTES {
        return Ok(Content::Large);
    }
    Ok(Content::Bytes(std::fs::read(path).map_err(GitError::Io)?))
}

fn read_head(
    repo: &gix::Repository,
    tree: &gix::Tree<'_>,
    rela: &BStr,
) -> Result<Content, GitError> {
    let entry = tree
        .lookup_entry_by_path(gix::path::from_bstr(rela))
        .map_err(internal)?;
    let Some(entry) = entry else {
        return Ok(Content::Absent);
    };
    if !entry.mode().is_blob_or_symlink() {
        return Ok(Content::Absent);
    }
    let header = repo.find_header(entry.oid()).map_err(internal)?;
    if header.size() > MAX_BLOB_BYTES {
        return Ok(Content::Large);
    }
    Ok(Content::Bytes(
        repo.find_object(entry.oid())
            .map_err(internal)?
            .data
            .clone(),
    ))
}

/// Answers a working-tree request on the wire; `None` for other messages.
pub async fn handle(
    service: &GitService,
    message: &Inbound,
    default_cwd: &Path,
    cancel: &CancellationToken,
) -> Option<Result<Frame, wire::WireError>> {
    let id = message.git_request_id()?;
    // Before anything is read (`runner.md` § Watching files).
    let read_at = crate::numbers::read_at();
    // Out of open files, the request waits for one (`runner.md` § Load).
    let result = demi_command_sdk::descriptors::retry(cancel, || {
        call(service, message, default_cwd, &read_at, cancel)
    })
    .await;
    Some(match result {
        Ok(reply) => wire::within_limit(reply, |reason| {
            wire::encode(&wire::Outbound::GitError {
                id: id.to_owned(),
                read_at: read_at.clone(),
                code: GitError::TooLarge.code().to_owned(),
                message: reason,
            })
        }),
        Err(error) => wire::encode(&wire::Outbound::GitError {
            id: id.to_owned(),
            read_at: read_at.clone(),
            code: error.code().to_owned(),
            message: error.message(),
        }),
    })
}

async fn call(
    service: &GitService,
    message: &Inbound,
    default_cwd: &Path,
    read_at: &wire::ReadAt,
    cancel: &CancellationToken,
) -> Result<Frame, GitError> {
    match message {
        Inbound::GitChanges { id, root } => {
            let root = resolve(default_cwd, root).map_err(GitError::Io)?;
            let changes = service.changes(&root, cancel).await?;
            reply(id, read_at, wire::GitResult::Changes(to_wire(changes)))
        }
        Inbound::GitBranches { id, root } => {
            let root = resolve(default_cwd, root).map_err(GitError::Io)?;
            let branches = service.branches(&root, cancel).await?;
            reply(id, read_at, wire::GitResult::Branches(branches))
        }
        Inbound::GitLog {
            id,
            root,
            branch,
            after,
        } => {
            let root = resolve(default_cwd, root).map_err(GitError::Io)?;
            let log = service
                .log(&root, branch.as_deref(), after.as_deref(), cancel)
                .await?;
            reply(id, read_at, wire::GitResult::Log(log))
        }
        Inbound::GitCommit { id, root, commit } => {
            let root = resolve(default_cwd, root).map_err(GitError::Io)?;
            let commit = service.commit(&root, commit, cancel).await?;
            reply(id, read_at, wire::GitResult::Commit(commit))
        }
        _ => Err(GitError::Internal("not a working-tree request".into())),
    }
}

fn reply(id: &str, read_at: &wire::ReadAt, result: wire::GitResult) -> Result<Frame, GitError> {
    wire::encode(&wire::Outbound::GitOk(wire::GitOk {
        id: id.to_owned(),
        read_at: read_at.clone(),
        result,
    }))
    .map_err(internal)
}

fn to_wire(changes: Changes) -> wire::GitChanges {
    wire::GitChanges {
        repository: changes.repository,
        git_dir: changes.git_dir.map(|dir| dir.to_string_lossy().into_owned()),
        head: changes.head,
        files: changes
            .files
            .into_iter()
            .map(|change| wire::GitChange {
                path: change.path,
                status: change.status,
                kind: change.kind,
                from: change.from,
                added: change.added,
                removed: change.removed,
            })
            .collect(),
        truncated: changes.truncated,
        watched: changes.watched,
    }
}
