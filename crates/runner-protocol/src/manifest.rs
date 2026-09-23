//! The command manifest the backend sends a runner: every root command's
//! declaration tree, the native packages its commands bind to, and the hash
//! that identifies the whole (`commands.md` § Manifests).

use std::collections::{BTreeMap, HashSet};

use demi_command_service::protocol::{PackageDescriptor, ProtocolError, canonical_digest};
use demi_command_tree::{DeclarationError, Node};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// The SHA-256 of the canonical JSON of `roots` and `packages`.
    pub hash: String,
    pub roots: BTreeMap<String, Root>,
    /// Each package descriptor, under its digest.
    pub packages: BTreeMap<String, PackageDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Root {
    pub tree: Node,
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Declaration(#[from] DeclarationError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
}

impl Manifest {
    /// Decodes a manifest and verifies it: each descriptor matches the digest
    /// it is filed under and package ids are unique, each root names its
    /// tree, each tree follows the declaration rules, each native command's
    /// binding resolves to a package that serves it, and `hash` covers the
    /// rest.
    pub fn parse(value: Value) -> Result<Self, ManifestError> {
        let manifest: Self = serde_json::from_value(value)?;
        let mut ids = HashSet::new();
        for (digest, descriptor) in &manifest.packages {
            if !ids.insert(&descriptor.id) || descriptor.digest()? != *digest {
                return Err(ManifestError::Invalid(format!(
                    "duplicate or corrupt native package: {}",
                    descriptor.id
                )));
            }
        }
        for (name, root) in &manifest.roots {
            if name != root.tree.name() {
                return Err(ManifestError::Invalid("manifest root name mismatch".into()));
            }
            root.tree.validate()?;
            for binding in root.tree.leaves().into_iter().filter_map(|leaf| leaf.binding()) {
                let resolves = manifest
                    .packages
                    .get(&binding.descriptor_hash)
                    .is_some_and(|descriptor| {
                        descriptor.id == binding.package
                            && descriptor.operations.contains(&binding.operation)
                    });
                if !resolves {
                    return Err(ManifestError::Invalid(
                        "unresolved native command binding".into(),
                    ));
                }
            }
        }
        #[derive(Serialize)]
        struct Body<'a> {
            roots: &'a BTreeMap<String, Root>,
            packages: &'a BTreeMap<String, PackageDescriptor>,
        }
        let body = Body {
            roots: &manifest.roots,
            packages: &manifest.packages,
        };
        if canonical_digest(&body)? != manifest.hash {
            return Err(ManifestError::Invalid("command manifest hash mismatch".into()));
        }
        Ok(manifest)
    }
}
