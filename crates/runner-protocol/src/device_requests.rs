//! A request the backend makes for the user from the device, such as a
//! provider entry's that connects through it
//! (`crates/core/src/device_requests.rs`): the runner
//! makes it to its URL with its own HTTP client, as any program on the device
//! reaches an address, and answers its status and headers, the body
//! travelling as pipes each way.

use std::time::Duration;

use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How long the runner's connection to the address may take to open: a
/// port nothing listens on refuses at once, and a host that does not answer
/// the connection fails the request, as for an unreachable endpoint.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The most headers one request or answer carries: the HTTP/1 parser that
/// reads an answer holds its headers in an array of a fixed size.
pub const MAX_HEADERS: usize = 256;

/// One header of a request or an answer, its value as text: a header whose
/// value is not text is not carried.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct HttpHeader {
    #[garde(length(min = 1))]
    pub name: String,
    #[garde(skip)]
    pub value: String,
}
