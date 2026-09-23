//! Command declarations: the tree of groups and leaves a command manifest
//! carries, the rules a leaf's input declaration follows, and how a command
//! line selects a command, fills its input and renders its help
//! (`commands.md`).

mod help;
mod parse;

pub use parse::{Parsed, Selected, UsageError};

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_with::rust::unwrap_or_skip;

/// The deepest a command tree nests.
pub const MAX_DEPTH: usize = 32;

/// A declaration that breaks one of the tree's rules.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct DeclarationError(String);

/// A command group or a command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Node {
    Group(Group),
    Leaf(Leaf),
}

/// A command group: a name that only selects one of its subcommands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub name: String,
    pub summary: String,
    pub subcommands: Vec<Node>,
}

/// A command: its help texts, the JSON Schema of its input object, where its
/// input comes from on the command line, and how it runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawLeaf", into = "RawLeaf")]
pub struct Leaf {
    pub name: String,
    pub summary: String,
    pub success_output: Option<String>,
    pub failure_output: Option<String>,
    pub running_hint: Option<String>,
    pub input: Option<BTreeMap<String, Value>>,
    pub positionals: Option<Vec<String>>,
    pub stdin_field: Option<String>,
    pub rest_field: Option<String>,
    pub output: Option<LeafOutput>,
    pub kind: LeafKind,
}

/// The JSON Schema of a command's `--json` output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeafOutput {
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    pub json: Option<BTreeMap<String, Value>>,
}

/// How a command runs: as a call to the backend, or as an operation of a
/// native command package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeafKind {
    Rpc,
    Native(Binding),
}

/// The native package operation a command runs, and the digest of the
/// package descriptor that serves it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Binding {
    pub package: String,
    pub operation: String,
    pub descriptor_hash: String,
}

impl Node {
    pub fn name(&self) -> &str {
        match self {
            Self::Group(group) => &group.name,
            Self::Leaf(leaf) => &leaf.name,
        }
    }

    pub fn summary(&self) -> &str {
        match self {
            Self::Group(group) => &group.summary,
            Self::Leaf(leaf) => &leaf.summary,
        }
    }

    pub fn leaf(&self) -> Option<&Leaf> {
        match self {
            Self::Group(_) => None,
            Self::Leaf(leaf) => Some(leaf),
        }
    }

    /// The leaves of this tree, depth first.
    pub fn leaves(&self) -> Vec<&Leaf> {
        match self {
            Self::Group(group) => group.subcommands.iter().flat_map(Node::leaves).collect(),
            Self::Leaf(leaf) => vec![leaf],
        }
    }

    /// Checks the tree's rules: names, at most [`MAX_DEPTH`] levels, groups
    /// with distinctly named subcommands, and each leaf's input declaration.
    pub fn validate(&self) -> Result<(), DeclarationError> {
        self.validate_at(0)
    }

    fn validate_at(&self, depth: usize) -> Result<(), DeclarationError> {
        if depth > MAX_DEPTH {
            return Err(invalid(format!(
                "command tree exceeds {MAX_DEPTH} levels"
            )));
        }
        if !is_command_name(self.name()) {
            return Err(invalid(format!("invalid command name: {}", self.name())));
        }
        match self {
            Self::Group(group) => {
                if group.subcommands.is_empty() {
                    return Err(invalid(format!(
                        "command group {} has no subcommands",
                        group.name
                    )));
                }
                let mut names = HashSet::new();
                for child in &group.subcommands {
                    if !names.insert(child.name()) {
                        return Err(invalid(format!(
                            "duplicate command name: {}",
                            child.name()
                        )));
                    }
                    child.validate_at(depth + 1)?;
                }
                Ok(())
            }
            Self::Leaf(leaf) => leaf.validate(),
        }
    }
}

impl Leaf {
    pub fn binding(&self) -> Option<&Binding> {
        match &self.kind {
            LeafKind::Rpc => None,
            LeafKind::Native(binding) => Some(binding),
        }
    }

    /// The input object's properties, when the leaf declares an input.
    pub fn properties(&self) -> Option<&serde_json::Map<String, Value>> {
        self.input.as_ref()?.get("properties")?.as_object()
    }

    pub fn required(&self, field: &str) -> bool {
        self.input
            .as_ref()
            .and_then(|schema| schema.get("required"))
            .and_then(Value::as_array)
            .is_some_and(|fields| fields.iter().any(|name| name.as_str() == Some(field)))
    }

    pub fn json_output(&self) -> Option<&BTreeMap<String, Value>> {
        self.output.as_ref()?.json.as_ref()
    }

