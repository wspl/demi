//! A boot's files: the runtime record in the working pair, the runtime
//! directory's layout, and the credential files the sandbox reads.
//!
//! ```text
//! <data>/working/<device>/sandbox.json      {id, slot}: which boot, which network slot
//! /run/demi-machines/<sandbox>/
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

use demi_machines_protocol::Volume;
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

impl TryFrom<String> for SandboxId {
    type Error = InvalidSandboxId;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let valid = value.strip_prefix("demi-").is_some_and(demi_machines_protocol::is_image_name);
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

/// A boot's runtime directory, `/run/demi-machines/<sandbox>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeDirectory {
    root: PathBuf,
}

/// The mount points in the order they are released: the overlay first, the
/// filesystems it stacks on after it.
pub const MOUNT_POINTS: [&str; 5] = ["rootfs", "home", "system", "base", "credentials"];

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
    pub fn write_credentials(&self, _: &OffLoop, boot: &ManagedBoot, dns: &[Ipv4Addr]) -> io::Result<()> {
        let resolver: String = dns.iter().map(|address| format!("nameserver {address}\n")).collect();
        create(&self.resolver(), resolver.as_bytes(), 0o444)?;
        create(&self.hosts(), b"127.0.0.1 localhost\n127.0.1.1 demi-cloud\n", 0o444)?;
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

#[cfg(test)]
mod tests {
    use std::os::unix::fs::MetadataExt;

    use super::*;

    #[test]
    fn a_record_names_one_boot_and_its_slot() {
        let record: SandboxRecord =
            serde_json::from_str(r#"{"id":"demi-0f6c3d4e-8a9b-4c1d-9e2f-3a4b5c6d7e8f","slot":3}"#).unwrap();
        assert_eq!(record.slot, 3);
        for invalid in [
            r#"{"id":"0f6c3d4e","slot":3}"#,
            r#"{"id":"demi-../x","slot":3}"#,
            r#"{"id":"demi-a","slot":-1}"#,
            r#"{"id":"demi-a","slot":3,"token":"x"}"#,
        ] {
            assert!(serde_json::from_str::<SandboxRecord>(invalid).is_err(), "{invalid}");
        }
        assert!(SandboxId::new().as_str().starts_with("demi-"));
    }

    #[test]
    fn credential_modes_do_not_depend_on_the_umask() {
        let off = OffLoop::in_test();
        let temporary = tempfile::tempdir().unwrap();
        let directory = RuntimeDirectory::new(temporary.path(), "demi-test");
        directory.create(&off).unwrap();
        let boot = ManagedBoot::decode(br#"{"backendUrl":"https://backend.example.com","deviceToken":"tok"}"#).unwrap();
        // Giving the record to the sandbox's user needs root, and comes last.
        let written = directory.write_credentials(&off, &boot, &[Ipv4Addr::new(1, 1, 1, 1)]);
        if let Err(error) = &written {
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
        }
        let metadata = |path: PathBuf| std::fs::metadata(path).unwrap();
        assert_eq!(metadata(directory.boot()).mode() & 0o777, 0o400);
        assert_eq!(metadata(directory.resolver()).mode() & 0o777, 0o444);
        assert_eq!(metadata(directory.hosts()).mode() & 0o777, 0o444);
        if written.is_ok() {
            assert_eq!(metadata(directory.boot()).uid(), USER_ID);
        }
        assert_eq!(std::fs::read_to_string(directory.resolver()).unwrap(), "nameserver 1.1.1.1\n");
        assert_eq!(ManagedBoot::decode(&std::fs::read(directory.boot()).unwrap()).unwrap(), boot);
    }

    #[test]
    fn removal_never_deletes_what_a_mount_point_still_holds() {
        let off = OffLoop::in_test();
        let temporary = tempfile::tempdir().unwrap();
        let directory = RuntimeDirectory::new(temporary.path(), "demi-test");
        directory.create(&off).unwrap();
        std::fs::write(directory.config(), "{}").unwrap();
        // A file inside a mount point stands for a mount that survived.
        std::fs::write(directory.home().join("project"), "work").unwrap();
        assert!(directory.remove(&off).is_err());
        assert_eq!(std::fs::read_to_string(directory.home().join("project")).unwrap(), "work");
        std::fs::remove_file(directory.home().join("project")).unwrap();
        directory.remove(&off).unwrap();
        assert!(!directory.root().exists());
        directory.remove(&off).unwrap();
    }
}
