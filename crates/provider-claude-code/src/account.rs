//! A Claude Code account (`providers.md` § Subscription secrets, § Token
//! refresh; `claude-code.md` § Accounts and sign-in): its secret document,
//! what the CLI's own sign-in wrote; how the account names itself; and the
//! refresh of its tokens by the one protocol every family follows, which
//! only the backend runs.

use std::sync::Arc;
use std::time::Duration;

use demi_provider_common::{Secret, json_body};
use demi_provider_common::credentials::{
    AccountDocument, AccountError, AccountLabel, AuthFailure, AuthReason, CredentialPool,
    RenewError, SecretDocument, read_secret, renew,
};
use demi_provider_common::oauth::{Lifetime, decode_json_response};
use demi_shared_types::{Clock, Timestamp};
use http::header::{ACCEPT, CONTENT_TYPE};
use reqwest::Url;
use serde::{Deserialize, Serialize};

use crate::FAMILY;

/// The CLI's own OAuth client, whose tokens the CLI's sign-in issues
/// (Claude Code 2.1.294).
pub(crate) const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";

/// How long before its expiry a token is refreshed, and a process holding
/// it replaced: a request may run for long, and the CLI cannot be given a
/// fresh token while it runs (`claude-code.md` § Accounts and sign-in).
const EXPIRY_SKEW: Duration = Duration::from_secs(30 * 60);

/// The `claude-code` family's secret document: what the CLI's sign-in wrote,
/// its tokens and their expiry, the scopes granted and the subscription
/// type, with the account's ID and email. Demi defines it; it is not the
/// CLI's own credentials file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ClaudeSecret {
    pub(crate) access_token: Secret,
    pub(crate) refresh_token: Secret,
    /// When the access token expires.
    pub(crate) expires_at: Timestamp,
    pub(crate) scopes: Vec<String>,
    /// Such as `max`; none when the sign-in did not say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) subscription_type: Option<String>,
    /// The account's ID, which tells it apart from the entry's others.
    pub(crate) account_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) email: Option<String>,
}

impl SecretDocument for ClaudeSecret {}

impl ClaudeSecret {
    /// How the account names itself: its email, else its ID, with its
    /// subscription type; its ID tells it apart.
    pub(crate) fn label(&self) -> AccountLabel {
        AccountLabel {
            label: self
                .email
                .clone()
                .unwrap_or_else(|| self.account_id.clone()),
            detail: self.subscription_type.clone(),
            identity_key: Some(format!("account:{}", self.account_id)),
        }
    }

    fn expiring(&self, now: Timestamp) -> bool {
        expiring(self.expires_at, now)
    }
}

/// Whether a token that expires at `expires_at` expires within thirty
/// minutes of `now`.
pub(crate) fn expiring(expires_at: Timestamp, now: Timestamp) -> bool {
    let skew = i64::try_from(EXPIRY_SKEW.as_millis()).unwrap_or(i64::MAX);
    expires_at.as_millisecond() - now.as_millisecond() <= skew
}

/// The account a provider stands for, and how its tokens are refreshed.
pub(crate) struct ClaudeAuth {
    pool: Arc<dyn CredentialPool>,
    account: Option<String>,
    /// The CLI's token endpoint, `https://platform.claude.com/v1/oauth/token`
    /// in the product.
    token_url: Url,
    clock: Arc<dyn Clock>,
}

