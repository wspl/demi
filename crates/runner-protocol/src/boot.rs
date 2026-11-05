//! The private, temporary credential file supplied to a managed runner
//! (`crates/machine-manager/src/sandbox/files.rs`).

use serde::{Deserialize, Serialize};

use crate::values::{BackendUrl, DeviceToken};

/// Where a managed runner reaches its backend: the backend's runner socket,
/// whose directory the machine manager mounts into the sandbox
/// (`crates/backend-http/src/lib.rs`). The record's URL still names the
/// backend's origin.
pub const BACKEND_SOCKET: &str = "/run/demi-backend/runners.sock";

/// A managed runner's temporary directory, on the system image and private
/// to `demi`: the runner's environment names it as `TMPDIR`, and the runner
/// empties it when it starts, once per boot
/// (`crates/machine-manager/src/storage/mod.rs`).
pub const TEMPORARY_DIRECTORY: &str = "/var/lib/demi/tmp";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedBoot {
    pub backend_url: BackendUrl,
    pub device_token: DeviceToken,
    /// The abstract Unix socket the runner reaches its backend on, in place
    /// of [`BACKEND_SOCKET`]: a Simple Isolation Cloud's, which has no file
    /// the backend's socket could be bound to without the Cloud being able
    /// to remove it. The machine manager names it ([`abstract_socket`]); the
    /// backend never does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abstract_socket: Option<String>,
}

/// The name of the abstract Unix socket a backend whose runner socket is the
/// file `file` listens on beside it: the file's path, so two backends on one
/// host have two names.
pub fn abstract_socket(file: &std::path::Path) -> String {
    format!("demi-runners:{}", file.display())
}

/// The path that connects to the abstract socket `name`, which tokio reads
/// as one when it starts with a NUL byte.
#[cfg(target_os = "linux")]
pub fn abstract_socket_path(name: &str) -> std::path::PathBuf {
    use std::os::unix::ffi::OsStringExt as _;
    let mut bytes = vec![0];
    bytes.extend_from_slice(name.as_bytes());
    std::ffi::OsString::from_vec(bytes).into()
}

/// A boot file that is not a valid boot record.
#[derive(Debug, thiserror::Error)]
#[error("invalid managed boot file: {0}")]
pub struct BootError(#[from] serde_json::Error);

impl ManagedBoot {
    /// Decodes a boot file's JSON; its values are checked as they are read.
    pub fn decode(bytes: &[u8]) -> Result<Self, BootError> {
        Ok(serde_json::from_slice(bytes)?)
    }
}
