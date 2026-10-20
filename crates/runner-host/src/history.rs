//! A repository's history, answered in process with gitoxide for the Change
//! view's Commits mode.
//!
//! It answers:
//!
//! - the local branches, most recently committed first;
//! - a branch's first-parent log, by pages;
//! - one commit's files against its first parent, with the branches it is on. A
//!   merge compares with its first parent, as GitHub and VS Code show it.
//!
//! A commit is named by an id or an abbreviation of at least 7 hex digits; one
//! that names two commits is `ambiguous`.

use std::{
    cell::RefCell,
    collections::{BTreeMap, HashSet},
    rc::Rc,
    io,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

use gix::bstr::{BStr, ByteSlice};
use gix::hash::ObjectId;
use gix::object::tree::diff::ChangeDetached;

use demi_command_protocol::line_counts;
use demi_runner_protocol::wire::{self, ChangeKind};

use crate::git::{GitError, Located, MAX_BLOB_BYTES, discover, internal, locate, root_relative_path};

/// The most commits one page of a log holds.
pub const LOG_PAGE: usize = 50;
/// The fewest hex digits that name a commit.
const MIN_ABBREVIATION: usize = 7;

fn not_found() -> GitError {
    GitError::Io(io::Error::from(io::ErrorKind::NotFound))
}

fn interrupted(interrupt: &AtomicBool) -> Result<(), GitError> {
    if interrupt.load(Ordering::Relaxed) {
        return Err(GitError::Cancelled);
    }
    Ok(())
}

/// The repository `root` lies in, with where `root` sits in it.
fn open(root: &Path) -> Result<(gix::Repository, Located), GitError> {
    let located = locate(root)?.ok_or(GitError::NotRepository)?;
    let repo = discover(root)?.ok_or(GitError::NotRepository)?;
    Ok((repo, located))
}

/// A local branch's name and the commit it points at.
struct Branch {
    name: String,
    tip: ObjectId,
    time: i64,
}

/// The local branches, most recently committed first, then by name. A
/// branch that points at no commit is left out.
fn local_branches(repo: &gix::Repository) -> Result<Vec<Branch>, GitError> {
    let platform = repo.references().map_err(internal)?;
    let mut branches = Vec::new();
    for reference in platform.local_branches().map_err(internal)? {
        let mut reference = reference.map_err(|error| GitError::Internal(error.to_string()))?;
        let name = reference.name().shorten().to_str_lossy().into_owned();
        let Ok(tip) = reference.peel_to_id() else {
            continue;
        };
        let Ok(commit) = tip.object().map(|object| object.try_into_commit()) else {
            continue;
        };
        let Ok(commit) = commit else {
            continue;
        };
        let time = commit.time().map_err(internal)?.seconds;
        branches.push(Branch {
            name,
            tip: tip.detach(),
            time,
        });
    }
    branches.sort_by(|a, b| b.time.cmp(&a.time).then_with(|| a.name.cmp(&b.name)));
    Ok(branches)
}

/// The branch HEAD names, its name whether or not it has a commit yet; none
/// when HEAD is detached.
fn current_branch(repo: &gix::Repository) -> Result<Option<String>, GitError> {
    let name = repo.head_name().map_err(internal)?;
    Ok(name.and_then(|name| {
        let name = name.as_bstr().to_str_lossy();
        name.strip_prefix("refs/heads/").map(str::to_owned)
    }))
}

pub fn branches(root: &Path) -> Result<wire::GitBranches, GitError> {
    let (repo, _) = open(root)?;
    let current = current_branch(&repo)?;
    let head = if current.is_none() {
        Some(repo.head_id().map_err(internal)?.to_string())
    } else {
        None
    };
    Ok(wire::GitBranches {
        branches: local_branches(&repo)?.into_iter().map(|branch| branch.name).collect(),
        current,
        head,
    })
}

/// The commit `text` names, a full id or an abbreviation of at least seven
/// hex digits: `ENOENT` when the repository has no such commit, `ambiguous`
/// when the abbreviation names more than one.
fn resolve(repo: &gix::Repository, text: &str) -> Result<ObjectId, GitError> {
    let hex = text.len() >= MIN_ABBREVIATION
        && text.len() <= repo.object_hash().len_in_hex()
        && text.bytes().all(|byte| byte.is_ascii_hexdigit());
    if !hex {
        return Err(not_found());
    }
    let prefix = gix::hash::Prefix::from_hex(&text.to_ascii_lowercase()).map_err(internal)?;
    let mut candidates = HashSet::new();
    repo.objects
        .lookup_prefix(prefix, Some(&mut candidates))
        .map_err(internal)?;
    let mut commits = candidates.into_iter().filter(|id| {
        repo.find_header(*id)
            .is_ok_and(|header| header.kind() == gix::object::Kind::Commit)
    });
    match (commits.next(), commits.next()) {
        (Some(id), None) => Ok(id),
        (Some(_), Some(_)) => Err(GitError::Ambiguous),
        (None, _) => Err(not_found()),
    }
}

fn find_commit<'repo>(repo: &'repo gix::Repository, id: ObjectId) -> Result<gix::Commit<'repo>, GitError> {
    repo.find_commit(id).map_err(|_| not_found())
}

