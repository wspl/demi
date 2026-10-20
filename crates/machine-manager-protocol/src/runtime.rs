//! The gVisor runtime a release's machine manager runs: no release carries
//! it. The manager is built against the one runsc version
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

/// The runtime release the manager is built with (`runtime/release.json`):
/// upstream's release, the same version on both architectures.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeRelease {
    /// Upstream's release, such as `20261005.0`.
    pub upstream: String,
    pub amd64_archive_sha512: String,
    pub arm64_archive_sha512: String,
}

impl RuntimeRelease {
    const MANIFEST: &str = include_str!("../../machine-manager/runtime/release.json");

    pub fn pinned() -> Self {
        serde_json::from_str(Self::MANIFEST).expect("the pinned runtime release is valid")
    }

    /// The version runsc reports.
    pub fn version(&self) -> String {
        format!("release-{}", self.upstream)
    }

    /// Upstream's distribution for this architecture.
    pub fn archive_url(&self) -> String {
        let architecture = if cfg!(target_arch = "aarch64") {
            "aarch64"
        } else {
            "x86_64"
        };
        format!(
            "https://storage.googleapis.com/gvisor/releases/release/{}/{architecture}/gvisor.tar.bz2",
            self.upstream
        )
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