impl ClaudeAuth {
    pub(crate) fn new(
        pool: Arc<dyn CredentialPool>,
        account: Option<String>,
        token_url: Url,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            pool,
            account,
            token_url,
            clock,
        }
    }

    fn document(&self) -> Result<Box<dyn AccountDocument>, AuthFailure> {
        let account = self
            .account
            .as_deref()
            .ok_or(AuthFailure::new(FAMILY, AuthReason::Missing))?;
        Ok(self.pool.document(account))
    }

    /// The account's stored sign-in, as it is.
    pub(crate) async fn stored(&self) -> Result<ClaudeSecret, AuthFailure> {
        let doc = self.document()?;
        read_secret::<ClaudeSecret>(&*doc)
            .await
            .map(|stored| stored.secret)
            .map_err(failure)
    }

    /// The account's sign-in, refreshed first when its access token expires
    /// within thirty minutes, or when it is still the one the vendor
    /// `refused`. A refresh that waited behind another uses the other's
    /// tokens unless they expire as soon.
    pub(crate) async fn credentials(
        &self,
        http: &reqwest::Client,
        refused: Option<&Secret>,
    ) -> Result<ClaudeSecret, AuthFailure> {
        let doc = self.document()?;
        let due = |secret: &ClaudeSecret| {
            refused.is_some_and(|token| secret.access_token == *token)
                || secret.expiring(self.clock.now())
        };
        renew(&*doc, due, |secret| self.refresh(http, secret))
            .await
            .map_err(|error| match error {
                RenewError::Account(error) => failure(error),
                RenewError::Refresh(message) => {
                    AuthFailure::new(FAMILY, AuthReason::Refresh(message))
                }
            })
    }

    /// Asks the CLI's token endpoint for new tokens with a `refresh_token`
    /// grant, the CLI's client and the scopes the sign-in has, as the CLI
    /// does. A refreshed sign-in keeps the refresh token and the scopes the
    /// answer does not replace.
    async fn refresh(
        &self,
        http: &reqwest::Client,
        secret: ClaudeSecret,
    ) -> Result<ClaudeSecret, String> {
        let grant = RefreshGrant {
            grant_type: "refresh_token",
            refresh_token: secret.refresh_token.expose(),
            client_id: CLIENT_ID,
            scope: secret.scopes.join(" "),
        };
        let response = http
            .post(self.token_url.clone())
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "application/json")
            .body(json_body(&grant))
            .send()
            .await
            .map_err(|error| format!("Claude token refresh failed: {}", error.without_url()))?;
        if !response.status().is_success() {
            return Err(format!(
                "Claude token refresh failed with HTTP {}; sign in to the account again",
                response.status().as_u16()
            ));
        }
        let tokens: RefreshedTokens = decode_json_response(response)
            .await
            .map_err(|error| format!("Claude token refresh failed: {error}"))?;
        let lifetime = tokens
            .expires_in
            .0
            .ok_or("Claude token refresh failed: the answer names no expiry")?;
        let lifetime = i64::try_from(lifetime.as_millis()).unwrap_or(i64::MAX);
        let expires_at = Timestamp::from_millisecond(
            self.clock
                .now()
                .as_millisecond()
                .saturating_add(lifetime),
        )
        .map_err(|_| "Claude token refresh failed: the expiry is out of range".to_owned())?;
        let scopes = match tokens.scope {
            Some(scope) => scope.split_whitespace().map(str::to_owned).collect(),
            None => secret.scopes,
        };
        Ok(ClaudeSecret {
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token.unwrap_or(secret.refresh_token),
            expires_at,
            scopes,
            ..secret
        })
    }
}

/// The failure of an account whose sign-in cannot be read. A document that
/// does not decode, such as a setup token that releases before the CLI's
/// sign-in stored, asks the user to sign in again.
fn failure(error: AccountError) -> AuthFailure {
    match error {
        AccountError::Invalid(error) => AuthFailure::new(
            FAMILY,
            AuthReason::Invalid(format!(
                "its secret document is {error}. Remove the account and sign in again"
            )),
        ),
        error => AuthFailure::of_account(FAMILY, error),
    }
}

/// The CLI's refresh request, in JSON.
#[derive(Serialize)]
struct RefreshGrant<'a> {
    grant_type: &'static str,
    refresh_token: &'a str,
    client_id: &'static str,
    scope: String,
}

/// The token endpoint's answer to a refresh. One without an access token is
/// a failed refresh however it is spelled.
#[derive(Deserialize)]
struct RefreshedTokens {
    access_token: Secret,
    #[serde(default)]
    refresh_token: Option<Secret>,
    #[serde(default)]
    expires_in: Lifetime,
    #[serde(default)]
    scope: Option<String>,
}
