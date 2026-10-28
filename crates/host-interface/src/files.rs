//! Batched reads of a Host's files: each path with the most bytes to answer,
//! and what each path is. A plugin asks them through its port, and the
//! product and the agent runtime read a project's files with them.

use demi_shared_types::B64Bytes;
use serde::{Deserialize, Serialize};

/// One path read on a Host, as the Host names it, and the most
/// bytes of a file to answer: absolute, or, for a command, relative to its
/// job's directory, which the Host's own rules resolve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostRead {
    pub path: String,
    pub limit: u64,
    /// Whether a file within the limit is read by its content: the Host
    /// hashes it, and its bytes travel only when the user's blobs lack them
    /// and no read of the same content is under way. A file over the limit
    /// then answers its size and no bytes.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub content: bool,
}

impl HostRead {
    pub fn new(path: String, limit: u64) -> Self {
        Self { path, limit, content: false }
    }

    /// A read of a whole file of at most `limit` bytes by its content, such as
    /// a command's input image, which a batch of commands reads alike.
    pub fn content(path: String, limit: u64) -> Self {
        Self { path, limit, content: true }
    }
}

/// What a path on a Host is. A symbolic link is followed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum HostFile {
    Missing,
    /// A directory's entries, in the Host's order.
    Directory {
        entries: Vec<HostEntry>,
    },
    /// A file's first bytes, at most the read's limit, and its size.
    File {
        bytes: B64Bytes,
        size: u64,
    },
    /// Neither a file nor a directory.
    Other,
    /// The Host could not read it, such as for want of permission.
    Unreadable {
        message: String,
    },
}

/// An entry of a directory on a Host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostEntry {
    pub name: String,
    pub kind: EntryKind,
}

/// An entry's kind, without following a symbolic link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Directory,
    Symlink,
    Other,
}
