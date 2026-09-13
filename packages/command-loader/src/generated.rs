// Generated from the TypeScript manifest contract.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeVariant0 {
    #[serde(rename = "name")]
    pub name: String,
    #[serde(rename = "summary")]
    pub summary: String,
    #[serde(rename = "subcommands")]
    pub subcommands: Vec<Node>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeVariant1Variant0Output {
    #[serde(rename = "json", default, skip_serializing_if = "Option::is_none")]
    pub json: Option<std::collections::BTreeMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeVariant1Variant0 {
    #[serde(rename = "name")]
    pub name: String,
    #[serde(rename = "summary")]
    pub summary: String,
    #[serde(
        rename = "successOutput",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub success_output: Option<String>,
    #[serde(
        rename = "failureOutput",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub failure_output: Option<String>,
    #[serde(
        rename = "runningHint",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub running_hint: Option<String>,
    #[serde(rename = "input", default, skip_serializing_if = "Option::is_none")]
    pub input: Option<std::collections::BTreeMap<String, serde_json::Value>>,
    #[serde(
        rename = "positionals",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub positionals: Option<Vec<String>>,
    #[serde(
        rename = "stdinField",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub stdin_field: Option<String>,
    #[serde(rename = "restField", default, skip_serializing_if = "Option::is_none")]
    pub rest_field: Option<String>,
    #[serde(rename = "output", default, skip_serializing_if = "Option::is_none")]
    pub output: Option<NodeVariant1Variant0Output>,
    #[serde(rename = "kind")]
    pub kind: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeVariant1Variant1Binding {
    #[serde(rename = "package")]
    pub package: String,
    #[serde(rename = "operation")]
    pub operation: String,
    #[serde(rename = "descriptorHash")]
    pub descriptor_hash: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeVariant1Variant1 {
    #[serde(rename = "name")]
    pub name: String,
    #[serde(rename = "summary")]
    pub summary: String,
    #[serde(
        rename = "successOutput",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub success_output: Option<String>,
    #[serde(
        rename = "failureOutput",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub failure_output: Option<String>,
    #[serde(
        rename = "runningHint",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub running_hint: Option<String>,
    #[serde(rename = "input", default, skip_serializing_if = "Option::is_none")]
    pub input: Option<std::collections::BTreeMap<String, serde_json::Value>>,
    #[serde(
        rename = "positionals",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub positionals: Option<Vec<String>>,
    #[serde(
        rename = "stdinField",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub stdin_field: Option<String>,
    #[serde(rename = "restField", default, skip_serializing_if = "Option::is_none")]
    pub rest_field: Option<String>,
    #[serde(rename = "output", default, skip_serializing_if = "Option::is_none")]
    pub output: Option<NodeVariant1Variant0Output>,
    #[serde(rename = "kind")]
    pub kind: String,
    #[serde(rename = "binding")]
    pub binding: NodeVariant1Variant1Binding,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum NodeVariant1 {
    Variant0(Box<NodeVariant1Variant0>),
    Variant1(Box<NodeVariant1Variant1>),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum Node {
    Variant0(Box<NodeVariant0>),
    Variant1(Box<NodeVariant1>),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestRootsValue {
    #[serde(rename = "tree")]
    pub tree: Node,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    #[serde(rename = "hash")]
    pub hash: String,
    #[serde(rename = "roots")]
    pub roots: std::collections::BTreeMap<String, ManifestRootsValue>,
    #[serde(rename = "packages")]
    pub packages: std::collections::BTreeMap<String, demi_command_protocol::PackageDescriptor>,
}
