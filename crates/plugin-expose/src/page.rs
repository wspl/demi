//! The expose menu's data and calls (`expose.md` § Product surface): the
//! plugin's state for the user's pages, which follows the user's exposes,
//! and the menu's two methods.

use demi_plugin_interface::{
    ExposeRefusal, Method, Page, PluginError, PluginPort, PortFailure, PortRefusal, Scope, State,
    Topic,
};
use demi_shared_types::{MAX_SAFE_INTEGER, Timestamp};
use demi_web_api_protocol::exposes::ExposeAddress;
use demi_web_api_protocol::ids::{DeviceId, ExposeId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::LIFETIME;
use crate::numbers::numbered;

/// The plugin's state for the user's pages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExposeState {
    /// Whether the instance has an expose domain; without one the product
    /// shows no expose controls.
    pub available: bool,
    /// Every live expose of the user's, soonest expiry first.
    pub exposes: Vec<ExposeEntry>,
}

/// An expose as the menu shows it, with the name of the device it is on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExposeEntry {
    pub id: ExposeId,
    #[schemars(range(min = 1, max = MAX_SAFE_INTEGER))]
    pub number: u64,
    pub device_id: DeviceId,
    /// The device's name, the Cloud's as `Cloud`.
    pub device_name: String,
    pub address: ExposeAddress,
    pub url: String,
    pub expires_at: Timestamp,
}

/// `renew { expose }` and `remove { expose }`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExposeCall {
    /// The expose's id.
    pub expose: String,
}

pub(crate) fn page() -> Page {
    Page::new("@demicodes/plugin-expose")
        .user_state(State::new::<ExposeState>().follows(Topic::Exposes))
        .method(Method::new::<ExposeCall, ()>("renew", Scope::User))
        .method(Method::new::<ExposeCall, ()>("remove", Scope::User))
}

pub(crate) async fn state(port: &PluginPort) -> Result<Value, PluginError> {
    let listed = port.exposes().await?;
    let exposes = numbered(port, listed.exposes)
        .await?
        .into_iter()
        .map(|entry| ExposeEntry {
            id: entry.expose.id,
            number: entry.number,
            device_id: entry.expose.device,
            device_name: entry.expose.device_name,
            address: entry.expose.address,
            url: entry.expose.url,
            expires_at: entry.expose.expires_at,
        })
        .collect();
    let state = ExposeState {
        available: listed.available,
        exposes,
    };
    serde_json::to_value(state).map_err(PluginError::failed)
}

/// Renews or removes the user's expose; one the user does not have, or one
/// that expired, is refused as `expose_not_found`. The new state reaches
/// the pages because it follows the user's exposes.
pub(crate) async fn call(
    method: &str,
    params: Map<String, Value>,
    port: &PluginPort,
) -> Result<Value, PluginError> {
    let ExposeCall { expose } =
        serde_json::from_value(Value::Object(params)).map_err(|error| PluginError::Usage {
            message: error.to_string(),
        })?;
    let not_found = || PluginError::refused("expose_not_found", format!("No expose {expose}"));
    let Ok(id) = ExposeId::try_from(expose.as_str()) else {
        return Err(not_found());
    };
    let done = match method {
        "renew" => port.renew_expose(id, LIFETIME).await.map(|_| ()),
        "remove" => port.remove_expose(id).await,
        method => {
            return Err(PluginError::failed(format!(
                "the expose plugin has no method {method}"
            )));
        }
    };
    match done {
        Ok(()) => Ok(Value::Null),
        Err(PortFailure::Refused(PortRefusal::Expose {
            reason: ExposeRefusal::NotFound,
            ..
        })) => Err(not_found()),
        Err(failure) => Err(failure.into()),
    }
}
