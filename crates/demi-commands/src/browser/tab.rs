use std::{
    sync::{Arc, Weak},
    time::Duration,
};

use chromiumoxide::layout::Point;
use chromiumoxide::{
    Browser, Page,
    cdp::browser_protocol::{
        input::{
            DispatchKeyEventParams, DispatchMouseEventParams, DispatchMouseEventType,
            InsertTextParams, MouseButton,
        },
        page::{CaptureScreenshotFormat, EventJavascriptDialogOpening},
        target::{CloseTargetParams, EventTargetDestroyed},
    },
};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::sync::{Mutex, watch};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use super::{
    BrowserError, Result, element, evaluation, keyboard,
    observation::{Observation, References},
    operation::{CONTROL_TIMEOUT, Operation, after_cleanup},
    protocol::BrowserCreatedBy,
    protocol::BrowserTarget,
};

pub(super) enum InputRelease {
    Mouse(DispatchMouseEventParams),
    Key(DispatchKeyEventParams),
}

pub(super) struct TabState {
    pub operations: Mutex<References>,
    pub dialog: watch::Sender<Option<Arc<EventJavascriptDialogOpening>>>,
    pub deferred_release: Mutex<Vec<InputRelease>>,
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
            deferred_release: Mutex::new(Vec::new()),
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
    id: Arc<str>,
    pub(super) created_by: BrowserCreatedBy,
}

impl BrowserTab {
    pub(super) fn new(
        page: Page,
        browser: Weak<Mutex<Browser>>,
        ended: CancellationToken,
        state: Arc<TabState>,
        id: Arc<str>,
        created_by: BrowserCreatedBy,
    ) -> Self {
        Self {
            page,
            browser,
            ended,
            state,
            id,
            created_by,
        }
    }

    /// Transport identity only; the conversation registry will issue public tab IDs.
    pub fn target_id(&self) -> &str {
        self.page.target_id().as_ref()
    }

    pub fn id(&self) -> String {
        self.id.to_string()
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
        let (_, state) = self
            .ready_element(&target, &mut references, element::CLICK, &operation)
            .await?;
        self.click_at(state.point(), MouseButton::Left, 1, 0, &operation)
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
        let (target, state) = self
            .ready_element(target, references, element::FILL, operation)
            .await?;
        let native = state.fill_kind == element::FillKind::Native;
        let mode: FillMode = operation
            .run(async {
                if native {
                    operation.begin_input();
                }
                let mode = element::call(
                    &self.page,
                    &target,
                    include_str!("fill.js"),
                    vec![json!(text), json!(native)],
                )
                .await?;
                if mode == FillMode::Invalid {
                    operation.input_not_delivered();
                    return Err(BrowserError::Configuration(
                        "browser rejected the native input value".into(),
                    ));
                }
                if native {
                    operation.complete_input();
                }
                Ok(mode)
            })
            .await?;
        if mode == FillMode::Text {
            if text.is_empty() {
                self.press(&keyboard::combination("Delete")?, operation)
                    .await?;
            } else {
                operation
                    .run(async {
                        operation.begin_input();
                        self.page.execute(InsertTextParams::new(text)).await?;
                        operation.complete_input();
                        Ok(())
                    })
                    .await?;
            }
        }
        Ok(())
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
        conditions: &[&str],
        operation: &Operation<'_>,
    ) -> Result<(element::TargetElement, element::ElementState)> {
        self.ready_element_with_failure(target, references, conditions, operation, &mut None)
            .await
    }

    /// Nested option waits retain their known failure while refreshing the browser target.
    pub(super) async fn ready_element_with_failure(
        &self,
        target: &BrowserTarget,
        references: &mut References,
        conditions: &[&str],
        operation: &Operation<'_>,
        last_failure: &mut Option<BrowserError>,
    ) -> Result<(element::TargetElement, element::ElementState)> {
        loop {
            let attempt = async {
                let observation = operation
                    .run(Observation::capture(&self.page, references))
                    .await?;
                let matches = operation
                    .run(observation.resolve(&self.page, target, references))
                    .await?;
                let selected = operation.run(element::single(&self.page, matches)).await?;
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
                    let state = element::prepared_state(
                        &self.page,
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

    /// A dialog can block acknowledgement of native input; retain cleanup in the tab.
    pub(super) async fn input<T>(
        &self,
        operation: &Operation<'_>,
        input: impl std::future::Future<Output = Result<T>>,
    ) -> Result<T> {
        let mut dialog = self.state.dialog.subscribe();
        if dialog.borrow().is_some() {
            return Err(BrowserError::DialogBlocked);
        }
        operation
            .run(async {
                tokio::select! {
                    result = input => result,
                    opened = dialog.wait_for(|dialog| dialog.is_some()) => {
                        opened.map_err(|_| BrowserError::Closed)?;
                        Err(BrowserError::DialogBlocked)
                    },
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
        let click = async {
            operation.begin_input();
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
        let result = self.input(operation, click).await;
        if result.is_ok() {
            operation.complete_input();
        }
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
            self.state
                .deferred_release
                .lock()
                .await
                .push(InputRelease::Mouse(release));
            return Err(BrowserError::DialogBlocked);
        }
        let cleanup = tokio::time::timeout(CONTROL_TIMEOUT, self.page.execute(release))
            .await
            .map_err(|_| BrowserError::Timeout)
            .and_then(|result| result.map(|_| ()).map_err(BrowserError::from));
        after_cleanup(result, cleanup)
    }
}

#[derive(serde::Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum FillMode {
    Invalid,
    Native,
    Text,
}