fn entry(commit: &gix::Commit<'_>) -> Result<wire::GitLogEntry, GitError> {
    let author = commit.author().map_err(internal)?;
    Ok(wire::GitLogEntry {
        id: commit.id.to_string(),
        message: commit.message_raw().map_err(internal)?.to_str_lossy().into_owned(),
        author: author.trim().name.to_str_lossy().into_owned(),
        time: wire::commit_time(commit.time().map_err(internal)?.seconds),
    })
}

/// One page of `branch`'s first-parent history, HEAD's without one, after
/// the commit `after` names.
pub fn log(
    root: &Path,
    branch: Option<&str>,
    after: Option<&str>,
    interrupt: &AtomicBool,
) -> Result<wire::GitLog, GitError> {
    let (repo, _) = open(root)?;
    let start = match after {
        Some(after) => {
            let after = find_commit(&repo, resolve(&repo, after)?)?;
            after.parent_ids().next().map(|id| id.detach())
        }
        None => match branch {
            Some(name) => {
                let mut reference = repo
                    .try_find_reference(format!("refs/heads/{name}").as_str())
                    .map_err(internal)?
                    .ok_or_else(not_found)?;
                Some(reference.peel_to_id().map_err(internal)?.detach())
            }
            // An unborn HEAD has no history yet.
            None => repo.head_id().ok().map(|id| id.detach()),
        },
    };
    let mut commits = Vec::new();
    let mut next = start;
    while let Some(id) = next {
        interrupted(interrupt)?;
        if commits.len() == LOG_PAGE {
            break;
        }
        let commit = find_commit(&repo, id)?;
        commits.push(entry(&commit)?);
        next = commit.parent_ids().next().map(|id| id.detach());
    }
    Ok(wire::GitLog {
        commits,
        more: next.is_some(),
    })
}

/// The commit `text` names: its entry, its files under the root against its
/// first parent, at most `max_files`, and the local branches it is on.
pub fn commit(
    root: &Path,
    text: &str,
    max_files: usize,
    interrupt: &AtomicBool,
) -> Result<wire::GitCommit, GitError> {
    let (repo, located) = open(root)?;
    let id = resolve(&repo, text)?;
    let commit = find_commit(&repo, id)?;
    let logged = entry(&commit)?;
    let parents: Vec<ObjectId> = commit.parent_ids().map(|id| id.detach()).collect();
    let tree = commit.tree().map_err(internal)?;
    let parent_tree = match parents.first() {
        Some(parent) => Some(find_commit(&repo, *parent)?.tree().map_err(internal)?),
        None => None,
    };
    let changes = repo
        .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)
        .map_err(internal)?;
    let mut files = BTreeMap::new();
    let mut truncated = false;
    for change in changes {
        interrupted(interrupt)?;
        let Some(file) = commit_file(&repo, &located, &change)? else {
            continue;
        };
        if files.len() == max_files {
            truncated = true;
            break;
        }
        files.insert(file.path.clone(), file);
    }
    let branches = containing(&repo, id, interrupt)?;
    Ok(wire::GitCommit {
        commit: wire::GitCommitEntry {
            id: logged.id,
            message: logged.message,
            author: logged.author,
            time: logged.time,
            parents: parents.iter().map(ToString::to_string).collect(),
        },
        files: files.into_values().collect(),
        truncated,
        branches,
    })
}

/// A blob's bytes for counting lines; none beyond the size limit, which
/// counts no lines, as a binary file does.
fn blob(repo: &gix::Repository, id: &gix::hash::oid) -> Result<Option<Vec<u8>>, GitError> {
    let header = repo.find_header(id).map_err(internal)?;
    if header.size() > MAX_BLOB_BYTES {
        return Ok(None);
    }
    Ok(Some(repo.find_object(id).map_err(internal)?.data.clone()))
}

