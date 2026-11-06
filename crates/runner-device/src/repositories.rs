//! The repositories the Change view offers for a working directory
//! (`git_repositories`), found with the working-tree requests' `discover`:
//!
//! - the one that holds the directory, at it or above it;
//! - each one up to two levels below it, as a checkout such as `gclient`'s
//!   nests them (`src/engine` inside `src`). The scan enters repositories,
//!   but not `.git` or `node_modules` folders, as VS Code's scan does not,
//!   and follows no symbolic link, which could lead it out of the directory
//!   or round in a loop;
//! - the one that holds each directory the request names, at any depth,
//!   where the agents changed files. A directory a command removed counts by
//!   the nearest one still there.
//!
//! The answer says whether the directory is in a repository, which the page
//! shows as the directory itself, and lists the others by their work trees'
//! paths, each once, spelled from the directory as the request named it
//! (`/tmp/atlas/src`, not `/private/tmp/atlas/src`), so the page compares them
//! with the paths it holds. A folder the scan cannot read, or a candidate
//! whose repository cannot be opened, is left out rather than failing the
//! whole list.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

use demi_runner_protocol::wire;

use crate::git::{GitError, locate};
use crate::history::interrupted;

/// How many levels below the working directory the scan looks.
const SCAN_DEPTH: usize = 2;
/// The folders the scan does not enter: a repository's own store, and
/// dependency folders, which hold many folders and are no work of the
/// user's.
const SKIPPED: [&str; 2] = [".git", "node_modules"];

/// The repositories `root`, a canonical path, is in or holds, and those
/// holding `paths`, below `root` spelled from `named`.
pub(crate) fn repositories(
    root: &Path,
    named: &Path,
    paths: &[PathBuf],
    interrupt: &Arc<AtomicBool>,
) -> Result<wire::GitRepositories, GitError> {
    let holding = work_tree(root)?;
    let mut found = BTreeSet::new();
    let mut level = vec![root.to_path_buf()];
    for _ in 0..SCAN_DEPTH {
        let mut next = Vec::new();
        for folder in level {
            let Ok(entries) = std::fs::read_dir(&folder) else {
                continue;
            };
            for entry in entries.flatten() {
                interrupted(interrupt)?;
                // A symbolic link's own type is not a directory.
                if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    continue;
                }
                if SKIPPED.iter().any(|skipped| entry.file_name() == *skipped) {
                    continue;
                }
                let child = entry.path();
                if child.join(".git").symlink_metadata().is_ok()
                    && let Ok(Some(tree)) = work_tree(&child)
                    && tree == child
                {
                    found.insert(tree);
                }
                next.push(child);
            }
        }
        level = next;
    }
    for path in paths {
        interrupted(interrupt)?;
        // Canonical, as `root` is, so a path through a symbolic link compares.
        let Some(existing) = path
            .ancestors()
            .find(|ancestor| ancestor.is_dir())
            .and_then(|existing| std::fs::canonicalize(existing).ok())
        else {
            continue;
        };
        if !existing.starts_with(root) {
            continue;
        }
        if let Ok(Some(tree)) = work_tree(&existing)
            && tree.starts_with(root)
        {
            found.insert(tree);
        }
    }
    if let Some(holding) = &holding {
        found.remove(holding);
    }
    // Each lies below `root`, where it was found or checked.
    let repositories = found
        .iter()
        .filter_map(|tree| tree.strip_prefix(root).ok())
        .map(|relative| named.join(relative).to_string_lossy().into_owned())
        .collect();
    Ok(wire::GitRepositories {
        in_repository: holding.is_some(),
        repositories,
    })
}

/// The work tree of the repository that holds `directory`, if any.
fn work_tree(directory: &Path) -> Result<Option<PathBuf>, GitError> {
    Ok(locate(directory)?.map(|located| located.workdir))
}
