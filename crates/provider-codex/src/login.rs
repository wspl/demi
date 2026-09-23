//! Codex's device login (`providers.md` § Login and publication), the
//! protocol of the Codex CLI's own device-code sign-in: ask for a user code,
//! let the user confirm it at `…/codex/device` from any browser, poll until
//! the service issues an authorization code with its own PKCE verifier, and
//! exchange that for tokens. No vendor CLI and no browser on the backend.

use std::{sync::Arc, time::Duration};

use demi_core::{Clock, LoginPending, Timestamp};
use demi_provider::{
    Secret,
    credentials::{
        AccountKit, AccountsCapability, AccountsError, AddAccount, LoginError, NewAccount,
        SecretDocument,
    },
    oauth::{DEVICE_LOGIN_LIFETIME, PollInterval, ResponseError, decode_json_response},
    wire::NonEmpty,
};
use futures_util::future::BoxFuture;
use reqwest::{StatusCode, Url, header::CONTENT_TYPE};
use serde::Deserialize;

use crate::auth::{AccountId, CLIENT_ID, CodexSecret, token_account};

/// What Codex adds to the account operations: its device login.
pub(crate) struct CodexKit {
    pub(crate) http: reqwest::Client,
    /// The sign-in service, such as `https://auth.openai.com`.
    pub(crate) auth_url: Url,
    pub(crate) clock: Arc<dyn Clock>,
}

impl AccountKit for CodexKit {
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

impl CodexKit {
    fn url(&self, path: &str) -> Url {
        demi_provider::endpoint_url(&self.auth_url, path)
    }

    async fn device_login(
        &self,
        pending: &(dyn Fn(LoginPending) + Send + Sync),
    ) -> Result<NewAccount, LoginError> {
        let started = tokio::time::Instant::now();
        let code = self.user_code().await?;
        let lifetime = i64::try_from(DEVICE_LOGIN_LIFETIME.as_millis()).unwrap_or(i64::MAX);
        let expires_at = self.clock.now().as_millisecond().saturating_add(lifetime);
        pending(LoginPending {
            verification_url: self.url("/codex/device").to_string(),
            user_code: Some(code.user_code.clone()),
            expires_at: Timestamp::from_millisecond(expires_at).ok(),
        });
        let authorization = self.authorization(&code, started).await?;
        let tokens = self.exchange(&authorization).await?;
        let account_id = token_account(&tokens.access_token)
            .account_id
            .or_else(|| token_account(&tokens.id_token).account_id)
            .and_then(AccountId::new)
            .ok_or_else(|| {
                LoginError::Failed("The Codex sign-in names no ChatGPT account".into())
            })?;
        let secret = CodexSecret {
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token,
            id_token: tokens.id_token,
            account_id,
            last_refresh: self.clock.now(),
        };
        Ok(NewAccount {
            label: secret.label(),
            secret: secret.encode(),
        })
    }

    /// Asks for a user code. A 404 means device login is not offered.
    async fn user_code(&self) -> Result<UserCode, LoginError> {
        let body = serde_json::json!({ "client_id": CLIENT_ID });
        let response = self
            .post_json("/api/accounts/deviceauth/usercode", &body)
            .await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Err(LoginError::Unavailable(
                "Device-code login is not enabled for this Codex account".into(),
            ));
        }
        if !response.status().is_success() {
            let status = response.status().as_u16();
            return Err(LoginError::Failed(format!(
                "Device code request failed with HTTP {status}"
            )));
        }
        let answer: UserCodeAnswer = decode_json_response(response)
            .await
            .map_err(|error| malformed("Device code", &error))?;
        let user_code = answer.user_code.or(answer.usercode).ok_or_else(|| {
            LoginError::Failed("Device code response is malformed: user_code is missing".into())
        })?;
        Ok(UserCode {
            device_auth_id: answer.device_auth_id.0,
            user_code: user_code.0,
            interval: answer.interval.0,
        })
    }

    /// Polls until the user confirms: 403 and 404 mean not yet, any other
    /// failure ends the login, and so does its lifetime. Dropping the future
    /// cancels the login at once, even between polls.
    async fn authorization(
        &self,
        code: &UserCode,
        started: tokio::time::Instant,
    ) -> Result<Authorization, LoginError> {
        let body = serde_json::json!({ "device_auth_id": code.device_auth_id, "user_code": code.user_code });
        loop {
            let response = self
                .post_json("/api/accounts/deviceauth/token", &body)
                .await?;
            let status = response.status();
            if status.is_success() {
                let answer: AuthorizationAnswer = decode_json_response(response)
                    .await
                    .map_err(|error| malformed("Device authorization", &error))?;
                return Ok(Authorization {
                    code: answer.authorization_code,
                    verifier: answer.code_verifier,
                });
            }
            if status != StatusCode::FORBIDDEN && status != StatusCode::NOT_FOUND {
                return Err(LoginError::Failed(format!(
                    "Device authorization failed with HTTP {}",
                    status.as_u16()
                )));
            }
            if started.elapsed() >= DEVICE_LOGIN_LIFETIME {
                let minutes = DEVICE_LOGIN_LIFETIME.as_secs() / 60;
                return Err(LoginError::Failed(format!(
                    "Device-code login timed out after {minutes} minutes"
                )));
            }
            tokio::time::sleep(code.interval).await;
        }
    }

    /// Exchanges the authorization code for tokens, with the verifier the
    /// service generated.
    async fn exchange(&self, authorization: &Authorization) -> Result<Tokens, LoginError> {
        let redirect = self.url("/deviceauth/callback").to_string();
        let form = [
            ("grant_type", "authorization_code"),
            ("code", authorization.code.expose()),
            ("redirect_uri", redirect.as_str()),
            ("client_id", CLIENT_ID),
            ("code_verifier", authorization.verifier.expose()),
        ];
        let response = self
            .http
            .post(self.url("/oauth/token"))
            .form(&form)
            .send()
            .await
            .map_err(transport)?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            return Err(LoginError::Failed(format!(
                "Device-code token exchange failed with HTTP {status}"
            )));
        }
        decode_json_response(response)
            .await
            .map_err(|error| malformed("Token exchange", &error))
    }

    async fn post_json(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<reqwest::Response, LoginError> {
        self.http
            .post(self.url(path))
            .header(CONTENT_TYPE, "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(transport)
    }
}

fn transport(error: reqwest::Error) -> LoginError {
    LoginError::Failed(format!(
        "Codex sign-in request failed: {}",
        error.without_url()
    ))
}

fn malformed(step: &str, error: &ResponseError) -> LoginError {
    LoginError::Failed(format!("{step} failed: {error}"))
}

struct UserCode {
    device_auth_id: String,
    user_code: String,
    interval: Duration,
}

/// The user-code answer. Deployments spell the code's field two ways.
#[derive(Deserialize)]
struct UserCodeAnswer {
    device_auth_id: NonEmpty,
    #[serde(default)]
    user_code: Option<NonEmpty>,
    #[serde(default)]
    usercode: Option<NonEmpty>,
    #[serde(default)]
    interval: PollInterval,
}

/// The confirmed login: an authorization code and the verifier of the PKCE
/// pair the service generated.
#[derive(Deserialize)]
struct AuthorizationAnswer {
    authorization_code: Secret,
    code_verifier: Secret,
}

struct Authorization {
    code: Secret,
    verifier: Secret,
}

#[derive(Deserialize)]
struct Tokens {
    id_token: Secret,
    access_token: Secret,
    refresh_token: Secret,
}
