//! Which download URLs the release record `claude-code.ensure` installs from
//! may name; the record's shape is
//! `demi_command_package_claude_code_protocol::Release`.

use demi_command_package_claude_code_protocol::{Artifact, Release};

use crate::install::EnsureError;

/// Decodes a record and checks that every platform's URL is an HTTPS one,
/// not only this machine's.
pub(crate) fn parse(input: &[u8]) -> Result<Release, EnsureError> {
    let release =
        Release::parse(input).map_err(|error| EnsureError::InvalidRelease(error.to_string()))?;
    for (platform, artifact) in &release.platforms {
        https(artifact).map_err(|reason| {
            EnsureError::InvalidRelease(format!("platform {platform}: {reason}"))
        })?;
    }
    Ok(release)
}

fn https(artifact: &Artifact) -> Result<(), String> {
    let url = url::Url::parse(&artifact.url).map_err(|error| format!("url: {error}"))?;
    if url.scheme() != "https" {
        return Err("url must be https".into());
    }
    Ok(())
}
