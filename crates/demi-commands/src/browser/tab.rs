use std::{
    sync::{Arc, Weak},
    time::Duration,
};

use chromiumoxide::layout::Point;
use chromiumoxide::{
    Browser, Element, Page,
    cdp::{
        browser_protocol::{
            input::{
                DispatchMouseEventParams, DispatchMouseEventType, InsertTextParams, MouseButton,
            },
            page::{CaptureScreenshotFormat, EventJavascriptDialogOpening},
            target::{CloseTargetParams, EventTargetDestroyed},
        },
        js_protocol::runtime::{CallArgument, CallFunctionOnParams},
    },
};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::sync::{Mutex, watch};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use super::{
    BrowserError, Result, evaluation,
    observation::{Observation, References},
    operation::{CONTROL_TIMEOUT, Operation, after_cleanup},
    protocol::BrowserCreatedBy,
    protocol::BrowserTarget,
};

pub(super) struct TabState {
    pub operations: Mutex<References>,
    pub dialog: watch::Sender<Option<Arc<EventJavascriptDialogOpening>>>,
    pub deferred_release: Mutex<Option<DispatchMouseEventParams>>,
}

impl TabState {
    pub(super) async fn observe(
        page: &Page,
        ended: CancellationToken,
        tasks: &TaskTracker,
        failure: watch::Sender<Option<String>>,
    ) -> Result<Arc<Self>> {
        // Headless dialogs are closed only by this controller. Handling clears
        // the exact opening incarnation, so a subsequent prompt cannot be erased
        // by a delayed close event delivered through a separate event stream.
        let mut opening = page
            .event_listener::<EventJavascriptDialogOpening>()
            .await?;
        let state = Arc::new(Self {
            operations: Mutex::new(References::default()),
            dialog: watch::channel(None).0,
            deferred_release: Mutex::new(None),
        });
        let observed = state.clone();
        tasks.spawn(async move {
            loop {
                let event = tokio::select! {
                    biased;
                    _ = ended.cancelled() => break,
                    event = opening.next() => event.map(|event| event.map(Some)),
                };
                match event {
                    Some(Ok(dialog)) => {
                        observed.dialog.send_replace(dialog);
                    }
                    Some(Err(error)) => {
                        failure.send_replace(Some(format!(
                            "browser dialog observation failed: {error}"
                        )));
                        ended.cancel();
                        break;
                    }
                    None => break,
                }
            }
        });
        Ok(state)
    }
}

#[derive(Clone)]
pub struct BrowserTab {
    pub(super) page: Page,
    browser: Weak<Mutex<Browser>>,
    pub(super) ended: CancellationToken,
    pub(super) state: Arc<TabState>,
    generation: Arc<str>,
    pub(super) created_by: BrowserCreatedBy,
}

impl BrowserTab {
    pub(super) fn new(
        page: Page,
        browser: Weak<Mutex<Browser>>,
        ended: CancellationToken,
        state: Arc<TabState>,
        generation: Arc<str>,
        created_by: BrowserCreatedBy,
    ) -> Self {
        Self {
            page,
            browser,
            ended,
            state,
            generation,
            created_by,
        }
    }

    /// Transport identity only; the conversation registry will issue public tab IDs.
    pub fn target_id(&self) -> &str {
        self.page.target_id().as_ref()
    }

    pub(super) fn id(&self) -> String {
        format!("{}-{}", self.generation, self.target_id())
    }

    pub async fn read_only(
        &self,
        expression: &str,
        cancellation: &CancellationToken,
        timeout: Duration,
    ) -> Result<Value> {
        let _guard = self
            .state
            .operations
            .try_lock()
            .map_err(|_| BrowserError::Busy)?;
        Operation::new(&self.ended, cancellation, timeout)
            .run(evaluation::read_only(&self.page, expression))
            .await
    }

    pub async fn screenshot(
        &self,
        cancellation: &CancellationToken,
        timeout: Duration,
    ) -> Result<Vec<u8>> {
        self.capture(false, cancellation, timeout).await
    }

