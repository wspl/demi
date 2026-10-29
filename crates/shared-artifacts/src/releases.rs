//! Demi's releases on GitHub, which `demi-server` downloads and the backend
//! offers an administrator: the newest of a channel, and a release's assets.
//!
//! Every release is tagged `v<version>`. A published one is GitHub's
//! *latest* in turn; a pre-release, a Canary, never is. Demi has no stable
//! channel yet: what it publishes is a beta.

use semver::Version;

/// The repository whose GitHub releases are Demi's.
pub const REPOSITORY: &str = "wspl/demi";

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
        // The feed, not the API's list of releases: each release in the list
        // describes every one of its dozens of assets, some 100 KB a release,
        // and finding the highest read them all, a page of 100 at a time.
        Channel::Canary => match get(&format!("https://github.com/{REPOSITORY}/releases.atom"), "application/atom+xml").await? {
            Some(feed) => highest_in_feed(&String::from_utf8_lossy(&feed)),
            None => Ok(None),
        },
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

/// The highest Demi release among the entries of the repository's releases
/// feed, with when it was published.
///
/// The feed lists the ten releases GitHub published last, newest first, each
/// with its tag at the end of its `id` and its publication as `updated`.
/// The release workflow refuses a version below the highest published one
/// (`.github/workflows/release.yml`), so the highest is among them. A tag that is no `v<version>`, such as the gVisor build's, is
/// no Demi release.
fn highest_in_feed(feed: &str) -> std::io::Result<Option<Published>> {
    let document = roxmltree::Document::parse(feed).map_err(std::io::Error::other)?;
    let child = |entry: roxmltree::Node<'_, '_>, name: &str| {
        entry
            .children()
            .find(|node| node.has_tag_name((ATOM, name)))
            .and_then(|node| node.text())
            .map(str::trim)
            .map(str::to_owned)
    };
    let mut highest: Option<Published> = None;
    for entry in document.root_element().children().filter(|node| node.has_tag_name((ATOM, "entry"))) {
        let Some(version) = child(entry, "id")
            .and_then(|id| Version::parse(id.rsplit('/').next()?.strip_prefix('v')?).ok())
        else {
            continue;
        };
        if highest.as_ref().is_some_and(|highest| highest.version >= version) {
            continue;
        }
        let at = child(entry, "updated")
            .ok_or_else(|| std::io::Error::other(format!("the feed's entry of {version} says no time")))?
            .parse()
            .map_err(std::io::Error::other)?;
        highest = Some(Published { version, at });
    }
    Ok(highest)
}

/// The Atom namespace, the feed's.
const ATOM: &str = "http://www.w3.org/2005/Atom";

/// Reads `path` of the repository's GitHub API; none when GitHub has
/// nothing there.
async fn github<T: serde::de::DeserializeOwned>(path: &str) -> std::io::Result<Option<T>> {
    match get(&format!("https://api.github.com/repos/{REPOSITORY}/{path}"), "application/vnd.github+json").await? {
        Some(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(std::io::Error::other),
        None => Ok(None),
    }
}

/// The body GitHub answers `url` with, of the media type `accept`; none when
/// it has nothing there.
async fn get(url: &str, accept: &str) -> std::io::Result<Option<bytes::Bytes>> {
    let client = crate::client().map_err(std::io::Error::other)?;
    let response = client
        .get(url)
        .header("user-agent", "demi")
        .header("accept", accept)
        .send()
        .await
        .map_err(|error| std::io::Error::other(error.without_url().to_string()))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let response = response
        .error_for_status()
        .map_err(|error| std::io::Error::other(error.without_url().to_string()))?;
    response
        .bytes()
        .await
        .map(Some)
        .map_err(|error| std::io::Error::other(error.without_url().to_string()))
}
