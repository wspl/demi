//! The directories a project's files are searched in (`skills.md` § Project
//! skills, `instructions.md` § Which files are read): the working directory
//! and each directory above it up to the root of its git repository, the
//! nearest that holds `.git`; outside a repository, the working directory
//! alone.

use crate::{HostFile, HostRead};

/// `cwd` and every directory above it, nearest first.
pub fn ancestors(cwd: &str) -> Vec<String> {
    let mut directory = cwd.trim_end_matches('/').to_owned();
    if directory.is_empty() {
        return vec!["/".to_owned()];
    }
    let mut ancestors = vec![directory.clone()];
    while let Some((parent, _)) = directory.rsplit_once('/') {
        directory = if parent.is_empty() {
            "/".to_owned()
        } else {
            parent.to_owned()
        };
        ancestors.push(directory.clone());
        if directory == "/" {
            break;
        }
    }
    ancestors
}

/// The path of `name` in `directory`.
pub fn join(directory: &str, name: &str) -> String {
    format!("{}/{name}", directory.trim_end_matches('/'))
}

/// The reads that find the repository's root among `ancestors`: `.git` in
/// each, in their order.
pub fn root_reads(ancestors: &[String]) -> Vec<HostRead> {
    ancestors
        .iter()
        .map(|directory| HostRead {
            path: join(directory, ".git"),
            limit: 0,
        })
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
