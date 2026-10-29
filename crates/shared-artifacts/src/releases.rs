//! Demi's releases on GitHub, which `demi-server` downloads and the backend
//! offers an administrator: the newest of a channel, and a release's assets.
//!
//! Every release is tagged `v<version>`. A published one is GitHub's
//! *latest* in turn; a pre-release, a Canary, never is. Demi has no stable
//! channel yet: what it publishes is a beta.

use semver::Version;

/// The repository whose GitHub releases are Demi's.
pub const REPOSITORY: &str = "wspl/demi";

/// The most releases one page of GitHub's list gives.
const PAGE: usize = 100;

/// Which releases count as the newest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// The releases Demi publishes: GitHub's latest.
    Beta,
    /// Every release, the Canary pre-releases included, the newest builds
    /// first.
    Canary,
}

/// How many redirects a request for a release's asset follows: GitHub sends
/// one, to short-lived storage, and two servers can redirect to each other
/// without end.
pub const REDIRECTS: usize = 10;

/// A release as GitHub's API describes it, of which only the tag, whether
/// it is a pre-release and when it was published matter.
#[derive(serde::Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    prerelease: bool,
    /// None for a draft, which GitHub shows only to the repository's members.
    #[serde(default)]
    published_at: Option<String>,
}

impl Release {
    /// The Demi version its tag names: none for a tag that is no
    /// `v<version>`, such as the gVisor build's `runsc-release-*`.
    fn version(&self) -> Option<Version> {
        Version::parse(self.tag_name.strip_prefix('v')?).ok()
    }

    /// The release as one of Demi's, or why it is none.
    fn published(self) -> std::io::Result<Published> {
        let version = self
            .version()
            .ok_or_else(|| std::io::Error::other(format!("the release's tag {} names no version", self.tag_name)))?;
        let at = self
            .published_at
            .ok_or_else(|| std::io::Error::other(format!("the release {} is not published", self.tag_name)))?
            .parse()
            .map_err(std::io::Error::other)?;
        Ok(Published { version, at })
    }
}

/// One of Demi's published releases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    pub version: Version,
    /// When GitHub published it.
    pub at: jiff::Timestamp,
}

/// The channel whose releases include `version`: Canary for a pre-release,
/// Beta for one Demi published.
pub async fn channel_of(version: &Version) -> std::io::Result<Channel> {
    let release = github::<Release>(&format!("releases/tags/v{version}"))
        .await?
        .ok_or_else(|| std::io::Error::other(format!("{REPOSITORY} has no release v{version}")))?;
    Ok(if release.prerelease { Channel::Canary } else { Channel::Beta })
}

/// The newest release of `channel`, or none when it has none: no release is
/// published yet, or none is tagged `v<version>`.
pub async fn newest(channel: Channel) -> std::io::Result<Option<Published>> {
    match channel {
        Channel::Canary => {
            let mut highest: Option<(Version, Release)> = None;
            for page in 1.. {
                let releases: Vec<Release> = github(&format!("releases?per_page={PAGE}&page={page}"))
                    .await?
                    .unwrap_or_default();
                let whole = releases.len() == PAGE;
                for release in releases {
                    let Some(version) = release.version() else { continue };
                    if highest.as_ref().is_none_or(|(highest, _)| &version > highest) {
                        highest = Some((version, release));
                    }
                }
                if !whole {
                    break;
                }
            }
            highest.map(|(_, release)| release.published()).transpose()
        }
        Channel::Beta => github::<Release>("releases/latest").await?.map(Release::published).transpose(),
    }
}

/// The size of `version`'s asset `name`, as its download answers a `HEAD`:
/// the release's own addresses, which GitHub's API and its limit on requests
/// stay out of.
pub async fn asset_size(version: &Version, name: &str) -> std::io::Result<u64> {
    let failed = |error: reqwest::Error| std::io::Error::other(format!("{name}: {}", error.without_url()));
    let client = crate::download_client().map_err(std::io::Error::other)?;
    let mut url = reqwest::Url::parse(&asset_url(version, name)).map_err(std::io::Error::other)?;
    for _ in 0..=REDIRECTS {
        let response = client.head(url.clone()).header("user-agent", "demi").send().await.map_err(failed)?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|location| location.to_str().ok())
                .ok_or_else(|| std::io::Error::other(format!("{name}: a redirect names no location")))?;
            url = url.join(location).map_err(std::io::Error::other)?;
            continue;
        }
        let response = response.error_for_status().map_err(failed)?;
        // The header itself: a response to a `HEAD` has no body to size.
        return response
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|length| length.to_str().ok()?.parse().ok())
            .ok_or_else(|| std::io::Error::other(format!("{name}: the download names no size")));
    }
    Err(std::io::Error::other(format!("{name}: more than {REDIRECTS} redirects")))
}

/// The address of `version`'s asset `name`.
pub fn asset_url(version: &Version, name: &str) -> String {
    format!("https://github.com/{REPOSITORY}/releases/download/v{version}/{name}")
}

/// The page of `version`'s release, with its notes.
pub fn page_url(version: &Version) -> String {
    format!("https://github.com/{REPOSITORY}/releases/tag/v{version}")
}

/// Reads `path` of the repository's GitHub API; none when GitHub has
/// nothing there.
async fn github<T: serde::de::DeserializeOwned>(path: &str) -> std::io::Result<Option<T>> {
    let client = crate::client().map_err(std::io::Error::other)?;
    let response = client
        .get(format!("https://api.github.com/repos/{REPOSITORY}/{path}"))
        .header("user-agent", "demi")
        .header("accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| std::io::Error::other(error.without_url().to_string()))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let response = response
        .error_for_status()
        .map_err(|error| std::io::Error::other(error.without_url().to_string()))?;
    let bytes = response
        .bytes()
        .await
        .map_err(|error| std::io::Error::other(error.without_url().to_string()))?;
    serde_json::from_slice(&bytes).map(Some).map_err(std::io::Error::other)
}
