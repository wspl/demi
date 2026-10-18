//! Settings › Web Search (`web.md` § Settings › Web Search): the plugin's
//! user state, which says how the user is connected to Exa, and the methods
//! that sign in and out and save and remove the API key.

use base64::Engine;
use demi_plugin_interface::{
    Method, Page, PluginError, PluginPort, Scope, State,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};

use crate::account;
use crate::search::{Failure, SearchError};

/// How the user is connected to Exa.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WebState {
    /// The user's sign-in, while signed in.
    pub account: Option<Account>,
    /// The user's saved API key.
    pub key: Option<SavedKey>,
    /// The way searches go: the key over the sign-in, over neither.
    pub in_use: InUse,
}

/// The user's Exa sign-in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    /// The account's email, when Exa's token names it.
    pub email: Option<String>,
}

/// A saved API key, of which a page learns only the end.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SavedKey {
    pub last_four: String,
    /// Why the search that checks it could not tell, such as Exa's rate
    /// limit, until a search with it succeeds; none for a key Exa took.
    pub unchecked: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InUse {
    Key,
    Account,
    Anonymous,
}

/// `sign_in {}`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SignIn {}

/// What `sign_in` answers: Exa's sign-in page, which the page opens.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SignInStarted {
    pub url: String,
}

/// `sign_out {}`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SignOut {}

/// `save_key { key }`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveKey {
    #[schemars(length(min = 1, max = 256))]
    pub key: String,
}

/// `remove_key {}`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoveKey {}

/// Why `save_key` refused a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WebReason {
    /// Exa refused the key.
    KeyRefused,
    /// Exa did not answer the search that checks the key.
    ExaUnavailable,
}

impl WebReason {
    fn word(self) -> &'static str {
        match self {
            Self::KeyRefused => "key_refused",
            Self::ExaUnavailable => "exa_unavailable",
        }
    }
}

pub(crate) fn page() -> Page {
    Page::new("@demicodes/plugin-web")
        .user_state(State::new::<WebState>())
        .method(Method::new::<SignIn, SignInStarted>("sign_in", Scope::User))
        .method(Method::new::<SignOut, ()>("sign_out", Scope::User))
        .method(Method::new::<SaveKey, ()>("save_key", Scope::User))
        .method(Method::new::<RemoveKey, ()>("remove_key", Scope::User))
        .reasons::<WebReason>()
}

/// The user's state as the page shows it.
pub(crate) async fn state(port: &PluginPort) -> Result<WebState, PluginError> {
    let key = account::key(port).await?;
    let account = account::token(port).await?.map(|token| Account {
        email: email(&token),
    });
    let in_use = match (&key, &account) {
        (Some(_), _) => InUse::Key,
        (None, Some(_)) => InUse::Account,
        (None, None) => InUse::Anonymous,
    };
    let key = match key {
        Some(key) => Some(SavedKey {
            last_four: key.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect(),
            unchecked: account::unchecked(port).await?,
        }),
        None => None,
    };
    Ok(WebState {
        account,
        key,
        in_use,
    })
}

pub(crate) async fn call(
    method: &str,
    params: Map<String, Value>,
    port: &PluginPort,
) -> Result<Value, PluginError> {
    match method {
        "sign_in" => {
            let SignIn {} = decode(params)?;
            let url = port.oauth_start().await?;
            Ok(serde_json::to_value(SignInStarted { url }).expect("a result encodes"))
        }
        "sign_out" => {
            let SignOut {} = decode(params)?;
            // The plugin host marks the state changed as it forgets the
            // tokens.
            port.oauth_sign_out().await?;
            account::forget_sign_in(port).await?;
            Ok(Value::Null)
        }
        "save_key" => {
            let SaveKey { key } = decode(params)?;
            let key = key.trim();
            // A key out of credit or at its rate limit is saved, since Exa
            // took it, but the check could not tell whether it searches:
            // the page says why, in words that stay true, and the next
            // search decides.
            let unchecked = match crate::rest::check(port, key).await {
                Ok(()) => None,
                Err(SearchError::Failed(Failure::NoCredit)) => {
                    Some("your Exa account has no credit left".to_owned())
                }
                Err(SearchError::Failed(Failure::RateLimit(_) | Failure::RateLimitUnknown)) => {
                    Some("Exa’s rate limit was reached".to_owned())
                }
                Err(SearchError::Failed(Failure::KeyRefused)) => {
                    return Err(refused(WebReason::KeyRefused, "Exa refused this key"));
                }
                Err(SearchError::Failed(failure)) => {
                    return Err(refused(WebReason::ExaUnavailable, failure.to_string()));
                }
                // A key has no sign-in to be refused.
                Err(SearchError::SignedOut) => unreachable!("a key search has no token"),
                Err(SearchError::Port(failure)) => return Err(failure.into()),
            };
            let saved = port.secret(account::key_name()).await?;
            port.write_secret(
                account::key_name(),
                key.to_owned(),
                saved.map(|saved| saved.revision),
            )
            .await?;
            account::set_unchecked(port, unchecked).await?;
            port.changed(Scope::User).await?;
            Ok(Value::Null)
        }
        "remove_key" => {
            let RemoveKey {} = decode(params)?;
            if let Some(saved) = port.secret(account::key_name()).await? {
                port.remove_secret(account::key_name(), saved.revision).await?;
                account::set_unchecked(port, None).await?;
                port.changed(Scope::User).await?;
            }
            Ok(Value::Null)
        }
        _ => Err(PluginError::failed(format!("no method {method}"))),
    }
}

fn refused(reason: WebReason, message: impl Into<String>) -> PluginError {
    PluginError::refused(reason.word(), message)
}

fn decode<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, PluginError> {
    serde_json::from_value(Value::Object(params)).map_err(|error| {
        PluginError::failed(format!("the parameters do not decode as declared: {error}"))
    })
}

/// The email an access token names, when it is a JWT whose claims carry
/// one. The token is only read, never checked: it came from Exa through the
/// plugin host, and the email is only shown.
fn email(token: &str) -> Option<String> {
    let claims = token.split('.').nth(1)?;
    let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(claims.trim_end_matches('='))
        .ok()?;
    let claims: Value = serde_json::from_slice(&claims).ok()?;
    claims.get("email")?.as_str().map(str::to_owned)
}
