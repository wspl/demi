//! Setup, sign-in and the caller's own account (`web-api.md` § Account API).

use std::borrow::Cow;
use std::fmt;

use demi_core::Timestamp;
use garde::Validate;
use garde::rules::length::chars::HasChars;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

use crate::ids::UserId;
use crate::text::{EmailAddress, Trimmed};

/// An account's role (`product.md` § User system). A role administers only
/// the roles below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The instance's first account, created by setup.
    Master,
    Admin,
    User,
}

serde_plain::derive_display_from_serialize!(Role);
serde_plain::derive_fromstr_from_deserialize!(Role);

/// An account as the browser sees it; its password hash never leaves the
/// backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserDto {
    pub id: UserId,
    pub email: EmailAddress,
    /// Empty until the user chooses one.
    pub nickname: String,
    pub role: Role,
    pub created_at: Timestamp,
}

/// `{ user }`: the answer of setup, login, `GET/PATCH /auth/me` and a
/// confirmed email change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Identity {
    pub user: UserDto,
}

/// `GET /setup`: whether the instance still needs its master account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SetupStatus {
    pub needed: bool,
}

/// A password as a request carries it. `Debug` never shows it.
#[derive(Clone, Deserialize)]
#[serde(transparent)]
pub struct Password(String);

impl Password {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl From<String> for Password {
    fn from(password: String) -> Self {
        Self(password)
    }
}

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Password(..)")
    }
}

impl HasChars for Password {
    fn num_chars(&self) -> usize {
        self.0.chars().count()
    }
}

/// Inline, so each field's garde bounds apply to its schema.
impl JsonSchema for Password {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "Password".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string" })
    }
}

/// `POST /setup`: the master account's address and password.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct SetupRequest {
    /// An `EmailAddress` is valid once it is decoded.
    #[garde(skip)]
    pub email: EmailAddress,
    #[garde(length(chars, min = 8, max = 1024))]
    pub password: Password,
}

/// `POST /auth/login`.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct Credentials {
    /// An `EmailAddress` is valid once it is decoded.
    #[garde(skip)]
    pub email: EmailAddress,
    #[garde(length(chars, min = 1))]
    pub password: Password,
}

/// `PATCH /auth/me`.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct NicknamePatch {
    #[garde(length(chars, min = 1, max = 80))]
    pub nickname: Trimmed,
}

/// `PUT /auth/password`: the current password and the next one.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct PasswordChange {
    #[garde(length(chars, min = 1))]
    pub current: Password,
    #[garde(length(chars, min = 8, max = 1024))]
    pub next: Password,
}

/// `POST /auth/email`: the new address and the current password.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct EmailChangeStart {
    /// An `EmailAddress` is valid once it is decoded.
    #[garde(skip)]
    pub email: EmailAddress,
    #[garde(length(chars, min = 1))]
    pub password: Password,
}

/// A pending email change; its code went to `email` and never appears here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EmailChallengeDto {
    pub id: String,
    pub email: EmailAddress,
    pub expires_at: Timestamp,
}

/// The 202 answer of `POST /auth/email`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EmailChangeStarted {
    pub challenge: EmailChallengeDto,
}

/// `POST /auth/email/confirm`: the challenge and the six-digit code the new
/// address received.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct EmailChangeConfirm {
    #[garde(length(min = 1))]
    pub id: String,
    #[garde(pattern(r"^[0-9]{6}$"))]
    pub code: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_travels_in_camel_case_with_its_role_as_a_word() {
        let user = UserDto {
            id: UserId::try_from("u-1".to_owned()).unwrap(),
            email: EmailAddress::try_from("ana@example.test".to_owned()).unwrap(),
            nickname: String::new(),
            role: Role::Master,
            created_at: Timestamp::from_millisecond(1_790_000_000_000).unwrap(),
        };
        assert_eq!(
            serde_json::to_value(&user).unwrap(),
            serde_json::json!({
                "id": "u-1",
                "email": "ana@example.test",
                "nickname": "",
                "role": "master",
                "createdAt": "2026-09-21T14:13:20.000Z",
            })
        );
        assert_eq!("admin".parse::<Role>().unwrap(), Role::Admin);
    }

    #[test]
    fn a_verification_code_is_six_ascii_digits() {
        let confirm = |code: &str| {
            serde_json::from_value::<EmailChangeConfirm>(serde_json::json!({ "id": "c", "code": code }))
                .unwrap()
                .validate()
        };
        assert!(confirm("012345").is_ok());
        for refused in ["abcdef", "12345", "1234567", "١٢٣٤٥٦", "12 456"] {
            assert!(confirm(refused).is_err(), "{refused}");
        }
    }

    #[test]
    fn a_request_schema_is_strict_with_its_bounds_and_a_response_schema_tolerant() {
        let schema = |value: schemars::Schema| serde_json::to_value(value).unwrap();
        let setup = schema(schemars::schema_for!(SetupRequest));
        assert_eq!(setup["additionalProperties"], false);
        assert_eq!(setup["properties"]["email"]["format"], "email");
        assert_eq!(setup["properties"]["password"]["minLength"], 8);
        assert_eq!(setup["properties"]["password"]["maxLength"], 1024);
        let nickname = schema(schemars::schema_for!(NicknamePatch));
        assert_eq!(nickname["properties"]["nickname"]["maxLength"], 80);
        let confirm = schema(schemars::schema_for!(EmailChangeConfirm));
        assert_eq!(confirm["properties"]["code"]["pattern"], "^[0-9]{6}$");
        let user = schema(schemars::schema_for!(UserDto));
        assert_ne!(user["additionalProperties"], false);
        assert_eq!(user["properties"]["createdAt"]["format"], "date-time");
    }

    #[test]
    fn a_password_never_shows_in_debug_output() {
        let credentials: Credentials =
            serde_json::from_value(serde_json::json!({ "email": "ana@example.test", "password": "hunter22" }))
                .unwrap();
        assert!(!format!("{credentials:?}").contains("hunter22"));
    }
}
