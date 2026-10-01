//! Shared DOM conditions for browser actions, reads, and element waits.

use std::time::Duration;

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

use crate::driver::operation::{BrowserError, CONTROL_TIMEOUT, Operation, Result};
use crate::tabs::{session::References, tab::BrowserTab};

use crate::page::{
    observation::{Observation, can_resample},
    protocol::BrowserTarget,
};

// Chromiumoxide's Element constructor is private and resolves an existing backend
// node through four CDP calls. Browser targets need only these two CDP identities.
// Resolve the remote object once; the admitted command releases its object group.
// Public node references remain owned by the existing References registry.
pub(crate) struct TargetElement {
    pub page: Page,
    pub frame_chain: Vec<(Page, BackendNodeId)>,
    pub backend_node_id: BackendNodeId,
    pub remote_object_id: RemoteObjectId,
}

pub(crate) const OBJECT_GROUP: &str = "demi-browser-command";

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

pub(crate) const CLICK: &[&str] = &["visible", "enabled", "geometry", "stable", "hit"];
pub(crate) const FILL: &[&str] = &["fillable", "visible", "enabled", "editable"];
pub(crate) const TYPE: &[&str] = &["visible", "enabled", "editable"];
pub(crate) const KEY: &[&str] = &["visible", "enabled"];
pub(crate) const SELECT: &[&str] = &["selectable", "enabled"];
pub(crate) const GEOMETRY: &[&str] = &["geometry"];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ElementState {
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
pub(crate) async fn call<T: DeserializeOwned>(
    page: &Page,
    element: &TargetElement,
    script: &str,
    args: Vec<Value>,
) -> Result<T> {
    call_with_gesture(page, element, script, args, false).await
}

/// Invoke a browser chooser control with Chrome's explicit user-gesture flag.
pub(crate) async fn call_with_user_gesture<T: DeserializeOwned>(
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

pub(crate) async fn state(
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
pub(crate) async fn single(
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
pub(crate) async fn prepared_state(
    page: &Page,
    target: &TargetElement,
    conditions: &[&str],
    scroll: bool,
    operation: &crate::driver::operation::Operation<'_>,
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
    if scroll && !frames.is_empty() && conditions.contains(&"geometry") {
        // DOM animation frames do not flush the browser process's OOPIF transform.
        operation
            .run(crate::tabs::viewport::paint(page))
            .await?;
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
    operation: &crate::driver::operation::Operation<'_>,
) -> Result<ElementState> {
    let probe = crate::driver::handles::fresh("probe")?;
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
            crate::driver::operation::CONTROL_TIMEOUT,
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
        crate::driver::operation::after_cleanup(result, cleanup)
    } else {
        result
    }
}

#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum FillKind {
    None,
    Native,
    Text,
}

/// Run a browser form algorithm with access to the one element-state computation.
pub(crate) async fn call_with_states<T: DeserializeOwned>(
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
pub(crate) async fn frame_offset(frame: &TargetElement) -> Result<[f64; 2]> {
    call(&frame.page, frame, "function() { const box = this.getBoundingClientRect(); return [box.left + this.clientLeft, box.top + this.clientTop]; }", vec![]).await
}

/// Release command-owned browser objects without blocking on an open dialog.
pub(crate) async fn release_objects(tab: &BrowserTab) -> Result<()> {
    // Chrome cannot answer Runtime commands while a dialog is open. The next
    // ordinary command releases the group; context destruction also releases it.
    if tab.state().dialog.is_open() || tab.ended().is_cancelled() {
        return Ok(());
    }
    let cleanup = tokio::time::timeout(CONTROL_TIMEOUT, async {
        let snapshot = crate::driver::frames::capture(tab.page()).await?;
        let mut released = std::collections::HashSet::new();
        for document in snapshot.frames {
            if released.insert(document.page.target_id().clone()) {
                document
                    .page
                    .execute(
                        chromiumoxide::cdp::js_protocol::runtime::ReleaseObjectGroupParams::new(
                            OBJECT_GROUP,
                        ),
                    )
                    .await?;
            }
        }
        Ok::<_, BrowserError>(())
    })
    .await
    .map_err(|_| BrowserError::Timeout)
    .and_then(std::convert::identity);
    // A destroyed tab or connection no longer retains remote objects.
    match cleanup {
        Err(BrowserError::Closed | BrowserError::Connection(_) | BrowserError::TabNotFound) => {
            Ok(())
        }
        result => result,
    }
}

/// Resolve afresh while waiting; ambiguity is never an implicit first match.
pub(crate) async fn ready(
    tab: &BrowserTab,
    target: &BrowserTarget,
    references: &mut References,
    conditions: &[&str],
    operation: &Operation<'_>,
) -> Result<(TargetElement, ElementState)> {
    ready_with_failure(tab, target, references, conditions, operation, &mut None).await
}

/// Nested option waits retain their known failure while refreshing the browser target.
pub(crate) async fn ready_with_failure(
    tab: &BrowserTab,
    target: &BrowserTarget,
    references: &mut References,
    conditions: &[&str],
    operation: &Operation<'_>,
    last_failure: &mut Option<BrowserError>,
) -> Result<(TargetElement, ElementState)> {
    loop {
        let attempt = async {
            let observation = operation
                .run(Observation::capture(tab.page(), references))
                .await?;
            let matches = match operation
                .run(observation.resolve(tab.page(), target, references))
                .await
            {
                Ok(matches) => matches,
                // The locator can see an insertion after its DOM snapshot.
                // No input has been delivered; the readiness loop resamples.
                Err(BrowserError::StaleReference) if can_resample(target) => Vec::new(),
                Err(error) => return Err(error),
            };
            let selected = operation.run(single(tab.page(), matches)).await?;
            if let Some(element) = selected {
                last_failure.get_or_insert_with(|| BrowserError::NotActionable {
                    condition: if conditions.contains(&"stable") {
                        "stable"
                    } else {
                        "attached"
                    }
                    .into(),
                    interceptor: None,
                });
                let state = prepared_state(
                    tab.page(),
                    &element,
                    conditions,
                    conditions.contains(&"visible") || conditions.contains(&"geometry"),
                    operation,
                )
                .await?;
                if state.failed.is_none() {
                    return Ok(Some((element, state)));
                }
                if state.permanent {
                    return Err(state.failure());
                }
                *last_failure = Some(state.failure());
            }
            operation
                .run(async {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    Ok(())
                })
                .await?;
            Ok(None)
        }
        .await;
        match attempt {
            Ok(Some(element)) => return Ok(element),
            Ok(None) => {}
            Err(error) if error.is_deadline() => {
                return Err(error.with_deadline_cause(
                    last_failure.take().unwrap_or(BrowserError::TargetNotFound),
                ));
            }
            Err(error) => return Err(error),
        }
    }
}
