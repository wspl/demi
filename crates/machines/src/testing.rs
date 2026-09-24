//! Fixtures for the tests that import a Cloud image (`images.md` § Release
//! artifacts): a release directory with its archive and manifest. The
//! crate's unit tests use them, and so do the manager's process tests
//! (`tests/manager`) through the `testing` feature.

use std::{io::Write, path::Path};

use demi_machines_protocol::image::Architecture;
use serde_json::json;
use sha2::Digest as _;

/// A release directory whose archive holds the given entries and whose
/// manifest lists the given executables. It is removed when dropped.
pub struct CloudImage {
    directory: tempfile::TempDir,
}

/// An archive entry: a regular file's contents, or a symbolic link's target.
pub enum Entry {
    File(&'static [u8]),
    Link(&'static str),
}

/// The runner every image embeds, with its contents.
pub const RUNNER: (&str, &[u8]) = ("/usr/bin/demi-runner", b"runner");
/// The init every image embeds, with its contents.
pub const TINI: (&str, &[u8]) = ("/usr/bin/tini", b"tini");

/// The entries of a small root: the runner, tini as init and a skeleton
/// profile.
pub fn entries() -> Vec<(&'static str, Entry)> {
    vec![
        ("usr/bin/demi-runner", Entry::File(RUNNER.1)),
        ("usr/bin/tini", Entry::File(TINI.1)),
        ("usr/sbin/init", Entry::Link("../bin/tini")),
        ("etc/skel/.profile", Entry::File(b"export EDITOR=vi\n")),
    ]
}

/// The SHA-256 of `bytes` in hexadecimal, as manifests write it.
pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", sha2::Sha256::digest(bytes))
}

impl CloudImage {
    pub fn new(entries: &[(&str, Entry)], executables: &[(&str, &[u8])], architecture: Architecture) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let archive = directory.path().join("rootfs.tar.zst");
        let encoder = zstd::stream::write::Encoder::new(std::fs::File::create(&archive).unwrap(), 0)
            .unwrap()
            .auto_finish();
        let mut builder = tar::Builder::new(encoder);
        for (path, entry) in entries {
            let mut header = tar::Header::new_gnu();
            match entry {
                Entry::File(bytes) => {
                    header.set_entry_type(tar::EntryType::Regular);
                    header.set_mode(0o755);
                    header.set_size(bytes.len() as u64);
                    header.set_cksum();
                    builder.append_data(&mut header, path, *bytes).unwrap();
                }
                Entry::Link(target) => {
                    header.set_entry_type(tar::EntryType::Symlink);
                    header.set_size(0);
                    builder.append_link(&mut header, path, target).unwrap();
                }
            }
        }
        builder.into_inner().unwrap().flush().unwrap();
        let bytes = std::fs::read(&archive).unwrap();
        let runner = json!({ "sha256": digest(RUNNER.1), "size": RUNNER.1.len() });
        let executables: serde_json::Map<_, _> = executables
            .iter()
            .map(|(path, bytes)| ((*path).to_owned(), json!({ "sha256": digest(bytes), "size": bytes.len() })))
            .collect();
        let manifest = json!({
            "formatVersion": 1,
            "os": "linux",
            "architecture": architecture,
            "rootfs": { "sha256": digest(&bytes), "size": bytes.len(), "file": "rootfs.tar.zst" },
            "ubuntu": "26.04",
            "packages": [],
            "executables": executables,
            "releases": [],
            "runner": {
                "release": "f".repeat(64),
                "wire": 18,
                "commandProtocol": 1,
                "targets": { architecture.target(): runner },
            },
            "tools": [],
        });
        std::fs::write(directory.path().join("manifest.json"), serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
        Self { directory }
    }

    pub fn path(&self) -> &Path {
        self.directory.path()
    }

    /// Changes the manifest, for a release that fails a check.
    pub fn edit(&self, change: impl FnOnce(&mut serde_json::Value)) {
        let path = self.path().join("manifest.json");
        let mut manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        change(&mut manifest);
        std::fs::write(path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    }
}
