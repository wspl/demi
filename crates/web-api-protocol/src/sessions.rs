//! The account's signed-in browsers, verifying the session, and handing the
//! account to a phone by a code (`backend.md` § Authentication and ownership).

use std::borrow::Cow;

use demi_shared_types::Timestamp;
use garde::Validate;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

use crate::auth::Password;

demi_shared_types::id!(
    /// A session's id, which names it in the account's list; the token its
    /// cookie holds never leaves the browser it signed in.
    SessionId
);

demi_shared_types::id!(
    /// A handoff code's id, which the session the code made names.
    HandoffId
);

/// A signed-in browser of the account, as Settings › Account lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionDto {
    pub id: SessionId,
    /// The browser and system it signed in from, such as `Safari on iPhone`.
    pub name: String,
    /// When it signed in.
    pub created_at: Timestamp,
    /// When it last made a request, to the hour.
    pub last_active_at: Timestamp,
    /// Whether it is the session of the page that reads the list.
    pub current: bool,
    /// The handoff code that made it, if one did.
    pub handoff_id: Option<HandoffId>,
    /// The nickname of the administrator whose sign-in code made it, if one
    /// did: their email while they have no nickname.
    pub issued_by: Option<String>,
    /// Whether its browser has notifications on: it keeps a push subscription,
    /// which the session's end removes. A change of it is announced with the
    /// sessions.
    pub notifications: bool,
}

/// `GET /auth/sessions`: the account's sessions, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionList {
    pub sessions: Vec<SessionDto>,
}

/// `POST /auth/verify`: the account's password.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct VerifyPassword {
    #[garde(length(chars, min = 1))]
    pub password: Password,
}

/// A path of the product, such as `/chat/7f3a`: one slash, then anything
/// but a backslash, white space or a control character, so a browser
/// resolves it on the product's own origin and never on another site's
/// (`//evil.example`, `/\evil.example`, and a tab a URL parser drops).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ProductPath(String);

/// Why a text is not a path of the product.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("must be a path of this site, such as /chat")]
pub struct NotProductPath;

/// The longest path a code opens.
const PRODUCT_PATH_MAX: usize = 2048;

impl ProductPath {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ProductPath {
    type Error = NotProductPath;

    fn try_from(text: String) -> Result<Self, NotProductPath> {
        let mut after_slash = text.strip_prefix('/').ok_or(NotProductPath)?.chars();
        let fits = text.len() <= PRODUCT_PATH_MAX
            && after_slash.clone().next() != Some('/')
            && !after_slash.any(|c| c == '\\' || c.is_whitespace() || c.is_control());
        if !fits {
            return Err(NotProductPath);
        }
        Ok(Self(text))
    }
}

impl From<ProductPath> for String {
    fn from(path: ProductPath) -> Self {
        path.0
    }
}

impl JsonSchema for ProductPath {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "ProductPath".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string", "pattern": "^/", "maxLength": PRODUCT_PATH_MAX })
    }
}

/// `POST /auth/handoff/code`: the page the phone opens.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct HandoffRequest {
    /// A `ProductPath` is valid once it is decoded.
    #[garde(skip)]
    pub to: ProductPath,
}

/// `POST /auth/handoff/code`'s answer: the code's id, the URL its QR code
/// holds, with the code after `#`, and how long it lasts, which the page
/// counts from the answer's arrival, never comparing the server's clock with
/// its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HandoffCode {
    pub id: HandoffId,
    pub url: String,
    pub expires_in_ms: u32,
}

/// `POST /auth/handoff`: the code the phone's page read from its address.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct HandoffSpend {
    #[garde(length(min = 1, max = 128))]
    pub code: String,
}

/// `POST /auth/handoff`'s answer: the page to go to, signed in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HandoffAnswer {
    pub to: ProductPath,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_product_path_stays_on_the_products_origin() {
        for accepted in ["/", "/chat", "/chat/7f3a?panel=files#end", "/settings/account"] {
            assert!(ProductPath::try_from(accepted.to_owned()).is_ok(), "{accepted}");
        }
        for refused in [
            "",
            "chat",
            "//evil.example/chat",
            "/\\evil.example",
            "/\t/evil.example",
            "/ chat",
            "https://evil.example/chat",
            "javascript:alert(1)",
        ] {
            assert!(ProductPath::try_from(refused.to_owned()).is_err(), "{refused:?}");
        }
        assert!(ProductPath::try_from(format!("/{}", "a".repeat(PRODUCT_PATH_MAX))).is_err());
    }
}
