//! Which way a search goes (`web.md` § Accounts): the user's API key, kept
//! as a secret, over the Exa sign-in the plugin host keeps, over neither.
//!
//! The host forgets a sign-in Exa refuses to renew, after which the user
//! reads as never signed in. So the plugin keeps a value of its own saying
//! the user was signed in, written while a token is live, and a search that
//! finds the sign-in gone while that value says otherwise fails once with
//! the words for an expired sign-in, rather than turning anonymous unsaid.

use demi_plugin_interface::{PluginError, PluginPort, PortFailure, PortRefusal, SecretName};
use serde_json::json;

use crate::search::Failure;

/// The secret that holds the API key.
pub(crate) fn key_name() -> SecretName {
    SecretName::try_from("api-key").expect("a valid secret name")
}

/// The value saying the user was signed in to Exa.
const SIGNED_IN: &str = "signed-in";

/// The value saying why the saved key is not checked yet, as Exa's words.
const UNCHECKED: &str = "unchecked-key";

/// Why the saved key is not checked yet; none for a key Exa took.
pub(crate) async fn unchecked(port: &PluginPort) -> Result<Option<String>, PortFailure> {
    Ok(port
        .value(UNCHECKED)
        .await?
        .and_then(|stored| stored.value.as_str().map(str::to_owned)))
}

/// Records why the saved key is not checked, or, with none, that it is.
pub(crate) async fn set_unchecked(
    port: &PluginPort,
    reason: Option<String>,
) -> Result<(), PortFailure> {
    let stored = port.value(UNCHECKED).await?;
    let written = match (reason, stored) {
        (Some(reason), stored) => port
            .write_value(UNCHECKED, json!(reason), stored.map(|stored| stored.revision))
            .await
            .map(drop),
        (None, Some(stored)) => port.remove_value(UNCHECKED, stored.revision).await,
        (None, None) => Ok(()),
    };
    match written {
        // Another request wrote it first, from the same key's search.
        Ok(()) | Err(PortFailure::Refused(PortRefusal::Conflict)) => Ok(()),
        Err(error) => Err(error),
    }
}

/// How a search reaches Exa.
pub(crate) enum Way {
    /// The REST API with the user's key.
    Key(String),
    /// The MCP server with the access token of the user's sign-in.
    Account(String),
    /// The MCP server, anonymously.
    Anonymous,
}

/// The user's saved key.
pub(crate) async fn key(port: &PluginPort) -> Result<Option<String>, PortFailure> {
    Ok(port.secret(key_name()).await?.map(|secret| secret.value))
}

/// A live access token of the user's sign-in, or none while signed out.
/// A live token records that the user is signed in.
pub(crate) async fn token(port: &PluginPort) -> Result<Option<String>, PortFailure> {
    match port.oauth_token().await {
        Ok(token) => {
            if port.value(SIGNED_IN).await?.is_none() {
                match port.write_value(SIGNED_IN, json!(true), None).await {
                    // Another request recorded it first.
                    Ok(_) | Err(PortFailure::Refused(PortRefusal::Conflict)) => {}
                    Err(error) => return Err(error),
                }
            }
            Ok(Some(token))
        }
        Err(PortFailure::Refused(PortRefusal::SignedOut)) => Ok(None),
        Err(error) => Err(error),
    }
}

/// The way a search goes now; [`Failure::Expired`] once when the sign-in
/// the user had is gone.
pub(crate) async fn way(port: &PluginPort) -> Result<Result<Way, Failure>, PluginError> {
    if let Some(key) = key(port).await? {
        return Ok(Ok(Way::Key(key)));
    }
    if let Some(token) = token(port).await? {
        return Ok(Ok(Way::Account(token)));
    }
    if forget_sign_in(port).await? {
        return Ok(Err(Failure::Expired));
    }
    Ok(Ok(Way::Anonymous))
}

/// Forgets that the user was signed in; whether the user was.
pub(crate) async fn forget_sign_in(port: &PluginPort) -> Result<bool, PluginError> {
    let Some(marker) = port.value(SIGNED_IN).await? else {
        return Ok(false);
    };
    match port.remove_value(SIGNED_IN, marker.revision).await {
        // Another request forgot it first.
        Ok(()) | Err(PortFailure::Refused(PortRefusal::Conflict)) => {}
        Err(error) => return Err(error.into()),
    }
    Ok(true)
}
