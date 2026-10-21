//! Ordered browser pointer delivery with cancellation-safe button release.
use chromiumoxide::cdp::browser_protocol::input::{
    DispatchMouseEventParams, DispatchMouseEventType, MouseButton,
};
use chromiumoxide::layout::Point;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::driver::operation::{BrowserError, CONTROL_TIMEOUT, Operation, Result, after_cleanup};
use crate::tabs::{
    dialog::InputRelease, navigation::NavigationObservation, session::References,
    tab::BrowserTab,
};

use crate::page::{
    element, keyboard,
    protocol::{ActionResult, BrowserTarget, DragInput},
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
                .run(crate::page::actions::coordinates(tab, point))
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

/// Clicks `point` as Playwright does: the move, then each press and release
/// with its click count, go out in order without waiting for Chrome's answer
/// to the one before, and the click is done once Chrome answered them all,
/// the last after the page handled its event. A press that went out without
/// its release is released on every failure, cancellation included.
pub(crate) async fn click_at(
    tab: &BrowserTab,
    point: Point,
    button: MouseButton,
    count: i64,
    modifiers: i64,
    operation: &Operation<'_>,
) -> Result<()> {
    let held = match button {
        MouseButton::Left => 1,
        MouseButton::Right => 2,
        MouseButton::Middle => 4,
        MouseButton::Back => 8,
        MouseButton::Forward => 16,
        MouseButton::None => 0,
    };
    let event = |kind, button, buttons, clicks| {
        DispatchMouseEventParams::builder()
            .r#type(kind)
            .x(point.x)
            .y(point.y)
            .button(button)
            .buttons(buttons)
            .click_count(clicks)
            .modifiers(modifiers)
            .build()
            .map_err(BrowserError::Configuration)
    };
    let mut events = vec![event(
        DispatchMouseEventType::MouseMoved,
        MouseButton::None,
        0,
        0,
    )?];
    for clicks in 1..=count {
        events.push(event(
            DispatchMouseEventType::MousePressed,
            button.clone(),
            held,
            clicks,
        )?);
        events.push(event(
            DispatchMouseEventType::MouseReleased,
            button.clone(),
            0,
            clicks,
        )?);
    }
    let sent = AtomicUsize::new(0);
    let result = tab
        .input(operation, async {
            operation.begin_input();
            dispatch_in_order(tab.page(), &events, &sent).await
        })
        .await;
    if result.is_ok() {
        operation.complete_input();
        return result;
    }
    // The move is event 0; presses are the odd events and each release
    // follows its press, so the button is held when the last one sent is a
    // press.
    let sent = sent.load(Ordering::Acquire);
    if tab.ended().is_cancelled() || sent == 0 || sent % 2 == 1 {
        return result;
    }
    release_mouse(tab, result, events[sent].clone()).await
}

/// Sends `events` to Chrome in order without waiting for each answer, then
/// waits for every answer; an answer that fails fails the whole. `sent`
/// counts the events handed to the connection, which reach Chrome whatever
/// becomes of this future.
async fn dispatch_in_order(
    page: &chromiumoxide::Page,
    events: &[DispatchMouseEventParams],
    sent: &AtomicUsize,
) -> Result<()> {
    let mut answers = Vec::with_capacity(events.len());
    for event in events {
        let mut answer = Box::pin(page.command_future(event.clone())?);
        std::future::poll_fn(|context| answer.as_mut().poll_send(context)).await?;
        sent.fetch_add(1, Ordering::Release);
        answers.push(answer);
    }
    for answer in answers {
        answer.await?;
    }
    Ok(())
}

/// Clicks the element `target` names once it is actionable, as Playwright
/// does: the input goes out behind a hit-target interceptor in the element's
/// frame, and a click whose first event landed elsewhere, after a layout
/// shift between the checks and the input, reached none of the page's
/// handlers, so the action checks again and clicks again. `decide` says from
/// the element's state whether to click at all; the answer names the element
/// and says whether it was clicked. `navigation`, when the click waits for a
/// navigation it starts, opens its window just before the input.
pub(crate) async fn click_target(
    tab: &BrowserTab,
    target: &BrowserTarget,
    references: &mut References,
    button: MouseButton,
    count: i64,
    modifiers: i64,
    operation: &Operation<'_>,
    decide: impl Fn(&element::ElementState) -> Result<bool>,
    mut navigation: Option<&mut NavigationObservation>,
) -> Result<(element::Ready, bool)> {
    let mut last_failure = None;
    let mut pauses = element::CheckPauses::action();
    loop {
        let ready = element::ready_with_failure(
            tab,
            target,
            references,
            element::CLICK,
            operation,
            &mut last_failure,
        )
        .await?;
        if !decide(&ready.state)? {
            return Ok((ready, false));
        }
        let clicked = click_intercepted(
            tab,
            &ready,
            button.clone(),
            count,
            modifiers,
            operation,
            navigation.as_deref_mut(),
        )
        .await;
        match clicked {
            Err(failure @ BrowserError::NotActionable { .. }) => {
                operation.input_not_delivered();
                last_failure = Some(failure);
                let paused = operation
                    .run(async {
                        pauses.wait_next().await;
                        Ok(())
                    })
                    .await;
                if let Err(error) = paused {
                    return Err(match last_failure.take() {
                        Some(cause) if error.is_deadline() => error.with_deadline_cause(cause),
                        _ => error,
                    });
                }
            }
            result => return result.map(|()| (ready, true)),
        }
    }
}

/// Clicks `ready`'s element at its checked point with Playwright's hit-target
/// interceptor installed: `not_actionable` with what the click landed on when
/// its first event missed the element, and so no event reached the page.
async fn click_intercepted(
    tab: &BrowserTab,
    ready: &element::Ready,
    button: MouseButton,
    count: i64,
    modifiers: i64,
    operation: &Operation<'_>,
    navigation: Option<&mut NavigationObservation>,
) -> Result<()> {
    let element = &ready.element;
    let key = crate::driver::handles::fresh("intercept")?;
    let installed = operation
        .run(element::call::<element::ElementState>(
            &element.page,
            element,
            &element::ELEMENT_STATE,
            vec![
                json!(["hit"]),
                json!(false),
                json!(key),
                json!(false),
                Value::Null,
                json!(true),
            ],
        ))
        .await;
    let clicked = match installed {
        Ok(state) if state.failed.is_none() => match navigation.map(|navigation| navigation.input_begins()) {
            Some(Err(error)) => Err(error),
            _ => click_at(tab, ready.state.point(), button, count, modifiers, operation).await,
        },
        Ok(state) => Err(state.failure()),
        Err(error) => Err(error),
    };
    // An ended tab took the interceptor along.
    if tab.ended().is_cancelled() {
        return clicked;
    }
    // Stopped on every other path, cancellation and a failed install
    // included, whose request may have installed it all the same.
    let stop = element::function_call(
        element,
        &element::ELEMENT_STATE,
        vec![json!([]), json!(false), json!(key), json!(true)],
    )?;
    // A dialog, the click's or the page's own, stops the page's scripts:
    // the dialog's owner stops the interceptor once the dialog closes.
    if tab.state().dialog.is_open() {
        tab.state()
            .dialog
            .defer(vec![InputRelease::Script(element.page.clone(), stop)])
            .await;
        return clicked;
    }
    let stopped = tokio::time::timeout(
        CONTROL_TIMEOUT,
        element::evaluated::<Option<String>>(&element.page, stop),
    )
    .await
    .map_err(|_| BrowserError::Timeout)
    .and_then(std::convert::identity);
    match stopped {
        Ok(Some(interceptor)) if clicked.is_ok() => Err(BrowserError::NotActionable {
            condition: "hit".into(),
            interceptor: Some(interceptor),
        }),
        Ok(_) => clicked,
        // A navigation the click started replaced the document, and the
        // interceptor with it.
        Err(
            BrowserError::Cdp(_)
            | BrowserError::Closed
            | BrowserError::Connection(_)
            | BrowserError::TabNotFound,
        ) => clicked,
        Err(error) => after_cleanup(clicked, Err(error)),
    }
}
