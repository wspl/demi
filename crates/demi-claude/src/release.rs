//! The release record `claude.ensure` installs from.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::{install::EnsureError, version};

/// One CLI version and its official artifact for each platform key.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Release {
    pub version: String,
    pub platforms: BTreeMap<String, Artifact>,
}

/// Where one platform's executable is, its byte size and its SHA-256 in lowercase hex.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

/// Which download URLs a release record may name.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Transport {
    /// HTTPS only.
    #[default]
    Https,
    /// HTTPS, and plain HTTP to `127.0.0.1` for a test's fixture server.
    #[cfg(test)]
    HttpsOrLoopbackHttp,
}

impl Release {
    /// Parse and validate a record. Every entry is validated, not only this machine's.
    pub(crate) fn parse(input: &[u8], transport: Transport) -> Result<Self, EnsureError> {
        let release: Self = serde_json::from_slice(input)
            .map_err(|error| EnsureError::InvalidRelease(error.to_string()))?;
        if !version::is_valid(&release.version) {
            return Err(EnsureError::InvalidRelease(
                "version must be MAJOR.MINOR.PATCH with an optional -PRERELEASE".into(),
            ));
        }
        for (platform, artifact) in &release.platforms {
            artifact.validate(transport).map_err(|reason| {
                EnsureError::InvalidRelease(format!("platform {platform}: {reason}"))
            })?;
        }
        Ok(release)
    }
}

impl Artifact {
    fn validate(&self, transport: Transport) -> Result<(), String> {
        let url = reqwest::Url::parse(&self.url).map_err(|error| format!("url: {error}"))?;
        let allowed = match transport {
            Transport::Https => url.scheme() == "https",
            #[cfg(test)]
            Transport::HttpsOrLoopbackHttp => {
                url.scheme() == "https"
                    || (url.scheme() == "http" && url.host_str() == Some("127.0.0.1"))
            }
        };
        if !allowed {
            return Err("url must be https".into());
        }
        if self.size == 0 {
            return Err("size must be positive".into());
        }
        if self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err("sha256 must be 64 lowercase hexadecimal digits".into());
        }
        Ok(())
    }

    /// The URL's host, for messages.
    pub(crate) fn host(&self) -> String {
        reqwest::Url::parse(&self.url)
            .ok()
            .and_then(|url| url.host_str().map(String::from))
            .unwrap_or_default()
    }
}
