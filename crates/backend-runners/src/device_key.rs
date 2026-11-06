//! A device's value identity (`crates/backend-device-access/src/lib.rs`): whose a device handle is, the device it is on and the directory its
//! work starts in. Only this crate makes keys, and it makes a conversation's
//! only against a lease of that conversation's file gate
//! (`Devices::conversation_device`).

use demi_device_interface::DeviceKey;
use demi_web_api_protocol::ids::{ConversationId, DeviceId};

/// Whose a device handle is.
pub(crate) enum DeviceOwner<'a> {
    /// A conversation's primary or attached device, reached through the
    /// conversation's device access.
    Conversation(&'a ConversationId),
    /// Device access, which touches no conversation's files.
    DeviceAccess,
    /// Machine access: the user's Cloud itself, for work that is no
    /// conversation's.
    MachineAccess,
}

/// Handles with equal keys are the same device, the same owner on the same
/// device, starting work in the same directory.
pub(crate) fn device_key(device: &DeviceId, owner: DeviceOwner<'_>, cwd: &str) -> DeviceKey {
    match owner {
        DeviceOwner::Conversation(conversation) => {
            DeviceKey::new(format!("conversation {conversation} {device} {cwd}"))
        }
        DeviceOwner::DeviceAccess => DeviceKey::new(format!("device {device} {cwd}")),
        DeviceOwner::MachineAccess => DeviceKey::new(format!("machine {device} {cwd}")),
    }
}

/// The conversation a device key names, when it is a conversation's device.
/// Neither ids nor the owner word hold a space, so the key's second word is
/// the conversation.
pub fn conversation_of(key: &DeviceKey) -> Option<&str> {
    let mut words = key.as_str().splitn(3, ' ');
    match (words.next(), words.next()) {
        (Some("conversation"), Some(conversation)) => Some(conversation),
        _ => None,
    }
}

/// The device a conversation's device key names, its third word.
pub fn device_of(key: &DeviceKey) -> Option<&str> {
    let mut words = key.as_str().splitn(4, ' ');
    match (words.next(), words.next(), words.next()) {
        (Some("conversation"), Some(_), Some(device)) => Some(device),
        _ => None,
    }
}
