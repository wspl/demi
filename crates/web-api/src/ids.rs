//! Identifiers of the records the browser names.

use std::sync::LazyLock;

use regex::Regex;

demi_core::id!(
    /// A user account's id, which the backend assigns: like every identity, a
    /// nonempty string compared exactly.
    UserId
);

demi_core::id!(
    /// A provider entry's id, which the backend assigns.
    ProviderId
);

demi_core::id!(
    /// A subscription account's id within its entry, such as
    /// `cred-3f2a9c01d4e5b6a7`.
    CredentialId
);

demi_core::id!(
    /// A device login in progress, which the backend names when it starts.
    LoginId
);

demi_core::id!(
    /// A device's id, which the backend assigns when the device is paired or
    /// its Cloud is first used.
    DeviceId
);

demi_core::id!(
    /// A workspace's id, which the backend assigns.
    WorkspaceId
);

demi_core::id!(
    /// An upload's id, which the backend assigns and a frame names the
    /// upload by.
    AttachmentId
);

/// What a UUID looks like where the browser checks one: RFC 9562 versions 1
/// to 8, the nil UUID and the max UUID, in either case.
const UUID_PATTERN: &str = "^([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[1-8][0-9a-fA-F]{3}-[89abAB][0-9a-fA-F]{3}-[0-9a-fA-F]{12}|00000000-0000-0000-0000-000000000000|ffffffff-ffff-ffff-ffff-ffffffffffff)$";

/// Why a text is not a conversation id.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("must be a UUID")]
pub struct NotUuid;

fn uuid(text: &str) -> Result<(), NotUuid> {
    static PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(UUID_PATTERN).expect("the UUID pattern compiles"));
    if PATTERN.is_match(text) {
        Ok(())
    } else {
        Err(NotUuid)
    }
}

demi_core::id!(
    /// A conversation's id, which the browser chooses: a UUID, kept in the
    /// case it arrived in. No two conversations have ids that differ only in
    /// case (`storage.md` § Encodings and digests).
    ConversationId,
    check = uuid,
    error = NotUuid,
    schema = { "type": "string", "pattern": UUID_PATTERN }
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_conversation_id_is_a_uuid_in_the_case_it_arrived_in() {
        for accepted in [
            "0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b",
            "0B6F7F3E-8F3A-4C1E-9D2B-7A1C2E3F4A5B",
            "00000000-0000-0000-0000-000000000000",
            "ffffffff-ffff-ffff-ffff-ffffffffffff",
        ] {
            assert_eq!(ConversationId::try_from(accepted).unwrap().as_str(), accepted);
        }
        for refused in [
            "",
            "conversation-1",
            "0b6f7f3e-8f3a-0c1e-9d2b-7a1c2e3f4a5b",
            "0b6f7f3e-8f3a-4c1e-7d2b-7a1c2e3f4a5b",
            "0b6f7f3e8f3a4c1e9d2b7a1c2e3f4a5b",
            "../0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b",
        ] {
            assert_eq!(ConversationId::try_from(refused), Err(NotUuid), "{refused}");
        }
        let decoded: Result<ConversationId, _> = serde_json::from_str(r#""not-a-uuid""#);
        assert_eq!(decoded.unwrap_err().to_string(), "must be a UUID");
    }
}
