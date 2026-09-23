//! Which download URLs the release record `claude.ensure` installs from may
//! name; the record's shape is `demi_claude_protocol::Release`.

use demi_claude_protocol::{Artifact, Release};

use crate::install::EnsureError;

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

/// Decodes a record and checks that every platform's URL may be downloaded,
/// not only this machine's.
pub(crate) fn parse(input: &[u8], transport: Transport) -> Result<Release, EnsureError> {
    let release =
        Release::parse(input).map_err(|error| EnsureError::InvalidRelease(error.to_string()))?;
    for (platform, artifact) in &release.platforms {
        allowed(artifact, transport).map_err(|reason| {
            EnsureError::InvalidRelease(format!("platform {platform}: {reason}"))
        })?;
    }
    Ok(release)
}

fn allowed(artifact: &Artifact, transport: Transport) -> Result<(), String> {
    let url = reqwest::Url::parse(&artifact.url).map_err(|error| format!("url: {error}"))?;
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
    Ok(())
}

/// The URL's host, for messages.
pub(crate) fn host(artifact: &Artifact) -> String {
    reqwest::Url::parse(&artifact.url)
        .ok()
        .and_then(|url| url.host_str().map(String::from))
        .unwrap_or_default()
}
