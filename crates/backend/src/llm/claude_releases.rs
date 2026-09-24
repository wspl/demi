//! The vendor's Claude Code releases (`claude-code.md` § Which version): the
//! newest version, read from the official distribution's `latest` pointer
//! and that version's manifest, and believed for six hours. Concurrent
//! readers share one read, a version's manifest is read once, and a redirect
//! is refused. The reads run on the edge's runtime, whoever asks.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use demi_claude_protocol::{Artifact, Release, is_version};
use serde::Deserialize;
use tokio::runtime::Handle;
use tokio::time::Instant;
use url::Url;

/// The vendor's official distribution.
pub(crate) const DEFAULT_RELEASES_URL: &str = "https://downloads.claude.ai/claude-code-releases";

/// How long the newest version is believed before the pointer is read again.
const LATEST_TTL: Duration = Duration::from_secs(6 * 60 * 60);

/// How long one read of the distribution may take.
const READ_TIMEOUT: Duration = Duration::from_secs(15);

/// Why the newest release could not be read, in words that complete
/// "Claude Code could not be installed: …".
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub(crate) struct ReleaseError(String);

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

/// What the backend has read of the distribution.
#[derive(Default)]
struct Known {
    /// The newest release and when it was read.
    newest: Option<(Instant, Release)>,
    /// Each version's release, which never changes once published.
    releases: HashMap<String, Release>,
}

/// The vendor's releases.
pub(crate) struct ClaudeReleases {
    /// The distribution's address, without a trailing slash.
    base: String,
    /// A client that follows no redirect and gives up after `READ_TIMEOUT`.
    http: reqwest::Client,
    edge: Handle,
    /// Held across a read, so that readers who wait for it share it.
    known: tokio::sync::Mutex<Known>,
}

impl ClaudeReleases {
    /// The releases of the distribution at `base`, read on `edge`.
    pub(crate) fn new(base: &Url, edge: Handle) -> Result<Self, reqwest::Error> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(READ_TIMEOUT)
            .build()?;
        Ok(Self {
            base: base.as_str().trim_end_matches('/').to_owned(),
            http,
            edge,
            known: tokio::sync::Mutex::default(),
        })
    }

    /// The vendor's newest release: the one read within six hours, or a new
    /// read. `refresh` reads the pointer again unless a read finished after
    /// the call began.
    pub(crate) async fn latest(&self, refresh: bool) -> Result<Release, ReleaseError> {
        let asked = Instant::now();
        let mut known = self.known.lock().await;
        if let Some((read_at, release)) = &known.newest {
            let fresh = if refresh {
                *read_at >= asked
            } else {
                read_at.elapsed() < LATEST_TTL
            };
            if fresh {
                return Ok(release.clone());
            }
        }
        let pointer = self.text("latest".to_owned()).await.map_err(ReleaseError)?;
        let version = pointer.trim();
        if !is_version(version) {
            return Err(ReleaseError("the distribution named no version".into()));
        }
        let release = match known.releases.get(version) {
            Some(release) => release.clone(),
            None => {
                let release = self.release(version).await.map_err(|reason| {
                    ReleaseError(format!("the release of version {version} could not be read ({reason})"))
                })?;
                known.releases.insert(version.to_owned(), release.clone());
                release
            }
        };
        known.newest = Some((Instant::now(), release.clone()));
        Ok(release)
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
        garde::Validate::validate(&release).map_err(|report| report.to_string().trim_end().to_owned())?;
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
    use demi_provider::testing::{MockResponse, MockVendor};
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
    async fn the_newest_release_is_the_pointers_version_from_its_manifest_believed_until_a_refresh() {
        let vendor = MockVendor::start().await;
        let releases = ClaudeReleases::new(&vendor.url("/r").parse().unwrap(), Handle::current()).unwrap();
        vendor.respond_at("/r/latest", text("2.1.3\n"));
        vendor.respond_at("/r/2.1.3/manifest.json", manifest("2.1.3"));
        // Concurrent readers share one read.
        let (first, second) = tokio::join!(releases.latest(false), releases.latest(false));
        let release = first.unwrap();
        assert_eq!(second.unwrap(), release);
        assert_eq!(release.version, "2.1.3");
        let linux = &release.platforms["linux-x64"];
        assert_eq!(linux.url, vendor.url("/r/2.1.3/linux-x64/claude"));
        assert_eq!((linux.size, linux.sha256.as_str()), (1024, "ab".repeat(32).as_str()));
        assert_eq!(release.platforms["win32-x64"].url, vendor.url("/r/2.1.3/win32-x64/claude.exe"));
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
        assert!(mislabelled.ends_with("(its manifest names version 2.1.4)"), "{mislabelled}");
        let elsewhere = MockResponse::status(302).header("location", &vendor.url("/r/moved"));
        vendor.respond_at("/r/latest", elsewhere);
        let redirected = releases.latest(true).await.unwrap_err().to_string();
        assert_eq!(redirected, "the distribution answered 302 Found");
        assert_eq!(releases.latest(false).await.unwrap(), release);
    }
}
