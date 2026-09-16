use chromiumoxide::{
    Page,
    cdp::js_protocol::runtime::{CallArgument, CallFunctionOnParams, EvaluateParams, TimeDelta},
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
        .await
        .map_err(evaluation_error)?
        .into_value::<String>()
        .map_err(|error| BrowserError::InvalidResult(error.to_string()))?;
    if json.len() > MAX_EVAL_BYTES {
        return Err(BrowserError::InvalidResult("result is too large".into()));
    }
    serde_json::from_str(&json).map_err(|error| BrowserError::InvalidResult(error.to_string()))
}

/// Bind observed nodes as lexical values while Chrome enforces read-only evaluation.
pub(super) async fn targeted(
    _page: &Page,
    expression: &str,
    elements: &[super::element::TargetElement],
    all: bool,
) -> Result<Value> {
    if expression.len() > MAX_EVAL_BYTES {
        return Err(BrowserError::ResultTooLarge);
    }
    let first = elements.first().ok_or(BrowserError::TargetNotFound)?;
    if elements.iter().any(|element| {
        element.page.target_id() != first.page.target_id()
            || !element
                .frame_chain
                .iter()
                .map(|(page, node)| (page.target_id(), node))
                .eq(first
                    .frame_chain
                    .iter()
                    .map(|(page, node)| (page.target_id(), node)))
    }) {
        return Err(BrowserError::UnsupportedCapability(
            "eval --all requires matches in one frame document; narrow with --frame or --within"
                .into(),
        ));
    }
    let binding = if all {
        "const elements = args;"
    } else {
        "const element = args[0];"
    };
    let script = format!(
        "function(...args) {{ {binding} const document = this.ownerDocument || this; return ({})(({}), {MAX_EVAL_BYTES}); }}",
        include_str!("read-only.js"),
        expression
    );
    let request = CallFunctionOnParams::builder()
        .object_id(first.remote_object_id.clone())
        .function_declaration(script)
        .arguments(
            elements
                .iter()
                .map(|element| {
                    CallArgument::builder()
                        .object_id(element.remote_object_id.clone())
                        .build()
                })
                .collect::<Vec<_>>(),
        )
        .throw_on_side_effect(true)
        .return_by_value(true)
        .await_promise(false)
        .build()
        .map_err(BrowserError::Configuration)?;
    let json = first
        .page
        .evaluate_function(request)
        .await
        .map_err(evaluation_error)?
        .into_value::<String>()
        .map_err(|error| BrowserError::InvalidResult(error.to_string()))?;
    if json.len() > MAX_EVAL_BYTES {
        return Err(BrowserError::ResultTooLarge);
    }
    serde_json::from_str(&json).map_err(|error| BrowserError::InvalidResult(error.to_string()))
}

fn evaluation_error(error: chromiumoxide::error::CdpError) -> BrowserError {
    if let chromiumoxide::error::CdpError::JavascriptException(details) = &error
        && details
            .exception
            .as_ref()
            .and_then(|exception| exception.description.as_deref())
            .is_some_and(|description| {
                description.contains("Possible side-effect in debug-evaluate")
            })
    {
        return BrowserError::SideEffectRejected;
    }
    BrowserError::from(error)
}
