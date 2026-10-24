//! The linked worktrees of the repositories of the Host's projects, which the
//! backend keeps with each project so that a move to a worktree selects its
//! project without asking the Host
//! (`crates/backend-database/src/selection.rs` § Worktrees).
//!
//! The backend names the projects' directories (`projects`); the runner reports
//! each one's worktrees whole (`worktrees`) when it is named, and again when
//! they changed, checked at the moments a change could lead to a move: before
//! a command's request, a job's end and a folder's listing reach the backend
//! ([`demi_runner_protocol::wire::Lane::report`]). The report goes into the
//! lane ahead of that message, so the backend has it first. For example,
//! `git worktree add ../app-2 && demi host move ../app-2` reports `app-2`
//! before the move's request.
//!
//! A check reads no repository: it compares what git's worktree commands
//! change, by time, with what it saw at the last report, about 350 `stat`
//! calls, 0.4 ms, for 50 projects:
//!
//! - `add`, `remove` and `prune` change the `worktrees/` folder of the
//!   repository's git directory;
//! - `move` and `repair` rewrite a worktree's `gitdir` file there, and change
//!   no folder's time;
//! - `git init` makes the project's `.git`.
//!
//! Only a repository whose stamp changed is read, with gix, which reads git's
//! own files, relative worktree paths included.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use demi_runner_protocol::wire::{self, Lane, LinkedWorktree, Outbound};

/// The projects' directories and what was reported of each; shared by every
/// connection of the registration.
#[derive(Clone, Default)]
pub struct Worktrees(Arc<Shared>);

#[derive(Default)]
struct Shared {
    followed: Mutex<Followed>,
    /// How many namings of the directories came, which numbers each one.
    named: AtomicU64,
}

#[derive(Default)]
struct Followed {
    /// The number of the naming the directories are of.
    naming: u64,
    projects: BTreeMap<String, Option<Stamp>>,
}

/// What a repository's worktrees were read at: each file and folder whose
/// change changes them, with its modification time, none for one that is not
/// there.
type Stamp = Vec<(PathBuf, Option<SystemTime>)>;

impl Worktrees {
    /// The backend named the projects' directories: numbered now, in the order
    /// the namings came, and followed by [`Naming::follow`] off the loop that
    /// takes them, which reads repositories.
    pub fn named(&self, directories: Vec<String>) -> Naming {
        Naming {
            worktrees: self.clone(),
            number: self.0.named.fetch_add(1, Ordering::Relaxed) + 1,
            directories,
        }
    }

    /// Pushes into `lane` the worktrees of each project whose repository
    /// changed since its last report.
    pub fn report(&self, lane: &Lane) {
        let mut followed = self.0.followed.lock().unwrap_or_else(PoisonError::into_inner);
        report_changed(&mut followed.projects, lane);
    }
}

/// A naming of the projects' directories, which [`Worktrees::named`] numbered.
pub struct Naming {
    worktrees: Worktrees,
    number: u64,
    directories: Vec<String>,
}

impl Naming {
    /// The directories replace the ones followed, and each is reported again,
    /// as the backend keeps nothing from an earlier connection's reports. A
    /// naming that a later one overtook changes nothing, so the latest stays
    /// followed.
    pub fn follow(self, lane: &Lane) {
        let mut followed = self.worktrees.0.followed.lock().unwrap_or_else(PoisonError::into_inner);
        if followed.naming > self.number {
            return;
        }
        followed.naming = self.number;
        followed.projects = self.directories.into_iter().map(|directory| (directory, None)).collect();
        report_changed(&mut followed.projects, lane);
    }
}

fn report_changed(followed: &mut BTreeMap<String, Option<Stamp>>, lane: &Lane) {
    for (directory, last) in followed.iter_mut() {
        let stamp = stamp(Path::new(directory));
        if last.as_ref() == Some(&stamp) {
            continue;
        }
        let worktrees = linked_worktrees(Path::new(directory));
        match wire::encode(&Outbound::Worktrees {
            directory: directory.clone(),
            worktrees,
        }) {
            Ok(frame) => {
                lane.push(frame);
                *last = Some(stamp);
            }
            Err(error) => {
                tracing::warn!(event = "worktrees.report_invalid", %error, "a project's worktrees do not encode");
            }
        }
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|metadata| metadata.modified()).ok()
}

/// The stamp of the repository whose main worktree `directory` would be.
fn stamp(directory: &Path) -> Stamp {
    let dot_git = directory.join(".git");
    let mut stamp = vec![(dot_git.clone(), modified(&dot_git))];
    let Some(git_dir) = git_dir(&dot_git) else {
        return stamp;
    };
    // A linked worktree's git directory names the common one; its own
    // repository's worktrees are no project's.
    let commondir = git_dir.join("commondir");
    stamp.push((commondir.clone(), modified(&commondir)));
    let worktrees = git_dir.join("worktrees");
    stamp.push((worktrees.clone(), modified(&worktrees)));
    if let Ok(entries) = std::fs::read_dir(&worktrees) {
        let mut files: Vec<PathBuf> = entries.flatten().map(|entry| entry.path().join("gitdir")).collect();
        files.sort();
        stamp.extend(files.into_iter().map(|file| {
            let time = modified(&file);
            (file, time)
        }));
    }
    stamp
}

/// The git directory `.git` names: itself, or the one its `gitdir:` line
/// names.
fn git_dir(dot_git: &Path) -> Option<PathBuf> {
    let metadata = std::fs::metadata(dot_git).ok()?;
    if metadata.is_dir() {
        return Some(dot_git.to_owned());
    }
    let text = std::fs::read_to_string(dot_git).ok()?;
    let named = Path::new(text.strip_prefix("gitdir:")?.trim());
    Some(dot_git.parent()?.join(named))
}

/// The linked worktrees of the repository whose main worktree is
/// `directory`; none for any other directory.
fn linked_worktrees(directory: &Path) -> Vec<LinkedWorktree> {
    let Ok(repo) = gix::open(directory) else {
        return Vec::new();
    };
    if repo.git_dir() != repo.common_dir() || repo.workdir().is_none() {
        return Vec::new();
    }
    let Ok(proxies) = repo.worktrees() else {
        return Vec::new();
    };
    proxies
        .iter()
        .filter_map(|proxy| {
            let base = proxy.base().ok()?;
            // The spelling the Host names directories by, as `fs_directory`
            // answers it; as git wrote it where it is gone.
            let path = dunce::canonicalize(&base).unwrap_or(base);
            Some(LinkedWorktree {
                name: proxy.id().to_string(),
                path: path.to_string_lossy().into_owned(),
            })
        })
        .collect()
}
