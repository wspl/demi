//! A Grok Build account's sign-in (`providers.md` § Subscription secrets, §
//! Token refresh): its secret document, what its tokens say about the user,
//! and the OIDC refresh of its tokens by the one protocol every family
//! follows.

use std::{sync::Arc, time::Duration};

use demi_core::{Clock, Timestamp};
use demi_provider::{
    Secret,
    credentials::{
        AccountDocument, AccountLabel, AuthFailure, AuthReason, CredentialPool, SecretDocument,
        read_secret, renew,
    },
    oauth::{Lifetime, decode_json_response, jwt_claims},
    quota::unix_seconds,
    wire::{Reported, ReportedString},
};
use reqwest::{Url, header::ACCEPT};
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

/// How account failures name the family.
const FAMILY: &str = "Grok";

/// How long before its expiry a token is refreshed.
const EXPIRY_SKEW: Duration = Duration::from_secs(5 * 60);

/// The Grok Build family's secret document: the OAuth tokens and their
/// expiry, the issuer and client that issued them, the team or organization
/// the tokens act for, and the user they belong to. Demi defines it; it is
/// not the Grok CLI's `auth.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GrokSecret {
    pub(crate) access_token: Secret,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    pub(crate) refresh_token: Option<Secret>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    pub(crate) expires_at: Option<Timestamp>,
    /// Such as `https://auth.x.ai`; refreshes go to its `/oauth2/token`.
    pub(crate) issuer: Issuer,
    pub(crate) client_id: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    pub(crate) principal: Option<Principal>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    pub(crate) user_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    pub(crate) email: Option<String>,
}

impl SecretDocument for GrokSecret {}

/// The issuer of a sign-in: an `http` or `https` URL, written as its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Issuer(pub(crate) Url);

impl Serialize for Issuer {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.0.as_str())
    }
}

impl<'de> Deserialize<'de> for Issuer {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        let url = Url::parse(&text)
            .ok()
            .filter(|url| matches!(url.scheme(), "http" | "https"))
            .ok_or_else(|| serde::de::Error::custom("an issuer is an http or https URL"))?;
        Ok(Self(url))
    }
}

/// Whom the tokens act for, such as a team: its type and id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Principal {
    pub(crate) kind: String,
    pub(crate) id: String,
}

/// The claims Demi reads from a Grok token, which spells its principal claims
/// in both snake and camel case. The signature is not checked and the vendor
/// judges the token, so a claim of the wrong type reads as absent.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Claims {
    exp: Reported<f64>,
    pub(crate) sub: ReportedString,
    pub(crate) email: ReportedString,
    principal_type: ReportedString,
    #[serde(rename = "principalType")]
    principal_type_camel: ReportedString,
    principal_id: ReportedString,
    #[serde(rename = "principalId")]
    principal_id_camel: ReportedString,
}

impl Claims {
    pub(crate) fn of(token: &str) -> Self {
        jwt_claims(token).unwrap_or_default()
    }

    /// The principal a token acts for, when it names both its type and id.
    pub(crate) fn principal(&self) -> Option<Principal> {
        let kind = self
            .principal_type
            .0
            .clone()
            .or_else(|| self.principal_type_camel.0.clone())?;
        let id = self
            .principal_id
            .0
            .clone()
            .or_else(|| self.principal_id_camel.0.clone())?;
        Some(Principal { kind, id })
    }

    fn expiry(&self) -> Option<Timestamp> {
        unix_seconds(self.exp.0?)
    }
}

impl GrokSecret {
    /// How the account names itself: its email, else its user, else its
    /// sign-in. The user, or the team or organization the tokens act for,
    /// tells it apart from the entry's other accounts.
    pub(crate) fn label(&self) -> AccountLabel {
        let issuer = self.issuer.0.as_str().trim_end_matches('/');
        let identity = match &self.user_id {
            Some(user) => format!("{issuer}::{user}"),
            None => match &self.email {
                Some(email) => format!("{issuer}::{email}"),
                None => format!("{issuer}::{}", self.client_id),
            },
        };
        let label = self
            .email
            .clone()
            .or_else(|| self.user_id.clone())
            .unwrap_or_else(|| identity.clone());
        AccountLabel {
            label,
            detail: Some("oidc".into()),
            identity_key: Some(identity),
        }
    }

    /// When the access token expires: the stored time, else its `exp`
    /// claim.
    fn expiry(&self) -> Option<Timestamp> {
        self.expires_at
            .or_else(|| Claims::of(self.access_token.expose()).expiry())
    }

