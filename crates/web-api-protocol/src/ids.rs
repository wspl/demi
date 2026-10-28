//! Identifiers of the records the web app names.

use std::sync::LazyLock;

use regex::Regex;

demi_shared_types::id!(
    /// A user account's id, which the backend assigns: like every identity, a
    /// nonempty string compared exactly.
    UserId
);

demi_shared_types::id!(
    /// A provider entry's id, which the backend assigns.
    ProviderId
);

demi_shared_types::id!(
    /// A subscription account's id within its entry, such as
    /// `cred-3f2a9c01d4e5b6a7`.
    CredentialId
);

demi_shared_types::id!(
    /// One of a user's credentials, which the backend assigns and its
    /// name can change under.
    UserCredentialId
);

demi_shared_types::id!(
    /// One of a user's exposed ports (`crates/backend-user-shard/src/shard/exposed_ports.rs`), which the
    /// backend assigns.
    ExposedPortId
);

demi_shared_types::id!(
    /// One of a user's port forwards (`crates/backend-user-shard/src/shard/port_forwards.rs`), which the
    /// backend assigns and its name can change under.
    PortForwardId
);

demi_shared_types::id!(
    /// A device login in progress, which the backend names when it starts.
    LoginId
);

/// A device's id, declared where the socket's frames can name it too.
pub use demi_shared_types::DeviceId;

demi_shared_types::id!(
    /// A project's id, which the backend assigns.
    ProjectId
);

demi_shared_types::id!(
    /// A subagent profile's id, which the backend assigns and the profile's
    /// name can change under.
    ProfileId
);

demi_shared_types::id!(
    /// A share's token: 128 random bits in URL-safe base64, the only key to the
    /// share.
    ShareToken
);

demi_shared_types::id!(
    /// An upload's id, which the backend assigns and a frame names the
    /// upload by.
    AttachmentId
);

/// What a UUID looks like where the web app checks one: RFC 9562 versions 1
/// to 8, the nil UUID and the max UUID, in either case.
const UUID_PATTERN: &str = "^([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[1-8][0-9a-fA-F]{3}-[89abAB][0-9a-fA-F]{3}-[0-9a-fA-F]{12}|00000000-0000-0000-0000-000000000000|ffffffff-ffff-ffff-ffff-ffffffffffff)$";

/// Why a text is not a conversation id.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("must be a UUID")]
pub struct NotUuid;

fn uuid(text: &str) -> Result<(), NotUuid> {
    static PATTERN: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(UUID_PATTERN).expect("the UUID pattern compiles"));
    if PATTERN.is_match(text) {
        Ok(())
    } else {
        Err(NotUuid)
    }
}

demi_shared_types::id!(
    /// A conversation's id, which the web app chooses: a UUID, kept in the
    /// case it arrived in. No two conversations have ids that differ only in
    /// case (`crates/backend-database/src/columns.rs`).
    ConversationId,
    check = uuid,
    error = NotUuid,
    schema = { "type": "string", "pattern": UUID_PATTERN }
);

demi_shared_types::id!(
    /// A message saved for later's id, which the page chooses: a UUID. A save
    /// sent again with the same id and content is the same save.
    SavedMessageId,
    check = uuid,
    error = NotUuid,
    schema = { "type": "string", "pattern": UUID_PATTERN }
);

demi_shared_types::id!(
    /// A Cloud reset's id, which the page chooses: a UUID. A retry with the
    /// same id is the same reset (`crates/backend-cloud/src/reset.rs`).
    OperationId,
    check = uuid,
    error = NotUuid,
    schema = { "type": "string", "pattern": UUID_PATTERN }
);
