//! One tab admission covers observation, targeting, input and associated waits.

use std::time::Duration;

use chromiumoxide::{
    cdp::browser_protocol::{
        input::{DispatchMouseEventParams, DispatchMouseEventType, MouseButton},
        page::{GetLayoutMetricsParams, GetNavigationHistoryParams},
        target::GetTargetInfoParams,
    },
    layout::Point,
};
use serde::Serialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::driver::operation::{BrowserError, Operation, Result};
use crate::tabs::{
    navigation::{Navigation, NavigationObservation, history_step},
    tab::BrowserTab,
};

use crate::page::{
    element, evaluation, keyboard,
    observation::{self, Observation, has_target_flags},
    protocol::{
        ActionResult, BrowserOperation, BrowserTarget, BrowserViewport, Capability,
        ContentReadResult, DEFAULT_NODES, Dialog, DialogInspectResult, DialogOutcome, DialogResult,
        ElementState, EvalResult, FindResult, HistoryEntry, HistoryResult, INLINE_BYTES,
        InfoResult, InspectResult, Load, LogsResult, MAX_NODES, NavigationResult, ProbeResult,
        ReadProperty, ReadResult, ViewportMode, ViewportResult, WaitResult,
    },
};

/// A tab operation's result, printed as the operation's own result type.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum TabResult {
    Info(InfoResult),
    Navigation(NavigationResult),
    History(HistoryResult),
    Inspect(InspectResult),
    Find(FindResult),
    Read(ReadResult),
    Probe(ProbeResult),
    Action(ActionResult),
    Wait(WaitResult),
    Eval(EvalResult),
    Logs(LogsResult),
    Viewport(ViewportResult),
    DialogInspect(DialogInspectResult),
    Dialog(DialogResult),
    ContentRead(ContentReadResult),
}

/// Execute a schema-validated tab command through the same admission used by the service.
pub async fn execute(
    tab: &BrowserTab,
    operation: &str,
    args: Value,
    cancel: &CancellationToken,
) -> Result<Value> {
    let command = BrowserOperation::parse(operation, args)
        .map_err(|error| BrowserError::Configuration(error.to_string()))?;
    if command.tab() != Some(tab.id()) {
        return Err(BrowserError::TabNotFound);
    }
    if matches!(
        command,
        BrowserOperation::Open(_)
            | BrowserOperation::Show(_)
            | BrowserOperation::Tabs(_)
            | BrowserOperation::Stop(_)
            | BrowserOperation::Close(_)
            | BrowserOperation::Screenshot(_)
            | BrowserOperation::Capabilities(_)
    ) {
        return Err(BrowserError::Configuration(
            "this operation requires the browser conversation controller".into(),
        ));
    }
    let result = self::command(
        tab,
        &command,
        cancel,
        tokio::time::Instant::now() + command.timeout(),
    )
    .await?;
    crate::driver::output::value(result)
}

pub async fn metadata(
    tab: &BrowserTab,
    cancel: &CancellationToken,
    timeout: std::time::Duration,
) -> Result<InfoResult> {
    tab.operation(cancel, tokio::time::Instant::now() + timeout)
        .run(async {
            let (url, title) = target_info(tab).await?;
            let dialog = tab.state().dialog.open().map(|dialog| Dialog {
                r#type: crate::tabs::tab::dialog_type(&dialog.r#type),
                message: dialog.message.clone(),
            });
            Ok(InfoResult {
                tab: tab.id().clone(),
                url,
                title,
                viewport: tab.viewport(),
                dialog,
            })
        })
        .await
}

pub async fn command(
    tab: &BrowserTab,
    command: &BrowserOperation,
    cancel: &CancellationToken,
    deadline: tokio::time::Instant,
) -> Result<TabResult> {
    let operation = tab.operation(cancel, deadline);
    let mut session = tab
        .state()
        .gate
        .try_checkout()
        .ok_or_else(|| operation.failure(BrowserError::Busy, tab.id().as_str(), None))?;
    command_admitted(tab, command, cancel, deadline, &mut session.references).await
}

