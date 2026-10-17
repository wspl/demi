//! A request the backend makes for the user to one of the device's exposed
//! ports (`port-exposure.md`): the runner makes it to `127.0.0.1:<port>`,
//! or `https://localhost:<port>`, with its own HTTP client, and answers its
//! status and headers, the body travelling as pipes each way.

use std::time::Duration;

use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How long the runner's connection to the port may take to open: a port
/// nothing listens on refuses at once, and one that does not answer the
/// connection fails the request, as for an unreachable endpoint.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The most headers one request or answer carries.
pub const MAX_HEADERS: usize = 256;

/// Which scheme the address had: `http` reaches `127.0.0.1:<port>`,
/// `https` reaches `https://localhost:<port>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HttpScheme {
    Http,
    Https,
}

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
