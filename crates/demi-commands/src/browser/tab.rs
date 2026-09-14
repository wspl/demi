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
            page::{
                CaptureScreenshotFormat, CaptureScreenshotParams, EventScreencastFrame,
                ScreencastFrameAckParams, StartScreencastFormat, StartScreencastParams,
                StopScreencastParams,
            },
            target::{CloseTargetParams, EventTargetDestroyed},
        },
        js_protocol::runtime::{CallArgument, CallFunctionOnParams},
    },
};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::sync::{Mutex, watch};
use tokio_util::sync::CancellationToken;

use super::{
    BrowserError, Result, evaluation,
    operation::{CONTROL_TIMEOUT, Operation, after_cleanup},
};

#[derive(Default)]
pub(super) struct TabState {
    operations: Mutex<()>,
    capture: Mutex<()>,
}

#[derive(Clone)]
pub struct BrowserTab {
    page: Page,
    browser: Weak<Mutex<Browser>>,
    ended: CancellationToken,
    state: Arc<TabState>,
}

impl BrowserTab {
    pub(super) fn new(
        page: Page,
        browser: Weak<Mutex<Browser>>,
        ended: CancellationToken,
        state: Arc<TabState>,
    ) -> Self {
        Self {
            page,
            browser,
            ended,
            state,
        }
    }

    /// Transport identity only; the conversation registry will issue public tab IDs.
    pub fn target_id(&self) -> &str {
        self.page.target_id().as_ref()
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
                        CaptureScreenshotParams::builder()
                            .format(CaptureScreenshotFormat::Png)
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
        let _guard = self
            .state
            .operations
            .try_lock()
            .map_err(|_| BrowserError::Busy)?;
        let operation = Operation::new(&self.ended, cancellation, timeout);
        let (_, point) = self.ready_element(selector, false, &operation).await?;
        self.click_at(point, &operation).await
    }

    pub async fn fill_css(
        &self,
        selector: &str,
        text: &str,
        cancellation: &CancellationToken,
        timeout: Duration,
    ) -> Result<()> {
        let _guard = self
            .state
            .operations
            .try_lock()
            .map_err(|_| BrowserError::Busy)?;
        let operation = Operation::new(&self.ended, cancellation, timeout);
        let (element, point) = self.ready_element(selector, true, &operation).await?;
        self.click_at(point, &operation).await?;
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

    /// Publish only the newest frame. The capture ends when its final receiver leaves.
    pub async fn frames(
        &self,
        cancellation: &CancellationToken,
        output: watch::Sender<Option<Arc<EventScreencastFrame>>>,
    ) -> Result<()> {
        let _capture = self
            .state
            .capture
            .try_lock()
            .map_err(|_| BrowserError::Busy)?;
        let mut events = Operation::new(&self.ended, cancellation, CONTROL_TIMEOUT)
            .run(async {
                Ok(self
                    .page
                    .event_listener_with_capacity::<EventScreencastFrame>(2)
                    .await?)
            })
            .await?;
        let start = Operation::new(&self.ended, cancellation, CONTROL_TIMEOUT)
            .run(async {
                self.page
                    .execute(
                        StartScreencastParams::builder()
                            .format(StartScreencastFormat::Jpeg)
                            .quality(80)
                            .build(),
                    )
                    .await?;
                Ok(())
            })
            .await;
        let result = match start {
            Err(error) => Err(error),
            Ok(()) => loop {
                tokio::select! {
                    biased;
                    _ = self.ended.cancelled() => break Err(BrowserError::Closed),
                    _ = cancellation.cancelled() => break Ok(()),
                    _ = output.closed() => break Ok(()),
                    event = events.next() => {
                        let frame = match event {
                            Some(Ok(frame)) => frame,
                            Some(Err(error)) => break Err(error.into()),
                            None => break Err(BrowserError::Closed),
                        };
                        let acknowledgement = self.page.execute(ScreencastFrameAckParams::new(frame.session_id));
                        let result = Operation::new(&self.ended, cancellation, CONTROL_TIMEOUT)
                            .run(async { acknowledgement.await?; Ok(()) }).await;
                        if let Err(error) = result { break Err(error); }
                        output.send_replace(Some(frame));
                    }
                }
            },
        };
        drop(events);
        if self.ended.is_cancelled() {
            // The resource owner closes Chrome; this path must not restart capture.
            return result;
        }
        let cleanup = tokio::time::timeout(
            CONTROL_TIMEOUT,
            self.page.execute(StopScreencastParams::default()),
        )
        .await;
        let cleanup = cleanup
            .map_err(|_| BrowserError::Timeout)
            .and_then(|result| result.map(|_| ()).map_err(BrowserError::from));
        after_cleanup(result, cleanup)
    }

    /// Resolve afresh while waiting; ambiguity is never an implicit first match.
    async fn ready_element(
        &self,
        selector: &str,
        editable: bool,
        operation: &Operation<'_>,
    ) -> Result<(Element, Point)> {
        operation
            .run(async {
                loop {
                    let mut matches = self.page.find_elements(selector).await?;
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
    async fn click_at(&self, point: Point, operation: &Operation<'_>) -> Result<()> {
        let result = operation
            .run(async {
                self.page.click(point).await?;
                Ok(())
            })
            .await;
        if result.is_ok() || self.ended.is_cancelled() {
            return result;
        }
        let release = DispatchMouseEventParams::builder()
            .r#type(DispatchMouseEventType::MouseReleased)
            .button(MouseButton::Left)
            .x(point.x)
            .y(point.y)
            .click_count(1)
            .build()
            .map_err(BrowserError::Configuration)?;
        let cleanup = tokio::time::timeout(CONTROL_TIMEOUT, self.page.execute(release))
            .await
            .map_err(|_| BrowserError::Timeout)
            .and_then(|result| result.map(|_| ()).map_err(BrowserError::from));
        after_cleanup(result, cleanup)
    }
}
