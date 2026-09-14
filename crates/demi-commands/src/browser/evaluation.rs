use chromiumoxide::{
    Page,
    cdp::js_protocol::runtime::{EvaluateParams, TimeDelta},
};
use serde_json::Value;

use super::{BrowserError, Result};

const MAX_EVAL_BYTES: usize = 1024 * 1024;

/// Evaluate and serialize while Chrome enforces the entire expression's read-only scope.
pub(super) async fn read_only(page: &Page, expression: &str) -> Result<Value> {
    if expression.len() > MAX_EVAL_BYTES {
        return Err(BrowserError::InvalidResult(
            "expression is too large".into(),
        ));
    }
    let wrapped = format!(
        "{}(\n({})\n, {MAX_EVAL_BYTES})",
        include_str!("read-only.js"),
        expression
    );
    let request = EvaluateParams::builder()
        .expression(wrapped)
        .throw_on_side_effect(true)
        .return_by_value(true)
        .await_promise(false)
        .timeout(TimeDelta::new(1000.0))
        .build()
        .map_err(BrowserError::Configuration)?;
    let json = page
        .evaluate_expression(request)
        .await?
        .into_value::<String>()
        .map_err(|error| BrowserError::InvalidResult(error.to_string()))?;
    if json.len() > MAX_EVAL_BYTES {
        return Err(BrowserError::InvalidResult("result is too large".into()));
    }
    serde_json::from_str(&json).map_err(|error| BrowserError::InvalidResult(error.to_string()))
}