    pub(super) async fn capture(
        &self,
        full_page: bool,
        cancellation: &CancellationToken,
        timeout: Duration,
    ) -> Result<Vec<u8>> {
        let _guard = self
            .state
            .operations
            .try_lock()
            .map_err(|_| BrowserError::Busy)?;
        Operation::new(&self.ended, cancellation, timeout)
            .run(async {
                Ok(self
                    .page
                    .screenshot(
                        chromiumoxide::page::ScreenshotParams::builder()
                            .format(CaptureScreenshotFormat::Png)
                            .full_page(full_page)
                            .build(),
                    )
                    .await?)
            })
            .await
    }

    pub async fn click_css(
        &self,
        selector: &str,
        cancellation: &CancellationToken,
        timeout: Duration,
    ) -> Result<()> {
        let mut references = self
            .state
            .operations
            .try_lock()
            .map_err(|_| BrowserError::Busy)?;
        let operation = Operation::new(&self.ended, cancellation, timeout);
        let target = serde_json::from_value(json!({ "css": selector }))
            .map_err(|error| BrowserError::Configuration(error.to_string()))?;
        let (_, point) = self
            .ready_element(&target, &mut references, false, &operation)
            .await?;
        self.click_at(point, MouseButton::Left, 1, 0, &operation)
            .await
    }

    pub async fn fill_css(
        &self,
        selector: &str,
        text: &str,
        cancellation: &CancellationToken,
        timeout: Duration,
    ) -> Result<()> {
        let mut references = self
            .state
            .operations
            .try_lock()
            .map_err(|_| BrowserError::Busy)?;
        let operation = Operation::new(&self.ended, cancellation, timeout);
        let target = serde_json::from_value(json!({ "css": selector }))
            .map_err(|error| BrowserError::Configuration(error.to_string()))?;
        self.fill(&target, &mut references, text, &operation).await
    }

    pub(super) async fn fill(
        &self,
        target: &BrowserTarget,
        references: &mut References,
        text: &str,
        operation: &Operation<'_>,
    ) -> Result<()> {
        let (element, point) = self
            .ready_element(target, references, true, operation)
            .await?;
        self.click_at(point, MouseButton::Left, 1, 0, &operation)
            .await?;
        operation
            .run(async {
                // The library types text but has no replace-selection operation.
                // Select this editable element's contents, then use native text input.
                self.page
                    .evaluate_function(
                        CallFunctionOnParams::builder()
                            .object_id(element.remote_object_id.clone())
                            .function_declaration(include_str!("select-contents.js"))
                            .await_promise(false)
                            .build()
                            .map_err(BrowserError::Configuration)?,
                    )
                    .await?;
                self.page.execute(InsertTextParams::new(text)).await?;
                Ok(())
            })
            .await
    }

    /// A tab owner closes the target without running beforeunload hooks.
    pub async fn close(&self, cancellation: &CancellationToken, timeout: Duration) -> Result<()> {
        let _guard = self
            .state
            .operations
            .try_lock()
            .map_err(|_| BrowserError::Busy)?;
        Operation::new(&self.ended, cancellation, timeout)
            .run(async {
                let browser = self.browser.upgrade().ok_or(BrowserError::Closed)?;
                let browser = browser.lock().await;
                let mut events = browser.event_listener::<EventTargetDestroyed>().await?;
                browser
                    .execute(CloseTargetParams::new(self.page.target_id().clone()))
                    .await?;
                drop(browser);
                while let Some(event) = events.next().await {
                    let event = event?;
                    if event.target_id == *self.page.target_id() {
                        return Ok(());
                    }
                }
                Err(BrowserError::Closed)
            })
            .await
    }

