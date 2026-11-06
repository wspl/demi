//! The directories a project's instruction files, workflows and skills are
//! searched in: the working
//! directory and each directory above it up to the root of its git repository,
//! the nearest that holds `.git`. Outside a repository, the working directory
//! alone.
//!
//! A device's paths are walked and joined in its own form ([`DevicePaths`]): on
//! Windows `C:\Users\alex\app` walks up to `C:\`, and its `.git` is
//! `C:\Users\alex\app\.git`.

use demi_device_paths::DevicePaths;

use crate::{DeviceFile, DeviceRead};

/// The reads that find the repository's root among `ancestors`, the
/// directories [`DevicePaths::ancestors`] gives: `.git` in each, in their
/// order.
pub fn root_reads(paths: DevicePaths, ancestors: &[String]) -> Vec<DeviceRead> {
    ancestors
        .iter()
        .map(|directory| DeviceRead::new(paths.join(directory, ".git"), 0))
        .collect()
}

/// The `.git` paths whose coming moves the repository's root nearer, from
/// `ancestors` and the answers to their [`root_reads`]: in each directory
/// below the root, or in every one outside a repository. The root's own
/// `.git` is left out: git writes in it at every command.
pub fn root_watch(paths: DevicePaths, ancestors: &[String], git: &[DeviceFile]) -> Vec<String> {
    let in_repository = git
        .iter()
        .any(|file| !matches!(file, DeviceFile::Missing | DeviceFile::Unreadable { .. }));
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
pub fn searched<'a>(ancestors: &'a [String], git: &[DeviceFile]) -> &'a [String] {
    let root = git
        .iter()
        .position(|file| !matches!(file, DeviceFile::Missing | DeviceFile::Unreadable { .. }));
    match root {
        Some(root) => &ancestors[..=root],
        None => &ancestors[..1],
    }
}
