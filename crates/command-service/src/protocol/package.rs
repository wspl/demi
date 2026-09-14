use sha2::{Digest, Sha256};

use super::{PackageArtifact, PackageDescriptor, ProtocolError};

impl PackageDescriptor {
    pub fn parse(value: serde_json::Value) -> Result<Self, ProtocolError> {
        Ok(serde_json::from_value(value)?)
    }

    pub fn digest(&self) -> Result<String, ProtocolError> {
        super::generated::package_descriptor_validate(self)
            .map_err(|_| ProtocolError::InvalidMetadata)?;
        canonical_digest(self)
    }
}

pub fn canonical_digest<T: serde::Serialize>(value: &T) -> Result<String, ProtocolError> {
    let bytes = serde_json_canonicalizer::to_vec(value)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub fn target_artifact<'a>(
    package: &'a PackageDescriptor,
    target: &str,
) -> Result<&'a PackageArtifact, ProtocolError> {
    package
        .targets
        .get(target)
        .ok_or(ProtocolError::InvalidMetadata)
}

/// The build target used by native artifacts and their runtime dependencies.
pub fn host_target() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return "aarch64-apple-darwin";
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return "x86_64-apple-darwin";
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return "aarch64-unknown-linux-musl";
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return "x86_64-unknown-linux-musl";
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    return "aarch64-pc-windows-msvc";
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return "x86_64-pc-windows-msvc";
    #[cfg(not(any(
        all(
            target_os = "macos",
            any(target_arch = "aarch64", target_arch = "x86_64")
        ),
        all(
            target_os = "linux",
            any(target_arch = "aarch64", target_arch = "x86_64")
        ),
        all(
            target_os = "windows",
            any(target_arch = "aarch64", target_arch = "x86_64")
        )
    )))]
    compile_error!("unsupported runner target");
}