    /// Resolve afresh while waiting; ambiguity is never an implicit first match.
    pub(super) async fn ready_element(
        &self,
        target: &BrowserTarget,
        references: &mut References,
        editable: bool,
        operation: &Operation<'_>,
    ) -> Result<(Element, Point)> {
        operation
            .run(async {
                loop {
                    let observation = Observation::capture(&self.page, references).await?;
                    let mut matches = observation.resolve(&self.page, target, references).await?;
                    if matches.len() > 1 {
                        return Err(BrowserError::Ambiguous(matches.len()));
                    }
                    if let Some(element) = matches.pop() {
                        element.scroll_into_view().await?;
                        match element.clickable_point().await {
                            Ok(point) => {
                                if !point.x.is_finite() || !point.y.is_finite() {
                                    return Err(BrowserError::InvalidResult(
                                        "non-finite browser coordinates".into(),
                                    ));
                                }
                                let ready = self
                                    .page
                                    .evaluate_function(
                                        CallFunctionOnParams::builder()
                                            .object_id(element.remote_object_id.clone())
                                            .function_declaration(include_str!("actionability.js"))
                                            .arguments(vec![
                                                CallArgument::builder()
                                                    .value(json!(point.x))
                                                    .build(),
                                                CallArgument::builder()
                                                    .value(json!(point.y))
                                                    .build(),
                                                CallArgument::builder()
                                                    .value(json!(editable))
                                                    .build(),
                                            ])
                                            .return_by_value(true)
                                            .build()
                                            .map_err(BrowserError::Configuration)?,
                                    )
                                    .await?
                                    .into_value::<bool>()
                                    .map_err(|error| {
                                        BrowserError::InvalidResult(error.to_string())
                                    })?;
                                if ready {
                                    return Ok((element, point));
                                }
                            }
                            Err(chromiumoxide::error::CdpError::ChromeMessage(message))
                                if message
                                    == "Node is either not visible or not an HTMLElement" =>
                            {
                                // Upstream has no typed no-quad error. This exact error means
                                // the element has no clickable area yet; wait within the deadline.
                            }
                            Err(error) => return Err(error.into()),
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
            .await
    }

    /// Complete or release input at the point approved by the browser action checks.
    pub(super) async fn click_at(
        &self,
        point: Point,
        button: MouseButton,
        count: i64,
        modifiers: i64,
        operation: &Operation<'_>,
    ) -> Result<()> {
        let mut dialog = self.state.dialog.subscribe();
        let click = async {
            if button == MouseButton::Left && modifiers == 0 {
                self.page
                    .click_with(
                        point,
                        chromiumoxide::types::ClickOptions::builder()
                            .click_count(count)
                            .build(),
                    )
                    .await?;
            } else {
                // The library click API supports only left-button clicks without modifiers.
                let event = DispatchMouseEventParams::builder()
                    .x(point.x)
                    .y(point.y)
                    .button(button.clone())
                    .click_count(count)
                    .modifiers(modifiers);
                self.page.move_mouse(point).await?;
                self.page
                    .execute(
                        event
                            .clone()
                            .r#type(DispatchMouseEventType::MousePressed)
                            .build()
                            .map_err(BrowserError::Configuration)?,
                    )
                    .await?;
                self.page
                    .execute(
                        event
                            .r#type(DispatchMouseEventType::MouseReleased)
                            .build()
                            .map_err(BrowserError::Configuration)?,
                    )
                    .await?;
            }
            Ok(())
        };
        let result = operation
            .run(async {
                tokio::select! {
                    result = click => result,
                    opened = dialog.wait_for(|dialog| dialog.is_some()) => {
                        opened.map_err(|_| BrowserError::Closed)?;
                        Err(BrowserError::DialogBlocked)
                    },
                }
            })
            .await;
        if result.is_ok() || self.ended.is_cancelled() {
            return result;
        }
        let release = DispatchMouseEventParams::builder()
            .r#type(DispatchMouseEventType::MouseReleased)
            .button(button)
            .x(point.x)
            .y(point.y)
            .click_count(count)
            .modifiers(0)
            .build()
            .map_err(BrowserError::Configuration)?;
        if self.state.dialog.borrow().is_some() {
            // Chrome can pause before acknowledging mouse-down or mouse-up. The
            // retained tab owns the required release until the dialog is handled.
            *self.state.deferred_release.lock().await = Some(release);
            return Err(BrowserError::DialogBlocked);
        }
        let cleanup = tokio::time::timeout(CONTROL_TIMEOUT, self.page.execute(release))
            .await
            .map_err(|_| BrowserError::Timeout)
            .and_then(|result| result.map(|_| ()).map_err(BrowserError::from));
        after_cleanup(result, cleanup)
    }
}
