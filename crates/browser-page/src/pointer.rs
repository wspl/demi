//! Ordered browser pointer delivery with cancellation-safe button release.
use chromiumoxide::cdp::browser_protocol::input::{
    DispatchMouseEventParams, DispatchMouseEventType, MouseButton,
};
use chromiumoxide::layout::Point;
use serde_json::json;

use demi_browser_driver::operation::{
    BrowserError, CONTROL_TIMEOUT, Operation, Result, after_cleanup,
};
use demi_browser_tabs::{dialog::InputRelease, tab::BrowserTab};

use crate::{
    keyboard,
    protocol::{ActionResult, DragInput},
};

pub(crate) async fn drag(
    tab: &BrowserTab,
    input: &DragInput,
    operation: &Operation<'_>,
) -> Result<ActionResult> {
    let mut points = Vec::new();
    for point in &input.point {
        points.push(
            operation
                .run(crate::actions::coordinates(tab, point))
                .await?,
        );
    }
    let modifiers = keyboard::modifiers(input.modifier.as_deref());
    let first = points[0];
    let mut release_point = first;
    let result = tab
        .input(operation, async {
            operation.begin_input();
            for (index, point) in points.iter().enumerate() {
                release_point = *point;
                tab.page()
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
                    tab.page()
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
    release_mouse(tab, result, release).await?;
    operation.complete_input();
    Ok(ActionResult::new("drag", json!(true)))
}

/// The retained browser tab owns any mouse release blocked by a dialog.
pub(crate) async fn release_mouse(
    tab: &BrowserTab,
    result: Result<()>,
    release: DispatchMouseEventParams,
) -> Result<()> {
    if tab.ended().is_cancelled() {
        return result;
    }
    if tab.state().dialog.is_open() {
        tab.state()
            .dialog
            .defer(vec![InputRelease::Mouse(release)])
            .await;
        return after_cleanup(result, Err(BrowserError::DialogBlocked));
    }
    let cleanup = tokio::time::timeout(CONTROL_TIMEOUT, tab.page().execute(release))
        .await
        .map_err(|_| BrowserError::Timeout)
        .and_then(|result| result.map(|_| ()).map_err(BrowserError::from));
    after_cleanup(result, cleanup)
}

/// Complete or release input at the point approved by the browser action checks.
pub(crate) async fn click_at(
    tab: &BrowserTab,
    point: Point,
    button: MouseButton,
    count: i64,
    modifiers: i64,
    operation: &Operation<'_>,
) -> Result<()> {
    let click = async {
        operation.begin_input();
        if button == MouseButton::Left && modifiers == 0 {
            tab.page()
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
            tab.page().move_mouse(point).await?;
            tab.page()
                .execute(
                    event
                        .clone()
                        .r#type(DispatchMouseEventType::MousePressed)
                        .build()
                        .map_err(BrowserError::Configuration)?,
                )
                .await?;
            tab.page()
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
    let result = tab.input(operation, click).await;
    if result.is_ok() {
        operation.complete_input();
    }
    if result.is_ok() || tab.ended().is_cancelled() {
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
    release_mouse(tab, result, release).await
}
