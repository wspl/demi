//! The page's question whether a socket is still alive, which every page
//! socket of the backend takes beside its own messages: the synchronization
//! channel, the conversation socket, the file watch and a device's signaling
//! socket. The backend answers each at once with its protocol's `pong` carrying
//! the same id.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// `{ ping: id }`: the page asks the far end to answer `pong` with `id`, as
/// it does for each socket that still looks open when it returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PagePing {
    pub ping: u32,
}
