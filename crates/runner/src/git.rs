//! Working-tree requests answered in process (`runner.md` § Working tree):
//! the uncommitted changes under a directory, and a file as the last commit
//! has it. Computation runs on blocking threads with gitoxide; a filesystem
//! watch per directory turns later requests incremental.

use crate::file_diff::line_counts;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use gix::bstr::{BStr, BString, ByteSlice};
use notify::Watcher as _;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::connection::wire::{self as wire, Inbound, Outbound};
use crate::paths::resolve;

/// The list stops here and reports `truncated`.
pub const MAX_FILES: usize = 5_000;
/// Files beyond this size count no lines; `git_show` refuses them.
pub const MAX_BLOB_BYTES: u64 = 8 * 1024 * 1024;
/// Beyond this many recorded paths a watch is no better than a whole walk.
const MAX_TOUCHED: usize = 10_000;
const MAX_ROOTS: usize = 8;
const IDLE: Duration = Duration::from_secs(15 * 60);
const TIMEOUT: Duration = Duration::from_secs(30);
const CONCURRENT_COMPUTATIONS: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
}

impl ChangeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeKind::Added => "added",
            ChangeKind::Modified => "modified",
            ChangeKind::Deleted => "deleted",
            ChangeKind::Renamed => "renamed",
        }
    }
}

/// One changed file, its path relative to the requested directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub path: String,
    pub kind: ChangeKind,
    pub from: Option<String>,
    pub added: u64,
    pub removed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changes {
    pub repository: bool,
    pub head: Option<String>,
    pub files: Vec<Change>,
    pub truncated: bool,
    pub watched: bool,
}

impl Changes {
    fn outside_repository() -> Self {
        Changes {
            repository: false,
            head: None,
            files: Vec::new(),
            truncated: false,
            watched: false,
        }
    }
}

#[derive(Debug)]
pub enum GitError {
    NotRepository,
    Timeout,
    TooLarge,
    Cancelled,
    Internal(String),
    Io(io::Error),
}

impl GitError {
    pub fn code(&self) -> &str {
        match self {
            GitError::NotRepository => "not_repository",
            GitError::Timeout => "timeout",
            GitError::TooLarge => "too_large",
            GitError::Cancelled => "cancelled",
            GitError::Internal(_) => "internal",
            GitError::Io(error) => crate::fs::error_code(error).unwrap_or("EIO"),
        }
    }

    pub fn message(&self) -> String {
        match self {
            GitError::NotRepository => "not inside a git repository".into(),
            GitError::Timeout => "the working-tree request timed out".into(),
            GitError::TooLarge => "the file is too large".into(),
            GitError::Cancelled => "the working-tree request was cancelled".into(),
            GitError::Internal(message) => message.clone(),
            GitError::Io(error) => error.to_string(),
        }
    }
}

impl std::fmt::Display for GitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for GitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            GitError::Io(error) => Some(error),
            _ => None,
        }
    }
}

fn internal<E: std::error::Error + Send + Sync + 'static>(error: E) -> GitError {
    // Running out of open files stays an IO error, so the request waits for
    // one (`runner.md` § Load).
    if demi_command_service::descriptors::exhausted(&error) {
        return GitError::Io(io::Error::other(error));
    }
    GitError::Internal(error.to_string())
}

/// The working-tree service of one backend connection: its watched
/// directories and the bound on concurrent computations.
#[derive(Clone)]
pub struct GitService {
    shared: Arc<Shared>,
}

struct Shared {
    roots: Mutex<HashMap<PathBuf, Arc<Root>>>,
    computations: Arc<Semaphore>,
    max_files: usize,
}

/// One requested directory: its baseline, its watch, and what the watch
/// recorded since the baseline was computed.
struct Root {
    path: PathBuf,
    state: tokio::sync::Mutex<RootState>,
    touched: Arc<Mutex<Touched>>,
    last_used: Mutex<Instant>,
}

struct RootState {
    baseline: Option<Baseline>,
    watcher: Option<notify::RecommendedWatcher>,
    /// A watch is tried once per root; a failure means whole walks from then on.
    watch_tried: bool,
}

struct Baseline {
    head: Option<String>,
    files: BTreeMap<String, Change>,
    truncated: bool,
}

/// What the watch recorded: paths to re-examine, or the need for a whole walk.
#[derive(Default)]
struct Touched {
    paths: HashSet<PathBuf>,
    whole: bool,
    /// The watch reported an error; it is dropped and the next walk is whole.
    broken: bool,
}

/// Where a directory sits in its repository.
#[derive(Clone)]
struct Located {
    workdir: PathBuf,
    git_dir: PathBuf,
    /// The directory relative to the work tree, with `/` separators; empty at the work tree itself.
    prefix: String,
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
    files: BTreeMap<String, Change>,
    truncated: bool,
}

