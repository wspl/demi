//! Ordered browser pointer delivery with cancellation-safe button release.
use super::{
    BrowserError, BrowserTab, Result, keyboard,
    operation::{CONTROL_TIMEOUT, Operation, after_cleanup},
    protocol::DragInput,
    tab::InputRelease,
};
use chromiumoxide::cdp::browser_protocol::input::{
    DispatchMouseEventParams, DispatchMouseEventType, MouseButton,
};
use serde_json::{Value, json};

impl BrowserTab {
    pub(super) async fn drag(&self, input: &DragInput, operation: &Operation<'_>) -> Result<Value> {
        let mut points = Vec::new();
        for point in &input.point {
            points.push(operation.run(self.coordinates(point)).await?);
        }
        let modifiers = keyboard::modifiers(input.modifier.as_deref());
        let first = points[0];
        let mut release_point = first;
        let result = self
            .input(operation, async {
                operation.begin_input();
                for (index, point) in points.iter().enumerate() {
                    release_point = *point;
                    self.page
                        .execute(
                            DispatchMouseEventParams::builder()
                                .r#type(DispatchMouseEventType::MouseMoved)
                                .x(point.x)
                                .y(point.y)
                                .button(if index == 0 {
                                    MouseButton::None
                                } else {
                                    MouseButton::Left
                                })
                                .buttons(if index == 0 { 0 } else { 1 })
                                .modifiers(modifiers)
                                .build()
                                .map_err(BrowserError::Configuration)?,
                        )
                        .await?;
                    if index == 0 {
                        self.page
                            .execute(
                                DispatchMouseEventParams::builder()
                                    .r#type(DispatchMouseEventType::MousePressed)
                                    .x(first.x)
                                    .y(first.y)
                                    .button(MouseButton::Left)
                                    .buttons(1)
                                    .click_count(1)
                                    .modifiers(modifiers)
                                    .build()
                                    .map_err(BrowserError::Configuration)?,
                            )
                            .await?;
                    }
                }
                Ok(())
            })
            .await;
        let release = DispatchMouseEventParams::builder()
            .r#type(DispatchMouseEventType::MouseReleased)
            .x(release_point.x)
            .y(release_point.y)
            .button(MouseButton::Left)
            .buttons(0)
            .modifiers(if result.is_ok() { modifiers } else { 0 })
            .click_count(1)
            .build()
            .map_err(BrowserError::Configuration)?;
        self.release_mouse(result, release).await?;
        operation.complete_input();
        Ok(json!({"operation": "drag", "result": true}))
    }

    /// The retained browser tab owns any mouse release blocked by a dialog.
    pub(super) async fn release_mouse(
        &self,
        result: Result<()>,
        release: DispatchMouseEventParams,
    ) -> Result<()> {
        if self.ended.is_cancelled() {
            return result;
        }
        if self.state.dialog.borrow().is_some() {
            self.state
                .deferred_release
                .lock()
                .await
                .push(InputRelease::Mouse(release));
            return after_cleanup(result, Err(BrowserError::DialogBlocked));
        }
        let cleanup = tokio::time::timeout(CONTROL_TIMEOUT, self.page.execute(release))
            .await
            .map_err(|_| BrowserError::Timeout)
            .and_then(|result| result.map(|_| ()).map_err(BrowserError::from));
        after_cleanup(result, cleanup)
    }
}
