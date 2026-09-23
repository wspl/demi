//! Grok Build's device login (`providers.md` § Login and publication): OAuth
//! device authorization (RFC 8628) at `auth.x.ai` with the request contract
//! of the Grok CLI, its fixed scopes, `referrer=grok-build` and its client
//! headers, then the team or organization the tokens act for, and the
//! user's details from the chat proxy's `/user`.

use std::{sync::Arc, time::Duration};

use demi_core::{Clock, LoginPending, Timestamp};
use demi_provider::{
    Secret,
    credentials::{
        AccountKit, AccountsCapability, AccountsError, AddAccount, LoginError, NewAccount,
        SecretDocument,
    },
    oauth::{DEVICE_LOGIN_LIFETIME, Lifetime, PollInterval, ResponseError, decode_json_response},
    wire::{NonEmpty, Reported},
};
use futures_util::future::BoxFuture;
use http::{HeaderMap, HeaderValue, header::AUTHORIZATION};
use reqwest::Url;
use serde::{Deserialize, Deserializer, de};

use crate::{
    auth::{Claims, GrokSecret, Issuer, Principal},
    request::CLIENT_VERSION,
};

/// The Grok CLI's OAuth client.
const CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";

/// The Grok CLI's scopes, fixed.
const SCOPE: &str = "openid profile email offline_access grok-cli:access api:access conversations:read conversations:write workspaces:read workspaces:write";

/// The shortest wait between polls.
const MIN_INTERVAL: Duration = Duration::from_secs(1);

/// How much longer to wait after the server asks to slow down.
const SLOW_DOWN: Duration = Duration::from_secs(5);

/// What Grok Build adds to the account operations: its device login.
pub(crate) struct GrokKit {
    pub(crate) http: reqwest::Client,
    /// The sign-in issuer, such as `https://auth.x.ai`.
    pub(crate) issuer: Url,
    /// The chat proxy, whose `/user` names the signed-in user.
    pub(crate) user_url: Url,
    pub(crate) clock: Arc<dyn Clock>,
}

impl AccountKit for GrokKit {
    fn capability(&self) -> AccountsCapability {
        AccountsCapability {
            login: true,
            add: false,
        }
    }

    fn login<'a>(
        &'a self,
        pending: &'a (dyn Fn(LoginPending) + Send + Sync),
    ) -> Option<BoxFuture<'a, Result<NewAccount, LoginError>>> {
        Some(Box::pin(self.device_login(pending)))
    }

    fn add(&self, _input: AddAccount) -> Option<Result<NewAccount, AccountsError>> {
        None
    }
}

/// The headers of the sign-in requests: the Grok CLI's version, and the
/// surface the login runs on, which for Demi is always its user interface.
fn oauth_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-grok-client-version",
        HeaderValue::from_static(CLIENT_VERSION),
    );
    headers.insert("x-grok-client-surface", HeaderValue::from_static("ui"));
    headers
}

impl GrokKit {
    async fn device_login(
        &self,
        pending: &(dyn Fn(LoginPending) + Send + Sync),
    ) -> Result<NewAccount, LoginError> {
        let device = self.device_code().await?;
        let deadline = tokio::time::Instant::now() + DEVICE_LOGIN_LIFETIME;
        let lifetime = i64::try_from(DEVICE_LOGIN_LIFETIME.as_millis()).unwrap_or(i64::MAX);
        let expires_at = self.clock.now().as_millisecond().saturating_add(lifetime);
        pending(LoginPending {
            verification_url: device.verification_url.clone(),
            user_code: Some(device.user_code.clone()),
            expires_at: Timestamp::from_millisecond(expires_at).ok(),
        });
        let tokens = self.poll(&device, deadline).await?;
        let secret = self.secret(tokens).await;
        Ok(NewAccount {
            label: secret.label(),
            secret: secret.encode(),
        })
    }

