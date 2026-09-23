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

use super::{
    BrowserError, BrowserTab, Result, element, evaluation, keyboard,
    navigation::{Navigation, NavigationObservation, history_step},
    observation::{self, Observation, has_target_flags},
    operation::Operation,
    protocol::{
        ActionResult, BrowserOperation, BrowserTarget, BrowserViewport, CapabilitiesResult,
        Capability, ContentReadResult, DEFAULT_NODES, Dialog, DialogInspectResult, DialogOutcome,
        DialogResult, ElementState, EvalResult, FindResult, HistoryEntry, HistoryResult,
        INLINE_BYTES, InfoResult, InspectResult, Load, LogsResult, MAX_NODES, NavigationResult,
        ProbeResult, ReadProperty, ReadResult, ViewportMode, ViewportResult, WaitResult,
    },
};

/// A tab operation's result, printed as the operation's own result type.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(super) enum TabResult {
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
    Capabilities(CapabilitiesResult),
}

impl BrowserTab {
    /// Execute a schema-validated tab command through the same admission used by the service.
    pub async fn execute(
        &self,
        operation: &str,
        args: Value,
        cancel: &CancellationToken,
    ) -> Result<Value> {
        let command = BrowserOperation::parse(operation, args)
            .map_err(|error| BrowserError::Configuration(error.to_string()))?;
        if command.tab() != Some(self.id()) {
            return Err(BrowserError::TabNotFound);
        }
        if matches!(
            command,
            BrowserOperation::Open(_)
                | BrowserOperation::Tabs(_)
                | BrowserOperation::Close(_)
                | BrowserOperation::Screenshot(_)
        ) {
            return Err(BrowserError::Configuration(
                "this operation requires the browser conversation controller".into(),
            ));
        }
        let result = self
            .command(
                &command,
                cancel,
                tokio::time::Instant::now() + command.timeout(),
            )
            .await?;
        super::output::value(result)
    }

    pub(super) async fn metadata(
        &self,
        cancel: &CancellationToken,
        timeout: std::time::Duration,
    ) -> Result<InfoResult> {
        Operation::for_tab(self, cancel, tokio::time::Instant::now() + timeout)
            .run(async {
                let (url, title) = self.target_info().await?;
                let dialog = self.state.dialog.open().map(|dialog| Dialog {
                    r#type: super::tab::dialog_type(&dialog.r#type),
                    message: dialog.message.clone(),
                });
                Ok(InfoResult {
                    tab: self.id().clone(),
                    url,
                    title,
                    viewport: self.viewport(),
                    dialog,
                })
            })
            .await
    }

