//! Files on a Host: directory listings, file text, and a conversation's
//! working tree (`web-api.md` § Device files and remote references, § File
//! text and working tree changes).

use demi_core::{MAX_SAFE_INTEGER, Nullable, Timestamp};
use demi_runner_protocol::wire::GitChanges;
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use typed_path::Utf8TypedPath;

use crate::query::StrictBool;

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

/// One entry of a listing. An entry that disappears while the directory is
/// listed is left out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryEntry {
    pub name: String,
    pub is_directory: bool,
    pub is_symbolic_link: bool,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub size: u64,
    pub modified_at: Timestamp,
}

/// `POST .../fs { path }`: a directory to make, with its parents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct CreateDirectory {
    #[garde(length(chars, min = 1))]
    pub path: String,
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
        let inside = !path.is_empty() && !path.starts_with('/') && !path.split('/').any(|segment| segment == "..");
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

/// `?path=` of a working tree file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TreeFileQuery {
    pub path: TreePath,
}

/// `?path=&download=` of the committed side of a working tree file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CommittedFileQuery {
    pub path: TreePath,
    #[serde(default)]
    pub download: StrictBool,
}

/// `?path=&edit=` of one retained edit segment of a command.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct EditQuery {
    pub path: NonEmptyPath,
    pub edit: u32,
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
