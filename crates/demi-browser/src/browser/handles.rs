/// 16 random bytes, the body of an opaque browser handle.
pub(super) fn random() -> super::Result<[u8; 16]> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(std::io::Error::other)?;
    Ok(bytes)
}

/// Allocate an opaque browser handle without exposing CDP or document identities.
pub(super) fn fresh(prefix: &str) -> super::Result<String> {
    Ok(super::protocol::handle(prefix, random()?))
}
