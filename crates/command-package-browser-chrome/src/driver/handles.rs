/// 16 random bytes, the body of an opaque browser handle.
pub(crate) fn random() -> crate::driver::operation::Result<[u8; 16]> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(std::io::Error::other)?;
    Ok(bytes)
}

/// Allocate an opaque browser handle without exposing CDP or document identities.
pub fn fresh(prefix: &str) -> crate::driver::operation::Result<String> {
    Ok(crate::driver::protocol::handle(prefix, random()?))
}
