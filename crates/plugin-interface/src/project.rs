//! The directories a project's instruction files and workflows are searched in
//! (`crates/backend-user-shard/src/conversation/workflows.rs`): the working
//! directory and each directory above it up to the root of its git repository,
//! the nearest that holds `.git`. Outside a repository, the working directory
//! alone.
//!
//! A Host's paths are walked and joined in its own form ([`HostPaths`]): on
//! Windows `C:\Users\alex\app` walks up to `C:\`, and its `.git` is
//! `C:\Users\alex\app\.git`.

use demi_host_interface::HostPaths;

use crate::{HostFile, HostRead};

/// The reads that find the repository's root among `ancestors`, the
/// directories [`HostPaths::ancestors`] gives: `.git` in each, in their
/// order.
pub fn root_reads(paths: HostPaths, ancestors: &[String]) -> Vec<HostRead> {
    ancestors
        .iter()
        .map(|directory| HostRead {
            path: paths.join(directory, ".git"),
            limit: 0,
    within: None,
})
        .collect()
}

/// The `.git` paths whose coming moves the repository's root nearer, from
/// `ancestors` and the answers to their [`root_reads`]: in each directory
/// below the root, or in every one outside a repository. The root's own
/// `.git` is left out: git writes in it at every command.
pub fn root_watch(paths: HostPaths, ancestors: &[String], git: &[HostFile]) -> Vec<String> {
    let in_repository = git
        .iter()
        .any(|file| !matches!(file, HostFile::Missing | HostFile::Unreadable { .. }));
    let below_root = if in_repository {
        searched(ancestors, git).len() - 1
    } else {
        ancestors.len()
    };
    ancestors[..below_root]
        .iter()
        .map(|directory| paths.join(directory, ".git"))
        .collect()
}

/// The directories searched, nearest first, from `ancestors` and the
/// answers to their [`root_reads`]: those up to the first that holds `.git`,
/// or the first alone when none does.
pub fn searched<'a>(ancestors: &'a [String], git: &[HostFile]) -> &'a [String] {
    let root = git
        .iter()
        .position(|file| !matches!(file, HostFile::Missing | HostFile::Unreadable { .. }));
    match root {
        Some(root) => &ancestors[..=root],
        None => &ancestors[..1],
    }
}
