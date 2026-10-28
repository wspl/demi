//! A source's origin: `owner/repo` for a repository on GitHub, an `https`
//! URL of any git repository, or, for a source a device fetches, an SSH
//! address, `user@host:path` or an `ssh://` URL, which only a device's own
//! keys open.

use sha2::{Digest, Sha256};

/// An origin the user may add.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Origin {
    /// The URL the fetch reads, before a test's rewrite.
    url: String,
}

impl Origin {
    /// The origin `text` names; why it names none otherwise.
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if let Some(rest) = text.strip_prefix("ssh://") {
            let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
            let (user, host) = authority.rsplit_once('@').map_or((None, authority), |(user, host)| (Some(user), host));
            if host.is_empty() || path.trim_matches('/').is_empty() {
                return Err(format!("\"{text}\" names no repository"));
            }
            let user = user.map(|user| format!("{user}@")).unwrap_or_default();
            return Ok(Self {
                url: format!("ssh://{user}{}/{}", host.to_ascii_lowercase(), trimmed(path)),
            });
        }
        if let Some((user, rest)) = text.split_once('@').filter(|(user, rest)| {
            !user.is_empty() && !user.contains(['/', ':']) && rest.contains(':') && !text.contains("://")
        }) {
            let (host, path) = rest.split_once(':').expect("the address holds a colon");
            if host.is_empty() || host.contains('/') || path.trim_matches('/').is_empty() {
                return Err(format!("\"{text}\" names no repository"));
            }
            // An absolute path is the one an `ssh://` URL names: one form for
            // both, so one repository is one source. A relative one is the
            // user's home's, which no URL names.
            if let Some(absolute) = path.strip_prefix('/') {
                return Ok(Self {
                    url: format!("ssh://{user}@{}/{}", host.to_ascii_lowercase(), trimmed(absolute)),
                });
            }
            return Ok(Self {
                url: format!("{user}@{}:{}", host.to_ascii_lowercase(), trimmed(path)),
            });
        }
        if let Some(rest) = text.strip_prefix("https://") {
            let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
            if host.is_empty() || path.trim_matches('/').is_empty() {
                return Err(format!("\"{text}\" names no repository"));
            }
            let path = path.trim_end_matches('/');
            let path = path.strip_suffix(".git").unwrap_or(path);
            return Ok(Self {
                url: format!("https://{}/{path}", host.to_ascii_lowercase()),
            });
        }
        let github = text.split_once('/').filter(|(owner, repo)| {
            github_name(owner) && github_name(repo.strip_suffix(".git").unwrap_or(repo))
        });
        match github {
            Some((owner, repo)) => Ok(Self {
                url: format!(
                    "https://github.com/{owner}/{}",
                    repo.strip_suffix(".git").unwrap_or(repo)
                ),
            }),
            None => Err(format!("\"{text}\" is neither owner/repo, an https URL nor an SSH address")),
        }
    }

    /// Whether only a device's keys open it: an SSH address, which the
    /// server, holding no key, cannot fetch.
    pub(crate) fn ssh(&self) -> bool {
        !self.url.starts_with("https://")
    }

    /// The repository's URL, with no `.git` and no trailing slash: two
    /// origins of the same URL are the same source.
    pub(crate) fn url(&self) -> &str {
        &self.url
    }

    /// The source's id: the first 12 hexadecimal digits of its URL's
    /// SHA-256.
    pub(crate) fn id(&self) -> String {
        format!("{:x}", Sha256::digest(&self.url))[..12].to_owned()
    }

    /// The repository's name, the last part of its URL, which a skill at
    /// the repository's root is named after when its front matter names
    /// none.
    pub(crate) fn repository(&self) -> &str {
        self.url.rsplit(['/', ':']).next().unwrap_or(&self.url)
    }
}

/// `path` without a trailing slash or `.git`.
fn trimmed(path: &str) -> &str {
    let path = path.trim_end_matches('/');
    path.strip_suffix(".git").unwrap_or(path)
}

/// Whether `part` can be a GitHub owner's or repository's name.
fn github_name(part: &str) -> bool {
    !part.is_empty()
        && part
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}
