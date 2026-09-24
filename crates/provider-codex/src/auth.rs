//! A Codex account's sign-in (`providers.md` § Subscription secrets, § Token
//! refresh): its secret document, what its tokens say about the account, and
//! the refresh of its tokens by the one protocol every family follows.

use std::{sync::Arc, time::Duration};

use demi_core::{Clock, Timestamp};
use demi_provider::{
    Secret,
    credentials::{
        AccountDocument, AccountLabel, AuthFailure, AuthReason, CredentialPool, SecretDocument,
        read_secret, renew,
    },
    oauth::{decode_json_response, jwt_claims},
    wire::{Reported, ReportedString},
};
use http::HeaderValue;
use reqwest::{Url, header::CONTENT_TYPE};
use serde::{Deserialize, Deserializer, Serialize, de};

/// The OAuth client of Codex's sign-in, for refreshes and device logins.
pub(crate) const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

/// How account failures name the family.
const FAMILY: &str = "Codex";

/// How long before its expiry a token is refreshed.
const EXPIRY_SKEW: Duration = Duration::from_secs(5 * 60);

/// How long a sign-in goes without a refresh.
const STALENESS: Duration = Duration::from_secs(8 * 24 * 60 * 60);

/// The Codex family's secret document: the ChatGPT sign-in's tokens, the
/// account they act for, and when they were last refreshed. Demi defines it;
/// it is not the Codex CLI's `auth.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CodexSecret {
    pub(crate) access_token: Secret,
    pub(crate) refresh_token: Secret,
    pub(crate) id_token: Secret,
    pub(crate) account_id: AccountId,
    pub(crate) last_refresh: Timestamp,
}

impl SecretDocument for CodexSecret {}

/// A ChatGPT account id: nonempty text that a header can carry, since every
/// request names the account in `ChatGPT-Account-ID`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct AccountId(String);

impl AccountId {
    pub(crate) fn new(id: String) -> Option<Self> {
        let valid = !id.is_empty() && HeaderValue::from_str(&id).is_ok();
        valid.then_some(Self(id))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn header_value(&self) -> HeaderValue {
        // Checked when the id was made.
        HeaderValue::from_str(&self.0).expect("an account id is a header value")
    }
}

impl<'de> Deserialize<'de> for AccountId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let id = String::deserialize(deserializer)?;
        Self::new(id).ok_or_else(|| de::Error::custom("an account id is nonempty header text"))
    }
}

/// The claims Demi reads from a ChatGPT token. Their signature is not
/// checked, and the vendor decides whether the token is good, so a claim of
/// the wrong type reads as absent.
#[derive(Debug, Default, Deserialize)]
struct Claims {
    #[serde(default)]
    exp: Reported<f64>,
    #[serde(default)]
    email: ReportedString,
    #[serde(default, rename = "https://api.openai.com/auth")]
    auth: Reported<AuthClaims>,
    #[serde(default, rename = "https://api.openai.com/profile")]
    profile: Reported<ProfileClaims>,
}

#[derive(Debug, Default, Deserialize)]
struct AuthClaims {
    #[serde(default)]
    chatgpt_account_id: ReportedString,
    #[serde(default)]
    chatgpt_account_is_fedramp: Reported<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct ProfileClaims {
    #[serde(default)]
    email: ReportedString,
}

/// What a token says about its account; nothing for a token Demi cannot
/// read, which the vendor still judges.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TokenAccount {
    pub(crate) account_id: Option<String>,
    pub(crate) email: Option<String>,
    pub(crate) fedramp: bool,
}

fn claims(token: &Secret) -> Claims {
    jwt_claims(token.expose()).unwrap_or_default()
}

pub(crate) fn token_account(token: &Secret) -> TokenAccount {
    let claims = claims(token);
    let auth = claims.auth.into_inner().unwrap_or_default();
    let profile_email = claims
        .profile
        .into_inner()
        .and_then(|profile| profile.email.into_inner());
    TokenAccount {
        account_id: auth.chatgpt_account_id.into_inner(),
        email: claims.email.into_inner().or(profile_email),
        fedramp: auth.chatgpt_account_is_fedramp.into_inner() == Some(true),
    }
}

/// When a token expires, by its `exp` claim in Unix seconds.
fn expiry(token: &Secret) -> Option<Timestamp> {
    let seconds = claims(token).exp.into_inner()?;
    demi_provider::quota::unix_seconds(seconds)
}