    async fn device_code(&self) -> Result<DeviceCode, LoginError> {
        let form = [
            ("client_id", CLIENT_ID),
            ("scope", SCOPE),
            ("referrer", "grok-build"),
        ];
        let response = self
            .http
            .post(demi_provider::endpoint_url(
                &self.issuer,
                "/oauth2/device/code",
            ))
            .headers(oauth_headers())
            .form(&form)
            .send()
            .await
            .map_err(transport)?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            return Err(LoginError::Failed(format!(
                "Grok device code request failed with HTTP {status}"
            )));
        }
        let answer: DeviceCodeAnswer = decode_json_response(response)
            .await
            .map_err(|error| malformed("Grok device code", &error))?;
        let verification = answer
            .verification_uri_complete
            .or(answer.verification_uri)
            .ok_or_else(|| {
                LoginError::Failed(
                    "Grok device code failed: the response names no verification_uri".into(),
                )
            })?;
        Ok(DeviceCode {
            device_code: answer.device_code,
            user_code: answer.user_code.0,
            verification_url: verification.0,
            interval: answer.interval.0,
        })
    }

    /// Polls for the tokens: waits the interval before each poll, at least a
    /// second and five seconds longer after each `slow_down`, continues on
    /// `authorization_pending`, and ends on any other answer or at the
    /// login's `deadline`. Dropping the future cancels the login at once.
    async fn poll(
        &self,
        device: &DeviceCode,
        deadline: tokio::time::Instant,
    ) -> Result<Tokens, LoginError> {
        let mut interval = device.interval.max(MIN_INTERVAL);
        let form = [
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", device.device_code.expose()),
            ("client_id", CLIENT_ID),
        ];
        loop {
            tokio::time::sleep(interval).await;
            if tokio::time::Instant::now() >= deadline {
                return Err(LoginError::Failed(
                    "Grok device login timed out before the user confirmed".into(),
                ));
            }
            let response = self
                .http
                .post(demi_provider::endpoint_url(&self.issuer, "/oauth2/token"))
                .headers(oauth_headers())
                .form(&form)
                .send()
                .await
                .map_err(transport)?;
            let status = response.status();
            if status.is_success() {
                return decode_json_response(response)
                    .await
                    .map_err(|error| malformed("Grok device token", &error));
            }
            // An error the answer does not state still ends the login, named
            // by its status.
            let refusal: TokenRefusal = decode_json_response(response).await.unwrap_or_default();
            match refusal.error.into_inner().as_deref() {
                Some("slow_down") => interval += SLOW_DOWN,
                Some("authorization_pending") => {}
                Some(error) => {
                    return Err(LoginError::Failed(format!(
                        "Grok device login failed: {error}"
                    )));
                }
                None => {
                    return Err(LoginError::Failed(format!(
                        "Grok device login failed: HTTP {}",
                        status.as_u16()
                    )));
                }
            }
        }
    }

    /// The account's secret document: the tokens, the user the id token
    /// names, the team or organization the access token acts for, which
    /// then is the account's user, and the proxy's details of the user when
    /// it names one.
    async fn secret(&self, tokens: Tokens) -> GrokSecret {
        let id = tokens
            .id_token
            .as_ref()
            .map(|token| Claims::of(token.expose()))
            .unwrap_or_default();
        let principal = Claims::of(tokens.access_token.expose()).principal();
        let (mut user_id, mut email) = (id.sub.into_inner(), id.email.into_inner());
        if let Some(principal) = &principal
            && (principal.kind == "Team" || principal.kind == "Organization")
        {
            user_id = Some(principal.id.clone());
            email = None;
        }
        let expires_at = tokens.expires_in.0.and_then(|lifetime| {
            let lifetime = i64::try_from(lifetime.as_millis()).ok()?;
            Timestamp::from_millisecond(self.clock.now().as_millisecond().saturating_add(lifetime))
                .ok()
        });
        let mut secret = GrokSecret {
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token,
            expires_at,
            issuer: Issuer(self.issuer.clone()),
            client_id: CLIENT_ID.into(),
            principal,
            user_id: user_id.filter(|user| !user.is_empty()),
            email: email.filter(|email| !email.is_empty()),
        };
        if let Some(user) = self.user(&secret.access_token).await {
            secret.user_id = Some(user.id);
            if let Some(principal) = user.principal {
                secret.principal = Some(principal);
            }
            if let Some(email) = user.email {
                secret.email = Some(email);
            }
        }
        secret
    }

    /// The proxy's details of the signed-in user. They only fill in an
    /// account that is already usable, so an answer that fails or names no
    /// user adds nothing.
    async fn user(&self, access_token: &Secret) -> Option<UserDetails> {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, access_token.bearer());
        headers.insert("x-xai-token-auth", HeaderValue::from_static("xai-grok-cli"));
        headers.insert(
            "x-grok-client-version",
            HeaderValue::from_static(CLIENT_VERSION),
        );
        headers.insert(
            "x-grok-client-mode",
            HeaderValue::from_static("interactive"),
        );
        let response = self
            .http
            .get(self.user_url.clone())
            .headers(headers)
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        let user: UserAnswer = serde_json::from_str(&response.text().await.ok()?).ok()?;
        let id = user.user_id.0.or(user.user_id_snake.0)?.0;
        let kind = user.principal_type.0.or(user.principal_type_snake.0);
        let principal_id = user.principal_id.0.or(user.principal_id_snake.0);
        let principal = kind.zip(principal_id).map(|(kind, id)| Principal {
            kind: kind.0,
            id: id.0,
        });
        Some(UserDetails {
            id,
            principal,
            email: user.email.0.map(|email| email.0),
        })
    }
}

