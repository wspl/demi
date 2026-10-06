//! What the backend and a runner both apply to a page's work on a Host's
//! files, so that the relay's routes and a direct channel answer alike
//! (`direct-channel.md` § Operations on the channel): a listing's entries,
//! the paths a delete keeps, how a failed file operation reads to the page,
//! and the page's file watch with the order of its states (`web-api.md`
//! § File watch).

use std::collections::BTreeMap;

use demi_shared_types::{MAX_SAFE_INTEGER, Timestamp};
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use typed_path::Utf8TypedPath;

/// One entry of a listing. An entry that disappears while the directory is
/// listed is left out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase")]
#[garde(allow_unvalidated)]
pub struct DirectoryEntry {
    pub name: String,
    pub is_directory: bool,
    pub is_symbolic_link: bool,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub size: u64,
    pub modified_at: Timestamp,
}

impl DirectoryEntry {
    /// A runner's listing entry as the page lists it; none for a time out
    /// of range.
    pub fn from_wire(entry: crate::wire::DirEntry) -> Option<Self> {
        Some(Self {
            name: entry.name,
            is_directory: entry.is_directory,
            is_symbolic_link: entry.is_symbolic_link,
            size: entry.size,
            modified_at: Timestamp::from_millisecond(entry.mtime.0).ok()?,
        })
    }
}

/// Whether deleting `path` would take with it a directory the Host needs:
/// a filesystem root, or one of `kept` or a directory holding it
/// (`web-api.md` § File text and working tree changes). Compared without
/// case, since a Host's filesystem may ignore it.
pub fn protected_path<'a>(path: &str, kept: impl IntoIterator<Item = &'a str>) -> bool {
    let top = Utf8TypedPath::derive(&path.to_lowercase()).normalize();
    if top.parent().is_none() {
        return true;
    }
    kept.into_iter().any(|directory| {
        Utf8TypedPath::derive(&directory.to_lowercase())
            .normalize()
            .starts_with(top.as_str())
    })
}

/// What a file operation's failure is to the page, by the errno-style code
/// the Host gave: nothing at the path, no permission, or anything else
/// (`web-api.md` § File text and working tree changes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsFailure {
    NotFound,
    Forbidden,
    Failed,
}

impl FsFailure {
    pub fn of(code: Option<&str>) -> Self {
        match code {
            Some("ENOENT") => Self::NotFound,
            Some("EACCES" | "EPERM") => Self::Forbidden,
            _ => Self::Failed,
        }
    }
}

/// The most folders and files outside the working tree a page's file watch
/// names (`web-api.md` § File watch).
pub const MAX_WATCHED_PATHS: usize = 64;

/// A message the page sends on its file watch, `WS .../fs/watch` or a
/// direct `watch` channel (`web-api.md` § File watch).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum FileWatchRequest {
    /// The folders and files outside the working tree the page shows now,
    /// each absolute; each message replaces the last.
    Paths {
        #[garde(length(max = MAX_WATCHED_PATHS), inner(length(chars, min = 1)))]
        paths: Vec<String>,
    },
}

/// A message a page's file watch carries to the page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FileWatchMessage {
    /// Where the Host's watch is; `reason` says why the Host cannot watch.
    State {
        state: FileWatchState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// Absolute paths on the Host that something changed at, each once.
    Changed { paths: Vec<String> },
    /// Nothing else was sent for 30 seconds: the watch is quiet, not dead.
    Heartbeat,
}

impl FileWatchMessage {
    fn state(state: FileWatchState) -> Self {
        Self::State {
            state,
            reason: None,
        }
    }
}

/// Where a page's file watch is (`web-api.md` § File watch).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileWatchState {
    /// The Host's watch runs: what is read from now on is covered.
    Live,
    /// The Host's watch lost reports: nothing read before is confirmed.
    Lost,
    /// The Host cannot watch.
    Unavailable,
    /// The Host is out of reach, or a stopped Cloud.
    Offline,
}

/// What one of the Host's watches reports to a page's watch that follows
/// it (`runner.md` § Watching files).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchReport {
    /// The watch runs: a change from now on is reported.
    Ready,
    /// Absolute paths something changed at.
    Changed(Vec<String>),
    /// The watch lost reports; it keeps running.
    Lost,
    /// The Host cannot watch the path, for this reason; nothing more comes.
    Failed(String),
}

/// Where one of the Host's watches is.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Coverage {
    Starting,
    Running,
    Failed(String),
}

