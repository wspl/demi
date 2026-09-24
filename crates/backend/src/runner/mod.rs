//! Runner management (`runner.md`, `commands.md`): a runner's socket at the
//! edge, pairing and the pending claims, each user's devices with their
//! connections, the backend's rules for the calls a connection relays, each
//! agent node's commands for them, the command context the backend gives
//! work, and the file listings and text the product reads from a Host.

pub(crate) mod accept;
pub(crate) mod claims;
pub(crate) mod codes;
pub(crate) mod command_context;
pub(crate) mod devices;
pub(crate) mod files;
mod policy;
pub(crate) mod router;

use demi_shell::HostKey;
use demi_web_api::ids::{ConversationId, DeviceId};

/// Whose a Host handle is (`sessions-and-targets.md` § Every way to a Host).
pub(crate) enum HostOwner<'a> {
    /// A conversation's main or attached Host, reached through the
    /// conversation's host access.
    Conversation(&'a ConversationId),
    /// Device access, which touches no conversation's files.
    DeviceAccess,
}

/// A Host's value identity: handles with equal keys are the same Host, the
/// same owner on the same device, starting work in the same directory.
pub(crate) fn host_key(device: &DeviceId, owner: HostOwner<'_>, cwd: &str) -> HostKey {
    match owner {
        HostOwner::Conversation(conversation) => HostKey::new(format!("conversation {conversation} {device} {cwd}")),
        HostOwner::DeviceAccess => HostKey::new(format!("device {device} {cwd}")),
    }
}

/// The conversation a Host key names, when it is a conversation's Host.
/// Neither ids nor the owner word hold a space, so the key's second word is
/// the conversation.
pub(crate) fn conversation_of(key: &HostKey) -> Option<&str> {
    let mut words = key.as_str().splitn(3, ' ');
    match (words.next(), words.next()) {
        (Some("conversation"), Some(conversation)) => Some(conversation),
        _ => None,
    }
}

/// The device a conversation's Host key names, its third word.
pub(crate) fn device_of(key: &HostKey) -> Option<&str> {
    let mut words = key.as_str().splitn(4, ' ');
    match (words.next(), words.next(), words.next()) {
        (Some("conversation"), Some(_), Some(device)) => Some(device),
        _ => None,
    }
}
