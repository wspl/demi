//! Demi's releases on GitHub, which `demi-server` downloads and the backend
//! offers an administrator: the newest of a channel, and a release's assets.
//!
//! Every release is tagged `v<version>`. A published one is GitHub's
//! *latest* in turn; a pre-release, which a deploy publishes, never is. Demi
//! has no stable channel yet: what it publishes is a beta.

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
    /// Every release, a deploy's pre-releases included, the newest builds
    /// first.
    Canary,
}

/// How many redirects a request for a release's asset follows: GitHub sends
/// one, to short-lived storage, and two servers can redirect to each other
/// without end.
pub const REDIRECTS: usize = 10;

/// A release as GitHub's API describes it, of which only the tag matters.
#[derive(serde::Deserialize)]
struct Release {
    tag_name: String,
}

/// The newest release of `channel`, or none when it has none: no release is
/// published yet, or none is tagged `v<version>`.
pub async fn newest(channel: Channel) -> std::io::Result<Option<Version>> {
    match channel {
        Channel::Canary => {
            let mut tags = Vec::new();
            for page in 1.. {
                let releases: Vec<Release> = github(&format!("releases?per_page={PAGE}&page={page}"))
                    .await?
                    .unwrap_or_default();
                let whole = releases.len() == PAGE;
                tags.extend(releases.into_iter().map(|release| release.tag_name));
                if !whole {
                    break;
                }
            }
            Ok(highest(tags.iter().map(String::as_str)))
        }
        Channel::Beta => {
            let Some(latest) = github::<Release>("releases/latest").await? else {
                return Ok(None);
            };
            let version = latest.tag_name.strip_prefix('v').ok_or_else(|| {
                std::io::Error::other(format!("the newest release's tag {} names no version", latest.tag_name))
            })?;
            Version::parse(version).map(Some).map_err(std::io::Error::other)
        }
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

/// The highest version among `tags` that are `v<version>`; any other tag,
/// such as the gVisor build's `runsc-release-*`, is no Demi release.
pub fn highest<'a>(tags: impl IntoIterator<Item = &'a str>) -> Option<Version> {
    tags.into_iter()
        .filter_map(|tag| Version::parse(tag.strip_prefix('v')?).ok())
        .max()
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