    /// Whether the access token expires within five minutes of `now`.
    fn expiring(&self, now: Timestamp) -> bool {
        let skew = i64::try_from(EXPIRY_SKEW.as_millis()).unwrap_or(i64::MAX);
        self.expiry()
            .is_some_and(|expiry| expiry.as_millisecond() - now.as_millisecond() <= skew)
    }

    fn credentials(&self) -> Credentials {
        Credentials {
            access_token: self.access_token.clone(),
            user_id: self.user_id.clone(),
            email: self.email.clone(),
        }
    }
}

/// What one request authenticates and identifies itself with.
#[derive(Debug, Clone)]
pub(crate) struct Credentials {
    pub(crate) access_token: Secret,
    pub(crate) user_id: Option<String>,
    pub(crate) email: Option<String>,
}

/// The account a Grok Build provider stands for, and how its tokens are
/// refreshed.
pub(crate) struct GrokAuth {
    pool: Arc<dyn CredentialPool>,
    account: Option<String>,
    clock: Arc<dyn Clock>,
}

impl GrokAuth {
    pub(crate) fn new(
        pool: Arc<dyn CredentialPool>,
        account: Option<String>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            pool,
            account,
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
    pub(crate) async fn stored(&self) -> Result<GrokSecret, AuthFailure> {
        let doc = self.document()?;
        let stored = read_secret::<GrokSecret>(&*doc).await;
        stored
            .map(|stored| stored.secret)
            .map_err(|error| AuthFailure::of_account(FAMILY, error))
    }

    /// The credentials of a request, refreshed first when the access token
    /// expires within five minutes, or when it is still the one a request
    /// was `refused` with, as with HTTP 401. A refresh that waited behind
    /// another uses the other's tokens unless they expire as soon. A sign-in
    /// without a refresh token is used as it is.
    pub(crate) async fn credentials(
        &self,
        http: &reqwest::Client,
        refused: Option<&Secret>,
    ) -> Result<Credentials, AuthFailure> {
        let doc = self.document()?;
        let needs_refresh = |secret: &GrokSecret| {
            let still_refused = refused.is_some_and(|token| secret.access_token == *token);
            secret.refresh_token.is_some() && (still_refused || secret.expiring(self.clock.now()))
        };
        let renewed = renew(&*doc, needs_refresh, |secret| self.refresh(http, secret)).await;
        renewed
            .map(|secret| secret.credentials())
            .map_err(|error| AuthFailure::of_renewal(FAMILY, error))
    }

    /// Asks the issuer for new tokens, for the principal the tokens act for.
    /// A refreshed sign-in keeps the refresh token the answer does not
    /// replace; its expiry comes from the answer, else from the new token.
    async fn refresh(
        &self,
        http: &reqwest::Client,
        secret: GrokSecret,
    ) -> Result<GrokSecret, String> {
        let Some(refresh_token) = &secret.refresh_token else {
            return Err("The Grok sign-in has no refresh token".into());
        };
        let mut form = vec![
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token.expose()),
            ("client_id", secret.client_id.as_str()),
        ];
        if let Some(principal) = &secret.principal {
            form.push(("principal_type", principal.kind.as_str()));
            form.push(("principal_id", principal.id.as_str()));
        }
        let response = http
            .post(demi_provider::endpoint_url(
                &secret.issuer.0,
                "/oauth2/token",
            ))
            .header(ACCEPT, "application/json")
            .form(&form)
            .send()
            .await
            .map_err(|error| format!("Grok token refresh failed: {}", error.without_url()))?;
        if !response.status().is_success() {
            return Err(format!(
                "Grok token refresh failed with HTTP {}",
                response.status().as_u16()
            ));
        }
        let tokens: RefreshedTokens = decode_json_response(response)
            .await
            .map_err(|error| format!("Grok token refresh failed: {error}"))?;
        let now = self.clock.now();
        let expires_at = match tokens.expires_in.0 {
            Some(lifetime) => {
                let lifetime = i64::try_from(lifetime.as_millis()).unwrap_or(i64::MAX);
                Timestamp::from_millisecond(now.as_millisecond().saturating_add(lifetime)).ok()
            }
            None => Claims::of(tokens.access_token.expose()).expiry(),
        };
        Ok(GrokSecret {
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token.or(secret.refresh_token),
            expires_at,
            ..secret
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
    expires_in: Lifetime,
}
