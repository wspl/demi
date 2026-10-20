//! The vendor's Claude Code releases: the newest version, read from the
//! official distribution's `latest` pointer and that version's manifest, and
//! believed for six hours.
//!
//! - Concurrent readers share one read, and a version's manifest is read once.
//! - A redirect is refused.
//! - The reads run on the edge's runtime, whoever asks.
//! - A start that has a version to run never waits for one, and reads in the
//!   background instead.
//!
//! Demi always follows the vendor's newest release. Provider entries store no
//! CLI version and expose no version-selection or update-disable setting. Demi
//! keeps no list of versions it was tested with, because such a list would be
//! out of date with each release and a version it does not name would still
//! run. The Claude Code scenario suite checks whichever CLI it is given against
//! what the product relies on.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use demi_command_package_claude_code_protocol::{Artifact, PACKAGE, Release, is_version};
use demi_provider_common::Provider;
use demi_provider_common::kept::KeptRead;
use demi_shared_types::Clock;
use jiff::SignedDuration;
use serde::Deserialize;
use tokio::runtime::Handle;
use url::Url;

/// The command package that installs the CLI `provider`'s requests start on
/// the Claude Code machine: Claude Code's, the one provider whose requests are a
/// process (`crates/command-package-claude-code/src/lib.rs`); none for an HTTP provider.
pub fn cli_package(provider: &dyn Provider) -> Option<String> {
    provider
        .capabilities()
        .process_host
        .then(|| PACKAGE.to_owned())
}

/// The vendor's official distribution.
pub const DEFAULT_RELEASES_URL: &str = "https://downloads.claude.ai/claude-code-releases";

/// How long the newest version is believed before the pointer is read again.
const LATEST_TTL: SignedDuration = SignedDuration::from_hours(6);

/// How long one read of the distribution may take.
const READ_TIMEOUT: Duration = Duration::from_secs(15);

/// Why the newest release could not be read, in words that complete
/// "Claude Code could not be installed: …".
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ReleaseError(String);

/// A version's `manifest.json`: each platform's executable name, SHA-256
/// and byte size. Fields Demi does not read are ignored.
#[derive(Deserialize)]
struct Manifest {
    version: String,
    platforms: BTreeMap<String, ManifestPlatform>,
}

#[derive(Deserialize)]
struct ManifestPlatform {
    binary: String,
    checksum: String,
    size: u64,
}

/// The vendor's releases.
pub struct ClaudeReleases(Arc<Distribution>);

/// The distribution and what the backend has read of it.
struct Distribution {
    /// The distribution's address, without a trailing slash.
    base: String,
    /// A client that follows no redirect and gives up after `READ_TIMEOUT`.
    http: reqwest::Client,
    edge: Handle,
    /// Each version's release, which never changes once published. A `std`
    /// mutex: no section awaits, and `newest` reads one at a time.
    releases: Mutex<HashMap<String, Release>>,
    /// The newest release, which a start reads without waiting for a read
    /// under way.
    newest: KeptRead<Release>,
}

impl ClaudeReleases {
    /// The releases of the distribution at `base`, read on `edge` and
    /// believed for six hours by `clock`.
    pub fn new(base: &Url, edge: Handle, clock: Arc<dyn Clock>) -> Result<Self, reqwest::Error> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(READ_TIMEOUT)
            .build()?;
        Ok(Self(Arc::new(Distribution {
            base: base.as_str().trim_end_matches('/').to_owned(),
            http,
            edge,
            releases: Mutex::default(),
            newest: KeptRead::new(LATEST_TTL, clock),
        })))
    }

    /// The vendor's newest release: the one read within six hours, or a new
    /// read. `refresh` reads the pointer again unless a read finished after
    /// the call began.
    pub async fn latest(&self, refresh: bool) -> Result<Release, ReleaseError> {
        self.0.latest(refresh).await
    }

    /// The newest release read within six hours, without waiting. Without
    /// one, as after the backend starts, it answers none and reads the
    /// pointer in the background, unless a read is under way, for a later
    /// call to answer with.
    pub fn believed(&self) -> Option<Release> {
        let distribution = self.0.clone();
        // The read ends within `READ_TIMEOUT`; the edge's runtime ending at
        // shutdown drops it before. A failed read is read again by a later
        // start, which still finds no release believed.
        let read_again = async move {
            if let Err(error) = distribution.latest(false).await {
                tracing::warn!("the newest Claude Code release was not read: {error}");
            }
        };
        let kept = self.0.newest.kept(&self.0.edge, read_again)?;
        kept.fresh.then_some(kept.value)
    }
}

impl Distribution {
    async fn latest(&self, refresh: bool) -> Result<Release, ReleaseError> {
        self.newest.read(refresh, || self.read_latest()).await
    }

    /// The release the `latest` pointer names, its manifest read once.
    async fn read_latest(&self) -> Result<Release, ReleaseError> {
        let pointer = self.text("latest".to_owned()).await.map_err(ReleaseError)?;
        let version = pointer.trim();
        if !is_version(version) {
            return Err(ReleaseError("the distribution named no version".into()));
        }
        if let Some(release) = self.releases().get(version) {
            return Ok(release.clone());
        }
        let release = self.release(version).await.map_err(|reason| {
            ReleaseError(format!(
                "the release of version {version} could not be read ({reason})"
            ))
        })?;
        self.releases()
            .insert(version.to_owned(), release.clone());
        Ok(release)
    }

