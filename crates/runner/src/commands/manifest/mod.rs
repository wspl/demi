//! Immutable command declarations and CLI validation shared with TypeScript embedders.

// Generated checks use uniform borrowed expressions across owned fields and references.
#[allow(
    clippy::needless_borrow,
    clippy::deref_addrof,
    clippy::len_zero,
    clippy::nonminimal_bool,
    clippy::collapsible_if,
    clippy::redundant_closure_call
)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/manifest.rs"));
}
mod help;
mod parse;
pub use generated::{
    Manifest, Node, NodeVariant0 as Group, NodeVariant1 as Leaf,
    NodeVariant1Variant1Binding as Binding,
};
pub use parse::{Parsed, Selected};

use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Protocol(#[from] demi_command_service::protocol::ProtocolError),
}

impl Manifest {
    pub fn parse(value: Value) -> Result<Self, Error> {
        let manifest: Self = serde_json::from_value(value)?;
        let mut ids = HashSet::new();
        for (hash, descriptor) in &manifest.packages {
            if !ids.insert(&descriptor.id) || descriptor.digest()? != *hash {
                return Err(Error::Invalid(format!(
                    "duplicate or corrupt native package: {}",
                    descriptor.id
                )));
            }
        }
        for (name, root) in &manifest.roots {
            if name != root.tree.name() {
                return Err(Error::Invalid("manifest root name mismatch".into()));
            }
            manifest.validate_node(&root.tree, 0)?;
        }
        #[derive(serde::Serialize)]
        struct Body<'a> {
            roots: &'a BTreeMap<String, generated::ManifestRootsValue>,
            packages: &'a BTreeMap<String, demi_command_service::protocol::PackageDescriptor>,
        }
        let body = Body {
            roots: &manifest.roots,
            packages: &manifest.packages,
        };
        if demi_command_service::protocol::canonical_digest(&body)? != manifest.hash {
            return Err(Error::Invalid("command manifest hash mismatch".into()));
        }
        Ok(manifest)
    }

    fn validate_node(&self, node: &Node, depth: usize) -> Result<(), Error> {
        if depth > 32 {
            return Err(Error::Invalid("command tree exceeds 32 levels".into()));
        }
        match node {
            Node::Variant0(group) => {
                let mut names = HashSet::new();
                for child in &group.subcommands {
                    if !names.insert(child.name()) {
                        return Err(Error::Invalid("duplicate command name".into()));
                    }
                    self.validate_node(child, depth + 1)?;
                }
            }
            Node::Variant1(leaf) => {
                if let Some(binding) = leaf.binding() {
                    let descriptor =
                        self.packages.get(&binding.descriptor_hash).ok_or_else(|| {
                            Error::Invalid("native package is not in manifest".into())
                        })?;
                    if descriptor.id != binding.package
                        || !descriptor.operations.contains(&binding.operation)
                    {
                        return Err(Error::Invalid("unresolved native command binding".into()));
                    }
                }
                leaf.validate()?;
            }
        }
        Ok(())
    }
}

impl Node {
    pub fn name(&self) -> &str {
        match self {
            Self::Variant0(group) => &group.name,
            Self::Variant1(leaf) => leaf.name(),
        }
    }
    pub fn summary(&self) -> &str {
        match self {
            Self::Variant0(group) => &group.summary,
            Self::Variant1(leaf) => leaf.summary(),
        }
    }
    pub fn leaf(&self) -> Option<&Leaf> {
        match self {
            Self::Variant0(_) => None,
            Self::Variant1(leaf) => Some(leaf),
        }
    }
}

macro_rules! leaf_fields {
    ($($name:ident: $kind:ty),* $(,)?) => {
        $(pub fn $name(&self) -> &$kind {
            match self { Self::Variant0(leaf) => &leaf.$name, Self::Variant1(leaf) => &leaf.$name }
        })*
    };
}

