//! The user's port forwards (`crates/backend-user-shard/src/shard/port_forwards.rs`): what Settings › Port
//! Forwarding and `demi host forward` read and send, and the rules a name
//! and an address follow wherever they arrive.

use demi_runner_protocol::forward::{ADDRESS_CHARS, ForwardRunState};
use demi_shared_types::{Nullable, Timestamp};
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::{double_option, unwrap_or_skip};

use crate::ids::{DeviceId, PortForwardId};
use crate::text::Trimmed;

/// The most characters of a forward's name, after trimming.
pub const NAME_MAX: usize = 64;
/// A new forward's name, and the stem of the second's, *Forward (2)*.
pub const DEFAULT_NAME: &str = "Forward";
/// The port a new forward listens on, unless another of the user's
/// forwards on that Host uses it.
pub const DEFAULT_PORT: u16 = 8080;
/// The address a new forward connects to.
pub const DEFAULT_ADDRESS: &str = "localhost:11434";

/// Where a forward connects on its Connect To Host: `host:port`, a name,
/// an IPv4 address or an IPv6 address in brackets, and a port from 1 to
/// 65535.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ForwardAddress(String);

/// Why text is no `host:port`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Enter an address as host:port, such as localhost:11434.")]
pub struct NotAnAddress;

impl ForwardAddress {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ForwardAddress {
    type Error = NotAnAddress;

    fn try_from(address: String) -> Result<Self, NotAnAddress> {
        let address = address.trim().to_owned();
        if address.chars().count() > ADDRESS_CHARS {
            return Err(NotAnAddress);
        }
        let (host, port) = address.rsplit_once(':').ok_or(NotAnAddress)?;
        let host_ok = match host.strip_prefix('[').and_then(|host| host.strip_suffix(']')) {
            Some(bracketed) => bracketed.parse::<std::net::Ipv6Addr>().is_ok(),
            None => {
                !host.is_empty()
                    && host
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
            }
        };
        let port_ok = port.parse::<u16>().is_ok_and(|port| port != 0);
        if !host_ok || !port_ok {
            return Err(NotAnAddress);
        }
        Ok(Self(address))
    }
}

impl std::str::FromStr for ForwardAddress {
    type Err = NotAnAddress;

    fn from_str(address: &str) -> Result<Self, NotAnAddress> {
        Self::try_from(address.to_owned())
    }
}

impl From<ForwardAddress> for String {
    fn from(address: ForwardAddress) -> Self {
        address.0
    }
}

impl std::fmt::Display for ForwardAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl JsonSchema for ForwardAddress {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ForwardAddress".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "type": "string", "minLength": 3, "maxLength": ADDRESS_CHARS })
    }
}

/// A forward as the page and `demi host forward list` see it: its fields,
/// and what its Listen On runner last announced of it, null while that
/// runner has said nothing, as when it is offline or the forward does not
/// listen (`crates/runner-protocol/src/forward.rs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PortForwardDto {
    pub id: PortForwardId,
    pub name: String,
    pub listen_device: DeviceId,
    pub listen_port: u16,
    /// Null until the user chooses the Host it connects on.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<DeviceId>")]
    pub connect_device: Option<DeviceId>,
    pub connect_address: ForwardAddress,
    pub enabled: bool,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<ForwardRunState>")]
    pub state: Option<ForwardRunState>,
    pub created_at: Timestamp,
}

/// `GET /port-forwards`: the user's forwards, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PortForwardList {
    pub forwards: Vec<PortForwardDto>,
}

/// The answer of a create and a change: the forward as it stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PortForwardAnswer {
    pub forward: PortForwardDto,
}

/// `POST /port-forwards` and `PATCH /port-forwards/:id`: what changes; a
/// new forward starts from its defaults, *Forward*, on, listening on the
/// first of the user's Hosts on port 8080 or the next port none of the
/// user's forwards there uses, connecting to `localhost:11434` on no Host
/// yet, and takes these over them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PortForwardPatch {
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Trimmed", length(min = 1, max = NAME_MAX))]
    #[garde(inner(length(chars, min = 1, max = NAME_MAX)))]
    pub name: Option<Trimmed>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "DeviceId")]
    #[garde(skip)]
    pub listen_device: Option<DeviceId>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "u16", range(min = 1))]
    #[garde(inner(range(min = 1)))]
    pub listen_port: Option<u16>,
    /// Null leaves it to be chosen.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "double_option")]
    #[schemars(with = "Nullable<DeviceId>")]
    #[garde(skip)]
    pub connect_device: Option<Option<DeviceId>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "ForwardAddress")]
    #[garde(skip)]
    pub connect_address: Option<ForwardAddress>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "bool")]
    #[garde(skip)]
    pub enabled: Option<bool>,
}

/// The words a refused forward body says to the user, who reads them as
/// they are.
pub fn refusal(report: &garde::Report) -> String {
    let mut sentences: Vec<String> = Vec::new();
    for (path, error) in report.iter() {
        let sentence = match path.to_string().as_str() {
            "name" => format!("Enter a name of up to {NAME_MAX} characters."),
            "listenPort" | "listen_port" => "Enter a port from 1 to 65535.".to_owned(),
            _ => error.message().to_owned(),
        };
        if !sentences.contains(&sentence) {
            sentences.push(sentence);
        }
    }
    sentences.join(" ")
}

/// The words that refuse a name another of the user's forwards has.
pub fn name_taken(name: &str) -> String {
    format!("A forward named {name} already exists")
}