/// One changed tree entry as a file of the commit, relative to the root:
/// none for a folder, a submodule, or a path outside the root. A rename
/// whose other side lies outside the root is an addition or a deletion
/// there, and a copy an addition.
fn commit_file(
    repo: &gix::Repository,
    located: &Located,
    change: &ChangeDetached,
) -> Result<Option<wire::GitCommitFile>, GitError> {
    let (mode, id) = change.entry_mode_and_id();
    let (source_mode, source_id) = change.source_entry_mode_and_id();
    let under = |location: &BStr| {
        root_relative_path(&location.to_str_lossy(), &located.prefix)
            .filter(|path| !path.is_empty())
            .map(str::to_owned)
    };
    let path = under(change.location());
    let source = under(change.source_location());
    let (kind, path, from) = match change {
        ChangeDetached::Addition { .. } => (ChangeKind::Added, path, None),
        ChangeDetached::Deletion { .. } => (ChangeKind::Deleted, path, None),
        ChangeDetached::Modification { .. } => (ChangeKind::Modified, path, None),
        ChangeDetached::Rewrite { copy: true, .. } => (ChangeKind::Added, path, None),
        ChangeDetached::Rewrite { copy: false, .. } => match (path, source) {
            (Some(path), Some(source)) => (ChangeKind::Renamed, Some(path), Some(source)),
            (Some(path), None) => (ChangeKind::Added, Some(path), None),
            (None, source) => (ChangeKind::Deleted, source, None),
        },
    };
    let Some(path) = path else {
        return Ok(None);
    };
    let before = !matches!(kind, ChangeKind::Added);
    let after = !matches!(kind, ChangeKind::Deleted);
    // A folder, or a submodule's commit, is no file of the commit.
    if (before && !source_mode.is_blob_or_symlink()) || (after && !mode.is_blob_or_symlink()) {
        return Ok(None);
    }
    let original = if before { blob(repo, source_id)? } else { Some(Vec::new()) };
    let modified = if after { blob(repo, id)? } else { Some(Vec::new()) };
    let (added, removed) = match (&original, &modified) {
        (Some(original), Some(modified)) => line_counts(
            before.then_some(original.as_slice()),
            after.then_some(modified.as_slice()),
        ),
        _ => (0, 0),
    };
    let status = match kind {
        ChangeKind::Added => "A",
        ChangeKind::Modified => "M",
        ChangeKind::Deleted => "D",
        ChangeKind::Renamed => "R",
    };
    Ok(Some(wire::GitCommitFile {
        path,
        status: status.to_owned(),
        kind,
        from,
        added,
        removed,
    }))
}

/// The local branches whose history holds `id`, the checked-out one first, then
/// most recently committed first.
///
/// Commit times say nothing of ancestry, as a rebase or a skewed clock shows,
/// so each walk goes until it finds `id`. A walk that ends without it has seen
/// only commits that cannot lead to it, which later walks skip.
fn containing(repo: &gix::Repository, id: ObjectId, interrupt: &AtomicBool) -> Result<Vec<String>, GitError> {
    let current = current_branch(repo)?;
    let unrelated: Rc<RefCell<HashSet<ObjectId>>> = Rc::default();
    let mut found = Vec::new();
    for branch in local_branches(repo)? {
        let mut holds = branch.tip == id;
        if !holds {
            let known = Rc::clone(&unrelated);
            let walk = repo
                .rev_walk([branch.tip])
                .selected(move |commit| !known.borrow().contains(commit))
                .map_err(internal)?;
            let mut seen = Vec::new();
            for info in walk {
                interrupted(interrupt)?;
                let commit = info.map_err(internal)?.id;
                if commit == id {
                    holds = true;
                    break;
                }
                seen.push(commit);
            }
            if !holds {
                unrelated.borrow_mut().extend(seen);
            }
        }
        if holds {
            found.push(branch.name);
        }
    }
    if let Some(at) = current.and_then(|current| found.iter().position(|name| *name == current)) {
        let current = found.remove(at);
        found.insert(0, current);
    }
    Ok(found)
}

/// The tree the file reads come from: `commit`'s when one is named, else
/// HEAD's.
pub fn tree_at<'repo>(repo: &'repo gix::Repository, commit: &str) -> Result<gix::Tree<'repo>, GitError> {
    let id = resolve(repo, commit)?;
    find_commit(repo, id)?.tree().map_err(internal)
}
