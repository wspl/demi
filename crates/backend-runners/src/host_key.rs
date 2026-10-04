//! A Host's value identity (`sessions-and-targets.md` § Every way to a
//! Host): whose a Host handle is, the device it is on and the directory its
//! work starts in. Only this crate makes keys, and it makes a conversation's
//! only against a lease of that conversation's file gate
//! (`Devices::conversation_host`).

use demi_host_interface::HostKey;
use demi_web_api_protocol::ids::{ConversationId, DeviceId};

/// Whose a Host handle is.
pub(crate) enum HostOwner<'a> {
    /// A conversation's primary or attached Host, reached through the
    /// conversation's host access.
    Conversation(&'a ConversationId),
    /// Device access, which touches no conversation's files.
    DeviceAccess,
    /// Machine access: the user's Cloud itself, for work that is no
    /// conversation's.
    MachineAccess,
}

/// Handles with equal keys are the same Host, the same owner on the same
/// device, starting work in the same directory.
pub(crate) fn host_key(device: &DeviceId, owner: HostOwner<'_>, cwd: &str) -> HostKey {
    match owner {
        HostOwner::Conversation(conversation) => {
            HostKey::new(format!("conversation {conversation} {device} {cwd}"))
        }
        HostOwner::DeviceAccess => HostKey::new(format!("device {device} {cwd}")),
        HostOwner::MachineAccess => HostKey::new(format!("machine {device} {cwd}")),
    }
}

/// The conversation a Host key names, when it is a conversation's Host.
/// Neither ids nor the owner word hold a space, so the key's second word is
/// the conversation.
pub fn conversation_of(key: &HostKey) -> Option<&str> {
    let mut words = key.as_str().splitn(3, ' ');
    match (words.next(), words.next()) {
        (Some("conversation"), Some(conversation)) => Some(conversation),
        _ => None,
    }
}

/// The device a conversation's Host key names, its third word.
pub fn device_of(key: &HostKey) -> Option<&str> {
    let mut words = key.as_str().splitn(4, ' ');
    match (words.next(), words.next(), words.next()) {
        (Some("conversation"), Some(_), Some(device)) => Some(device),
        _ => None,
    }
}
