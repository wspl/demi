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
