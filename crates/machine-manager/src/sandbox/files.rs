//! A boot's files: the runtime record in the working pair, the runtime
//! directory's layout, and the credential files the sandbox reads.
//!
//! The backend mints a managed-device token per boot and stores its hash. The
//! manager receives `{ backendUrl, deviceToken }` for that boot only, decodes
//! it with the runner protocol's managed-boot type, which refuses unknown
//! fields, requires the backend URL to equal its configured one, and supplies
//! the record as a private read-only file on a temporary mount.
//!
//! The token never appears in process arguments, image layers, persistent
//! working files, the OCI environment, logs or stored generations. Only the
//! non-secret path is an argument.
//!
//! Runtime metadata with sensitive paths stays in an owner-restricted temporary
//! directory. Cleanup removes it on a failed start and on a stop entry by
//! entry, so it never deletes the contents of a mount still attached beneath
//! it.
//!
//! A managed runner with a missing or invalid record fails and never enters
//! pairing. Each token permits one live connection and is scoped to its device,
//! and a rotation rejects the previous boot's.
//!
//! Sandbox root can inspect its own runner and credential: the boundary is
//! against other users and the infrastructure, not against the owner of that
//! Cloud.
//!
//! ```text
//! <data>/working/<device>/sandbox.json      {id, slot}: which boot, which network slot
//! /run/demi-machine-manager/<sandbox>/
//!     base/ system/ home/ rootfs/ credentials/   mount points
//!     config.json runtime.log                    the OCI bundle and runsc's output
//! ```

use std::{
    io::{self, Write},
    net::Ipv4Addr,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use fs_err::os::unix::fs::OpenOptionsExt;

use demi_machine_manager_protocol::Volume;
use demi_runner_protocol::boot::ManagedBoot;
use serde::{Deserialize, Serialize};

use crate::blocking::OffLoop;

/// The UID and GID of the sandbox's user, `demi`.
pub const USER_ID: u32 = 1000;

/// One boot's id, `demi-` and a UUID: it names the runtime directory, the
/// runsc container and the cgroup.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SandboxId(String);

#[derive(Debug, thiserror::Error)]
#[error("invalid sandbox id: {0:?}")]
pub struct InvalidSandboxId(String);

impl SandboxId {
    pub fn new() -> Self {
        Self(format!("demi-{}", uuid::Uuid::new_v4()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for SandboxId {
    fn default() -> Self {
        Self::new()
    }
}

impl TryFrom<String> for SandboxId {
    type Error = InvalidSandboxId;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let valid = value
            .strip_prefix("demi-")
            .is_some_and(demi_machine_manager_protocol::is_image_name);
        if !valid {
            return Err(InvalidSandboxId(value));
        }
        Ok(Self(value))
    }
}

impl From<SandboxId> for String {
    fn from(id: SandboxId) -> Self {
        id.0
    }
}

impl std::fmt::Display for SandboxId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// `sandbox.json` in the working pair: written before any resource of the
/// boot exists and removed after the last is released, so its presence
/// means recovery must fence the boot. It holds no credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxRecord {
    pub id: SandboxId,
    pub slot: u16,
}

/// A boot's runtime directory, `/run/demi-machine-manager/<sandbox>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeDirectory {
    root: PathBuf,
}

/// The mount points in the order they are released: the overlay first, the
/// filesystems it stacks on after it.
pub const MOUNT_POINTS: [&str; 6] = [
    "rootfs",
    "home",
    "system",
    "base",
    "programs",
    "credentials",
];

impl RuntimeDirectory {
    pub fn new(runtime: &Path, sandbox: &str) -> Self {
        Self {
            root: runtime.join(sandbox),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The read-only bind of the base's root.
    pub fn base(&self) -> PathBuf {
        self.root.join("base")
    }

    /// The read-only bind of the configured base's `/opt/demi`, which the
    /// sandbox sees in place of its own base's.
    pub fn programs(&self) -> PathBuf {
        self.root.join("programs")
    }

    /// Where a working image is mounted.
    pub fn volume(&self, volume: Volume) -> PathBuf {
        self.root.join(volume.to_string())
    }

    pub fn home(&self) -> PathBuf {
        self.volume(Volume::Home)
    }

    /// The overlay the sandbox sees as `/`.
    pub fn rootfs(&self) -> PathBuf {
        self.root.join("rootfs")
    }

    /// The private tmpfs of the boot's credential files.
    pub fn credentials(&self) -> PathBuf {
        self.root.join("credentials")
    }

    pub fn boot(&self) -> PathBuf {
        self.credentials().join("boot.json")
    }

    pub fn resolver(&self) -> PathBuf {
        self.credentials().join("resolv.conf")
    }

    pub fn hosts(&self) -> PathBuf {
        self.credentials().join("hosts")
    }

    /// The OCI bundle's configuration.
    pub fn config(&self) -> PathBuf {
        self.root.join("config.json")
    }

    /// runsc's own output when it starts the sandbox.
    pub fn log(&self) -> PathBuf {
        self.root.join("runtime.log")
    }

    /// Creates the directory, private to root, and its mount points.
    pub fn create(&self, _: &OffLoop) -> io::Result<()> {
        fs_err::create_dir(&self.root)?;
        fs_err::set_permissions(&self.root, std::fs::Permissions::from_mode(0o700))?;
        for name in MOUNT_POINTS {
            fs_err::create_dir(self.root.join(name))?;
        }
        Ok(())
    }

    /// Removes the directory entry by entry, so a mount that survived keeps
    /// its contents: removing its mount point fails instead. A directory
    /// that is gone already is fine.
    pub fn remove(&self, _: &OffLoop) -> io::Result<()> {
        let entries = match fs_err::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                fs_err::remove_dir(entry.path())?;
            } else {
                fs_err::remove_file(entry.path())?;
            }
        }
        fs_err::remove_dir(&self.root)
    }

    /// Writes the credential files on the mounted credentials tmpfs, each
    /// with its mode set whatever the service's umask: the boot record
    /// readable by the sandbox's user alone, the resolver and hosts files by
    /// everyone.
    pub fn write_credentials(
        &self,
        _: &OffLoop,
        boot: &ManagedBoot,
        dns: &[Ipv4Addr],
    ) -> io::Result<()> {
        let resolver: String = dns
            .iter()
            .map(|address| format!("nameserver {address}\n"))
            .collect();
        create(&self.resolver(), resolver.as_bytes(), 0o444)?;
        create(
            &self.hosts(),
            b"127.0.0.1 localhost\n127.0.1.1 demi-cloud\n",
            0o444,
        )?;
        create(&self.boot(), &serde_json::to_vec(boot)?, 0o400)?;
        fs_err::os::unix::fs::chown(self.boot(), Some(USER_ID), Some(USER_ID))
    }
}

/// A new file with `bytes` and exactly `mode`.
fn create(path: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
    let mut file = fs_err::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.set_permissions(std::fs::Permissions::from_mode(mode))
}