    pub(super) async fn command(
        &self,
        command: &BrowserOperation,
        cancel: &CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<TabResult> {
        let operation = Operation::for_tab(self, cancel, deadline);
        let mut session = self
            .state
            .gate
            .try_checkout()
            .ok_or_else(|| operation.failure(BrowserError::Busy, self.id().as_str(), None))?;
        self.command_admitted(command, cancel, deadline, &mut session.references)
            .await
    }

    /// Keep one browser-tab admission through a command and its attached artifact capture.
    pub(super) async fn command_admitted(
        &self,
        command: &BrowserOperation,
        cancel: &CancellationToken,
        deadline: tokio::time::Instant,
        references: &mut super::observation::References,
    ) -> Result<TabResult> {
        let operation = Operation::for_tab(self, cancel, deadline);
        let target = command.target();
        if !matches!(
            command,
            BrowserOperation::DialogInspect(_)
                | BrowserOperation::DialogAccept(_)
                | BrowserOperation::DialogDismiss(_)
        ) && self.state.dialog.is_open()
        {
            return Err(operation.failure(BrowserError::DialogBlocked, self.id().as_str(), None));
        }
        let mut navigation =
            if command.wait_url().is_some() || matches!(command, BrowserOperation::Click(_)) {
                Some(
                    operation
                        .run(NavigationObservation::subscribe(&self.page))
                        .await
                        .map_err(|error| operation.failure(error, self.id().as_str(), None))?,
                )
            } else {
                None
            };
        let current_url = operation
            .run(async { Ok(self.page.url().await?) })
            .await
            .map_err(|error| operation.failure(error, self.id().as_str(), None))?;
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
                    "wait requires exactly one URL, current-document load, or element condition"
                        .into(),
                ));
            }

            // Each branch has its own heap allocation; large command futures must
            // not be embedded together on native-service or test-thread stacks.
            let branch: futures_util::future::BoxFuture<'_, Result<TabResult>> = match command {
                BrowserOperation::Probe(input) => Box::pin(async {
                    Ok(TabResult::Probe(
                        self.probe(input, references, &operation).await?,
                    ))
                }),
                BrowserOperation::Logs(input) => Box::pin(async {
                    Ok(TabResult::Logs(self.state.console.read(input).await?))
                }),
                BrowserOperation::Drag(input) => {
                    Box::pin(async { Ok(TabResult::Action(self.drag(input, &operation).await?)) })
                }
                BrowserOperation::SelectText(input) => Box::pin(async {
                    let result = self
                        .select_text(required_target(&target)?, references, input, &operation)
                        .await?;
                    Ok(TabResult::Action(result))
                }),
                BrowserOperation::Info(_) => Box::pin(async {
                    Ok(TabResult::Info(
                        self.metadata(cancel, command.timeout()).await?,
                    ))
                }),
                BrowserOperation::Goto(input) => Box::pin(async {
                    let url = self
                        .navigate(
                            Navigation::Url(input.url.clone()),
                            input.load.unwrap_or_default(),
                            &operation,
                            references,
                        )
                        .await?;
                    Ok(TabResult::Navigation(self.completed_navigation(url).await))
                }),
                BrowserOperation::Reload(input) => Box::pin(async {
                    let url = self
                        .navigate(
                            Navigation::Reload,
                            input.load.unwrap_or_default(),
                            &operation,
                            references,
                        )
                        .await?;
                    Ok(TabResult::Navigation(self.completed_navigation(url).await))
                }),
                BrowserOperation::Back(_) | BrowserOperation::Forward(_) => Box::pin(async {
                    let back = matches!(command, BrowserOperation::Back(_));
                    let entry = history_step(self, back, &operation).await?;
                    let load = match command {
                        BrowserOperation::Back(input) => input.load,
                        BrowserOperation::Forward(input) => input.load,
                        _ => unreachable!(),
                    }
                    .unwrap_or_default();
                    let url = self
                        .navigate(Navigation::History(entry.id), load, &operation, references)
                        .await?;
                    Ok(TabResult::Navigation(self.completed_navigation(url).await))
                }),
                BrowserOperation::History(input) => Box::pin(async {
                    let history = operation
                        .run(async {
                            Ok(self
                                .page
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
                    let mut observation = Observation::capture(&self.page, references).await?;
                    observation.restrict(
                        input.within.as_ref(),
                        input.frame.as_deref(),
                        references,
                    )?;
                    let view = input.view.unwrap_or_default();
                    let limit = input.limit.unwrap_or(DEFAULT_NODES);
                    let (tree, truncated) = match view {
                        super::protocol::InspectView::Dom => observation.dom_tree(references, limit)?,
                        super::protocol::InspectView::Accessibility => {
                            let (nodes, truncated) = observation.tree(references, limit)?;
                            (observation::hierarchy(nodes), truncated)
                        }
                    };
                    let (url, title) = self.target_info().await?;
                    Ok(TabResult::Inspect(InspectResult {
                        tab: self.id().clone(),
                        url,
                        title,
                        view,
                        tree,
                        truncated,
                    }))
                }),
                BrowserOperation::Find(input) => Box::pin(async {
                    let observation = Observation::capture(&self.page, references).await?;
                    let elements = if input.query == Some(true) {
                        if target.as_ref().is_some_and(has_target_flags) {
                            return Err(BrowserError::Configuration(
                                "find --query cannot combine ordinary target flags".into(),
                            ));
                        }
                        let query =
                            super::query::parse(input.body.as_deref().ok_or_else(|| {
                                BrowserError::Configuration("find --query requires stdin".into())
                            })?)?;
                        observation.query(&self.page, &query, references).await?
                    } else {
                        if input.body.is_some() {
                            return Err(BrowserError::Configuration(
                                "find stdin requires --query".into(),
                            ));
                        }
                        observation
                            .resolve(&self.page, required_target(&target)?, references)
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
                    let observation = Observation::capture(&self.page, references).await?;
                    let elements = observation
                        .resolve(&self.page, required_target(&target)?, references)
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
                                if property == ReadProperty::Value && observation.protected(element)
                                {
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
                                    &self.page,
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
                                let state = element::state(&self.page, element, &[], false).await?;
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
                                    &self.page,
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
                        let observation = Observation::capture(&self.page, references).await?;
                        let elements = observation
                            .resolve(&self.page, required_target(&target)?, references)
                            .await?;
                        if elements.len() > 1 && input.all != Some(true) {
                            return Err(BrowserError::Ambiguous(elements.len()));
                        }
                        evaluation::targeted(
                            &self.page,
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
                        evaluation::read_only(&self.page, &input.expression).await?
                    };
                    Ok(TabResult::Eval(EvalResult { value }))
                }),
                BrowserOperation::Fill(input) => Box::pin(async {
                    self.fill(
                        required_target(&target)?,
                        references,
                        &input.text,
                        &operation,
                    )
                    .await?;
                    let result = self
                        .action_result(
                            &operation,
                            "fill",
                            input.wait_url.as_deref(),
                            navigation.as_mut(),
                        )
                        .await?;
                    Ok(TabResult::Action(result))
                }),
                BrowserOperation::Click(input) => Box::pin(async {
                    let button = match input.button.unwrap_or_default() {
                        super::protocol::MouseButton::Left => MouseButton::Left,
                        super::protocol::MouseButton::Middle => MouseButton::Middle,
                        super::protocol::MouseButton::Right => MouseButton::Right,
                    };
                    let modifiers = keyboard::modifiers(input.modifier.as_deref());
                    let point = match &input.xy {
                        Some(xy) => operation.run(self.coordinates(xy)).await?,
                        None => self
                            .ready_element(
                                required_target(&target)?,
                                references,
                                element::CLICK,
                                &operation,
                            )
                            .await?
                            .1
                            .point(),
                    };
                    self.click_at(
                        point,
                        button,
                        i64::from(input.count.unwrap_or(1)),
                        modifiers,
                        &operation,
                    )
                    .await?;
                    let result = self
                        .action_result(
                            &operation,
                            "click",
                            input.wait_url.as_deref(),
                            navigation.as_mut(),
                        )
                        .await?;
                    Ok(TabResult::Action(result))
                }),
                BrowserOperation::Move(input) => Box::pin(async {
                    let point = match &input.xy {
                        Some(xy) => operation.run(self.coordinates(xy)).await?,
                        None => self
                            .ready_element(
                                required_target(&target)?,
                                references,
                                element::GEOMETRY,
                                &operation,
                            )
                            .await?
                            .1
                            .point(),
                    };
                    operation
                        .run(async {
                            operation.begin_input();
                            self.page
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
                    let result = self.action_result(&operation, "move", None, None).await?;
                    Ok(TabResult::Action(result))
                }),
                BrowserOperation::Scroll(input) => Box::pin(async {
                    if input.dx.unwrap_or(0.0) == 0.0 && input.dy.unwrap_or(0.0) == 0.0 {
                        return Err(BrowserError::Configuration(
                            "scroll requires a nonzero dx or dy".into(),
                        ));
                    }
                    let point = match &input.xy {
                        Some(xy) => operation.run(self.coordinates(xy)).await?,
                        None if !target.as_ref().is_some_and(has_target_flags) => {
                            let viewport = operation
                                .run(async {
                                    Ok(self
                                        .page
                                        .execute(GetLayoutMetricsParams {})
                                        .await?
                                        .result
                                        .css_layout_viewport)
                                })
                                .await?;
                            Point {
                                x: viewport.client_width as f64 / 2.0,
                                y: viewport.client_height as f64 / 2.0,
                            }
                        }
                        None => self
                            .ready_element(
                                required_target(&target)?,
                                references,
                                element::GEOMETRY,
                                &operation,
                            )
                            .await?
                            .1
                            .point(),
                    };
                    operation
                        .run(async {
                            operation.begin_input();
                            self.page
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
                    let result = self.action_result(&operation, "scroll", None, None).await?;
                    Ok(TabResult::Action(result))
                }),
                BrowserOperation::Type(input) => Box::pin(async {
                    let focused = if target.as_ref().is_some_and(has_target_flags) {
                        let (element, _) = self
                            .ready_element(
                                required_target(&target)?,
                                references,
                                element::TYPE,
                                &operation,
                            )
                            .await?;
                        self.type_text(&element, &input.text, &operation).await?;
                        None
                    } else {
                        let (_, name) = self.current_focus(references, &operation).await?;
                        self.type_focused(None, &input.text, &operation).await?;
                        Some(name)
                    };
                    let mut result = self.action_result(&operation, "type", None, None).await?;
                    result.target = focused;
                    Ok(TabResult::Action(result))
                }),
                BrowserOperation::Key(input) => Box::pin(async {
                    let keys = keyboard::combination(&input.key)?;
                    let focused = if target.as_ref().is_some_and(has_target_flags) {
                        let (element, _) = self
                            .ready_element(
                                required_target(&target)?,
                                references,
                                element::KEY,
                                &operation,
                            )
                            .await?;
                        self.focus(&element, &operation).await?;
                        None
                    } else {
                        Some(self.current_focus(references, &operation).await?.1)
                    };
                    self.press(&keys, &operation).await?;
                    let mut result = self
                        .action_result(
                            &operation,
                            "key",
                            input.wait_url.as_deref(),
                            navigation.as_mut(),
                        )
                        .await?;
                    result.target = focused;
                    Ok(TabResult::Action(result))
                }),
                BrowserOperation::Check(input) => Box::pin(async {
                    let (_, state) = self
                        .ready_element(
                            required_target(&target)?,
                            references,
                            &["checkable"],
                            &operation,
                        )
                        .await?;
                    if state.needs_check(input.value)? {
                        let (element, state) = self
                            .ready_element(
                                required_target(&target)?,
                                references,
                                element::CLICK,
                                &operation,
                            )
                            .await?;
                        if state.needs_check(input.value)? {
                            self.click_at(state.point(), MouseButton::Left, 1, 0, &operation)
                                .await?;
                        }
                        let state = operation
                            .run(element::state(&self.page, &element, &[], false))
                            .await?;
                        if state.checked != Some(input.value) {
                            return Err(BrowserError::NotActionable {
                                condition: "click did not change checked state".into(),
                                interceptor: None,
                            });
                        }
                    }
                    let result = self.action_result(&operation, "check", None, None).await?;
                    Ok(TabResult::Action(result))
                }),
                BrowserOperation::Select(input) => Box::pin(async {
                    let values = self
                        .select_options(
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
                    Ok(TabResult::Action(ActionResult::new("select", json!(values))))
                }),
                BrowserOperation::DialogInspect(_) => Box::pin(async {
                    let dialog = self.state.dialog.open().map(|dialog| Dialog {
                        r#type: super::tab::dialog_type(&dialog.r#type),
                        message: dialog.message.clone(),
                    });
                    Ok(TabResult::DialogInspect(DialogInspectResult { dialog }))
                }),
                BrowserOperation::DialogAccept(_) | BrowserOperation::DialogDismiss(_) => {
                    Box::pin(async {
                        let dialog = self
                            .state
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
                        self.state.dialog.answer(&dialog, accept, text).await?;
                        Ok(TabResult::Dialog(DialogResult {
                            r#type: super::tab::dialog_type(&dialog.r#type),
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
                        r#ref: None,
                    };
                    if let Some(url) = &input.url {
                        let mut observation = NavigationObservation::subscribe(&self.page).await?;
                        observation.wait_url(&self.page, url).await?;
                        result.url = Some(observation.url);
                    } else if let Some(load) = input.load {
                        super::navigation::wait_current_load(&self.page, load).await?;
                    } else {
                        loop {
                            let observation = Observation::capture(&self.page, references).await?;
                            let target = required_target(&target)?;
                            let elements = match observation
                                .resolve_wait(&self.page, target, references)
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
                                    element::state(&self.page, element, &[], false).await?;
                                matched |= match state {
                                    ElementState::Enabled => conditions.enabled,
                                    ElementState::Visible | ElementState::Hidden => {
                                        conditions.visible
                                    }
                                    ElementState::Attached | ElementState::Detached => {
                                        conditions.attached
                                    }
                                };
                            }
                            if matches!(state, ElementState::Hidden | ElementState::Detached) {
                                matched = !matched;
                            }
                            if matched {
                                result.r#ref = observation
                                    .describe_elements(&elements, references, 1)?
                                    .into_iter()
                                    .next()
                                    .and_then(|node| node.r#ref);
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
                    self.set_viewport(viewport).await?;
                    Ok(TabResult::Viewport(ViewportResult { viewport }))
                }),
                BrowserOperation::ViewportReset(_) => Box::pin(async {
                    let viewport = self.web_viewport();
                    self.set_viewport(viewport).await?;
                    Ok(TabResult::Viewport(ViewportResult { viewport }))
                }),
                BrowserOperation::ContentRead(input) => Box::pin(async {
                    let format = input.format.unwrap_or_default();
                    let content = self.content(format, references).await?;
                    let truncated = input.output.is_none() && content.len() > INLINE_BYTES;
                    let end = if input.output.is_some() {
                        content.len()
                    } else {
                        content.floor_char_boundary(INLINE_BYTES.min(content.len()))
                    };
                    let (url, title) = self.target_info().await?;
                    Ok(TabResult::ContentRead(ContentReadResult::Inline {
                        url,
                        title,
                        format,
                        content: content[..end].to_owned(),
                        truncated,
                    }))
                }),
                BrowserOperation::Capabilities(_) => Box::pin(async {
                    let available = |id: &str| Capability {
                        id: id.into(),
                        available: true,
                        reason: None,
                        schema: None,
                    };
                    Ok(TabResult::Capabilities(CapabilitiesResult {
                        capabilities: vec![
                            available("accessibility"),
                            available("read-only-eval"),
                            super::clipboard::capability(&self.page).await?,
                            super::webmcp::capability(&self.page).await?,
                            super::cdp::capability(),
                            available("page-assets"),
                            available("cross-origin-frames"),
                        ],
                    }))
                }),
                BrowserOperation::Open(_)
                | BrowserOperation::Tabs(_)
                | BrowserOperation::Close(_)
                | BrowserOperation::Screenshot(_)
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
                | BrowserOperation::WebmcpCall(_) => Box::pin(async {
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
            self.release_objects().await
        };
        let result = super::operation::after_cleanup(result, cleanup);
        if navigation
            .as_ref()
            .is_some_and(NavigationObservation::document_changed)
        {
            references.invalidate();
        }
        result.map_err(|error| operation.failure(error, self.id().as_str(), current_url.as_deref()))
    }

    /// The tab's current URL and title.
    async fn target_info(&self) -> Result<(String, String)> {
        let info = self
            .page
            .execute(
                GetTargetInfoParams::builder()
                    .target_id(self.page.target_id().clone())
                    .build(),
            )
            .await?
            .result
            .target_info;
        Ok((info.url, info.title))
    }

    pub(super) async fn completed_navigation(&self, url: String) -> NavigationResult {
        // A metadata failure cannot reverse completed input. A subsequent
        // navigation's title must not be attributed to the document we observed.
        let title = match tokio::time::timeout(super::operation::CONTROL_TIMEOUT, self.target_info())
            .await
        {
            Ok(Ok((current, title))) if current == url => Some(title),
            _ => None,
        };
        NavigationResult {
            tab: self.id().clone(),
            url,
            title,
        }
    }

    async fn action_result(
        &self,
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
                        observation.wait_url(&self.page, pattern).await?;
                    } else if operation == "click" {
                        observation
                            .wait_load(Load::DomContentLoaded, true, deadline)
                            .await?;
                    }
                    Ok(())
                })
                .await
                .map_err(|error| {
                    deadline.failure(error, self.id().as_str(), Some(&observation.url))
                })?;
            result.url = Some(observation.url.clone());
        }
        Ok(result)
    }

    pub(super) async fn coordinates(&self, input: &str) -> Result<Point> {
        let point = coordinates(input)?;
        let viewport = self
            .page
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
}

/// Targeted browser commands carry the shared locator shape in their generated schema.
fn required_target(target: &Option<BrowserTarget>) -> Result<&BrowserTarget> {
    target
        .as_ref()
        .ok_or_else(|| BrowserError::Configuration("browser command requires a target".into()))
}

/// Pointer coordinates are finite viewport CSS pixels, supplied as one CLI argument.
fn coordinates(input: &str) -> Result<Point> {
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
