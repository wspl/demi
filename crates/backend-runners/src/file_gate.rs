//! A conversation's file gate (`sessions-and-targets.md` § Host operations):
//! every operation on the conversation's Hosts holds a lease of it, and a
//! transition that needs the conversation's files quiet reserves it. A lease
//! names the gate's conversation, and a Host handle owned by a conversation
//! is given out only against one (`Devices::conversation_host`), so no Host
//! of a conversation exists outside an operation that holds its file gate.
//! The gates live in the conversations' slots of host access, which takes
//! their leases.

use demi_shared_gates::{ActivityGate, GateLease, Purpose};
use demi_web_api_protocol::ids::ConversationId;

/// The file gate of one conversation.
pub struct FileGate {
    conversation: ConversationId,
    gate: ActivityGate,
}

/// A lease of a conversation's file gate; dropping it releases the gate.
pub struct FileLease {
    conversation: ConversationId,
    _lease: GateLease,
}

impl FileGate {
    /// The file gate of `conversation`, as its record spells the id.
    pub fn new(conversation: ConversationId) -> Self {
        Self {
            conversation,
            gate: ActivityGate::new(),
        }
    }

    /// A lease for `purpose`, once no reservation holds the gate.
    pub async fn enter(&self, purpose: Purpose) -> FileLease {
        FileLease {
            conversation: self.conversation.clone(),
            _lease: self.gate.enter(purpose).await,
        }
    }

    /// The gate itself, which transitions reserve and idle watches read. A
    /// lease taken from it names no conversation.
    pub fn gate(&self) -> &ActivityGate {
        &self.gate
    }
}

impl FileLease {
    /// The conversation whose file gate this is a lease of.
    pub fn conversation(&self) -> &ConversationId {
        &self.conversation
    }
}
