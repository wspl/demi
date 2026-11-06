//! A plugin's requests over a wire, as a program's plugin takes them on a
//! lane of each core's connection (`crates/core-api/src/plugin_lanes.rs`).
//! Each message names the request it belongs to; a port message and its
//! answer name the port message too.
//!
//! ```text
//! host -> plugin   { request: 7, call: Request }
//! plugin -> host   { request: 7, port: 1, message: PortMessage }
//! host -> plugin   { request: 7, port: 1, answer: PortAnswer }
//!                  { request: 7, port: 1, error: "..." }  or no answer
//! host -> plugin   { request: 7, cancel: true }
//! plugin -> host   { request: 7, reply: Reply }  or  { request: 7, error }
//! ```
//!
//! A port message may follow its request's reply, as a page call's task
//! keeps its port ([`crate::Plugin`]).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use demi_shared_types::MAX_SAFE_INTEGER;

use crate::{PluginError, PortAnswer, PortMessage, Reply, Request};

/// What the plugin host sends.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged, deny_unknown_fields)]
pub enum HostMessage {
    /// A request, by its number.
    Call {
        #[schemars(range(max = MAX_SAFE_INTEGER))]
        request: u64,
        call: Request,
    },
    /// The answer to the request's port message `port`.
    Answer {
        #[schemars(range(max = MAX_SAFE_INTEGER))]
        request: u64,
        #[schemars(range(max = MAX_SAFE_INTEGER))]
        port: u64,
        answer: PortAnswer,
    },
    /// The request's port message `port` got no answer, for the reason
    /// `error` says: the request is over, or the host failed.
    Unanswered {
        #[schemars(range(max = MAX_SAFE_INTEGER))]
        request: u64,
        #[schemars(range(max = MAX_SAFE_INTEGER))]
        port: u64,
        error: String,
    },
    /// The request is stopped: the plugin ends its work and replies.
    Cancel {
        #[schemars(range(max = MAX_SAFE_INTEGER))]
        request: u64,
        cancel: bool,
    },
}

/// What the plugin sends.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged, deny_unknown_fields)]
pub enum PluginMessage {
    /// The request's reply.
    Reply {
        #[schemars(range(max = MAX_SAFE_INTEGER))]
        request: u64,
        reply: Reply,
    },
    /// The request failed.
    Error {
        #[schemars(range(max = MAX_SAFE_INTEGER))]
        request: u64,
        error: PluginError,
    },
    /// A message of the request's port, by its number among the request's.
    Port {
        #[schemars(range(max = MAX_SAFE_INTEGER))]
        request: u64,
        #[schemars(range(max = MAX_SAFE_INTEGER))]
        port: u64,
        message: PortMessage,
    },
}
