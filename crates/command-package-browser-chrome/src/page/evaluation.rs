use std::time::Duration;

use chromiumoxide::{
    Page,
    cdp::browser_protocol::dom::DescribeNodeParams,
    cdp::js_protocol::runtime::{
        CallArgument, CallFunctionOnParams, EvaluateParams, ExecutionContextId,
        ReleaseObjectGroupParams, RemoteObject, RemoteObjectSubtype, RemoteObjectType, TimeDelta,
    },
    error::CdpError,
};
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use crate::driver::operation::{BrowserError, Result};
use crate::page::element::{OBJECT_GROUP, TargetElement};
use crate::tabs::tab::BrowserTab;

const MAX_EVAL_BYTES: usize = 1024 * 1024;

/// What the copy in `read-only.js` throws for a value JSON cannot hold, before
/// the place and the value's kind.
const UNSUPPORTED: &str = "demi-unsupported-result: ";

/// The object group whose last result the Command Line API names `$_`:
/// Chrome sets it for a call in this group, and a later evaluation in the
/// same context reads it (`includeCommandLineAPI`).
const CONSOLE_GROUP: &str = "console";

/// Runs `script` read-only, Chrome enforcing the whole run's read-only scope:
/// the value of its last statement, or none when that is `undefined`
/// (`browser.md` § Evaluation). The script runs as one block, whose
/// completion value is its last expression statement's value.
pub(crate) async fn read_only(page: &Page, script: &str) -> Result<Option<Value>> {
    checked_size(script)?;
    let value = block(page, None, format!("{{\n{script}\n}}"), false).await?;
    copy(page, None, &value).await
}

/// Runs `script` read-only with the target's element bound as `element`, or
/// with `--all` its elements as `elements`, and their document as
/// `document`. A top-level block cannot name remote objects, so a first
/// read-only call returns the bindings in the console group, whose result
/// the block reads as `$_` in the elements' context.
pub(crate) async fn targeted(
    _page: &Page,
    script: &str,
    elements: &[TargetElement],
    all: bool,
) -> Result<Option<Value>> {
    checked_size(script)?;
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
    let page = &first.page;
    let context = context_of(first).await?;
    let bindings = CallFunctionOnParams::builder()
        .object_id(first.remote_object_id.clone())
        .function_declaration(
            "function(...args) { return {element: args[0], elements: args, document: this.ownerDocument || this}; }",
        )
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
        .object_group(CONSOLE_GROUP)
        .throw_on_side_effect(true)
        .return_by_value(false)
        .await_promise(false)
        .build()
        .map_err(BrowserError::Configuration)?;
    let ran = async {
        page.evaluate_function(bindings)
            .await
            .map_err(evaluation_error)?;
        let bound = if all { "elements" } else { "element" };
        let value = block(
            page,
            context,
            format!("{{ const {{ {bound}, document }} = $_; {{\n{script}\n}} }}"),
            true,
        )
        .await?;
        copy(page, context, &value).await
    }
    .await;
    // The bindings are the console group's only objects of this session.
    let released = page.execute(ReleaseObjectGroupParams::new(CONSOLE_GROUP)).await;
    let value = ran?;
    released?;
    Ok(value)
}

fn checked_size(script: &str) -> Result<()> {
    if script.len() > MAX_EVAL_BYTES {
        return Err(BrowserError::ScriptRefused(
            "the script is larger than 1 MiB".into(),
        ));
    }
    Ok(())
}

/// The execution context of the document `element` is in: none for the
/// page's main frame, which chromiumoxide fills in, otherwise its frame's.
async fn context_of(element: &TargetElement) -> Result<Option<ExecutionContextId>> {
    let Some((owner_page, owner)) = element.frame_chain.last() else {
        return Ok(None);
    };
    let frame = owner_page
        .execute(DescribeNodeParams::builder().backend_node_id(*owner).build())
        .await?
        .result
        .node
        .frame_id
        .ok_or(BrowserError::StaleReference)?;
    Ok(Some(
        element
            .page
            .frame_execution_context(frame)
            .await?
            .ok_or(BrowserError::StaleReference)?,
    ))
}

/// Evaluates `block` read-only in `context`, keeping its value in Chrome.
async fn block(
    page: &Page,
    context: Option<ExecutionContextId>,
    block: String,
    command_line: bool,
) -> Result<RemoteObject> {
    let mut request = EvaluateParams::builder()
        .expression(block)
        .throw_on_side_effect(true)
        .return_by_value(false)
        .await_promise(false)
        .object_group(OBJECT_GROUP)
        .include_command_line_api(command_line)
        .timeout(TimeDelta::new(1000.0))
        .build()
        .map_err(BrowserError::Configuration)?;
    request.context_id = context;
    Ok(page
        .evaluate_expression(request)
        .await
        .map_err(evaluation_error)?
        .object()
        .clone())
}

/// Copies `value` into the JSON `read-only.js` writes, `{}` for `undefined`
/// and otherwise `{"value": …}`, under the same read-only check.
async fn copy(
    page: &Page,
    context: Option<ExecutionContextId>,
    value: &RemoteObject,
) -> Result<Option<Value>> {
    let argument = match (&value.r#type, &value.subtype, &value.object_id) {
        (RemoteObjectType::Undefined, _, _) => CallArgument::builder().build(),
        (_, Some(RemoteObjectSubtype::Null), _) => CallArgument::builder().value(Value::Null).build(),
        (_, _, Some(object)) => CallArgument::builder().object_id(object.clone()).build(),
        _ => match &value.unserializable_value {
            Some(unserializable) => CallArgument::builder()
                .unserializable_value(unserializable.clone())
                .build(),
            None => CallArgument::builder()
                .value(value.value.clone().unwrap_or(Value::Null))
                .build(),
        },
    };
    let mut request = CallFunctionOnParams::builder()
        .function_declaration(format!(
            "function(value) {{ return ({})(value, {MAX_EVAL_BYTES}); }}",
            include_str!("read-only.js"),
        ))
        .argument(argument)
        .throw_on_side_effect(true)
        .return_by_value(true)
        .await_promise(false)
        .build()
        .map_err(BrowserError::Configuration)?;
    match (&value.object_id, context) {
        (Some(object), _) => request.object_id = Some(object.clone()),
        (None, context) => request.execution_context_id = context,
    }
    let json = page
        .evaluate_function(request)
        .await
        .map_err(evaluation_error)?
        .into_value::<String>()
        .map_err(|error| BrowserError::InvalidResult(error.to_string()))?;
    let mut copied: Map<String, Value> = serde_json::from_str(&json)
        .map_err(|error| BrowserError::InvalidResult(error.to_string()))?;
    Ok(copied.remove("value"))
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

/// What a failed evaluation means: a side effect Chrome refused, a script
/// that does not compile (`await` among them), a value JSON cannot hold, an
/// exception the script threw, or the driver's error.
fn evaluation_error(error: CdpError) -> BrowserError {
    let CdpError::JavascriptException(details) = &error else {
        return BrowserError::from(error);
    };
    let thrown = thrown(details);
    if thrown.contains("Possible side-effect in debug-evaluate") {
        return BrowserError::SideEffectRejected;
    }
    if thrown.starts_with("SyntaxError") {
        return BrowserError::ScriptRefused(if thrown.contains("await is only valid") {
            "eval returns values that are already there and cannot wait: `await` needs the side effects eval refuses".into()
        } else {
            thrown
        });
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
