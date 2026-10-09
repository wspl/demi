use std::time::Duration;

use chromiumoxide::{
    Page,
    cdp::js_protocol::runtime::{CallArgument, CallFunctionOnParams, EvaluateParams, TimeDelta},
    error::CdpError,
    js::EvaluationResult,
};
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use crate::driver::operation::{BrowserError, Result};
use crate::page::script;
use crate::tabs::tab::BrowserTab;

const MAX_EVAL_BYTES: usize = 1024 * 1024;

/// What the copy in `read-only.js` throws for a value JSON cannot hold, before
/// the place and the value's kind.
const UNSUPPORTED: &str = "demi-unsupported-result: ";

/// Runs `script` read-only, Chrome enforcing the whole run's read-only scope:
/// the value of its last statement, or none when that is `undefined`
/// (`browser.md` § Evaluation).
pub(crate) async fn read_only(page: &Page, script: &str) -> Result<Option<Value>> {
    let mut refused = None;
    for body in bodies(script)? {
        let wrapped = format!(
            "(() => {{ const value = (() => {{\n{body}\n}})(); return ({})(value, {MAX_EVAL_BYTES}); }})()",
            include_str!("read-only.js"),
        );
        let request = EvaluateParams::builder()
            .expression(wrapped)
            .throw_on_side_effect(true)
            .return_by_value(true)
            .await_promise(false)
            .timeout(TimeDelta::new(1000.0))
            .build()
            .map_err(BrowserError::Configuration)?;
        if let Attempt::Done(value) = attempt(page.evaluate_expression(request).await, &mut refused)? {
            return Ok(value);
        }
    }
    Err(refused.expect("a script stands for at least one body"))
}

/// Runs `script` read-only with the target's element bound as `element`, or
/// with `--all` its elements as `elements`, and their document as
/// `document`.
pub(crate) async fn targeted(
    _page: &Page,
    script: &str,
    elements: &[crate::page::element::TargetElement],
    all: bool,
) -> Result<Option<Value>> {
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
    let mut refused = None;
    for body in bodies(script)? {
        let declaration = format!(
            "function(...args) {{ {binding} const document = this.ownerDocument || this; const value = (() => {{\n{body}\n}})(); return ({})(value, {MAX_EVAL_BYTES}); }}",
            include_str!("read-only.js"),
        );
        let request = CallFunctionOnParams::builder()
            .object_id(first.remote_object_id.clone())
            .function_declaration(declaration)
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
        if let Attempt::Done(value) = attempt(first.page.evaluate_function(request).await, &mut refused)? {
            return Ok(value);
        }
    }
    Err(refused.expect("a script stands for at least one body"))
}

/// The function bodies `script` may stand for ([`script::bodies`]).
fn bodies(script: &str) -> Result<Vec<String>> {
    if script.len() > MAX_EVAL_BYTES {
        return Err(BrowserError::ScriptRefused(
            "the script is larger than 1 MiB".into(),
        ));
    }
    Ok(script::bodies(script))
}

/// What one body's run came to.
enum Attempt {
    /// The value of its last statement: none for `undefined`.
    Done(Option<Value>),
    /// Chrome could not compile it, so another body is tried; `refused`
    /// keeps why.
    Retry,
}

/// Reads the JSON the copy in `read-only.js` returned, `{}` for `undefined`
/// and otherwise `{"value": …}`; a body Chrome could not compile is retried.
fn attempt(
    result: std::result::Result<EvaluationResult, CdpError>,
    refused: &mut Option<BrowserError>,
) -> Result<Attempt> {
    let json = match result {
        Ok(result) => result
            .into_value::<String>()
            .map_err(|error| BrowserError::InvalidResult(error.to_string()))?,
        Err(error) => {
            return match syntax_error(&error) {
                Some(syntax) => {
                    *refused = Some(syntax);
                    Ok(Attempt::Retry)
                }
                None => Err(evaluation_error(error)),
            };
        }
    };
    let mut copied: Map<String, Value> = serde_json::from_str(&json)
        .map_err(|error| BrowserError::InvalidResult(error.to_string()))?;
    Ok(Attempt::Done(copied.remove("value")))
}

/// The first line of what an evaluation threw.
fn thrown(details: &chromiumoxide::cdp::js_protocol::runtime::ExceptionDetails) -> String {
    details
        .exception
        .as_ref()
        .and_then(|exception| exception.description.as_deref())
        .unwrap_or(&details.text)
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// The refusal of a body Chrome could not compile; `await` cannot be
/// written in a script eval runs.
fn syntax_error(error: &CdpError) -> Option<BrowserError> {
    let CdpError::JavascriptException(details) = error else {
        return None;
    };
    let thrown = thrown(details);
    if !thrown.starts_with("SyntaxError") {
        return None;
    }
    Some(if thrown.contains("await is only valid") {
        BrowserError::ScriptRefused(
            "eval returns values that are already there and cannot wait: `await` needs the side effects eval refuses".into(),
        )
    } else {
        BrowserError::ScriptRefused(thrown)
    })
}

/// What a failed evaluation means: a side effect Chrome refused, a value
/// JSON cannot hold, an exception the script threw, or the driver's error.
fn evaluation_error(error: CdpError) -> BrowserError {
    let CdpError::JavascriptException(details) = &error else {
        return BrowserError::from(error);
    };
    let thrown = thrown(details);
    if thrown.contains("Possible side-effect in debug-evaluate") {
        return BrowserError::SideEffectRejected;
    }
    match thrown.split_once(UNSUPPORTED) {
        Some((_, place)) => BrowserError::UnsupportedResult(place.to_owned()),
        None => BrowserError::ScriptRefused(format!("the script threw {thrown}")),
    }
}

/// Runs `script` read-only in the tab, holding its operation lock: the
/// value of its last statement, with `undefined` read as null, as the
/// driver's own checks of a page read it.
pub async fn evaluate(
    tab: &BrowserTab,
    script: &str,
    cancellation: &CancellationToken,
    timeout: Duration,
) -> Result<Value> {
    let _session = tab.state().gate.try_checkout().ok_or(BrowserError::Busy)?;
    tab.operation(cancellation, tokio::time::Instant::now() + timeout)
        .run(read_only(tab.page(), script))
        .await
        .map(|value| value.unwrap_or(Value::Null))
}
