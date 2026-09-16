//! Shared DOM conditions for browser actions, reads, and element waits.

use super::{BrowserError, Result};
use chromiumoxide::{
    Page,
    cdp::{
        browser_protocol::dom::{BackendNodeId, ResolveNodeParams},
        js_protocol::runtime::{
            CallArgument, CallFunctionOnParams, RemoteObjectId, RemoteObjectSubtype,
            RemoteObjectType,
        },
    },
};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};

// Chromiumoxide's Element constructor is private and resolves an existing backend
// node through four CDP calls. Browser targets need only these two CDP identities.
// Resolve the remote object once; the admitted command releases its object group.
// Public node references remain owned by the existing References registry.
pub(super) struct TargetElement {
    pub page: Page,
    pub frame_chain: Vec<(Page, BackendNodeId)>,
    pub backend_node_id: BackendNodeId,
    pub remote_object_id: RemoteObjectId,
}

pub(super) const OBJECT_GROUP: &str = "demi-browser-command";

impl TargetElement {
    /// Resolve an observed browser node in one CDP request without another DOM scan.
    pub async fn resolve(page: &Page, backend_node_id: BackendNodeId) -> Result<Self> {
        let object = page
            .execute(
                ResolveNodeParams::builder()
                    .backend_node_id(backend_node_id)
                    .object_group(OBJECT_GROUP)
                    .build(),
            )
            .await?
            .result
            .object;
        let remote_object_id = object.object_id.ok_or(BrowserError::StaleReference)?;
        Ok(Self {
            page: page.clone(),
            frame_chain: Vec::new(),
            backend_node_id,
            remote_object_id,
        })
    }
}

impl TargetElement {
    pub fn identity(
        &self,
    ) -> (
        chromiumoxide::cdp::browser_protocol::target::TargetId,
        BackendNodeId,
    ) {
        (self.page.target_id().clone(), self.backend_node_id)
    }
}

pub(super) const CLICK: &[&str] = &["visible", "enabled", "geometry", "stable", "hit"];
pub(super) const FILL: &[&str] = &["fillable", "visible", "enabled", "editable"];
pub(super) const TYPE: &[&str] = &["visible", "enabled", "editable"];
pub(super) const KEY: &[&str] = &["visible", "enabled"];
pub(super) const SELECT: &[&str] = &["selectable", "enabled"];
pub(super) const GEOMETRY: &[&str] = &["geometry"];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ElementState {
    #[serde(rename = "fillKind")]
    pub fill_kind: FillKind,
    pub attached: bool,
    pub visible: bool,
    pub enabled: bool,
    pub checked: Option<bool>,
    pub radio: bool,
    pub failed: Option<String>,
    pub interceptor: Option<String>,
    pub permanent: bool,
    pub x: f64,
    pub y: f64,
}

impl ElementState {
    pub fn point(&self) -> chromiumoxide::layout::Point {
        chromiumoxide::layout::Point {
            x: self.x,
            y: self.y,
        }
    }

    /// Reject inapplicable checkbox requests before deciding whether a click is needed.
    pub fn needs_check(&self, desired: bool) -> Result<bool> {
        if self.radio && !desired {
            return Err(BrowserError::Configuration(
                "a radio cannot be unchecked".into(),
            ));
        }
        let checked = self.checked.ok_or_else(|| BrowserError::NotActionable {
            condition: "checkable".into(),
            interceptor: None,
        })?;
        Ok(checked != desired)
    }

    pub fn failure(&self) -> BrowserError {
        BrowserError::NotActionable {
            condition: self.failed.clone().unwrap_or_else(|| "attached".into()),
            interceptor: self.interceptor.clone(),
        }
    }
}

/// Evaluate a browser element algorithm and validate its returned payload at CDP entry.
pub(super) async fn call<T: DeserializeOwned>(
    page: &Page,
    element: &TargetElement,
    script: &str,
    args: Vec<Value>,
) -> Result<T> {
    call_with_gesture(page, element, script, args, false).await
}

/// Invoke a browser chooser control with Chrome's explicit user-gesture flag.
pub(super) async fn call_with_user_gesture<T: DeserializeOwned>(
    page: &Page,
    element: &TargetElement,
    script: &str,
    args: Vec<Value>,
) -> Result<T> {
    call_with_gesture(page, element, script, args, true).await
}

async fn call_with_gesture<T: DeserializeOwned>(
    _page: &Page,
    element: &TargetElement,
    script: &str,
    args: Vec<Value>,
    user_gesture: bool,
) -> Result<T> {
    let result = element
        .page
        .evaluate_function(
            CallFunctionOnParams::builder()
                .object_id(element.remote_object_id.clone())
                .function_declaration(script)
                .arguments(
                    args.into_iter()
                        .map(|value| CallArgument::builder().value(value).build())
                        .collect::<Vec<_>>(),
                )
                .await_promise(true)
                .user_gesture(user_gesture)
                .return_by_value(true)
                .build()
                .map_err(BrowserError::Configuration)?,
        )
        .await?;
    let object = result.object();
    let value = match &object.value {
        Some(value) => value.clone(),
        // Chrome omits value for null/undefined; Chromiumoxide's into_value
        // rejects that valid response, including animation-probe cancellation.
        None if object.subtype == Some(RemoteObjectSubtype::Null)
            || object.r#type == RemoteObjectType::Undefined =>
        {
            Value::Null
        }
        None => {
            return Err(BrowserError::InvalidResult(
                "element call returned no JSON value".into(),
            ));
        }
    };
    serde_json::from_value(value).map_err(|error| BrowserError::InvalidResult(error.to_string()))
}