fn transport(error: reqwest::Error) -> LoginError {
    LoginError::Failed(format!(
        "Grok sign-in request failed: {}",
        error.without_url()
    ))
}

fn malformed(step: &str, error: &ResponseError) -> LoginError {
    LoginError::Failed(format!("{step} failed: {error}"))
}

struct DeviceCode {
    device_code: Secret,
    user_code: String,
    verification_url: String,
    interval: Duration,
}

/// The device-code answer: what to show the user and how to poll. The
/// code's own lifetime is not read: a login lasts as long as every device
/// login does.
#[derive(Deserialize)]
struct DeviceCodeAnswer {
    device_code: Secret,
    user_code: UserCode,
    #[serde(default)]
    verification_uri: Option<VerificationUri>,
    #[serde(default)]
    verification_uri_complete: Option<VerificationUri>,
    #[serde(default)]
    interval: PollInterval,
}

/// The one-time code the user types: letters, digits and dashes only.
struct UserCode(String);

impl<'de> Deserialize<'de> for UserCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let code = String::deserialize(deserializer)?;
        let valid = !code.is_empty()
            && code
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '-');
        if !valid {
            return Err(de::Error::custom(
                "a user code is letters, digits and dashes",
            ));
        }
        Ok(Self(code))
    }
}

/// The address the user is told to open, so only a scheme a browser can be
/// trusted with passes: `https` anywhere, `http` on the loopback host, and
/// no control characters, which would make the link read differently from
/// the one shown.
struct VerificationUri(String);

impl<'de> Deserialize<'de> for VerificationUri {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let uri = String::deserialize(deserializer)?;
        let browsable = !uri.chars().any(char::is_control)
            && Url::parse(&uri).is_ok_and(|url| match url.scheme() {
                "https" => true,
                "http" => matches!(url.host_str(), Some("localhost" | "127.0.0.1")),
                _ => false,
            });
        if !browsable {
            return Err(de::Error::custom(
                "a verification address is https, or http on localhost",
            ));
        }
        Ok(Self(uri))
    }
}

/// The confirmed login's tokens.
#[derive(Deserialize)]
struct Tokens {
    access_token: Secret,
    #[serde(default)]
    refresh_token: Option<Secret>,
    #[serde(default)]
    expires_in: Lifetime,
    #[serde(default)]
    id_token: Option<Secret>,
}

/// A poll the server refused, with its error when it states one.
#[derive(Default, Deserialize)]
struct TokenRefusal {
    #[serde(default)]
    error: Reported<String>,
}

/// The proxy's `/user`, which spells its fields in camel and snake case; a
/// field of the wrong shape is left out.
#[derive(Default, Deserialize)]
#[serde(default)]
struct UserAnswer {
    #[serde(rename = "userId")]
    user_id: Reported<NonEmpty>,
    #[serde(rename = "user_id")]
    user_id_snake: Reported<NonEmpty>,
    #[serde(rename = "principalType")]
    principal_type: Reported<NonEmpty>,
    #[serde(rename = "principal_type")]
    principal_type_snake: Reported<NonEmpty>,
    #[serde(rename = "principalId")]
    principal_id: Reported<NonEmpty>,
    #[serde(rename = "principal_id")]
    principal_id_snake: Reported<NonEmpty>,
    email: Reported<NonEmpty>,
}

struct UserDetails {
    id: String,
    principal: Option<Principal>,
    email: Option<String>,
}
