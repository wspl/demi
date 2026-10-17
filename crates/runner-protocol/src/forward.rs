//! Port forwards between the user's Hosts (`port-forwarding.md`): what the
//! backend tells each runner of the forwards it is an end of, what the
//! Listen On runner announces of each, and a forwarded connection's channel
//! when two runners talk directly.

use std::time::Duration;

use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How long a forwarded connection's target has to accept it, and its two
/// relay pipes to both open, before the connection is closed as a refused
/// one is (`port-forwarding.md` § A port forward).
pub const DEADLINE: Duration = Duration::from_secs(10);
/// How long a pair's peer stays once the last connection between the pair
/// ended (`port-forwarding.md` § The path of a connection).
pub const PEER_IDLE: Duration = Duration::from_secs(60);

/// The waits before each new attempt at a direct peer while a pair is on
/// the relay: 1, 2 and 5 minutes, then every 10, as the page's
/// (`direct-channel.md` § Choosing the path).
pub const RETRY_WAITS: [Duration; 4] = [
    Duration::from_secs(60),
    Duration::from_secs(2 * 60),
    Duration::from_secs(5 * 60),
    Duration::from_secs(10 * 60),
];

/// The most characters of a Connect To address, `host:port`.
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

/// Why a pair of runners talks through the relay although it may talk
/// directly: the direct channel's reasons that two runners can meet
/// (`direct-channel.md` § What the user sees), the Listen On runner on the
/// page's side and the Connect To runner on the device's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PairReason {
    /// Every pair of addresses was checked and none answered, and both
    /// found their public address.
    Unreachable,
    /// One side found no public address: its network blocks UDP or STUN.
    Network { side: PairSide },
    /// The Connect To runner keeps as many peers as it may.
    Busy,
    /// A connected peer failed.
    Dropped,
    /// Crossing networks is off and no local pair answered.
    NotOffered,
}

/// A side of a pair, as the direct channel names a page's and a device's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum PairSide {
    /// The Listen On runner, which offers as a page does.
    Browser,
    /// The Connect To runner, which answers as a device does.
    Device,
}

/// The Listen On runner's first message on a forwarded connection's
/// channel: the forward it belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Validate)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum ForwardHeader {
    Forward {
        #[garde(length(min = 1))]
        forward: String,
    },
}

/// A credit on a channel that grants it (`port-forwarding.md` § Flow
/// control): the receiver lets the sender send `grant` bytes more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub grant: u32,
}
