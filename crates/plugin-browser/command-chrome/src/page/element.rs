//! Shared DOM conditions for browser actions, reads, and element waits.
//!
//! A navigation that replaces a referenced document fails a wait with
//! `stale_ref` instead of treating the old page's disappearance as a condition
//! on the new one. URL patterns are not implicit regular expressions.

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
    protocol::{BrowserTarget, ResolvedElement},
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

/// The page function of the checks before an action (`element-state.js`),
/// with the shared ARIA role resolution (`aria-role.js`) in its scope.
pub(crate) static ELEMENT_STATE: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(
        "async function(...args) {{ const aria = ({})(); return ({}).apply(this, args); }}",
        include_str!("aria-role.js"),
        include_str!("element-state.js"),
    )
});

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
    let mut call = function_call(element, script, args)?;
    call.user_gesture = Some(user_gesture);
    evaluated(&element.page, call).await
}

/// The call of the page function `script` on `element` with `args`, awaiting
/// its promise and answering its value as JSON.
pub(crate) fn function_call(
    element: &TargetElement,
    script: &str,
    args: Vec<Value>,
) -> Result<CallFunctionOnParams> {
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
        .map_err(BrowserError::Configuration)
}

/// Runs `call` in `page` and validates its returned payload at CDP entry.
pub(crate) async fn evaluated<T: DeserializeOwned>(
    page: &Page,
    call: CallFunctionOnParams,
) -> Result<T> {
    let result = page.evaluate_function(call).await?;
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
        &ELEMENT_STATE,
        vec![json!(conditions), json!(scroll)],
    )
    .await?;
    for (page, backend) in &element.frame_chain {
        let frame = TargetElement::resolve(page, *backend).await?;
        let state: ElementState = call(
            page,
            &frame,
            &ELEMENT_STATE,
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

/// How an action brings its element into view, as Playwright does: only when
/// it is out of view on the first check, and on each retry aligned in turn to
/// the end, the center and the start, so that an element a sticky header or
/// footer covers comes out from under it.
#[derive(Clone, Copy)]
pub(crate) enum Scroll {
    None,
    IfNeeded,
    Align(&'static str),
}

impl Scroll {
    /// The scroll of an action's check number `attempt`, the first being 0.
    pub(crate) fn attempt(attempt: usize) -> Self {
        match attempt % 4 {
            0 => Self::IfNeeded,
            1 => Self::Align("end"),
            2 => Self::Align("center"),
            _ => Self::Align("start"),
        }
    }

    fn argument(self) -> Value {
        match self {
            Self::None => json!(false),
            Self::IfNeeded => json!(true),
            Self::Align(alignment) => json!(alignment),
        }
    }
}

/// Scroll and hit-test each browser frame boundary using its own CDP session.
pub(crate) async fn prepared_state(
    page: &Page,
    target: &TargetElement,
    conditions: &[&str],
    scroll: Scroll,
    operation: &crate::driver::operation::Operation<'_>,
) -> Result<ElementState> {
    let scrolls = !matches!(scroll, Scroll::None);
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
    if scrolls {
        for element in frames.iter().rev().chain(std::iter::once(target)) {
            operation
                .run(call::<ElementState>(
                    &element.page,
                    element,
                    &ELEMENT_STATE,
                    vec![json!([]), scroll.argument()],
                ))
                .await?;
        }
    }
    if scrolls && !frames.is_empty() && conditions.contains(&"geometry") {
        // DOM animation frames do not flush the browser process's OOPIF transform.
        operation.run(crate::tabs::viewport::paint(page)).await?;
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
                    &ELEMENT_STATE,
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
            &ELEMENT_STATE,
            vec![json!(conditions), json!(false), json!(probe), json!(false)],
        ))
        .await;
    if result.is_err() && conditions.contains(&"stable") {
        let cleanup = tokio::time::timeout(
            crate::driver::operation::CONTROL_TIMEOUT,
            call::<Option<Value>>(
                page,
                target,
                &ELEMENT_STATE,
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
        *ELEMENT_STATE, script
    );
    call(page, target, &script, args).await
}

/// Translate a browser child document's coordinates through its embedding element.
pub(crate) async fn frame_offset(frame: &TargetElement) -> Result<[f64; 2]> {
    call(&frame.page, frame, "function() { const box = this.getBoundingClientRect(); return [box.left + this.clientLeft, box.top + this.clientTop]; }", vec![]).await
}

/// Release command-owned browser objects without blocking on an open dialog,
/// in the tab's own session and in the frame sessions the command read.
pub(crate) async fn release_objects(tab: &BrowserTab, references: &mut References) -> Result<()> {
    // Chrome cannot answer Runtime commands while a dialog is open. The next
    // ordinary command releases the group; context destruction also releases it.
    if tab.state().dialog.is_open() || tab.ended().is_cancelled() {
        return Ok(());
    }
    tokio::time::timeout(CONTROL_TIMEOUT, async {
        release_group(tab.page().clone()).await?;
        references.release_holders(release_group).await
    })
    .await
    .map_err(|_| BrowserError::Timeout)
    .and_then(std::convert::identity)
}

/// Releases the command's object group in `page`'s session.
async fn release_group(page: Page) -> Result<()> {
    let released = page
        .execute(chromiumoxide::cdp::js_protocol::runtime::ReleaseObjectGroupParams::new(
            OBJECT_GROUP,
        ))
        .await
        .map_err(BrowserError::from);
    match released {
        // A tab or a frame's renderer that went away, or a lost connection,
        // took its objects along.
        Ok(_)
        | Err(BrowserError::Closed | BrowserError::Connection(_) | BrowserError::TabNotFound) => {
            Ok(())
        }
        Err(error) => Err(error),
    }
}

/// An element a target resolved to that meets an action's conditions.
pub(crate) struct Ready {
    pub element: TargetElement,
    pub state: ElementState,
    /// What the page tree names the element, for the action's result.
    pub named: ResolvedElement,
}

/// The pauses between checks of a condition, Playwright's two schedules: an
/// action checks again at once after its first failed check, then after 20,
/// 100 and 100 ms, then every 500 ms (its `_retryAction`); a wait checks again
/// after 20, 50, 100 and 100 ms, then every 500 ms (its `waitFor`'s backoff).
/// Either way a long wait costs the page almost nothing.
pub(crate) struct CheckPauses {
    first: &'static [Duration],
    taken: usize,
}

impl CheckPauses {
    const THEN: Duration = Duration::from_millis(500);
    const ACTION: [Duration; 4] = [
        Duration::ZERO,
        Duration::from_millis(20),
        Duration::from_millis(100),
        Duration::from_millis(100),
    ];
    const WAIT: [Duration; 4] = [
        Duration::from_millis(20),
        Duration::from_millis(50),
        Duration::from_millis(100),
        Duration::from_millis(100),
    ];

    /// An action's pauses before it checks its element again.
    pub(crate) fn action() -> Self {
        Self {
            first: &Self::ACTION,
            taken: 0,
        }
    }

    /// A wait's pauses before it checks its condition again.
    pub(crate) fn wait() -> Self {
        Self {
            first: &Self::WAIT,
            taken: 0,
        }
    }

    /// Waits out the pause before the next check.
    pub(crate) async fn wait_next(&mut self) {
        let pause = self.first.get(self.taken).copied().unwrap_or(Self::THEN);
        self.taken += 1;
        if !pause.is_zero() {
            tokio::time::sleep(pause).await;
        }
    }
}

/// Resolve afresh while waiting; ambiguity is never an implicit first match.
pub(crate) async fn ready(
    tab: &BrowserTab,
    target: &BrowserTarget,
    references: &mut References,
    conditions: &[&str],
    operation: &Operation<'_>,
) -> Result<Ready> {
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
) -> Result<Ready> {
    let mut pauses = CheckPauses::action();
    let mut attempt = 0;
    loop {
        let scroll = if conditions.contains(&"visible") || conditions.contains(&"geometry") {
            Scroll::attempt(attempt)
        } else {
            Scroll::None
        };
        attempt += 1;
        let check = async {
            let located = match operation
                .run(Located::resolve(tab.page(), target, references))
                .await
            {
                Ok(located) => Some(located),
                // The locator can see an insertion after its DOM snapshot.
                // No input has been delivered; the readiness loop resamples.
                Err(BrowserError::StaleReference) if can_resample(target) => None,
                Err(error) => return Err(error),
            };
            if let Some((located, matches)) = located
                && let Some(element) = operation.run(single(tab.page(), matches)).await?
            {
                last_failure.get_or_insert_with(|| BrowserError::NotActionable {
                    condition: if conditions.contains(&"stable") {
                        "stable"
                    } else {
                        "attached"
                    }
                    .into(),
                    interceptor: None,
                });
                let state =
                    prepared_state(tab.page(), &element, conditions, scroll, operation).await?;
                if state.failed.is_none() {
                    let named = operation
                        .run(located.named(tab.page(), &element, references))
                        .await?;
                    return Ok(Some(Ready {
                        element,
                        state,
                        named,
                    }));
                }
                if state.permanent {
                    return Err(state.failure());
                }
                *last_failure = Some(state.failure());
            }
            operation
                .run(async {
                    pauses.wait_next().await;
                    Ok(())
                })
                .await?;
            Ok(None)
        }
        .await;
        match check {
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

/// Where a target's matches came from, which names the one an action takes.
enum Located {
    Direct(crate::page::locate::Direct),
    Observed(Observation),
}

impl Located {
    /// The target's matches: in the main document when it alone answers
    /// them, otherwise from the whole page.
    async fn resolve(
        page: &Page,
        target: &BrowserTarget,
        references: &mut References,
    ) -> Result<(Self, Vec<TargetElement>)> {
        if let Some((direct, matches)) =
            crate::page::locate::resolve(page, target, references).await?
        {
            return Ok((Self::Direct(direct), matches));
        }
        let observation = Observation::capture(page, references).await?;
        let matches = observation.resolve(page, target, references).await?;
        Ok((Self::Observed(observation), matches))
    }

    async fn named(
        &self,
        page: &Page,
        element: &TargetElement,
        references: &mut References,
    ) -> Result<ResolvedElement> {
        match self {
            Self::Direct(direct) => direct.named(page, element, references).await,
            Self::Observed(observation) => observation.named(element, references),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The check times of a pause schedule over a quiet minute.
    async fn checks(mut pauses: CheckPauses) -> Vec<u128> {
        let start = tokio::time::Instant::now();
        let mut checks = Vec::new();
        while start.elapsed() < Duration::from_secs(60) {
            checks.push(start.elapsed().as_millis());
            pauses.wait_next().await;
        }
        checks
    }

    /// A wait checks its condition at once, then after 20, 50, 100 and 100
    /// ms, then every 500 ms: a quiet minute's wait
    /// checks the page 124 times. Before, it checked every 50 ms, 1,200
    /// times, which kept the browser busy for as long as it waited.
    #[tokio::test(start_paused = true)]
    async fn a_wait_checks_quickly_at_first_and_then_every_half_second() {
        let checks = checks(CheckPauses::wait()).await;
        assert_eq!(checks[..7], [0, 20, 70, 170, 270, 770, 1270]);
        assert_eq!(checks.len(), 124);
    }

    /// An action checks its element again at once, as Playwright retries an
    /// action, then after 20, 100 and 100 ms, then every 500 ms.
    #[tokio::test(start_paused = true)]
    async fn an_action_checks_again_at_once_then_as_playwright_retries() {
        let checks = checks(CheckPauses::action()).await;
        assert_eq!(checks[..7], [0, 0, 20, 120, 220, 720, 1220]);
    }
}
