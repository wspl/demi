//! What the product shows about a provider entry besides its catalog: whether
//! its credential is usable, whether it can run, and its subscription
//! accounts (`providers.md` § Provider contract, § Credential vault). The
//! backend reads these when it answers and never stores them, so they accept
//! unknown fields.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use crate::{Nullable, Timestamp};

/// Whether a provider's credential is present and usable. Reading it never
/// makes an inference request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AuthState {
    Unknown {
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        message: Option<String>,
    },
    Authenticated {
        /// The account the credential acts for, when the provider can name
        /// it.
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        account_label: Option<String>,
    },
    Unauthenticated {
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        message: Option<String>,
    },
    Error {
        message: String,
    },
}

/// Whether a provider can run requests at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum RuntimeState {
    Unknown {
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        message: Option<String>,
    },
    Ready {
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        message: Option<String>,
    },
    Unavailable {
        message: String,
    },
    Error {
        message: String,
    },
}

/// A subscription account's public metadata; never token material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    /// The account's id within its entry.
    pub id: String,
    /// What the user knows the account by, such as an email address.
    pub label: String,
    /// A second line, such as the plan or the issuer.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub detail: Option<String>,
    /// When the account was last stored or refreshed.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<Timestamp>")]
    pub updated_at: Option<Timestamp>,
}

/// What a device login asks the user to do, reported once while the login
/// waits (`providers.md` § Login and publication).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoginPending {
    /// The address the user opens to confirm the login.
    pub verification_url: String,
    /// The one-time code the user enters there; null when the address
    /// carries it.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub user_code: Option<String>,
    /// When the code expires, when the vendor says.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<Timestamp>")]
    pub expires_at: Option<Timestamp>,
}
