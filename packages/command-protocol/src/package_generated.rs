// Generated from Zod contracts. Do not edit.

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageDescriptorTargetsValue {
    #[serde(rename = "sha256")]
    pub sha256: String,
    #[serde(rename = "size")]
    pub size: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageDescriptor {
    #[serde(rename = "id")]
    pub id: String,
    #[serde(rename = "version")]
    pub version: String,
    #[serde(rename = "protocolVersion")]
    pub protocol_version: u64,
    #[serde(rename = "operations")]
    pub operations: Vec<String>,
    #[serde(rename = "targets")]
    pub targets: std::collections::BTreeMap<String, PackageDescriptorTargetsValue>,
}

pub const TARGETS: &[&str] = &[
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-unknown-linux-musl",
    "x86_64-unknown-linux-musl",
    "aarch64-pc-windows-msvc",
    "x86_64-pc-windows-msvc",
];
