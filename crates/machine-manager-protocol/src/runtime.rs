//! The gVisor runtime a release's machine manager runs
//! (`builds-and-releases.md` § gVisor runtime): the one runsc version
//! `crates/machine-manager/runtime/release.json` pins, which the manager
//! requires and `demi-server` fetches onto a server, and where a server keeps
//! it.

use std::path::PathBuf;

use serde::Deserialize;

/// The directory of a server's gVisor versions, one directory each, named by
/// the version runsc reports.
pub const GVISOR_PATH: &str = "/opt/demi/gvisor";

/// The programs of a gVisor distribution the manager runs: runsc and the
/// sidecar programs runsc starts from `gvisor-bin/` beside it.
pub const PROGRAMS: [&str; 4] = [
    "runsc",
    "gvisor-bin/gvisor_sentry",
    "gvisor-bin/gvisor-sentry-prewarmer",
    "gvisor-bin/runsc-fd-parking",
];

/// The runtime release the manager is built with (`runtime/release.json`).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeRelease {
    pub upstream: String,
    pub commit: String,
    pub arm64_version: String,
    pub arm64_patch_sha256: String,
    /// The SHA-512 of the arm64 distribution the Runtime workflow published.
    pub arm64_archive_sha512: String,
    pub amd64_archive_sha512: String,
    pub bazel: String,
    pub bazel_arm64_sha256: String,
}

impl RuntimeRelease {
    const MANIFEST: &str = include_str!("../../machine-manager/runtime/release.json");

    pub fn pinned() -> Self {
        serde_json::from_str(Self::MANIFEST).expect("the pinned runtime release is valid")
    }

    /// The version runsc reports on this architecture: arm64 runs the build
    /// with the seccomp trap fix, amd64 the upstream release.
    pub fn version(&self) -> String {
        if cfg!(target_arch = "aarch64") {
            self.arm64_version.clone()
        } else {
            format!("release-{}", self.upstream)
        }
    }

    /// The distribution of this architecture: upstream's release on amd64,
    /// the Runtime workflow's build on arm64.
    pub fn archive_url(&self) -> String {
        if cfg!(target_arch = "aarch64") {
            format!(
                "https://github.com/wspl/demi/releases/download/runsc-{}/gvisor-arm64.tar.bz2",
                self.arm64_version
            )
        } else {
            format!(
                "https://storage.googleapis.com/gvisor/releases/release/{}/x86_64/gvisor.tar.bz2",
                self.upstream
            )
        }
    }

    /// The SHA-512 of `archive_url`'s archive, in lowercase hex.
    pub fn archive_sha512(&self) -> &str {
        if cfg!(target_arch = "aarch64") {
            &self.arm64_archive_sha512
        } else {
            &self.amd64_archive_sha512
        }
    }

    /// The directory of this version on a server.
    pub fn directory(&self) -> PathBuf {
        PathBuf::from(GVISOR_PATH).join(self.version())
    }
}
