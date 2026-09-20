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
    time::{Duration, Instant, SystemTime},
};

use gix::bstr::{BStr, BString, ByteSlice};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::connection::wire::{self as wire, Inbound, Outbound};
use crate::paths::resolve;
use crate::tree_watch::{TreeWatch, WatchEvent};

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
    watcher: Option<TreeWatch>,
    /// A watch is tried once per root; a failure means whole walks from then on.
    watch_tried: bool,
}

struct Baseline {
    head: Option<String>,
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
        let located = blocking(cancel, move || {
            Ok(locate(&root_path)?.map(|located| {
                let rules = rules_above(&root_path, &located);
                (located, rules)
            }))
        })
        .await?;
        let Some((located, rules)) = located else {
            // A directory that stops being a repository loses its baseline and watch.
            state.baseline = None;
            state.watcher = None;
            return Ok(Changes::outside_repository());
        };
        if !state.watch_tried {
            let root_path = root.path.clone();
            let git_dir = located.git_dir.clone();
            let touched = root.touched.clone();
            state.watcher = blocking(cancel, move || {
                Ok(start_watch(&root_path, &git_dir, touched))
            })
            .await?;
            state.watch_tried = true;
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
            Some(baseline) if watched && !touched.whole && baseline.rules_above == rules => {
                let scope = paths_scope(&touched.paths, &located, baseline);
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
        self.files.retain(|path, _| !within(path, root_relative));
        self.files.extend(computed.files);
        self.head = computed.head;
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
            Err(error) if demi_command_service::descriptors::exhausted(&error) => {
                Err(GitError::Io(error))
            }
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

impl Touched {
    /// Notes what a watch saw. Metadata alone under `.git` changes nothing
    /// git lists; any other change there, or to a `.gitignore` or
    /// `.gitattributes`, can change what git lists anywhere, so the next walk
    /// is whole.
    fn record(&mut self, event: WatchEvent, git_dir: &Path) {
        let (path, metadata) = match event {
            WatchEvent::Changed { path, metadata } => (path, metadata),
            WatchEvent::Lost => {
                self.whole = true;
                return;
            }
            WatchEvent::Failed => {
                self.broken = true;
                return;
            }
        };
        if path.starts_with(git_dir) {
            if !metadata {
                self.whole = true;
            }
        } else if !metadata && rules_file(&path) {
            self.whole = true;
        } else if !self.whole {
            self.paths.insert(path);
            if self.paths.len() > MAX_TOUCHED {
                self.whole = true;
                self.paths.clear();
            }
        }
    }
}

/// Whether git reads `path` to decide what it lists and how it compares
/// files: a `.gitignore` or a `.gitattributes`.
fn rules_file(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some(".gitignore" | ".gitattributes")
    )
}

/// Watches the root, and the repository's `.git` when that lies outside it.
fn start_watch(root: &Path, git_dir: &Path, touched: Arc<Mutex<Touched>>) -> Option<TreeWatch> {
    let mut trees = vec![root];
    if !git_dir.starts_with(root) {
        trees.push(git_dir);
    }
    let git_dir = git_dir.to_path_buf();
    TreeWatch::start(&trees, move |event| lock(&touched).record(event, &git_dir))
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
) -> Option<Result<Outbound, wire::WireError>> {
    let id = message.git_request_id()?;
    // Out of open files, the request waits for one (`runner.md` § Load).
    let result = demi_command_service::descriptors::retry(cancel, || {
        call(service, message, default_cwd, cancel)
    })
    .await;
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
                status: change.status,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_watch_notes_what_can_change_what_git_lists() {
        let git_dir = Path::new("/repo/.git");
        let changed = |path: &str, metadata| WatchEvent::Changed {
            path: PathBuf::from(path),
            metadata,
        };

        // Metadata alone under `.git` changes nothing.
        let mut touched = Touched::default();
        touched.record(changed("/repo/.git/objects/01/e79c", true), git_dir);
        touched.record(changed("/repo/.git/index", true), git_dir);
        assert!(touched.paths.is_empty());
        assert!(!touched.whole);

        // A file written or given another mode is re-examined, a rules file's
        // mode alone too.
        touched.record(changed("/repo/a.txt", false), git_dir);
        touched.record(changed("/repo/x.sh", true), git_dir);
        touched.record(changed("/repo/.gitignore", true), git_dir);
        let expected = ["/repo/a.txt", "/repo/x.sh", "/repo/.gitignore"].map(PathBuf::from);
        assert_eq!(touched.paths, HashSet::from(expected));
        assert!(!touched.whole);

        // Anything else under `.git`, a rules file changed, or lost events.
        for event in [
            changed("/repo/.git/index", false),
            changed("/repo/sub/.gitattributes", false),
            changed("/repo/.gitignore", false),
            WatchEvent::Lost,
        ] {
            let mut touched = Touched::default();
            touched.record(event, git_dir);
            assert!(touched.whole);
        }
        let mut touched = Touched::default();
        touched.record(WatchEvent::Failed, git_dir);
        assert!(touched.broken);
    }

    #[test]
    fn a_walk_over_one_path_of_a_staged_rename_takes_in_the_other() {
        let located = Located {
            workdir: PathBuf::from("/repo"),
            git_dir: PathBuf::from("/repo/.git"),
            prefix: "sub".into(),
        };
        // Its new path deleted from the disk too, the rename is not listed.
        let baseline = Baseline {
            head: None,
            files: BTreeMap::new(),
            renames: BTreeMap::from([("moved.txt".into(), "old/a.txt".into())]),
            rules_above: Vec::new(),
            truncated: false,
        };
        for (touched, examined) in [
            ("/repo/sub/moved.txt", ["moved.txt", "old/a.txt"]),
            ("/repo/sub/old", ["old", "moved.txt"]),
        ] {
            let touched = HashSet::from([PathBuf::from(touched)]);
            let Some(Scope::Paths {
                pathspecs,
                root_relative,
            }) = paths_scope(&touched, &located, &baseline)
            else {
                panic!("{touched:?} is a paths scope");
            };
            assert_eq!(root_relative, examined);
            let expected: Vec<BString> = examined
                .iter()
                .map(|path| literal_pathspec(&format!("sub/{path}")))
                .collect();
            assert_eq!(pathspecs, expected);
        }
        let unrelated = HashSet::from([PathBuf::from("/repo/sub/other.txt")]);
        let Some(Scope::Paths { root_relative, .. }) = paths_scope(&unrelated, &located, &baseline)
        else {
            panic!("other.txt is a paths scope");
        };
        assert_eq!(root_relative, ["other.txt"]);
    }
}
