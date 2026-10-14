//! Files on a Host: directory listings, file text, and a conversation's
//! working tree (`web-api.md` § Device files and remote references, § File
//! text and working tree changes).

pub use demi_runner_protocol::files::{
    DirectoryEntry, FileWatchMessage, FileWatchRequest, MAX_OUTSIDE_SUBSCRIPTIONS,
    MAX_PAGE_SUBSCRIPTIONS, PageSubscription, RefusedReason, SubscriptionKind,
};
pub use demi_runner_protocol::wire::{GitBranches, GitCommit, GitLog};
use demi_runner_protocol::wire::GitChanges;
use demi_shared_types::Nullable;
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;
use typed_path::Utf8TypedPath;

use crate::ids::DeviceId;
use crate::query::StrictBool;

/// One subscription a page asks of a conversation's file watch, under a
/// key of its own: a [`PageSubscription`], on the attached host `host`
/// names, or on the conversation's primary Host without one (`web-api.md`
/// § File watch).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct ConversationSubscription {
    #[garde(length(chars, min = 1, max = 1024))]
    pub key: String,
    #[garde(skip)]
    pub kind: SubscriptionKind,
    #[garde(length(chars, min = 1), custom(demi_command_protocol::without_nul))]
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "DeviceId")]
    #[garde(skip)]
    pub host: Option<DeviceId>,
}

impl ConversationSubscription {
    /// The host it names, and the subscription as that Host's watch takes it.
    pub fn into_parts(self) -> (Option<DeviceId>, PageSubscription) {
        let Self {
            key,
            kind,
            path,
            host,
        } = self;
        (host, PageSubscription { key, kind, path })
    }
}

impl From<FileWatchRequest> for ConversationWatchRequest {
    /// A watch of one Host's message, whose subscriptions name no host.
    fn from(request: FileWatchRequest) -> Self {
        match request {
            FileWatchRequest::Subscribe { subscriptions } => Self::Subscribe {
                subscriptions: subscriptions
                    .into_iter()
                    .map(|PageSubscription { key, kind, path }| ConversationSubscription {
                        key,
                        kind,
                        path,
                        host: None,
                    })
                    .collect(),
            },
            FileWatchRequest::Unsubscribe { keys } => Self::Unsubscribe { keys },
            FileWatchRequest::Renew { keys } => Self::Renew { keys },
        }
    }
}

/// A message the page sends on a conversation's file watch,
/// `WS /conversations/:id/fs/watch`: a [`FileWatchRequest`] whose
/// subscriptions may name an attached host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConversationWatchRequest {
    /// Subscriptions the page holds from now on.
    Subscribe {
        #[garde(length(max = MAX_PAGE_SUBSCRIPTIONS), dive)]
        subscriptions: Vec<ConversationSubscription>,
    },
    /// Subscriptions the page no longer holds, on whichever host.
    Unsubscribe {
        #[garde(length(max = MAX_PAGE_SUBSCRIPTIONS))]
        keys: Vec<String>,
    },
    /// Every subscription the page still holds, whose leases run again.
    Renew {
        #[garde(length(max = MAX_PAGE_SUBSCRIPTIONS))]
        keys: Vec<String>,
    },
}

/// `{ path, home, entries }`: a directory as a Host lists it. `home` is the
/// Host's home directory, null while it is unknown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Directory {
    pub path: String,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub home: Option<String>,
    pub entries: Vec<DirectoryEntry>,
}

/// `POST .../fs { path }`: a directory to make, with its parents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct CreateDirectory {
    #[garde(length(chars, min = 1))]
    pub path: String,
}

/// The most paths one existence question names.
pub const EXISTENCE_PATHS_MAX: usize = 200;

/// `POST .../fs/exists { paths }`: which of these paths of the
/// conversation's primary Host are regular files, as a message's inline
/// code names them (`file-previews.md` § Files named in messages).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct FileExistence {
    #[garde(length(min = 1, max = EXISTENCE_PATHS_MAX))]
    pub paths: Vec<AbsolutePath>,
}

/// The answer to a [`FileExistence`]: the paths asked about that are
/// regular files, in the order asked, a symbolic link followed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExistingFiles {
    pub files: Vec<String>,
}

/// `POST /devices/:id/fs { path }`: a directory to make on a device, named
/// the way the device names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct CreateDeviceDirectory {
    #[garde(skip)]
    pub path: AbsolutePath,
}

/// `{ path }`: the directory a create made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CreatedDirectory {
    pub path: String,
}

/// `GET .../fs/file`: a file's text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FileText {
    pub path: String,
    pub text: String,
}

/// `GET .../changes`: the runner's list of uncommitted changes, and the
/// directory its paths are relative to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkingTreeChanges {
    pub root: String,
    #[serde(flatten)]
    pub changes: GitChanges,
}

