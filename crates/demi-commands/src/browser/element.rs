//! Shared DOM conditions for browser actions, reads, and element waits.

use super::{BrowserError, Result};
use chromiumoxide::{
    Page,
    cdp::{
        browser_protocol::dom::{BackendNodeId, ResolveNodeParams},
        js_protocol::runtime::{CallArgument, CallFunctionOnParams, RemoteObjectId},
    },
};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};

// Chromiumoxide's Element constructor is private and resolves an existing backend
// node through four CDP calls. Browser targets need only these two CDP identities.
// Resolve the remote object once; the admitted command releases its object group.
// Public node references remain owned by the existing References registry.
pub(super) struct TargetElement {
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
            backend_node_id,
            remote_object_id,
        })
    }
}

impl From<chromiumoxide::Element> for TargetElement {
    fn from(element: chromiumoxide::Element) -> Self {
        Self {
            backend_node_id: element.backend_node_id,
            remote_object_id: element.remote_object_id,
        }
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
    page.evaluate_function(
        CallFunctionOnParams::builder()
            .object_id(element.remote_object_id.clone())
            .function_declaration(script)
            .arguments(
                args.into_iter()
                    .map(|value| CallArgument::builder().value(value).build())
                    .collect::<Vec<_>>(),
            )
            .await_promise(true)
            .return_by_value(true)
            .build()
            .map_err(BrowserError::Configuration)?,
    )
    .await?
    .into_value::<T>()
    .map_err(|error| BrowserError::InvalidResult(error.to_string()))
}

pub(super) async fn state(
    page: &Page,
    element: &TargetElement,
    conditions: &[&str],
    scroll: bool,
) -> Result<ElementState> {
    call(
        page,
        element,
        include_str!("element-state.js"),
        vec![json!(conditions), json!(scroll)],
    )
    .await
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

/// Join the browser animation-frame probe's cleanup before returning cancellation.
pub(super) async fn prepared_state(
    page: &Page,
    target: &TargetElement,
    conditions: &[&str],
    scroll: bool,
    operation: &super::operation::Operation<'_>,
) -> Result<ElementState> {
    let probe = super::handles::fresh("probe")?;
    let result = operation
        .run(call(
            page,
            target,
            include_str!("element-state.js"),
            vec![json!(conditions), json!(scroll), json!(probe), json!(false)],
        ))
        .await;
    if result.is_err() && conditions.contains(&"stable") {
        let cleanup = tokio::time::timeout(
            super::operation::CONTROL_TIMEOUT,
            call::<()>(
                page,
                target,
                include_str!("element-state.js"),
                vec![json!([]), json!(false), json!(probe), json!(true)],
            ),
        )
        .await
        .map_err(|_| BrowserError::Timeout)
        .and_then(std::convert::identity);
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