/// Keep one browser-tab admission through a command and its attached artifact capture.
pub async fn command_admitted(
    tab: &BrowserTab,
    command: &BrowserOperation,
    cancel: &CancellationToken,
    deadline: tokio::time::Instant,
    references: &mut crate::tabs::session::References,
) -> Result<TabResult> {
    let operation = tab.operation(cancel, deadline);
    let target = command.target();
    if !matches!(
        command,
        BrowserOperation::DialogInspect(_)
            | BrowserOperation::DialogAccept(_)
            | BrowserOperation::DialogDismiss(_)
    ) && tab.state().dialog.is_open()
    {
        return Err(operation.failure(BrowserError::DialogBlocked, tab.id().as_str(), None));
    }
    let mut navigation =
        if command.wait_url().is_some() || matches!(command, BrowserOperation::Click(_)) {
            Some(
                operation
                    .run(NavigationObservation::subscribe(tab.page()))
                    .await
                    .map_err(|error| operation.failure(error, tab.id().as_str(), None))?,
            )
        } else {
            None
        };
    let current_url = operation
        .run(async { Ok(tab.page().url().await?) })
        .await
        .map_err(|error| operation.failure(error, tab.id().as_str(), None))?;
    // The pages the page opened before an action, so that its result
    // names only those the action opened (`browser.md` § One tab registry).
    let opened_before = if opens_tabs(command) {
        operation.run(tab.opened()).await.ok()
    } else {
        None
    };
    let work = async {
        let xy = match command {
            BrowserOperation::Click(input) => input.xy.as_ref(),
            BrowserOperation::Move(input) => input.xy.as_ref(),
            BrowserOperation::Scroll(input) => input.xy.as_ref(),
            _ => None,
        };
        if xy.is_some() && target.as_ref().is_some_and(has_target_flags) {
            return Err(BrowserError::Configuration(
                "coordinates cannot be combined with an element target".into(),
            ));
        }
        if let BrowserOperation::Wait(input) = command
            && (usize::from(input.url.is_some())
                + usize::from(input.load.is_some())
                + usize::from(
                    input.state.is_some() || target.as_ref().is_some_and(has_target_flags),
                )
                != 1)
        {
            return Err(BrowserError::Configuration(
                "wait requires exactly one URL, current-document load, or element condition".into(),
            ));
        }

        // Each branch has its own heap allocation; large command futures must
        // not be embedded together on native-service or test-thread stacks.
        let branch: futures_util::future::BoxFuture<'_, Result<TabResult>> = match command {
            BrowserOperation::Probe(input) => Box::pin(async {
                Ok(TabResult::Probe(
                    crate::page::probe::probe(tab, input, references, &operation).await?,
                ))
            }),
            BrowserOperation::Logs(input) => {
                Box::pin(async { Ok(TabResult::Logs(tab.state().console.read(input).await?)) })
            }
            BrowserOperation::Drag(input) => Box::pin(async {
                Ok(TabResult::Action(
                    crate::page::pointer::drag(tab, input, &operation).await?,
                ))
            }),
            BrowserOperation::SelectText(input) => Box::pin(async {
                let result = crate::page::selection::select_text(
                    tab,
                    required_target(&target)?,
                    references,
                    input,
                    &operation,
                )
                .await?;
                Ok(TabResult::Action(result))
            }),
            BrowserOperation::Info(_) => Box::pin(async {
                Ok(TabResult::Info(
                    metadata(tab, cancel, command.timeout()).await?,
                ))
            }),
            BrowserOperation::Goto(input) => Box::pin(async {
                let url = tab
                    .navigate(
                        Navigation::Url(input.url.clone()),
                        input.load.unwrap_or_default(),
                        &operation,
                        references,
                    )
                    .await?;
                Ok(TabResult::Navigation(completed_navigation(tab, url).await))
            }),
            BrowserOperation::Reload(input) => Box::pin(async {
                let url = tab
                    .navigate(
                        Navigation::Reload,
                        input.load.unwrap_or_default(),
                        &operation,
                        references,
                    )
                    .await?;
                Ok(TabResult::Navigation(completed_navigation(tab, url).await))
            }),
            BrowserOperation::Back(_) | BrowserOperation::Forward(_) => Box::pin(async {
                let back = matches!(command, BrowserOperation::Back(_));
                let entry = history_step(tab, back, &operation).await?;
                let load = match command {
                    BrowserOperation::Back(input) => input.load,
                    BrowserOperation::Forward(input) => input.load,
                    _ => unreachable!(),
                }
                .unwrap_or_default();
                let url = tab
                    .navigate(Navigation::History(entry.id), load, &operation, references)
                    .await?;
                Ok(TabResult::Navigation(completed_navigation(tab, url).await))
            }),
            BrowserOperation::History(input) => Box::pin(async {
                let history = operation
                    .run(async {
                        Ok(tab
                            .page()
                            .execute(GetNavigationHistoryParams {})
                            .await?
                            .result)
                    })
                    .await?;
                let offset = input.offset.unwrap_or(0);
                let limit = input.limit.unwrap_or(DEFAULT_NODES);
                let entries = history
                    .entries
                    .iter()
                    .enumerate()
                    .skip(offset)
                    .take(limit)
                    .map(|(index, entry)| HistoryEntry {
                        index,
                        url: entry.url.clone(),
                        title: entry.title.clone(),
                        current: index as i64 == history.current_index,
                    })
                    .collect();
                Ok(TabResult::History(HistoryResult {
                    entries,
                    truncated: history.entries.len() > offset.saturating_add(limit),
                }))
            }),
            BrowserOperation::Inspect(input) => Box::pin(async {
                let mut observation = Observation::capture(tab.page(), references).await?;
                observation.restrict(input.within.as_ref(), input.frame.as_deref(), references)?;
                let view = input.view.unwrap_or_default();
                let limit = input.limit.unwrap_or(DEFAULT_NODES);
                let (tree, truncated) = match view {
                    crate::page::protocol::InspectView::Dom => {
                        observation.dom_tree(references, limit)?
                    }
                    crate::page::protocol::InspectView::Accessibility => {
                        let (nodes, truncated) = observation.tree(references, limit)?;
                        (observation::hierarchy(nodes), truncated)
                    }
                };
                let (url, title) = target_info(tab).await?;
                Ok(TabResult::Inspect(InspectResult {
                    tab: tab.id().clone(),
                    url,
                    title,
                    view,
                    tree,
                    truncated,
                }))
            }),
            BrowserOperation::Find(input) => Box::pin(async {
                let observation = Observation::capture(tab.page(), references).await?;
                let elements = if input.query == Some(true) {
                    if target.as_ref().is_some_and(has_target_flags) {
                        return Err(BrowserError::Configuration(
                            "find --query cannot combine ordinary target flags".into(),
                        ));
                    }
                    let query =
                        crate::page::query::parse(input.body.as_deref().ok_or_else(|| {
                            BrowserError::Configuration("find --query requires stdin".into())
                        })?)?;
                    observation.query(tab.page(), &query, references).await?
                } else {
                    if input.body.is_some() {
                        return Err(BrowserError::Configuration(
                            "find stdin requires --query".into(),
                        ));
                    }
                    observation
                        .resolve(tab.page(), required_target(&target)?, references)
                        .await?
                };
                let limit = input.limit.unwrap_or(DEFAULT_NODES);
                let offset = input.offset.unwrap_or(0).min(elements.len());
                let matches =
                    observation.describe_elements(&elements[offset..], references, limit)?;
                Ok(TabResult::Find(FindResult {
                    matches,
                    count: elements.len(),
                    truncated: elements.len() > offset.saturating_add(limit),
                }))
            }),
            BrowserOperation::Read(input) => Box::pin(async {
                let observation = Observation::capture(tab.page(), references).await?;
                let elements = observation
                    .resolve(tab.page(), required_target(&target)?, references)
                    .await?;
                if input.all != Some(true) && elements.is_empty() {
                    return Err(BrowserError::TargetNotFound);
                }
                if input.all != Some(true) && elements.len() != 1 {
                    return Err(BrowserError::Ambiguous(elements.len()));
                }
                if input.property.is_some() == input.attribute.is_some() {
                    return Err(BrowserError::Configuration(
                        "read requires exactly one --property or --attribute".into(),
                    ));
                }
                let mut values = Vec::new();
                for element in elements.iter().take(MAX_NODES) {
                    let value = match input.property {
                        Some(
                            property @ (ReadProperty::Text
                            | ReadProperty::Html
                            | ReadProperty::TextContent
                            | ReadProperty::Value),
                        ) => {
                            if property == ReadProperty::Value && observation.protected(element) {
                                return Err(BrowserError::ProtectedValue);
                            }
                            let dom_property = match property {
                                ReadProperty::Text => "innerText",
                                ReadProperty::Html => "outerHTML",
                                ReadProperty::TextContent => "textContent",
                                _ => "value",
                            };
                            let fallback =
                                if matches!(property, ReadProperty::Text | ReadProperty::Html) {
                                    json!("")
                                } else {
                                    Value::Null
                                };
                            element::call::<Value>(
                                tab.page(),
                                element,
                                "function(property, fallback) { return this[property] ?? fallback; }",
                                vec![json!(dom_property), fallback],
                            )
                            .await?
                        }
                        Some(
                            property @ (ReadProperty::Visible
                            | ReadProperty::Enabled
                            | ReadProperty::Checked),
                        ) => {
                            let state = element::state(tab.page(), element, &[], false).await?;
                            match property {
                                ReadProperty::Visible => json!(state.visible),
                                ReadProperty::Enabled => json!(state.enabled),
                                _ => json!(state.checked),
                            }
                        }
                        None => {
                            let attribute = input.attribute.as_ref().ok_or_else(|| {
                                BrowserError::Configuration(
                                    "attribute read requires --attribute".into(),
                                )
                            })?;
                            element::call::<Value>(
                                tab.page(),
                                element,
                                "function(name) { return this.getAttribute(name); }",
                                vec![json!(attribute)],
                            )
                            .await?
                        }
                    };
                    values.push(value);
                }
                let result = if input.all == Some(true) {
                    ReadResult::All {
                        values,
                        truncated: elements.len() > MAX_NODES,
                    }
                } else {
                    ReadResult::One {
                        value: values.remove(0),
                    }
                };
                Ok(TabResult::Read(result))
            }),
            BrowserOperation::Eval(input) => Box::pin(async {
                let value = if target.as_ref().is_some_and(has_target_flags) {
                    let observation = Observation::capture(tab.page(), references).await?;
                    let elements = observation
                        .resolve(tab.page(), required_target(&target)?, references)
                        .await?;
                    if elements.len() > 1 && input.all != Some(true) {
                        return Err(BrowserError::Ambiguous(elements.len()));
                    }
                    evaluation::targeted(
                        tab.page(),
                        &input.expression,
                        &elements,
                        input.all == Some(true),
                    )
                    .await?
                } else {
                    if input.all == Some(true) {
                        return Err(BrowserError::Configuration(
                            "eval --all requires an element locator".into(),
                        ));
                    }
                    evaluation::read_only(tab.page(), &input.expression).await?
                };
                Ok(TabResult::Eval(EvalResult { value }))
            }),
            BrowserOperation::Fill(input) => Box::pin(async {
                let named = crate::page::keyboard::fill(
                    tab,
                    required_target(&target)?,
                    references,
                    &input.text,
                    &operation,
                )
                .await?;
                let mut result = action_result(
                    tab,
                    &operation,
                    "fill",
                    input.wait_url.as_deref(),
                    navigation.as_mut(),
                )
                .await?;
                result.target = Some(named);
                Ok(TabResult::Action(result))
            }),
            BrowserOperation::Click(input) => Box::pin(async {
                let button = match input.button.unwrap_or_default() {
                    crate::page::protocol::MouseButton::Left => MouseButton::Left,
                    crate::page::protocol::MouseButton::Middle => MouseButton::Middle,
                    crate::page::protocol::MouseButton::Right => MouseButton::Right,
                };
                let modifiers = keyboard::modifiers(input.modifier.as_deref());
                let (point, named) = match &input.xy {
                    Some(xy) => (operation.run(coordinates(tab, xy)).await?, None),
                    None => {
                        let ready = crate::page::element::ready(
                            tab,
                            required_target(&target)?,
                            references,
                            element::CLICK,
                            &operation,
                        )
                        .await?;
                        (ready.state.point(), Some(ready.named))
                    }
                };
                crate::page::pointer::click_at(
                    tab,
                    point,
                    button,
                    i64::from(input.count.unwrap_or(1)),
                    modifiers,
                    &operation,
                )
                .await?;
                let mut result = action_result(
                    tab,
                    &operation,
                    "click",
                    input.wait_url.as_deref(),
                    navigation.as_mut(),
                )
                .await?;
                result.target = named;
                Ok(TabResult::Action(result))
            }),
            BrowserOperation::Move(input) => Box::pin(async {
                let (point, named) = match &input.xy {
                    Some(xy) => (operation.run(coordinates(tab, xy)).await?, None),
                    None => {
                        let ready = crate::page::element::ready(
                            tab,
                            required_target(&target)?,
                            references,
                            element::GEOMETRY,
                            &operation,
                        )
                        .await?;
                        (ready.state.point(), Some(ready.named))
                    }
                };
                operation
                    .run(async {
                        operation.begin_input();
                        tab.page()
                            .execute(
                                DispatchMouseEventParams::builder()
                                    .r#type(DispatchMouseEventType::MouseMoved)
                                    .x(point.x)
                                    .y(point.y)
                                    .modifiers(keyboard::modifiers(input.modifier.as_deref()))
                                    .build()
                                    .map_err(BrowserError::Configuration)?,
                            )
                            .await?;
                        operation.complete_input();
                        Ok(())
                    })
                    .await?;
                let mut result = action_result(tab, &operation, "move", None, None).await?;
                result.target = named;
                Ok(TabResult::Action(result))
            }),
            BrowserOperation::Scroll(input) => Box::pin(async {
                if input.dx.unwrap_or(0.0) == 0.0 && input.dy.unwrap_or(0.0) == 0.0 {
                    return Err(BrowserError::Configuration(
                        "scroll requires a nonzero dx or dy".into(),
                    ));
                }
                let (point, named) = match &input.xy {
                    Some(xy) => (operation.run(coordinates(tab, xy)).await?, None),
                    None if !target.as_ref().is_some_and(has_target_flags) => {
                        let viewport = operation
                            .run(async {
                                Ok(tab
                                    .page()
                                    .execute(GetLayoutMetricsParams {})
                                    .await?
                                    .result
                                    .css_layout_viewport)
                            })
                            .await?;
                        let center = Point {
                            x: viewport.client_width as f64 / 2.0,
                            y: viewport.client_height as f64 / 2.0,
                        };
                        (center, None)
                    }
                    None => {
                        let ready = crate::page::element::ready(
                            tab,
                            required_target(&target)?,
                            references,
                            element::GEOMETRY,
                            &operation,
                        )
                        .await?;
                        (ready.state.point(), Some(ready.named))
                    }
                };
                operation
                    .run(async {
                        operation.begin_input();
                        tab.page()
                            .execute(
                                DispatchMouseEventParams::builder()
                                    .r#type(DispatchMouseEventType::MouseWheel)
                                    .x(point.x)
                                    .y(point.y)
                                    .modifiers(keyboard::modifiers(input.modifier.as_deref()))
                                    .delta_x(input.dx.unwrap_or(0.0))
                                    .delta_y(input.dy.unwrap_or(0.0))
                                    .build()
                                    .map_err(BrowserError::Configuration)?,
                            )
                            .await?;
                        operation.complete_input();
                        Ok(())
                    })
                    .await?;
                let mut result = action_result(tab, &operation, "scroll", None, None).await?;
                result.target = named;
                Ok(TabResult::Action(result))
            }),
            BrowserOperation::Type(input) => Box::pin(async {
                let named = if target.as_ref().is_some_and(has_target_flags) {
                    let ready = crate::page::element::ready(
                        tab,
                        required_target(&target)?,
                        references,
                        element::TYPE,
                        &operation,
                    )
                    .await?;
                    crate::page::keyboard::type_text(tab, &ready.element, &input.text, &operation)
                        .await?;
                    Some(ready.named)
                } else {
                    let (_, focused) =
                        crate::page::keyboard::current_focus(tab, references, &operation).await?;
                    crate::page::keyboard::type_focused(tab, None, &input.text, &operation).await?;
                    focused
                };
                let mut result = action_result(tab, &operation, "type", None, None).await?;
                result.target = named;
                Ok(TabResult::Action(result))
            }),
            BrowserOperation::Key(input) => Box::pin(async {
                let keys = keyboard::combination(&input.key)?;
                let named = if target.as_ref().is_some_and(has_target_flags) {
                    let ready = crate::page::element::ready(
                        tab,
                        required_target(&target)?,
                        references,
                        element::KEY,
                        &operation,
                    )
                    .await?;
                    crate::page::keyboard::focus(tab, &ready.element, &operation).await?;
                    Some(ready.named)
                } else {
                    crate::page::keyboard::current_focus(tab, references, &operation)
                        .await?
                        .1
                };
                crate::page::keyboard::press(tab, &keys, &operation).await?;
                let mut result = action_result(
                    tab,
                    &operation,
                    "key",
                    input.wait_url.as_deref(),
                    navigation.as_mut(),
                )
                .await?;
                result.target = named;
                Ok(TabResult::Action(result))
            }),
            BrowserOperation::Check(input) => Box::pin(async {
                let checkable = crate::page::element::ready(
                    tab,
                    required_target(&target)?,
                    references,
                    &["checkable"],
                    &operation,
                )
                .await?;
                if checkable.state.needs_check(input.checked())? {
                    let element::Ready { element, state, .. } = crate::page::element::ready(
                        tab,
                        required_target(&target)?,
                        references,
                        element::CLICK,
                        &operation,
                    )
                    .await?;
                    if state.needs_check(input.checked())? {
                        crate::page::pointer::click_at(
                            tab,
                            state.point(),
                            MouseButton::Left,
                            1,
                            0,
                            &operation,
                        )
                        .await?;
                    }
                    let state = operation
                        .run(element::state(tab.page(), &element, &[], false))
                        .await?;
                    if state.checked != Some(input.checked()) {
                        return Err(BrowserError::NotActionable {
                            condition: "click did not change checked state".into(),
                            interceptor: None,
                        });
                    }
                }
                let mut result = action_result(tab, &operation, "check", None, None).await?;
                result.target = Some(checkable.named);
                Ok(TabResult::Action(result))
            }),
            BrowserOperation::Select(input) => Box::pin(async {
                let (named, options) = crate::page::select::select_options(
                    tab,
                    required_target(&target)?,
                    references,
                    [
                        input.value.as_ref().map(|values| json!(values)),
                        input.option_label.as_ref().map(|labels| json!(labels)),
                        input.option_index.as_ref().map(|indices| json!(indices)),
                    ],
                    &operation,
                )
                .await?;
                let mut result = ActionResult::new("select", json!(options));
                result.target = Some(named);
                Ok(TabResult::Action(result))
            }),
            BrowserOperation::DialogInspect(_) => Box::pin(async {
                let dialog = tab.state().dialog.open().map(|dialog| Dialog {
                    r#type: crate::tabs::tab::dialog_type(&dialog.r#type),
                    message: dialog.message.clone(),
                });
                Ok(TabResult::DialogInspect(DialogInspectResult { dialog }))
            }),
            BrowserOperation::DialogAccept(_) | BrowserOperation::DialogDismiss(_) => {
                Box::pin(async {
                    let dialog = tab
                        .state()
                        .dialog
                        .open()
                        .ok_or(BrowserError::DialogNotFound)?;
                    if let BrowserOperation::DialogAccept(input) = command
                    && (dialog.r#type == chromiumoxide::cdp::browser_protocol::page::DialogType::Alert ||
                        (input.text.is_some() && dialog.r#type != chromiumoxide::cdp::browser_protocol::page::DialogType::Prompt)) {
                        return Err(BrowserError::InvalidDialogAction);
                    }
                    let accept = matches!(command, BrowserOperation::DialogAccept(_));
                    let text = match command {
                        BrowserOperation::DialogAccept(input) => input.text.clone(),
                        _ => None,
                    };
                    tab.state().dialog.answer(&dialog, accept, text).await?;
                    Ok(TabResult::Dialog(DialogResult {
                        r#type: crate::tabs::tab::dialog_type(&dialog.r#type),
                        outcome: if accept {
                            DialogOutcome::Accepted
                        } else {
                            DialogOutcome::Dismissed
                        },
                    }))
                })
            }
            BrowserOperation::Wait(input) => Box::pin(async {
                let condition = match (&input.url, input.load, input.state) {
                    (Some(url), _, _) => url.clone(),
                    (None, Some(load), _) => load.to_string(),
                    (None, None, state) => state.unwrap_or_default().to_string(),
                };
                let mut result = WaitResult {
                    condition,
                    matched: true,
                    url: None,
                    target: None,
                };
                if let Some(url) = &input.url {
                    let mut observation = NavigationObservation::subscribe(tab.page()).await?;
                    observation.wait_url(tab.page(), url).await?;
                    result.url = Some(observation.url);
                } else if let Some(load) = input.load {
                    crate::tabs::navigation::wait_current_load(tab.page(), load).await?;
                } else {
                    loop {
                        let observation = Observation::capture(tab.page(), references).await?;
                        let target = required_target(&target)?;
                        let elements = match observation
                            .resolve_wait(tab.page(), target, references)
                            .await
                        {
                            Ok(elements) => elements,
                            Err(BrowserError::StaleReference)
                                if observation::can_resample(target) =>
                            {
                                // A locator can find a node inserted after the DOM snapshot.
                                // Resample it without treating the race as disappearance.
                                tokio::time::sleep(Duration::from_millis(50)).await;
                                continue;
                            }
                            Err(error) => return Err(error),
                        };
                        let state = input.state.unwrap_or_default();
                        let mut matched = false;
                        for element in &elements {
                            let conditions =
                                element::state(tab.page(), element, &[], false).await?;
                            matched |= match state {
                                ElementState::Enabled => conditions.enabled,
                                ElementState::Visible | ElementState::Hidden => conditions.visible,
                                ElementState::Attached | ElementState::Detached => {
                                    conditions.attached
                                }
                            };
                        }
                        if matches!(state, ElementState::Hidden | ElementState::Detached) {
                            matched = !matched;
                        }
                        if matched {
                            result.target = elements
                                .first()
                                .map(|element| observation.named(element, references))
                                .transpose()?;
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }
                Ok(TabResult::Wait(result))
            }),
            BrowserOperation::ViewportSet(input) => Box::pin(async {
                let viewport = BrowserViewport {
                    mode: ViewportMode::Custom,
                    width: input.width,
                    height: input.height,
                    device_pixel_ratio: input.scale.unwrap_or(1.0),
                };
                tab.set_viewport(viewport).await?;
                Ok(TabResult::Viewport(ViewportResult { viewport }))
            }),
            BrowserOperation::ViewportReset(_) => Box::pin(async {
                let viewport = tab.web_viewport();
                tab.set_viewport(viewport).await?;
                Ok(TabResult::Viewport(ViewportResult { viewport }))
            }),
            BrowserOperation::ContentRead(input) => Box::pin(async {
                let format = input.format.unwrap_or_default();
                let content = crate::page::content::read(tab, format, references).await?;
                let truncated = input.output.is_none() && content.len() > INLINE_BYTES;
                let end = if input.output.is_some() {
                    content.len()
                } else {
                    content.floor_char_boundary(INLINE_BYTES.min(content.len()))
                };
                let (url, title) = target_info(tab).await?;
                Ok(TabResult::ContentRead(ContentReadResult::Inline {
                    url,
                    title,
                    format,
                    content: content[..end].to_owned(),
                    truncated,
                }))
            }),
            BrowserOperation::Open(_)
            | BrowserOperation::Show(_)
            | BrowserOperation::Tabs(_)
            | BrowserOperation::Stop(_)
            | BrowserOperation::Close(_)
            | BrowserOperation::Screenshot(_)
            | BrowserOperation::Capabilities(_)
            | BrowserOperation::Upload(_)
            | BrowserOperation::Download(_)
            | BrowserOperation::ClipboardRead(_)
            | BrowserOperation::ClipboardWrite(_)
            | BrowserOperation::CdpTargets(_)
            | BrowserOperation::CdpDetach(_)
            | BrowserOperation::CdpSend(_)
            | BrowserOperation::CdpEvents(_)
            | BrowserOperation::ContentFetch(_)
            | BrowserOperation::AssetsList(_)
            | BrowserOperation::AssetsExport(_)
            | BrowserOperation::WebmcpList(_)
            | BrowserOperation::WebmcpCall(_)
            | BrowserOperation::Install(_) => Box::pin(async {
                unreachable!("conversation-level operation is dispatched before tab actions")
            }),
        };
        branch.await
    };
    let result = if matches!(
        command,
        BrowserOperation::Drag(_)
            | BrowserOperation::SelectText(_)
            | BrowserOperation::Fill(_)
            | BrowserOperation::Click(_)
            | BrowserOperation::Move(_)
            | BrowserOperation::Scroll(_)
            | BrowserOperation::Type(_)
            | BrowserOperation::Key(_)
            | BrowserOperation::Check(_)
            | BrowserOperation::Select(_)
            | BrowserOperation::Goto(_)
            | BrowserOperation::Reload(_)
            | BrowserOperation::Back(_)
            | BrowserOperation::Forward(_)
    ) {
        // Input branches own their cancellation cleanup and must not be dropped by
        // an enclosing cancellation race while releasing a key or mouse button.
        work.await
    } else {
        operation.run(work).await
    };
    let cleanup = if matches!(
        command,
        BrowserOperation::DialogInspect(_)
            | BrowserOperation::DialogAccept(_)
            | BrowserOperation::DialogDismiss(_)
    ) {
        Ok(())
    } else {
        crate::page::element::release_objects(tab).await
    };
    let result = crate::driver::operation::after_cleanup(result, cleanup);
    let result = match (result, opened_before) {
        (Ok(TabResult::Action(mut action)), Some(before)) => {
            // A tab the action opened is registered, and can be operated,
            // before the result names it. One the registry could not
            // register in time appears in a later `tabs`.
            let registered = operation.run(tab.popups());
            if let Ok(Ok(after)) =
                tokio::time::timeout(crate::driver::operation::CONTROL_TIMEOUT, registered).await
            {
                let opened: Vec<_> = after
                    .into_iter()
                    .filter(|id| !before.contains(id))
                    .collect();
                action.opened_tabs = (!opened.is_empty()).then_some(opened);
            }
            Ok(TabResult::Action(action))
        }
        (result, _) => result,
    };
    if navigation
        .as_ref()
        .is_some_and(NavigationObservation::document_changed)
    {
        references.invalidate();
    }
    result.map_err(|error| operation.failure(error, tab.id().as_str(), current_url.as_deref()))
}

/// The tab's current URL and title.
async fn target_info(tab: &BrowserTab) -> Result<(String, String)> {
    let info = tab
        .page()
        .execute(
            GetTargetInfoParams::builder()
                .target_id(tab.page().target_id().clone())
                .build(),
        )
        .await?
        .result
        .target_info;
    Ok((info.url, info.title))
}

pub(crate) async fn completed_navigation(tab: &BrowserTab, url: String) -> NavigationResult {
    // A metadata failure cannot reverse completed input. A subsequent
    // navigation's title must not be attributed to the document we observed.
    let title =
        match tokio::time::timeout(crate::driver::operation::CONTROL_TIMEOUT, target_info(tab))
            .await
        {
            Ok(Ok((current, title))) if current == url => Some(title),
            _ => None,
        };
    NavigationResult {
        tab: tab.id().clone(),
        url,
        title,
        list: None,
    }
}

async fn action_result(
    tab: &BrowserTab,
    deadline: &Operation<'_>,
    operation: &str,
    wait_url: Option<&str>,
    observation: Option<&mut NavigationObservation>,
) -> Result<ActionResult> {
    let mut result = ActionResult::new(operation, json!("completed"));
    if let Some(observation) = observation {
        deadline
            .run(async {
                if let Some(pattern) = wait_url {
                    observation.wait_url(tab.page(), pattern).await?;
                } else if operation == "click" {
                    observation
                        .wait_load(Load::DomContentLoaded, true, deadline)
                        .await?;
                }
                Ok(())
            })
            .await
            .map_err(|error| deadline.failure(error, tab.id().as_str(), Some(&observation.url)))?;
        result.url = Some(observation.url.clone());
    }
    Ok(result)
}

/// The viewport point `input` names, checked against the current viewport.
pub(crate) async fn coordinates(tab: &BrowserTab, input: &str) -> Result<Point> {
    let point = parse_coordinates(input)?;
    let viewport = tab
        .page()
        .execute(GetLayoutMetricsParams {})
        .await?
        .result
        .css_layout_viewport;
    if point.x >= viewport.client_width as f64 || point.y >= viewport.client_height as f64 {
        return Err(BrowserError::Configuration(
            "coordinates must lie inside the current viewport".into(),
        ));
    }
    Ok(point)
}

/// What the page's own command families offer (`browser.md` § Capabilities
/// and WebMCP).
pub async fn capabilities(page: &chromiumoxide::Page) -> Result<Vec<Capability>> {
    let available = |id: &str| Capability {
        id: id.into(),
        available: true,
        reason: None,
        schema: None,
    };
    Ok(vec![
        available("accessibility"),
        available("read-only-eval"),
        crate::page::clipboard::capability(page).await?,
        available("page-assets"),
        available("cross-origin-frames"),
    ])
}

/// Whether `command` is a pointer or form action, whose result names the tabs
/// it opened.
fn opens_tabs(command: &BrowserOperation) -> bool {
    matches!(
        command,
        BrowserOperation::Click(_)
            | BrowserOperation::Move(_)
            | BrowserOperation::Drag(_)
            | BrowserOperation::Scroll(_)
            | BrowserOperation::Fill(_)
            | BrowserOperation::Type(_)
            | BrowserOperation::Key(_)
            | BrowserOperation::Check(_)
            | BrowserOperation::Select(_)
            | BrowserOperation::SelectText(_)
    )
}

/// Targeted browser commands carry the shared locator shape in their generated schema.
fn required_target(target: &Option<BrowserTarget>) -> Result<&BrowserTarget> {
    target
        .as_ref()
        .ok_or_else(|| BrowserError::Configuration("browser command requires a target".into()))
}

/// Pointer coordinates are finite viewport CSS pixels, supplied as one CLI argument.
fn parse_coordinates(input: &str) -> Result<Point> {
    let (x, y) = input
        .split_once(',')
        .ok_or_else(|| BrowserError::Configuration("coordinates must be x,y".into()))?;
    let x: f64 = x
        .trim()
        .parse()
        .map_err(|_| BrowserError::Configuration("invalid x coordinate".into()))?;
    let y: f64 = y
        .trim()
        .parse()
        .map_err(|_| BrowserError::Configuration("invalid y coordinate".into()))?;
    if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 {
        return Err(BrowserError::Configuration(
            "coordinates must be finite, nonnegative viewport pixels".into(),
        ));
    }
    Ok(Point { x, y })
}