impl Leaf {
    leaf_fields! {
        name: String,
        summary: String,
        success_output: Option<String>,
        failure_output: Option<String>,
        running_hint: Option<String>,
        input: Option<BTreeMap<String, Value>>,
        positionals: Option<Vec<String>>,
        stdin_field: Option<String>,
        rest_field: Option<String>,
        output: Option<generated::NodeVariant1Variant0Output>,
    }
    pub fn binding(&self) -> Option<&Binding> {
        match self {
            Self::Variant0(_) => None,
            Self::Variant1(leaf) => Some(&leaf.binding),
        }
    }
    pub fn properties(&self) -> Option<&serde_json::Map<String, Value>> {
        self.input().as_ref()?.get("properties")?.as_object()
    }
    pub fn required(&self, field: &str) -> bool {
        self.input()
            .as_ref()
            .and_then(|schema| schema.get("required"))
            .and_then(Value::as_array)
            .is_some_and(|fields| fields.iter().any(|name| name.as_str() == Some(field)))
    }
    pub fn json_output(&self) -> Option<&BTreeMap<String, Value>> {
        self.output().as_ref()?.json.as_ref()
    }
    fn validate(&self) -> Result<(), Error> {
        if let Some(schema) = self.input() {
            if schema.get("type").and_then(Value::as_str) != Some("object") {
                return Err(Error::Invalid(
                    "command input must describe an object".into(),
                ));
            }
            jsonschema::validator_for(&serde_json::to_value(schema)?)
                .map_err(|error| Error::Invalid(error.to_string()))?;
        }
        let mut fields = HashSet::new();
        let sources = self
            .positionals()
            .iter()
            .flatten()
            .chain(self.stdin_field().iter())
            .chain(self.rest_field().iter());
        for field in sources {
            if !fields.insert(field) {
                return Err(Error::Invalid(format!(
                    "multiple input sources for {field}"
                )));
            }
            if !self
                .properties()
                .is_some_and(|properties| properties.contains_key(field))
            {
                return Err(Error::Invalid(format!(
                    "input source has no schema: {field}"
                )));
            }
        }
        if let Some(properties) = self.properties() {
            for field in properties.keys() {
                let valid = field
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphanumeric)
                    && field
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
                if !valid {
                    return Err(Error::Invalid(format!("invalid input name: {field}")));
                }
                if !fields.contains(field) && matches!(field.as_str(), "help" | "json") {
                    return Err(Error::Invalid(format!("reserved command option: {field}")));
                }
            }
        }
        if let Some(field) = self.stdin_field()
            && self.properties().expect("source checked above")[field]
                .get("type")
                .and_then(Value::as_str)
                != Some("string")
        {
            return Err(Error::Invalid("stdin input must be a string".into()));
        }
        if let Some(field) = self.rest_field() {
            let property = &self.properties().expect("source checked above")[field];
            if property.get("type").and_then(Value::as_str) != Some("array")
                || property.pointer("/items/type").and_then(Value::as_str) != Some("string")
            {
                return Err(Error::Invalid("rest input must be a string array".into()));
            }
        }
        let mut optional = false;
        for field in self.positionals().iter().flatten() {
            if self.required(field) && optional {
                return Err(Error::Invalid(
                    "required positional follows optional positional".into(),
                ));
            }
            optional |= !self.required(field);
        }
        if let Some(schema) = self.json_output() {
            jsonschema::validator_for(&serde_json::to_value(schema)?)
                .map_err(|error| Error::Invalid(error.to_string()))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typescript_manifest_help_and_cli_cases_match_rust() {
        let fixture: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../packages/command-loader/src/fixtures/cli.json"
        )))
        .unwrap();
        let manifest = Manifest::parse(fixture["manifest"].clone()).unwrap();
        assert_eq!(
            manifest.roots["fixture"].tree.help("fixture"),
            fixture["help"].as_str().unwrap()
        );
        for case in fixture["cases"].as_array().unwrap() {
            let argv: Vec<String> = serde_json::from_value(case["argv"].clone()).unwrap();
            let result = (|| {
                let selected = manifest.select("fixture", &argv)?;
                let parsed = selected.parse(&argv)?;
                if parsed.help {
                    return Ok(parsed);
                }
                parsed.validate(
                    selected.node.leaf().unwrap(),
                    Some(case["stdin"].as_str().unwrap().into()),
                )
            })();
            if case["invalid"] == true {
                assert!(result.is_err(), "argv={argv:?}");
            } else {
                assert_eq!(
                    serde_json::to_value(result.unwrap()).unwrap(),
                    case["parsed"],
                    "argv={argv:?}"
                );
            }
        }
    }

    #[test]
    fn altered_descriptors_and_manifest_content_are_rejected() {
        let fixture: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../packages/command-loader/src/fixtures/cli.json"
        )))
        .unwrap();
        let mut value = fixture["manifest"].clone();
        value["roots"]["fixture"]["tree"]["summary"] = "corrupt".into();
        assert!(Manifest::parse(value).is_err());
        let mut value = fixture["manifest"].clone();
        let descriptor = value["packages"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap();
        descriptor["operations"] = serde_json::json!(["file.read", "file.read"]);
        assert!(Manifest::parse(value).is_err());
    }
}
