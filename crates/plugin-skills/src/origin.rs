//! A source's origin (`skills.md` § User skills): `owner/repo` for a
//! repository on GitHub, or an `https` URL of any git repository.

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
            None => Err(format!("\"{text}\" is neither owner/repo nor an https URL")),
        }
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
        self.url.rsplit('/').next().unwrap_or(&self.url)
    }
}

/// Whether `part` can be a GitHub owner's or repository's name.
fn github_name(part: &str) -> bool {
    !part.is_empty()
        && part
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}