    fn validate(&self) -> Result<(), DeclarationError> {
        if let Some(schema) = &self.input {
            if schema.get("type").and_then(Value::as_str) != Some("object") {
                return Err(invalid("command input must describe an object".into()));
            }
            compile_schema(schema)?;
        }
        let mut fields = HashSet::new();
        let sources = self
            .positionals
            .iter()
            .flatten()
            .chain(self.stdin_field.iter())
            .chain(self.rest_field.iter());
        for field in sources {
            if !fields.insert(field) {
                return Err(invalid(format!("multiple input sources for {field}")));
            }
            if !self
                .properties()
                .is_some_and(|properties| properties.contains_key(field))
            {
                return Err(invalid(format!("input source has no schema: {field}")));
            }
        }
        if let Some(properties) = self.properties() {
            for field in properties.keys() {
                if !is_command_name(field) {
                    return Err(invalid(format!("invalid input name: {field}")));
                }
                if !fields.contains(field) && matches!(field.as_str(), "help" | "json") {
                    return Err(invalid(format!("reserved command option: {field}")));
                }
            }
        }
        if let Some(field) = &self.stdin_field
            && self.property_type(field) != Some("string")
        {
            return Err(invalid("stdin input must be a string".into()));
        }
        if let Some(field) = &self.rest_field {
            let items = self
                .properties()
                .and_then(|properties| properties[field].pointer("/items/type"))
                .and_then(Value::as_str);
            if self.property_type(field) != Some("array") || items != Some("string") {
                return Err(invalid("rest input must be a string array".into()));
            }
        }
        let mut optional = false;
        for field in self.positionals.iter().flatten() {
            if self.required(field) && optional {
                return Err(invalid(
                    "required positional follows optional positional".into(),
                ));
            }
            optional |= !self.required(field);
        }
        if let Some(schema) = self.json_output() {
            compile_schema(schema)?;
        }
        Ok(())
    }

    /// The JSON type a declared input property has.
    fn property_type(&self, field: &str) -> Option<&str> {
        self.properties()?.get(field)?.get("type")?.as_str()
    }
}

/// Whether `name` can name a command or an input: an ASCII letter or digit,
/// then letters, digits, `_` and `-`.
pub fn is_command_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|byte| byte.is_ascii_alphanumeric())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn compile_schema(schema: &BTreeMap<String, Value>) -> Result<(), DeclarationError> {
    let schema = serde_json::to_value(schema).map_err(|error| invalid(error.to_string()))?;
    jsonschema::validator_for(&schema).map_err(|error| invalid(error.to_string()))?;
    Ok(())
}

fn invalid(message: String) -> DeclarationError {
    DeclarationError(message)
}

/// A leaf as the manifest writes it: `kind` names how it runs, and a native
/// leaf's `binding` sits beside it.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawLeaf {
    name: String,
    summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    success_output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    failure_output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    running_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    input: Option<BTreeMap<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    positionals: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    stdin_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    rest_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    output: Option<LeafOutput>,
    kind: RawKind,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    binding: Option<Binding>,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum RawKind {
    Rpc,
    Native,
}

impl TryFrom<RawLeaf> for Leaf {
    type Error = String;

    fn try_from(raw: RawLeaf) -> Result<Self, String> {
        let kind = match (raw.kind, raw.binding) {
            (RawKind::Rpc, None) => LeafKind::Rpc,
            (RawKind::Native, Some(binding)) => LeafKind::Native(binding),
            (RawKind::Rpc, Some(_)) => return Err("an rpc command has no binding".into()),
            (RawKind::Native, None) => return Err("a native command names its binding".into()),
        };
        Ok(Self {
            name: raw.name,
            summary: raw.summary,
            success_output: raw.success_output,
            failure_output: raw.failure_output,
            running_hint: raw.running_hint,
            input: raw.input,
            positionals: raw.positionals,
            stdin_field: raw.stdin_field,
            rest_field: raw.rest_field,
            output: raw.output,
            kind,
        })
    }
}

impl From<Leaf> for RawLeaf {
    fn from(leaf: Leaf) -> Self {
        let (kind, binding) = match leaf.kind {
            LeafKind::Rpc => (RawKind::Rpc, None),
            LeafKind::Native(binding) => (RawKind::Native, Some(binding)),
        };
        Self {
            name: leaf.name,
            summary: leaf.summary,
            success_output: leaf.success_output,
            failure_output: leaf.failure_output,
            running_hint: leaf.running_hint,
            input: leaf.input,
            positionals: leaf.positionals,
            stdin_field: leaf.stdin_field,
            rest_field: leaf.rest_field,
            output: leaf.output,
            kind,
            binding,
        }
    }
}