impl Default for GitService {
    fn default() -> Self {
        Self::with_limits(MAX_FILES)
    }
}

impl GitService {
    pub fn with_limits(max_files: usize) -> Self {
        GitService {
            shared: Arc::new(Shared {
                roots: Mutex::new(HashMap::new()),
                computations: Arc::new(Semaphore::new(CONCURRENT_COMPUTATIONS)),
                max_files,
            }),
        }
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
        let canonical = tokio::fs::canonicalize(root).await.map_err(GitError::Io)?;
        let root = self.root(canonical);
        let mut state = tokio::select! {
            guard = root.state.lock() => guard,
            _ = cancel.cancelled() => return Err(GitError::Cancelled),
        };
        let root_path = root.path.clone();
        let located = blocking(cancel, move || locate(&root_path)).await?;
        let Some(located) = located else {
            // A directory that stops being a repository loses its baseline and watch.
            state.baseline = None;
            state.watcher = None;
            return Ok(Changes::outside_repository());
        };
        if !state.watch_tried {
            state.watch_tried = true;
            state.watcher = start_watch(&root.path, &located.git_dir, root.touched.clone());
        }
        let touched = {
            let mut touched = lock(&root.touched);
            std::mem::take(&mut *touched)
        };
        if touched.broken {
            state.watcher = None;
        }
        let watched = state.watcher.is_some();
        let scope = match &state.baseline {
            Some(baseline) if watched && !touched.whole => {
                let scope = paths_scope(&touched.paths, &located);
                match scope {
                    Some(scope) => scope,
                    None => return Ok(baseline.to_changes(true)),
                }
            }
            _ => Scope::Whole,
        };
        // Past the computation limit a request waits its turn (`runner.md` § Load).
        let _permit = tokio::select! {
            permit = self.shared.computations.clone().acquire_owned() => {
                permit.expect("working-tree computations are never closed")
            }
            _ = cancel.cancelled() => return Err(GitError::Cancelled),
        };
        let interrupt = Arc::new(AtomicBool::new(false));
        let computed = {
            let located = located.clone();
            let interrupt = interrupt.clone();
            let max_files = self.shared.max_files;
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
                files: computed.files,
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
        baseline.truncate(self.shared.max_files);
        let changes = baseline.to_changes(watched);
        state.baseline = Some(baseline);
        Ok(changes)
    }

    /// The file at `path` under `root` as the last commit has it.
    pub async fn show(
        &self,
        root: &Path,
        path: &str,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>, GitError> {
        if cancel.is_cancelled() {
            return Err(GitError::Cancelled);
        }
        let root = tokio::fs::canonicalize(root).await.map_err(GitError::Io)?;
        let path = path.to_owned();
        blocking(cancel, move || {
            let located = locate(&root)?.ok_or(GitError::NotRepository)?;
            let repo = discover(&root)?.ok_or(GitError::NotRepository)?;
            let tree = head_tree(&repo)?;
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

    /// The root's state, made on first use; idle roots leave and the oldest
    /// makes room past the cap.
    fn root(&self, path: PathBuf) -> Arc<Root> {
        let mut roots = lock(&self.shared.roots);
        let now = Instant::now();
        roots.retain(|_, root| now.duration_since(*lock(&root.last_used)) < IDLE);
        if let Some(root) = roots.get(&path) {
            *lock(&root.last_used) = now;
            return root.clone();
        }
        if roots.len() >= MAX_ROOTS {
            let oldest = roots
                .iter()
                .min_by_key(|(_, root)| *lock(&root.last_used))
                .map(|(path, _)| path.clone());
            if let Some(oldest) = oldest {
                roots.remove(&oldest);
            }
        }
        let root = Arc::new(Root {
            path: path.clone(),
            state: tokio::sync::Mutex::new(RootState {
                baseline: None,
                watcher: None,
                watch_tried: false,
            }),
            touched: Arc::new(Mutex::new(Touched::default())),
            last_used: Mutex::new(now),
        });
        roots.insert(path, root.clone());
        root
    }
}

impl Baseline {
    /// Replaces what was known under each re-examined path with the fresh result.
    fn merge(&mut self, computed: Computed, root_relative: &[String]) {
        self.files.retain(|path, _| {
            !root_relative
                .iter()
                .any(|examined| path == examined || path.starts_with(&format!("{examined}/")))
        });
        self.files.extend(computed.files);
        self.head = computed.head;
        self.truncated |= computed.truncated;
    }

    fn truncate(&mut self, max_files: usize) {
        if self.files.len() > max_files {
            let keep: Vec<String> = self.files.keys().take(max_files).cloned().collect();
            self.files.retain(|path, _| keep.contains(path));
            self.truncated = true;
        }
    }

    fn to_changes(&self, watched: bool) -> Changes {
        Changes {
            repository: true,
            head: self.head.clone(),
            files: self.files.values().cloned().collect(),
            truncated: self.truncated,
            watched,
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Runs `work` on a blocking thread; a panic there answers `internal`.
async fn blocking<T: Send + 'static>(
    cancel: &CancellationToken,
    work: impl FnOnce() -> Result<T, GitError> + Send + 'static,
) -> Result<T, GitError> {
    let handle = tokio::task::spawn_blocking(work);
    tokio::select! {
        outcome = handle => outcome.map_err(|_| GitError::Internal("working-tree work panicked".into()))?,
        _ = cancel.cancelled() => Err(GitError::Cancelled),
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
fn discover(root: &Path) -> Result<Option<gix::Repository>, GitError> {
    match gix::discover(root) {
        Ok(repo) => Ok(Some(repo)),
        Err(error) if demi_command_service::descriptors::exhausted(&error) => Err(internal(error)),
        Err(gix::discover::Error::Discover(_)) => match std::fs::File::open(root) {
            Err(error) if demi_command_service::descriptors::exhausted(&error) => Err(GitError::Io(error)),
            _ => Ok(None),
        },
        Err(error) => Err(internal(error)),
    }
}

fn locate(root: &Path) -> Result<Option<Located>, GitError> {
    let Some(repo) = discover(root)? else {
        return Ok(None);
    };
    let Some(workdir) = repo.workdir() else {
        return Ok(None);
    };
    let workdir = std::fs::canonicalize(workdir).map_err(GitError::Io)?;
    let git_dir = std::fs::canonicalize(repo.git_dir()).map_err(GitError::Io)?;
    let prefix = match root.strip_prefix(&workdir) {
        Ok(relative) => unix_path(relative),
        Err(_) => return Ok(None),
    };
    Ok(Some(Located {
        workdir,
        git_dir,
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

/// The touched paths as a scope: `None` when nothing under the root was
/// touched, so the baseline stands.
fn paths_scope(touched: &HashSet<PathBuf>, located: &Located) -> Option<Scope> {
    let mut pathspecs = Vec::new();
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
        pathspecs.push(literal_pathspec(&relative));
        root_relative.push(under_root.to_owned());
    }
    if pathspecs.is_empty() {
        return None;
    }
    Some(Scope::Paths {
        pathspecs,
        root_relative,
    })
}

/// `rela` (relative to the work tree) relative to the root, or `None` outside it.
fn root_relative_path<'a>(rela: &'a str, prefix: &str) -> Option<&'a str> {
    if prefix.is_empty() {
        return Some(rela);
    }
    if rela == prefix {
        return Some("");
    }
    rela.strip_prefix(prefix)
        .and_then(|rest| rest.strip_prefix('/'))
}

fn start_watch(
    root: &Path,
    git_dir: &Path,
    touched: Arc<Mutex<Touched>>,
) -> Option<notify::RecommendedWatcher> {
    let git_dir_owned = git_dir.to_path_buf();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let mut touched = lock(&touched);
        let event = match event {
            Ok(event) => event,
            Err(_) => {
                touched.broken = true;
                return;
            }
        };
        if event.need_rescan() {
            touched.whole = true;
        }
        for path in event.paths {
            if path.starts_with(&git_dir_owned) {
                touched.whole = true;
            } else if !touched.whole {
                touched.paths.insert(path);
                if touched.paths.len() > MAX_TOUCHED {
                    touched.whole = true;
                    touched.paths.clear();
                }
            }
        }
    })
    .ok()?;
    watcher.watch(root, notify::RecursiveMode::Recursive).ok()?;
    if !git_dir.starts_with(root) {
        watcher
            .watch(git_dir, notify::RecursiveMode::Recursive)
            .ok()?;
    }
    Some(watcher)
}

fn head_tree(repo: &gix::Repository) -> Result<gix::Tree<'_>, GitError> {
    let id = repo.head_tree_id_or_empty().map_err(internal)?;
    Ok(id.object().map_err(internal)?.into_tree())
}

/// What the status found about a path, before it is judged against HEAD and the disk.
#[derive(Default)]
struct Signal {
    rename_from: Option<BString>,
}

/// Lists the changes under the located directory, or under `pathspecs` when
/// given, each judged against HEAD and the disk.
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
        .index_worktree_rewrites(Some(gix::diff::Rewrites::default()))
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
                gix::diff::index::Change::Rewrite {
                    source_location,
                    location,
                    copy,
                    ..
                } => {
                    let source = source_location.into_owned();
                    signals.entry(source.clone()).or_default();
                    let signal = signals.entry(location.into_owned()).or_default();
                    if !copy {
                        signal.rename_from = Some(source);
                    }
                }
                change => {
                    signals.entry(change.location().to_owned()).or_default();
                }
            },
            gix::status::Item::IndexWorktree(item) => match item {
                WorktreeItem::Modification {
                    rela_path, status, ..
                } => {
                    if !matches!(status, EntryStatus::NeedsUpdate(_)) {
                        signals.entry(rela_path).or_default();
                    }
                }
                WorktreeItem::DirectoryContents { entry, .. } => {
                    let untracked = entry.status == gix::dir::entry::Status::Untracked;
                    let file = matches!(
                        entry.disk_kind,
                        Some(gix::dir::entry::Kind::File | gix::dir::entry::Kind::Symlink)
                    );
                    if untracked && file {
                        signals.entry(entry.rela_path).or_default();
                    }
                }
                WorktreeItem::Rewrite {
                    source,
                    dirwalk_entry,
                    copy,
                    ..
                } => {
                    let source_path = match source {
                        RewriteSource::RewriteFromIndex {
                            source_rela_path, ..
                        } => Some(source_rela_path),
                        RewriteSource::CopyFromDirectoryEntry { .. } => None,
                    };
                    if let Some(source_path) = &source_path {
                        signals.entry(source_path.clone()).or_default();
                    }
                    let signal = signals.entry(dirwalk_entry.rela_path).or_default();
                    if !copy {
                        signal.rename_from = source_path;
                    }
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
        let Some(change) = judge(&repo, &tree, located, rela.as_bstr(), signal, path)? else {
            continue;
        };
        files.insert(path.to_owned(), change);
        if files.len() >= max_files {
            truncated = true;
            break;
        }
    }
    // A rename's source is gone from the disk, but it is not a deletion of its own.
    let rename_sources: Vec<String> = files
        .values()
        .filter_map(|change| change.from.clone())
        .collect();
    for source in rename_sources {
        files.remove(&source);
    }
    Ok(Computed {
        head,
        files,
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

/// Decides what a signalled path is against HEAD and the disk: nothing when
/// both sides agree.
fn judge(
    repo: &gix::Repository,
    tree: &gix::Tree<'_>,
    located: &Located,
    rela: &BStr,
    signal: Signal,
    path: &str,
) -> Result<Option<Change>, GitError> {
    let disk = read_disk(&located.workdir.join(gix::path::from_bstr(rela)))?;
    let head = read_head(repo, tree, rela)?;
    let rename_from = signal
        .rename_from
        .as_deref()
        .and_then(|source| {
            root_relative_path(&source.to_str_lossy(), &located.prefix).map(str::to_owned)
        })
        .filter(|source| !source.is_empty());
    let (kind, before) = match (head.present(), disk.present(), rename_from.as_deref()) {
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
    let same_bytes = matches!((before.bytes(), disk.bytes()), (Some(a), Some(b)) if a == b);
    if kind == ChangeKind::Modified && same_bytes {
        return Ok(None);
    }
    // A side too large to read counts no lines, the way a binary one does.
    let unread = matches!(before, Content::Large) || matches!(disk, Content::Large);
    let (added, removed) = if unread {
        (0, 0)
    } else {
        line_counts(before.bytes(), disk.bytes())
    };
    let from = if kind == ChangeKind::Renamed {
        rename_from
    } else {
        None
    };
    Ok(Some(Change {
        path: path.to_owned(),
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
) -> Option<Result<Outbound, wire::WireError>> {
    let id = message.git_request_id()?;
    // Out of open files, the request waits for one (`runner.md` § Load).
    let result =
        demi_command_service::descriptors::retry(cancel, || call(service, message, default_cwd, cancel)).await;
    Some(match result {
        Ok(reply) => wire::within_limit(reply, |reason| {
            wire::git_error(id.to_owned(), GitError::TooLarge.code().to_owned(), reason)
        }),
        Err(error) => wire::git_error(id.to_owned(), error.code().to_owned(), error.message()),
    })
}

async fn call(
    service: &GitService,
    message: &Inbound,
    default_cwd: &Path,
    cancel: &CancellationToken,
) -> Result<Outbound, GitError> {
    match message {
        Inbound::GitChanges { id, root } => {
            let root = resolve(root, default_cwd).map_err(GitError::Io)?;
            let changes = service.changes(&root, cancel).await?;
            wire::git_ok_changes(id.clone(), to_wire(changes)).map_err(internal)
        }
        _ => Err(GitError::Internal("not a working-tree request".into())),
    }
}

fn to_wire(changes: Changes) -> wire::GitOkChangesResult {
    wire::GitOkChangesResult {
        repository: changes.repository,
        head: changes.head,
        files: changes
            .files
            .into_iter()
            .map(|change| wire::GitOkChangesResultFilesItem {
                path: change.path,
                kind: change.kind.as_str().to_owned(),
                from: change.from,
                added: change.added,
                removed: change.removed,
            })
            .collect(),
        truncated: changes.truncated,
        watched: changes.watched,
    }
}
