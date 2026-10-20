//! The user's exposed ports (`crates/backend-user-shard/src/shard/exposed_ports.rs`): what a device's page in
//! Settings › Devices and `demi host expose` read and send, and the address
//! the backend's requests reach an exposed port at.

use demi_shared_types::{Nullable, Timestamp};
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::{double_option, unwrap_or_skip};

use crate::ids::{DeviceId, ExposedPortId};
use crate::text::Trimmed;

/// The most characters of an exposed port's label, after trimming.
pub const LABEL_MAX: usize = 64;

/// The address the backend's requests for the user reach `port` of
/// `device` at: the device's id is its host (`crates/backend-user-shard/src/shard/exposed_ports.rs`).
pub fn exposed_address(device: &DeviceId, port: u16) -> String {
    format!("http://{device}:{port}")
}

/// A port of one of the user's Hosts that the backend's requests for the
/// user may reach.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExposedPortDto {
    pub id: ExposedPortId,
    pub device_id: DeviceId,
    /// Null until the user enters it: the row exposes nothing meanwhile.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<u16>")]
    pub port: Option<u16>,
    /// Empty for none.
    pub label: String,
    /// `http://<device id>:<port>`, null while the port is missing.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub address: Option<String>,
    pub created_at: Timestamp,
}

/// `GET /exposed-ports`: the user's exposed ports, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExposedPortList {
    pub ports: Vec<ExposedPortDto>,
}

/// The answer of a create and a change: the row as it stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExposedPortAnswer {
    pub port: ExposedPortDto,
}

/// `POST /exposed-ports`: Expose Port on a device's page, which adds a row
/// with the port missing, or Expose Port on a failed request, which names
/// the port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NewExposedPort {
    #[garde(skip)]
    pub device_id: DeviceId,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "u16", range(min = 1))]
    #[garde(inner(range(min = 1)))]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Trimmed", length(max = LABEL_MAX))]
    #[garde(inner(length(chars, max = LABEL_MAX)))]
    pub label: Option<Trimmed>,
}

/// `PATCH /exposed-ports/:id`: what changes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExposedPortPatch {
    /// Null leaves the port to be entered.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "double_option")]
    #[schemars(with = "Nullable<u16>", range(min = 1))]
    #[garde(inner(inner(range(min = 1))))]
    pub port: Option<Option<u16>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Trimmed", length(max = LABEL_MAX))]
    #[garde(inner(length(chars, max = LABEL_MAX)))]
    pub label: Option<Trimmed>,
}

/// The words a refused body says to the user, who reads them as they are.
pub fn refusal(report: &garde::Report) -> String {
    let mut sentences: Vec<String> = Vec::new();
    for (path, error) in report.iter() {
        let sentence = match path.to_string().as_str() {
            "port" => "Enter a port from 1 to 65535.".to_owned(),
            "label" => format!("Enter a label of up to {LABEL_MAX} characters."),
            _ => error.message().to_owned(),
        };
        if !sentences.contains(&sentence) {
            sentences.push(sentence);
        }
    }
    sentences.join(" ")
}

/// The words that refuse a port another row of the device exposes.
pub fn port_taken(port: u16) -> String {
    format!("Port {port} is already exposed.")
}
