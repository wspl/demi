use std::sync::LazyLock;

use sha2::{Digest, Sha256};

use crate::{PackageArtifact, PackageDescriptor, ProtocolError};

static SCHEMA: LazyLock<jsonschema::Validator> = LazyLock::new(|| {
    let schema = serde_json::from_str(include_str!("package.schema.json"))
        .expect("generated package schema must be JSON");
    jsonschema::validator_for(&schema).expect("generated package schema must compile")
});

impl PackageDescriptor {
    pub fn parse(value: serde_json::Value) -> Result<Self, ProtocolError> {
        SCHEMA
            .validate(&value)
            .map_err(|_| ProtocolError::InvalidMetadata)?;
        Ok(serde_json::from_value(value)?)
    }

    pub fn digest(&self) -> Result<String, ProtocolError> {
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
