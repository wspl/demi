//! The newest Codex CLI release (`models.md` § The Codex catalog): the npm
//! registry's `latest` version of `@openai/codex`, as the CLI's own update
//! check finds it, which the catalog request names. One serves the whole
//! backend, whatever the number of Codex entries and accounts; it is kept by
//! provider-common's [`KeptRelease`].

use std::{fmt, sync::Arc};

use demi_provider_common::{
    release::{KeptRelease, ReleaseSource},
    wire::decode_untagged,
};
use demi_shared_types::Clock;
use reqwest::Url;
use semver::Version;
use serde::Deserialize;

/// The version of the newest Codex CLI release. A clone shares what its
/// original reads.
#[derive(Clone)]
pub struct CodexReleases(KeptRelease);

impl fmt::Debug for CodexReleases {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("CodexReleases").field(&self.0).finish()
    }
}

impl CodexReleases {
    /// The npm registry's record of `@openai/codex`'s newest release.
    pub const URL: &str = "https://registry.npmjs.org/@openai/codex/latest";

    /// The releases the record at `url` names, read with `http` and believed
    /// for six hours by `clock`.
    pub fn new(url: Url, http: reqwest::Client, clock: Arc<dyn Clock>) -> Self {
        let source = ReleaseSource {
            name: "Codex CLI",
            url,
            accept: "application/json",
            // The Codex CLI release built into the provider.
            built_in: "0.162.1",
            version: latest,
        };
        Self(KeptRelease::new(source, http, clock))
    }

    /// The version the catalog asks with now: the last one read, or the
    /// built-in one before a read succeeds. It starts no read.
    pub fn version(&self) -> String {
        self.0.version()
    }

    /// [`version`](Self::version), at once, reading the registry again in
    /// the background when the version is stale.
    pub(crate) fn current(&self) -> String {
        self.0.current()
    }
}

/// The `latest` version the registry's record names, which is what the CLI
/// sends.
fn latest(text: &str) -> Result<Version, String> {
    decode_untagged::<LatestRelease>(text)
        .map(|latest| latest.version)
        .map_err(|error| format!("its answer cannot be read: {error}"))
}

/// The registry's record of a package's `latest` version; only the field
/// Demi reads.
#[derive(Deserialize)]
struct LatestRelease {
    version: Version,
}