/// The page's file watch as either end keeps it (`web-api.md` § File
/// watch): the working tree's watch and one watch of each path outside it
/// the page names, and the states it owes the page. The page learns which
/// of its paths are covered from the order of the states: the first `live`
/// covers the working tree; each `paths` message is answered by one `live`
/// once every watch it names runs; a `lost` is followed by a `live` of its
/// own once the watches run again. A Host that cannot watch a path says
/// `unavailable` once, and the watch says no state after it.
///
/// It holds no watch itself: its owner starts and stops the Host's watches
/// it names and hands it their reports.
#[derive(Debug)]
pub struct PageWatch {
    tree: Coverage,
    followed: BTreeMap<String, Coverage>,
    /// The first `live` went out.
    initial: bool,
    /// A `lost` went out whose `live` is owed.
    rescan: bool,
    /// `paths` messages taken whose `live` is owed.
    answers: usize,
    /// The page heard that the Host cannot watch; no state follows.
    unavailable: bool,
}

/// The Host's watches a `paths` message starts and stops.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct WatchedPaths {
    pub start: Vec<String>,
    pub stop: Vec<String>,
}

impl Default for PageWatch {
    fn default() -> Self {
        Self::new()
    }
}

impl PageWatch {
    /// A watch whose working tree's watch is starting.
    pub fn new() -> Self {
        Self {
            tree: Coverage::Starting,
            followed: BTreeMap::new(),
            initial: false,
            rescan: false,
            answers: 0,
            unavailable: false,
        }
    }

    /// Follows the paths of a `paths` message, and only them: the watches to
    /// start and to stop. A path its owner could not start a watch for, as
    /// on a connection that ended, it drops with `forget`.
    pub fn paths(&mut self, paths: Vec<String>) -> WatchedPaths {
        let mut changed = WatchedPaths::default();
        let named: std::collections::BTreeSet<&String> = paths.iter().collect();
        self.followed.retain(|path, _| {
            let kept = named.contains(path);
            if !kept {
                changed.stop.push(path.clone());
            }
            kept
        });
        for path in &paths {
            if !self.followed.contains_key(path) {
                self.followed.insert(path.clone(), Coverage::Starting);
                changed.start.push(path.clone());
            }
        }
        self.answers += 1;
        changed
    }

    /// Stops following `path`, whose watch could not start.
    pub fn forget(&mut self, path: &str) {
        self.followed.remove(path);
    }

    /// Takes one watch's report, `path` naming the watch of a path outside
    /// the working tree and none the working tree's: the messages the page
    /// hears of it at once.
    pub fn report(&mut self, path: Option<&str>, report: WatchReport) -> Vec<FileWatchMessage> {
        let coverage = match path {
            None => &mut self.tree,
            Some(path) => match self.followed.get_mut(path) {
                Some(coverage) => coverage,
                // A path the page no longer names.
                None => return Vec::new(),
            },
        };
        match report {
            WatchReport::Ready => {
                if *coverage == Coverage::Starting {
                    *coverage = Coverage::Running;
                }
                Vec::new()
            }
            WatchReport::Failed(reason) => {
                *coverage = Coverage::Failed(reason);
                Vec::new()
            }
            WatchReport::Changed(paths) => vec![FileWatchMessage::Changed { paths }],
            // Before the first `live` nothing was covered.
            WatchReport::Lost if self.initial && !self.unavailable => {
                self.rescan = true;
                vec![FileWatchMessage::state(FileWatchState::Lost)]
            }
            WatchReport::Lost => Vec::new(),
        }
    }

    /// The states the watches owe the page now: `unavailable` once a watch
    /// failed, and otherwise, once every watch runs, one `live` for the
    /// first, for a `lost` and for each `paths` message not yet answered.
    pub fn settle(&mut self) -> Vec<FileWatchMessage> {
        if self.unavailable {
            return Vec::new();
        }
        let coverages = std::iter::once(&self.tree).chain(self.followed.values());
        let mut starting = false;
        let mut failed = None;
        for coverage in coverages {
            match coverage {
                Coverage::Starting => starting = true,
                Coverage::Failed(reason) => failed = failed.or(Some(reason.clone())),
                Coverage::Running => {}
            }
        }
        if let Some(reason) = failed {
            self.unavailable = true;
            return vec![FileWatchMessage::State {
                state: FileWatchState::Unavailable,
                reason: Some(reason),
            }];
        }
        if starting {
            return Vec::new();
        }
        let owed = usize::from(!self.initial) + usize::from(self.rescan) + self.answers;
        self.initial = true;
        self.rescan = false;
        self.answers = 0;
        vec![FileWatchMessage::state(FileWatchState::Live); owed]
    }

    /// Whether the page heard that the Host cannot watch, after which the
    /// watch says no state.
    pub fn unavailable(&self) -> bool {
        self.unavailable
    }
}