    fn releases(&self) -> std::sync::MutexGuard<'_, HashMap<String, Release>> {
        // A section only looks a release up or adds one.
        self.releases.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Version `version`'s release record, from its manifest.
    async fn release(&self, version: &str) -> Result<Release, String> {
        let text = self.text(format!("{version}/manifest.json")).await?;
        let manifest: Manifest = serde_json::from_str(&text).map_err(|error| error.to_string())?;
        if manifest.version != version {
            return Err(format!("its manifest names version {}", manifest.version));
        }
        let platforms = manifest
            .platforms
            .into_iter()
            .map(|(platform, entry)| {
                let artifact = Artifact {
                    url: format!("{}/{version}/{platform}/{}", self.base, entry.binary),
                    size: entry.size,
                    sha256: entry.checksum,
                };
                (platform, artifact)
            })
            .collect();
        let release = Release {
            version: manifest.version,
            platforms,
        };
        garde::Validate::validate(&release)
            .map_err(|report| report.to_string().trim_end().to_owned())?;
        Ok(release)
    }

    /// The text at `path` under the distribution, or why it was not read.
    async fn text(&self, path: String) -> Result<String, String> {
        let url = format!("{}/{path}", self.base);
        let http = self.http.clone();
        let read = self.edge.spawn(async move {
            let response = http
                .get(&url)
                .send()
                .await
                .map_err(|error| format!("the distribution did not answer ({error})"))?;
            let status = response.status();
            if !status.is_success() {
                return Err(format!("the distribution answered {status}"));
            }
            response
                .text()
                .await
                .map_err(|error| format!("the distribution did not answer ({error})"))
        });
        read.await
            .unwrap_or_else(|error| Err(format!("the distribution was not read ({error})")))
    }
}

#[cfg(test)]
mod tests {
    use demi_provider_common::testing::{MockResponse, MockVendor};
    use serde_json::json;

    use super::*;

    fn text(body: &str) -> MockResponse {
        MockResponse::status(200).chunk(body.to_owned())
    }

    fn manifest(version: &str) -> MockResponse {
        let body = json!({
            "version": version,
            "commit": "ignored",
            "platforms": {
                "linux-x64": { "binary": "claude", "checksum": "ab".repeat(32), "size": 1024 },
                "win32-x64": { "binary": "claude.exe", "checksum": "cd".repeat(32), "size": 2048 },
            },
        });
        text(&body.to_string())
    }

    #[tokio::test]
    async fn the_newest_release_is_the_pointers_version_from_its_manifest_believed_until_a_refresh()
    {
        let vendor = MockVendor::start().await;
        let clock = Arc::new(demi_provider_common::testing::FixedClock(
            "2026-09-18T14:00:00Z".parse().unwrap(),
        ));
        let releases =
            ClaudeReleases::new(&vendor.url("/r").parse().unwrap(), Handle::current(), clock)
                .unwrap();
        vendor.respond_at("/r/latest", text("2.1.3\n"));
        vendor.respond_at("/r/2.1.3/manifest.json", manifest("2.1.3"));
        // Concurrent readers share one read.
        let (first, second) = tokio::join!(releases.latest(false), releases.latest(false));
        let release = first.unwrap();
        assert_eq!(second.unwrap(), release);
        assert_eq!(release.version, "2.1.3");
        let linux = &release.platforms["linux-x64"];
        assert_eq!(linux.url, vendor.url("/r/2.1.3/linux-x64/claude"));
        assert_eq!(
            (linux.size, linux.sha256.as_str()),
            (1024, "ab".repeat(32).as_str())
        );
        assert_eq!(
            release.platforms["win32-x64"].url,
            vendor.url("/r/2.1.3/win32-x64/claude.exe")
        );
        assert_eq!(vendor.requests().len(), 2);

        // A refresh reads the pointer again; a version's manifest is read once.
        vendor.respond_at("/r/latest", text("2.1.3"));
        assert_eq!(releases.latest(true).await.unwrap(), release);
        assert_eq!(vendor.requests().len(), 3);

        // A pointer that names no version, a release that cannot be read and a
        // redirect fail; no other version is chosen, and what was read stays.
        vendor.respond_at("/r/latest", text("latest-and-greatest"));
        let unnamed = releases.latest(true).await.unwrap_err();
        assert_eq!(unnamed.to_string(), "the distribution named no version");
        vendor.respond_at("/r/latest", text("2.1.4"));
        vendor.respond_at("/r/2.1.4/manifest.json", MockResponse::status(404));
        let missing = releases.latest(true).await.unwrap_err().to_string();
        assert_eq!(
            missing,
            "the release of version 2.1.4 could not be read (the distribution answered 404 Not Found)"
        );
        vendor.respond_at("/r/latest", text("2.1.5"));
        vendor.respond_at("/r/2.1.5/manifest.json", manifest("2.1.4"));
        let mislabelled = releases.latest(true).await.unwrap_err().to_string();
        assert!(
            mislabelled.ends_with("(its manifest names version 2.1.4)"),
            "{mislabelled}"
        );
        let elsewhere = MockResponse::status(302).header("location", &vendor.url("/r/moved"));
        vendor.respond_at("/r/latest", elsewhere);
        let redirected = releases.latest(true).await.unwrap_err().to_string();
        assert_eq!(redirected, "the distribution answered 302 Found");
        assert_eq!(releases.latest(false).await.unwrap(), release);
    }
}