pub(super) async fn state(
    page: &Page,
    element: &TargetElement,
    conditions: &[&str],
    scroll: bool,
) -> Result<ElementState> {
    let mut result: ElementState = call(
        page,
        element,
        include_str!("element-state.js"),
        vec![json!(conditions), json!(scroll)],
    )
    .await?;
    for (page, backend) in &element.frame_chain {
        let frame = TargetElement::resolve(page, *backend).await?;
        let state: ElementState = call(
            page,
            &frame,
            include_str!("element-state.js"),
            vec![json!([]), json!(false)],
        )
        .await?;
        result.visible &= state.visible;
        result.enabled &= state.enabled;
    }
    if result.failed.is_none() {
        if conditions.contains(&"visible") && !result.visible {
            result.failed = Some("visible".into());
        } else if conditions.contains(&"enabled") && !result.enabled {
            result.failed = Some("enabled".into());
        }
    }
    Ok(result)
}

/// Single-target browser mutations use the unique visible match when one exists.
pub(super) async fn single(
    page: &Page,
    mut elements: Vec<TargetElement>,
) -> Result<Option<TargetElement>> {
    if elements.len() <= 1 {
        return Ok(elements.pop());
    }
    let count = elements.len();
    let mut visible = Vec::new();
    for element in elements {
        if state(page, &element, &[], false).await?.visible {
            visible.push(element);
        }
    }
    if visible.len() == 1 {
        Ok(visible.pop())
    } else {
        Err(BrowserError::Ambiguous(count))
    }
}

/// Scroll and hit-test each browser frame boundary using its own CDP session.
pub(super) async fn prepared_state(
    page: &Page,
    target: &TargetElement,
    conditions: &[&str],
    scroll: bool,
    operation: &super::operation::Operation<'_>,
) -> Result<ElementState> {
    let mut frames = Vec::new();
    for (page, backend) in &target.frame_chain {
        frames.push(
            operation
                .run(TargetElement::resolve(page, *backend))
                .await?,
        );
    }
    // Scrolling the target can scroll its ancestors, including an OOPIF's
    // embedding document. Finish every scroll before observing stability in
    // those documents; otherwise input can race the parent's compositor update.
    if scroll {
        for element in frames.iter().rev().chain(std::iter::once(target)) {
            operation
                .run(call::<ElementState>(
                    &element.page,
                    element,
                    include_str!("element-state.js"),
                    vec![json!([]), json!(true)],
                ))
                .await?;
        }
    }
    for frame in frames.iter().rev() {
        let mut frame_conditions = vec!["geometry"];
        for condition in ["visible", "enabled", "stable"] {
            if conditions.contains(&condition) {
                frame_conditions.push(condition);
            }
        }
        let state = local_prepared_state(&frame.page, frame, &frame_conditions, operation).await?;
        if state.failed.is_some() {
            return Ok(state);
        }
    }
    let mut state = local_prepared_state(page, target, conditions, operation).await?;
    if state.failed.is_some() {
        return Ok(state);
    }
    for frame in &frames {
        let offset = operation.run(frame_offset(frame)).await?;
        state.x += offset[0];
        state.y += offset[1];
        if conditions.contains(&"hit") {
            let hit: ElementState = operation
                .run(call(
                    &frame.page,
                    frame,
                    include_str!("element-state.js"),
                    vec![
                        json!(["visible", "geometry", "hit"]),
                        json!(false),
                        Value::Null,
                        json!(false),
                        json!([state.x, state.y]),
                    ],
                ))
                .await?;
            if hit.failed.is_some() {
                return Ok(hit);
            }
        }
    }
    Ok(state)
}

/// Join the browser animation-frame probe's cleanup before returning cancellation.
async fn local_prepared_state(
    page: &Page,
    target: &TargetElement,
    conditions: &[&str],
    operation: &super::operation::Operation<'_>,
) -> Result<ElementState> {
    let probe = super::handles::fresh("probe")?;
    let result = operation
        .run(call(
            page,
            target,
            include_str!("element-state.js"),
            vec![json!(conditions), json!(false), json!(probe), json!(false)],
        ))
        .await;
    if result.is_err() && conditions.contains(&"stable") {
        let cleanup = tokio::time::timeout(
            super::operation::CONTROL_TIMEOUT,
            call::<Option<Value>>(
                page,
                target,
                include_str!("element-state.js"),
                vec![json!([]), json!(false), json!(probe), json!(true)],
            ),
        )
        .await
        .map_err(|_| BrowserError::Timeout)
        .and_then(std::convert::identity)
        .map(|_| ());
        // A destroyed context cannot retain a pending animation callback.
        let cleanup = match cleanup {
            Err(BrowserError::Connection(_) | BrowserError::Closed) => Ok(()),
            result => result,
        };
        super::operation::after_cleanup(result, cleanup)
    } else {
        result
    }
}

#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(super) enum FillKind {
    None,
    Native,
    Text,
}

/// Run a browser form algorithm with access to the one element-state computation.
pub(super) async fn call_with_states<T: DeserializeOwned>(
    page: &Page,
    target: &TargetElement,
    script: &str,
    args: Vec<Value>,
) -> Result<T> {
    let script = format!(
        "async function(...args) {{ const elementState = ({}); return ({}).apply(this, args); }}",
        include_str!("element-state.js"),
        script
    );
    call(page, target, &script, args).await
}

/// Translate a browser child document's coordinates through its embedding element.
pub(super) async fn frame_offset(frame: &TargetElement) -> Result<[f64; 2]> {
    call(&frame.page, frame, "function() { const box = this.getBoundingClientRect(); return [box.left + this.clientLeft, box.top + this.clientTop]; }", vec![]).await
}
