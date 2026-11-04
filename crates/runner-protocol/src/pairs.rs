//! Two runners of one user that talk directly (`crates/runner-direct/src/pairs.rs`):
//! the pair's one WebRTC peer, which either runner may have offered and on
//! which either opens channels, and what each channel carries: a forwarded
//! connection (`crate::forward`), or a pipe's bytes between two moves
//! (`crates/runner-protocol/src/wire/pipe_frames.rs`).

use std::time::Duration;

use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How long a pair's peer stays once the last connection between the pair
/// ended (`crates/runner-direct/src/pairs.rs`).
pub const PEER_IDLE: Duration = Duration::from_secs(60);

/// The waits before each new attempt at a direct peer while a pair is on
/// the relay: 1, 2 and 5 minutes, then every 10, as the page's
/// (`packages/web/src/direct/index.ts`).
pub const RETRY_WAITS: [Duration; 4] = [
    Duration::from_secs(60),
    Duration::from_secs(2 * 60),
    Duration::from_secs(5 * 60),
    Duration::from_secs(10 * 60),
];

/// Why a pair of runners talks through the relay although it may talk
/// directly: the direct channel's reasons that two runners can meet
/// (`packages/web-ui/src/devices/direct.ts`), the runner that offered on the
/// page's side and the one that answered on the device's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PairReason {
    /// Every pair of addresses was checked and none answered, and both
    /// found their public address.
    Unreachable,
    /// One side found no public address: its network blocks UDP or STUN.
    Network { side: PairSide },
    /// The runner that answers keeps as many peers as it may.
    Busy,
    /// The peer connected, and its measurement found it worse than the
    /// relay (`crates/runner-direct/src/measure.rs`).
    Slower,
    /// A connected peer failed.
    Dropped,
    /// Crossing networks is off and no local pair answered.
    NotOffered,
}

/// A side of a pair, as the direct channel names a page's and a device's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum PairSide {
    /// The runner that offered the peer, as a page does.
    Browser,
    /// The runner that answered, as a device does.
    Device,
}

/// The first message on a channel one runner of a pair opens: what the
/// channel carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Validate)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum PairHeader {
    /// A connection of the forward that the opening runner listens for.
    Forward {
        #[garde(length(min = 1))]
        forward: String,
    },
    /// The bytes of pipe `pipe`, whose source is the opening runner, from
    /// byte `at` on, until the channel's `{ end: true }`.
    Pipe {
        #[garde(length(min = 1))]
        pipe: String,
        #[garde(skip)]
        at: u64,
    },
    /// The verdict of the runner that measured the pair's peer, the one
    /// whose device id sorts first: the other runner uses the peer only
    /// once it hears `slower: false`, and the measuring one closes it on
    /// `true` (`crates/runner-direct/src/pairs.rs`).
    Measured {
        #[garde(skip)]
        slower: bool,
    },
}

/// How a pair of runners may talk, from their two devices' routes: the
/// backend says it with each forward and each pipe between them, and with
/// each offer of a peer one runner makes the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum PairRoute {
    /// Through the relay only: an end is a Cloud, which runs beside the
    /// backend, or a device whose route is Relay Only.
    Relay,
    /// Directly once the peer's measurement finds it no worse than the
    /// relay, which carries the pair meanwhile.
    Automatic,
    /// Directly once the peer connects, unmeasured: a device's route is
    /// Prefer P2P.
    Direct,
}

/// A credit on a channel that grants it (`crates/runner-direct/src/channel.rs`):
/// the receiver lets the sender send `grant` bytes more. On a pipe's channel it
/// also confirms that the receiver took them, which the sender then never sends
/// again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub grant: u32,
}
