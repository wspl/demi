//! The newest release of a vendor's CLI, which a provider names in its
//! requests as that CLI does, since the vendor's service decides by it what
//! it serves (`models.md` § The Codex catalog, `providers.md` § Endpoints):
//! read from where the CLI's own update check reads it, kept six hours and
//! read again in the background by [`KeptRead`], so no request waits for it,
//! with the version built into the provider answering until the first read
//! succeeds. One serves the whole backend for its vendor.

use std::{fmt, sync::Arc, time::Duration};

use demi_shared_types::Clock;
use http::header::ACCEPT;
use jiff::SignedDuration;
use reqwest::Url;
use semver::Version;
use tokio::runtime::Handle;

use crate::kept::KeptRead;

/// How long the newest release is believed before it is read again.
const LIFETIME: SignedDuration = SignedDuration::from_hours(6);

/// How long one read may take.
const READ_TIMEOUT: Duration = Duration::from_secs(15);

/// Where a CLI's newest release is read and how its answer names it.
pub struct ReleaseSource {
    /// The CLI's name, for the logs, such as `Codex CLI`.
    pub name: &'static str,
    /// The address the CLI's update check reads.
    pub url: Url,
    /// The media type the answer is asked in.
    pub accept: &'static str,
    /// The release built into the provider, a bare `MAJOR.MINOR.PATCH`.
    pub built_in: &'static str,
    /// The version the answer's text names, or why it names none.
    pub version: fn(&str) -> Result<Version, String>,
}

/// The version of a CLI's newest release. A clone shares what its original
/// reads.
#[derive(Clone)]
pub struct KeptRelease(Arc<Kept>);

struct Kept {
    source: ReleaseSource,
    http: reqwest::Client,
    newest: KeptRead<Version>,
}

impl fmt::Debug for KeptRelease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KeptRelease")
            .field("name", &self.0.source.name)
            .field("url", &self.0.source.url.as_str())
            .finish_non_exhaustive()
    }
}

impl KeptRelease {
    /// The release `source` names, read with `http` and believed for six
    /// hours by `clock`.
    pub fn new(source: ReleaseSource, http: reqwest::Client, clock: Arc<dyn Clock>) -> Self {
        Self(Arc::new(Kept {
            source,
            http,
            newest: KeptRead::new(LIFETIME, clock),
        }))
    }

    /// The release built into `source`, believed as read now by `clock`, so
    /// that no read starts while `clock` stays within six hours: a test
    /// whose requests name the built-in release opens no socket for it,
    /// whose answer would differ by system and, on a paused clock, let the
    /// read's timeout move the time.
    #[cfg(feature = "testing")]
    pub fn built_in(source: ReleaseSource, clock: Arc<dyn Clock>) -> Self {
        let version = source.built_in.parse().expect("the built-in release is a version");
        Self(Arc::new(Kept {
            source,
            http: reqwest::Client::new(),
            newest: KeptRead::read_now(LIFETIME, clock, version),
        }))
    }

    /// The version a request names now, the last one read or the built-in
    /// one before a read succeeds. It starts no read.
    pub fn version(&self) -> String {
        self.named(self.0.newest.last())
    }

    /// [`version`](Self::version), at once. When the version is older than
    /// six hours or missing, it is read in the background for a later
    /// request, on the runtime of the caller.
    pub fn current(&self) -> String {
        let kept = self.0.clone();
        // The read ends within `READ_TIMEOUT`. A failed one keeps the version
        // read before, and a later request reads again.
        let read_again = async move {
            let name = kept.source.name;
            match kept.newest.read(false, || kept.newest_release()).await {
                Ok(version) => tracing::debug!("the newest {name} release is {version}"),
                Err(reason) => tracing::warn!("the newest {name} release was not read: {reason}"),
            }
        };
        let kept = self.0.newest.kept(&Handle::current(), read_again);
        self.named(kept.map(|kept| kept.value))
    }

    fn named(&self, read: Option<Version>) -> String {
        read.map_or_else(|| self.0.source.built_in.to_owned(), |version| version.to_string())
    }
}

impl Kept {
    /// The newest release the source names; a prerelease or a build is no
    /// answer, as the CLI itself names neither.
    async fn newest_release(&self) -> Result<Version, String> {
        let response = self
            .http
            .get(self.source.url.clone())
            .header(ACCEPT, self.source.accept)
            .timeout(READ_TIMEOUT)
            .send()
            .await
            .map_err(|error| format!("its source did not answer ({})", error.without_url()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("its source answered {status}"));
        }
        let text = response
            .text()
            .await
            .map_err(|error| format!("its source did not answer ({})", error.without_url()))?;
        let version = (self.source.version)(&text)?;
        if !version.pre.is_empty() || !version.build.is_empty() {
            return Err(format!("it names version {version}, which is not a release"));
        }
        Ok(version)
    }
}
