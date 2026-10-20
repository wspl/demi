//! The CLI's own files in an account's directory
//! (`crates/provider-claude-code/src/account.rs`), as Claude Code 2.1.301
//! writes them: its sign-in in
//! `.credentials.json` (`claudeAiOauth`), and the account it signed in in
//! `.claude.json` (`oauthAccount`). Demi reads only the account of a new
//! sign-in, and writes both once, for the sign-in an earlier release kept
//! in an account's record.

use bytes::Bytes;
use demi_provider_common::Secret;
use demi_shared_types::Timestamp;
use serde::Deserialize;
use serde_json::json;

/// The file the CLI keeps its sign-in in, in its configuration directory.
pub(crate) const CREDENTIALS: &str = ".credentials.json";
/// The file the CLI keeps its account in there.
pub(crate) const CONFIG: &str = ".claude.json";

/// What Demi reads of the CLI's `.claude.json`: the signed-in account.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConfigFile {
    pub(crate) oauth_account: OauthAccount,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OauthAccount {
    pub(crate) account_uuid: String,
    #[serde(default)]
    pub(crate) email_address: Option<String>,
}

/// A sign-in as the CLI's files hold it, to write into a directory.
pub(crate) struct SignIn<'a> {
    pub(crate) access_token: &'a Secret,
    pub(crate) refresh_token: &'a Secret,
    pub(crate) expires_at: Timestamp,
    pub(crate) scopes: &'a [String],
    pub(crate) subscription_type: Option<&'a str>,
    pub(crate) account_id: &'a str,
    pub(crate) email: Option<&'a str>,
}

impl SignIn<'_> {
    /// The CLI's two files, the credentials last: their presence is what
    /// says a directory holds a sign-in, so they are put in place once the
    /// account's file is.
    pub(crate) fn files(&self) -> Vec<(String, Bytes)> {
        let mut tokens = json!({
            "accessToken": self.access_token.expose(),
            "refreshToken": self.refresh_token.expose(),
            "expiresAt": self.expires_at.as_millisecond(),
            "scopes": self.scopes,
        });
        if let Some(subscription) = self.subscription_type {
            tokens["subscriptionType"] = json!(subscription);
        }
        let mut account = json!({ "accountUuid": self.account_id });
        if let Some(email) = self.email {
            account["emailAddress"] = json!(email);
        }
        let file = |value: serde_json::Value| Bytes::from(value.to_string());
        vec![
            (CONFIG.to_owned(), file(json!({ "oauthAccount": account }))),
            (
                CREDENTIALS.to_owned(),
                file(json!({ "claudeAiOauth": tokens })),
            ),
        ]
    }
}
