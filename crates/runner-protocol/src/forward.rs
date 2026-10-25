//! Port forwards between the user's Hosts (`crates/backend-user-shard/src/shard/port_forwards.rs`): what the
//! backend tells each runner of the forwards it is an end of, and what the
//! Listen On runner announces of each. A forwarded connection's channel when
//! two runners talk directly is a pair's ([`crate::pairs`]).

use std::time::Duration;

use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::pairs::PairReason;

/// How long a forwarded connection's target has to accept it, and its two
/// relay pipes to both open, before the connection is closed as a refused
/// one is (`crates/backend-user-shard/src/shard/port_forwards.rs`).
pub const DEADLINE: Duration = Duration::from_secs(10);
/// The most characters of a Connect To address, `host:port`: a host name is at
/// most 253 characters (RFC 1035) and a port five.
pub const ADDRESS_CHARS: usize = 260;

/// A forward the runner is an end of, as the backend tells it: whether it
/// listens, on which port, and whether it connects, to which address. A
/// forward between one Host and itself does both.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Forward {
    #[garde(length(min = 1))]
    pub id: String,
    /// The port this runner listens on, on `127.0.0.1`; none when another
    /// Host listens.
    #[garde(inner(range(min = 1)))]
    pub listen: Option<u16>,
    /// The address this runner connects each connection to; none when
    /// another Host connects.
    #[garde(inner(length(min = 1, max = ADDRESS_CHARS)))]
    pub connect: Option<String>,
    /// The device at the forward's other end, this one's own for a forward
    /// between a Host and itself.
    #[garde(length(min = 1))]
    pub peer: String,
    /// Whether that device's runner is connected now.
    #[garde(skip)]
    pub peer_online: bool,
    /// Whether the two runners may talk directly: neither is a Cloud and
    /// neither device's route is Relay Only.
    #[garde(skip)]
    pub direct: bool,
}

/// What the Listen On runner announces of a forward as it changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ForwardRunState {
    /// It listens, with this many connections open on each path, and why
    /// its connections go through the relay when a direct path failed.
    Listening {
        direct: u32,
        relay: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<PairReason>,
    },
    /// Another program, or another forward, holds the port.
    PortInUse,
}