impl CodexSecret {
    /// How the account names itself: its email, else its id; its account id
    /// tells it apart from the entry's others.
    pub(crate) fn label(&self) -> AccountLabel {
        let id = token_account(&self.id_token);
        let access = token_account(&self.access_token);
        let label = id
            .email
            .or(access.email)
            .unwrap_or_else(|| self.account_id.as_str().to_owned());
        AccountLabel {
            label,
            detail: Some("chatgpt".into()),
            identity_key: Some(self.account_id.as_str().to_owned()),
        }
    }

    fn credentials(&self) -> Credentials {
        let fedramp =
            token_account(&self.id_token).fedramp || token_account(&self.access_token).fedramp;
        Credentials {
            access_token: self.access_token.clone(),
            account_id: self.account_id.clone(),
            fedramp,
        }
    }

    /// Whether the sign-in needs a refresh at `now`: its access token
    /// expires within five minutes, or it was last refreshed eight or more
    /// days ago.
    fn due(&self, now: Timestamp) -> bool {
        let now = now.as_millisecond();
        let expiring = expiry(&self.access_token)
            .is_some_and(|expiry| expiry.as_millisecond() - now <= millis(EXPIRY_SKEW));
        let stale = now - self.last_refresh.as_millisecond() >= millis(STALENESS);
        expiring || stale
    }
}

fn millis(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

/// What one request authenticates with.
#[derive(Debug, Clone)]
pub(crate) struct Credentials {
    pub(crate) access_token: Secret,
    pub(crate) account_id: AccountId,
    pub(crate) fedramp: bool,
}

/// The account a Codex provider stands for, and how its tokens are
/// refreshed.
pub(crate) struct CodexAuth {
    pool: Arc<dyn CredentialPool>,
    account: Option<String>,
    token_url: Url,
    clock: Arc<dyn Clock>,
}

impl CodexAuth {
    pub(crate) fn new(
        pool: Arc<dyn CredentialPool>,
        account: Option<String>,
        auth_url: &Url,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            pool,
            account,
            token_url: demi_provider::endpoint_url(auth_url, "/oauth/token"),
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
    pub(crate) async fn stored(&self) -> Result<CodexSecret, AuthFailure> {
        let doc = self.document()?;
        let stored = read_secret::<CodexSecret>(&*doc).await;
        stored
            .map(|stored| stored.secret)
            .map_err(|error| AuthFailure::of_account(FAMILY, error))
    }

    /// The credentials of a request, refreshed first when the sign-in is
    /// due, or when it still holds the access token a request was `refused`
    /// with, as with HTTP 401. A refresh that waited behind another uses the
    /// other's tokens unless they are due as well.
    pub(crate) async fn credentials(
        &self,
        http: &reqwest::Client,
        refused: Option<&Secret>,
    ) -> Result<Credentials, AuthFailure> {
        let doc = self.document()?;
        let needs_refresh = |secret: &CodexSecret| {
            let still_refused = refused.is_some_and(|token| secret.access_token == *token);
            still_refused || secret.due(self.clock.now())
        };
        let renewed = renew(&*doc, needs_refresh, |secret| self.refresh(http, secret)).await;
        renewed
            .map(|secret| secret.credentials())
            .map_err(|error| AuthFailure::of_renewal(FAMILY, error))
    }

    /// Asks the sign-in service for new tokens. A refreshed sign-in keeps
    /// the tokens the answer does not replace and the account it acts for.
    async fn refresh(
        &self,
        http: &reqwest::Client,
        secret: CodexSecret,
    ) -> Result<CodexSecret, String> {
        let body = serde_json::json!({
            "client_id": CLIENT_ID,
            "grant_type": "refresh_token",
            "refresh_token": secret.refresh_token.expose(),
        });
        let response = http
            .post(self.token_url.clone())
            .header(CONTENT_TYPE, "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(|error| format!("Codex token refresh failed: {}", error.without_url()))?;
        if !response.status().is_success() {
            return Err(format!(
                "Codex token refresh failed with HTTP {}",
                response.status().as_u16()
            ));
        }
        let tokens: RefreshedTokens = decode_json_response(response)
            .await
            .map_err(|error| format!("Codex token refresh failed: {error}"))?;
        Ok(CodexSecret {
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token.unwrap_or(secret.refresh_token),
            id_token: tokens.id_token.unwrap_or(secret.id_token),
            account_id: secret.account_id,
            last_refresh: self.clock.now(),
        })
    }
}

/// The token endpoint's answer to a refresh. One without an access token is
/// a failed refresh however it is spelled.
#[derive(Deserialize)]
struct RefreshedTokens {
    access_token: Secret,
    #[serde(default)]
    refresh_token: Option<Secret>,
    #[serde(default)]
    id_token: Option<Secret>,
}
