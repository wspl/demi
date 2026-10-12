//! The newest Codex CLI release (`models.md` § The Codex catalog): the npm
//! registry's `latest` version of `@openai/codex`, as the CLI's own update
//! check finds it, which the catalog request names. One serves the whole
//! backend, whatever the number of Codex entries and accounts.

use std::{fmt, sync::Arc, time::Duration};

use demi_provider_common::{kept::KeptRead, wire::decode_untagged};
use demi_shared_types::Clock;
use http::header::ACCEPT;
use jiff::SignedDuration;
use reqwest::Url;
use semver::Version;
use serde::Deserialize;
use tokio::runtime::Handle;

/// The Codex CLI release built into the provider, which the catalog sends
/// until the newest one is first read.
const BUILT_IN: &str = "0.162.1";

/// How long the newest release is believed before the registry is read
/// again.
const LIFETIME: SignedDuration = SignedDuration::from_hours(6);

/// How long one read of the registry may take.
const READ_TIMEOUT: Duration = Duration::from_secs(15);

/// The version of the newest Codex CLI release, kept six hours and read
/// again in the background, so that no catalog request waits for it. A
/// clone shares what its original reads.
#[derive(Clone)]
pub struct CodexReleases(Arc<Registry>);

struct Registry {
    /// The registry's `latest` record of `@openai/codex`.
    url: Url,
    http: reqwest::Client,
    newest: KeptRead<Version>,
}

impl fmt::Debug for CodexReleases {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CodexReleases")
            .field("url", &self.0.url.as_str())
            .finish_non_exhaustive()
    }
}

impl CodexReleases {
    /// The npm registry's record of `@openai/codex`'s newest release.
    pub const URL: &str = "https://registry.npmjs.org/@openai/codex/latest";

    /// The releases the record at `url` names, read with `http` and believed
    /// for six hours by `clock`.
    pub fn new(url: Url, http: reqwest::Client, clock: Arc<dyn Clock>) -> Self {
        Self(Arc::new(Registry {
            url,
            http,
            newest: KeptRead::new(LIFETIME, clock),
        }))
    }

    /// The version the catalog asks with now: the last one read, or the
    /// built-in one before a read succeeds. It starts no read.
    pub fn version(&self) -> String {
        named(self.0.newest.last())
    }

    /// [`version`](Self::version), at once. When the version is older than
    /// six hours or missing, the registry is read in the background for a
    /// later request, on the runtime of the caller.
    pub(crate) fn current(&self) -> String {
        let registry = self.0.clone();
        // The read ends within `READ_TIMEOUT`. A failed one keeps the version
        // read before, and a later request reads again.
        let read_again = async move {
            match registry.newest.read(false, || registry.latest()).await {
                Ok(version) => tracing::debug!("the newest Codex CLI release is {version}"),
                Err(reason) => {
                    tracing::warn!("the newest Codex CLI release was not read: {reason}");
                }
            }
        };
        let kept = self.0.newest.kept(&Handle::current(), read_again);
        named(kept.map(|kept| kept.value))
    }
}

/// The version a request names when `read` is the last one read.
fn named(read: Option<Version>) -> String {
    read.map_or_else(|| BUILT_IN.to_owned(), |version| version.to_string())
}

impl Registry {
    /// The `latest` version the registry names: a bare `MAJOR.MINOR.PATCH`,
    /// which is what the CLI sends; a prerelease is no answer.
    async fn latest(&self) -> Result<Version, String> {
        let response = self
            .http
            .get(self.url.clone())
            .header(ACCEPT, "application/json")
            .timeout(READ_TIMEOUT)
            .send()
            .await
            .map_err(|error| format!("the registry did not answer ({})", error.without_url()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("the registry answered {status}"));
        }
        let text = response
            .text()
            .await
            .map_err(|error| format!("the registry did not answer ({})", error.without_url()))?;
        let latest: LatestRelease =
            decode_untagged(&text).map_err(|error| format!("its answer cannot be read: {error}"))?;
        let version = latest.version;
        if !version.pre.is_empty() || !version.build.is_empty() {
            return Err(format!("it names version {version}, which is not a release"));
        }
        Ok(version)
    }
}

/// The registry's record of a package's `latest` version; only the field
/// Demi reads.
#[derive(Deserialize)]
struct LatestRelease {
    version: Version,
}
