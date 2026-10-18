//! The newest stable Grok CLI release (`providers.md` § Endpoints): the
//! plain-text pointer `https://x.ai/cli/stable`, as Grok's own install script
//! finds it, which every request names in `x-grok-client-version`. One
//! serves the whole backend, whatever the number of Grok Build entries and
//! accounts; it is kept by provider-common's [`KeptRelease`], as Codex's is.

use std::{fmt, sync::Arc};

use demi_provider_common::release::{KeptRelease, ReleaseSource};
use demi_shared_types::Clock;
use http::HeaderValue;
use reqwest::Url;
use semver::Version;

/// The version of the newest stable Grok CLI release. A clone shares what
/// its original reads.
#[derive(Clone)]
pub struct GrokReleases(KeptRelease);

impl fmt::Debug for GrokReleases {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("GrokReleases").field(&self.0).finish()
    }
}

impl GrokReleases {
    /// The pointer Grok's install script reads.
    pub const URL: &str = "https://x.ai/cli/stable";

    /// The releases the pointer at `url` names, read with `http` and
    /// believed for six hours by `clock`.
    pub fn new(url: Url, http: reqwest::Client, clock: Arc<dyn Clock>) -> Self {
        let source = ReleaseSource {
            name: "Grok CLI",
            url,
            accept: "text/plain",
            // The Grok CLI release built into the provider.
            built_in: "1.0.62",
            version: first_line,
        };
        Self(KeptRelease::new(source, http, clock))
    }

    /// The version a request names now, as header text, reading the pointer
    /// again in the background when the version is stale.
    pub(crate) fn current(&self) -> HeaderValue {
        HeaderValue::from_str(&self.0.current()).expect("a version is header text")
    }
}

/// The version the pointer's first line names.
fn first_line(text: &str) -> Result<Version, String> {
    let line = text.lines().next().unwrap_or_default();
    line.parse()
        .map_err(|error| format!("its answer {line:?} is no version: {error}"))
}