/// One changed file's two sides: `original` as the last commit or the edit
/// has it, empty for a new file; `modified` as the working tree or the edit
/// has it, empty for a deleted one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChangeSides {
    pub original: String,
    pub modified: String,
}

/// A path on a Host that names its root: `/…` on Unix, `C:\…` on Windows.
/// The Host decides what it names, so the meaning never depends on where a
/// runner stands.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "String")]
#[schemars(with = "String")]
pub struct AbsolutePath(String);

impl AbsolutePath {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for AbsolutePath {
    type Error = &'static str;

    fn try_from(path: String) -> Result<Self, &'static str> {
        if Utf8TypedPath::derive(&path).is_absolute() {
            Ok(Self(path))
        } else {
            Err("path must be an absolute path on the Host")
        }
    }
}

/// A path under a working tree's root: nonempty, not absolute, and without
/// a `..` segment, so it stays inside the root.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(try_from = "String")]
pub struct TreePath(String);

impl TreePath {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for TreePath {
    type Error = &'static str;

    fn try_from(path: String) -> Result<Self, &'static str> {
        let inside = !path.is_empty()
            && !path.starts_with('/')
            && !path.split('/').any(|segment| segment == "..");
        if inside {
            Ok(Self(path))
        } else {
            Err("path must be a relative path inside the working tree")
        }
    }
}

/// `?path=` naming a directory to list: the Host's starting directory when
/// omitted. Queries are the backend's alone and are not emitted.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct DirectoryQuery {
    #[serde(default)]
    pub path: Option<NonEmptyPath>,
}

/// `?path=` of a device listing: absolute on the device, its home when
/// omitted.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct DeviceDirectoryQuery {
    #[serde(default)]
    pub path: Option<AbsolutePath>,
}

/// `?path=` naming one file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FileQuery {
    pub path: NonEmptyPath,
}

/// `?path=` of a delete: absolute, so what it would take with it can be
/// told.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RemoveQuery {
    pub path: AbsolutePath,
}

/// `?path=&version=&download=` of the raw file route.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RawFileQuery {
    pub path: NonEmptyPath,
    /// The ETag the request expects the file to still have.
    #[serde(default)]
    pub version: Option<NonEmptyPath>,
    #[serde(default)]
    pub download: StrictBool,
}

/// `?path=&replace=` of an upload.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct UploadQuery {
    pub path: NonEmptyPath,
    #[serde(default)]
    pub replace: StrictBool,
}

/// `?directory=` of the working tree's changes list: the working tree's
/// root on the primary Host, the conversation's execution directory when
/// omitted (`web-api.md` § File text and working tree changes).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct ChangesQuery {
    #[serde(default)]
    pub directory: Option<AbsolutePath>,
}

/// `?path=&from=&directory=&commit=` of a working tree file: `path` is
/// relative to `directory`, the conversation's execution directory when
/// omitted; `from`, a renamed file's earlier path, whose content is the
/// old side; with `commit`, the file as that commit has it against its
/// first parent.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TreeFileQuery {
    pub path: TreePath,
    #[serde(default)]
    pub from: Option<TreePath>,
    #[serde(default)]
    pub directory: Option<AbsolutePath>,
    #[serde(default)]
    pub commit: Option<CommitName>,
}

/// `?path=&directory=&commit=&download=` of the committed side of a working
/// tree file: the last commit's, or `commit`'s.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CommittedFileQuery {
    pub path: TreePath,
    #[serde(default)]
    pub directory: Option<AbsolutePath>,
    #[serde(default)]
    pub commit: Option<CommitName>,
    #[serde(default)]
    pub download: StrictBool,
}

/// `?directory=&branch=&after=` of a page of a branch's history: HEAD's
/// without `branch`, from the newest without `after` (`web-api.md` § File
/// text and working tree changes).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct LogQuery {
    #[serde(default)]
    pub directory: Option<AbsolutePath>,
    #[serde(default)]
    pub branch: Option<NonEmptyPath>,
    #[serde(default)]
    pub after: Option<CommitName>,
}

/// `?directory=&commit=` of one commit.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CommitQuery {
    #[serde(default)]
    pub directory: Option<AbsolutePath>,
    pub commit: CommitName,
}

/// A commit's id or an abbreviation of it: 7 to 64 hex digits.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(try_from = "String")]
pub struct CommitName(String);

impl CommitName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CommitName {
    type Error = &'static str;

    fn try_from(name: String) -> Result<Self, &'static str> {
        if (7..=64).contains(&name.len()) && name.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            Ok(Self(name))
        } else {
            Err("commit must be 7 to 64 hex digits")
        }
    }
}

/// A nonempty query text, such as a path or an ETag.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(try_from = "String")]
pub struct NonEmptyPath(String);

impl NonEmptyPath {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for NonEmptyPath {
    type Error = &'static str;

    fn try_from(text: String) -> Result<Self, &'static str> {
        if text.is_empty() {
            Err("must not be empty")
        } else {
            Ok(Self(text))
        }
    }
}
