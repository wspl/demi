use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

/// Allocate an opaque browser handle without exposing CDP or document identities.
pub(super) fn fresh(prefix: &str) -> super::Result<String> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(std::io::Error::other)?;
    Ok(format!("{prefix}_{}", URL_SAFE_NO_PAD.encode(bytes)))
}
