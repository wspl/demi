//! What only the Chrome tests reach: evaluation in a target no command
//! addresses.

use chromiumoxide::{
    cdp::browser_protocol::target::{AttachToTargetReturns, TargetId},
    conn::Connection,
};
use serde_json::{Value, json};

use crate::driver::operation::{BrowserError, Result};
use crate::tabs::environment::BrowserEnvironment;

use crate::cdp::sessions::{WireEvent, roundtrip};

/// Evaluates `expression` in `target` over a CDP connection of its own, for
/// the Chrome tests: the public tab-scoped debugger deliberately cannot
/// address a target such as the capture extension's worker.
pub async fn evaluate_in(
    environment: &BrowserEnvironment,
    target: TargetId,
    expression: &str,
) -> Result<Value> {
    let address = environment.browser().call()?.websocket_address().clone();
    let mut socket = Connection::<WireEvent>::connect(address).await?;
    let attached = roundtrip(
        &mut socket,
        "Target.attachToTarget",
        json!({"targetId": target, "flatten": true}),
        None,
    )
    .await?;
    let attached: AttachToTargetReturns = serde_json::from_value(attached)
        .map_err(|error| BrowserError::InvalidResult(error.to_string()))?;
    let evaluated = roundtrip(
        &mut socket,
        "Runtime.evaluate",
        json!({"expression": expression, "returnByValue": true}),
        Some(attached.session_id),
    )
    .await?;
    Ok(evaluated["result"]["value"].clone())
}
