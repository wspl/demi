//! One tab admission covers observation, targeting, input and associated waits.

use std::time::Duration;

use chromiumoxide::{
    cdp::browser_protocol::{
        input::{DispatchMouseEventParams, DispatchMouseEventType, MouseButton},
        page::{GetLayoutMetricsParams, GetNavigationHistoryParams, HandleJavaScriptDialogParams},
        target::GetTargetInfoParams,
    },
    layout::Point,
};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{
    BrowserError, BrowserTab, Result, element, evaluation, keyboard,
    navigation::{Navigation, NavigationObservation},
    observation::{self, Observation, has_target_flags},
    operation::Operation,
    protocol::{BrowserCommand, BrowserTarget, DEFAULT_NODES, INLINE_BYTES, MAX_NODES},
};

impl BrowserTab {
    /// Execute a schema-validated tab command through the same admission used by the service.
    pub async fn execute(
        &self,
        operation: &str,
        args: Value,
        cancel: &CancellationToken,
    ) -> Result<Value> {
        let command = BrowserCommand::parse(operation, args)
            .map_err(|error| BrowserError::Configuration(error.to_string()))?;
        if command.tab() != Some(self.id().as_str()) {
            return Err(BrowserError::TabNotFound);
        }
        if matches!(
            command,
            BrowserCommand::Open(_)
                | BrowserCommand::Tabs(_)
                | BrowserCommand::Close(_)
                | BrowserCommand::Screenshot(_)
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
        super::protocol::validate_result(operation, result)
            .map_err(|error| BrowserError::InvalidResult(error.to_string()))
    }

    pub(super) async fn metadata(
        &self,
        cancel: &CancellationToken,
        timeout: std::time::Duration,
    ) -> Result<Value> {
        Operation::for_tab(self, cancel, tokio::time::Instant::now() + timeout)
            .run(async {
                let mut result = self.navigation_result().await?;
                result["viewport"] = self.viewport().report();
                Ok(result)
            })
            .await
    }

    pub(super) async fn command(
        &self,
        command: &BrowserCommand,
        cancel: &CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<Value> {
        let operation = Operation::for_tab(self, cancel, deadline);
        let mut references = self
            .state
            .operations
            .try_lock()
            .map_err(|_| operation.failure(BrowserError::Busy, &self.id(), None))?;
        self.command_admitted(command, cancel, deadline, &mut references)
            .await
    }

    /// Keep one browser-tab admission through a command and its attached artifact capture.
    pub(super) async fn command_admitted(
        &self,
        command: &BrowserCommand,
        cancel: &CancellationToken,
        deadline: tokio::time::Instant,
        references: &mut super::observation::References,
    ) -> Result<Value> {
        let operation = Operation::for_tab(self, cancel, deadline);
        let target = command.target();
        if !matches!(
            command,
            BrowserCommand::DialogInspect(_)
                | BrowserCommand::DialogAccept(_)
                | BrowserCommand::DialogDismiss(_)
        ) && self.state.dialog.borrow().is_some()
        {
            return Err(operation.failure(BrowserError::DialogBlocked, &self.id(), None));
        }
        let mut navigation =
            if command.wait_url().is_some() || matches!(command, BrowserCommand::Click(_)) {
                Some(
                    operation
                        .run(NavigationObservation::subscribe(&self.page))
                        .await
                        .map_err(|error| operation.failure(error, &self.id(), None))?,
                )
            } else {
                None
            };
        let current_url = operation
            .run(async { Ok(self.page.url().await?) })
            .await
            .map_err(|error| operation.failure(error, &self.id(), None))?;
        let work = async {
            let xy = match command {
                BrowserCommand::Click(input) => input.xy.as_ref(),
                BrowserCommand::Move(input) => input.xy.as_ref(),
                BrowserCommand::Scroll(input) => input.xy.as_ref(),
                _ => None,
            };
            if xy.is_some() && target.as_ref().is_some_and(has_target_flags) {
                return Err(BrowserError::Configuration(
                    "coordinates cannot be combined with an element target".into(),
                ));
            }
            if let BrowserCommand::Wait(input) = command
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
            let branch: futures_util::future::BoxFuture<'_, Result<Value>> = match command {
                BrowserCommand::Probe(input) => Box::pin(self.probe(input, references, &operation)),
                BrowserCommand::Logs(input) => {
                    Box::pin(async { self.state.console.lock().await.read(input) })
                }
                BrowserCommand::Drag(input) => Box::pin(self.drag(input, &operation)),
                BrowserCommand::SelectText(input) => Box::pin(async {
                    self.select_text(required_target(&target)?, references, input, &operation)
                        .await
                }),
                BrowserCommand::Info(_) => {
                    Box::pin(async { self.metadata(cancel, command.timeout()).await })
                }
                BrowserCommand::Goto(input) => Box::pin(async {
                    let url = self
                        .navigate(
                            Navigation::Url(input.url.clone()),
                            input.load.as_deref().unwrap_or("domcontentloaded"),
                            &operation,
                            references,
                        )
                        .await?;
                    Ok(self.completed_navigation(url).await)
                }),
                BrowserCommand::Reload(input) => Box::pin(async {
                    let url = self
                        .navigate(
                            Navigation::Reload,
                            input.load.as_deref().unwrap_or("domcontentloaded"),
                            &operation,
                            references,
                        )
                        .await?;
                    Ok(self.completed_navigation(url).await)
                }),
                BrowserCommand::Back(_) | BrowserCommand::Forward(_) => Box::pin(async {
                    let history = operation
                        .run(async {
                            Ok(self
                                .page
                                .execute(GetNavigationHistoryParams {})
                                .await?
                                .result)
                        })
                        .await?;
                    let index = history.current_index
                        + if matches!(command, BrowserCommand::Back(_)) {
                            -1
                        } else {
                            1
                        };
                    let entry = usize::try_from(index)
                        .ok()
                        .and_then(|index| history.entries.get(index))
                        .ok_or(BrowserError::HistoryBoundary)?;
                    let load = match command {
                        BrowserCommand::Back(input) => input.load.as_deref(),
                        BrowserCommand::Forward(input) => input.load.as_deref(),
                        _ => unreachable!(),
                    }
                    .unwrap_or("domcontentloaded");
                    let url = self
                        .navigate(Navigation::History(entry.id), load, &operation, references)
                        .await?;
                    Ok(self.completed_navigation(url).await)
                }),
                BrowserCommand::History(input) => Box::pin(async {
                    let history = operation
                        .run(async {
                            Ok(self
                                .page
                                .execute(GetNavigationHistoryParams {})
                                .await?
                                .result)
                        })
                        .await?;
                    let offset = input.offset.unwrap_or(0) as usize;
                    let limit = input.limit.map_or(DEFAULT_NODES, |limit| limit as usize);
                    let entries: Vec<_> = history.entries.iter().enumerate().skip(offset).take(limit).map(|(index, entry)| json!({ "index": index, "url": entry.url, "title": entry.title, "current": index as i64 == history.current_index })).collect();
                    Ok(
                        json!({ "entries": entries, "truncated": history.entries.len() > offset.saturating_add(limit) }),
                    )
                }),
                BrowserCommand::Inspect(input) => Box::pin(async {
                    let mut observation = Observation::capture(&self.page, references).await?;
                    observation.restrict(
                        input.within.as_deref(),
                        input.frame.as_deref(),
                        references,
                    )?;
                    let view = input.view.as_deref().unwrap_or("accessibility");
                    let limit = input.limit.map_or(DEFAULT_NODES, |limit| limit as usize);
                    let (tree, truncated) = if view == "dom" {
                        observation.dom_tree(references, limit)?
                    } else {
                        let (nodes, truncated) = observation.tree(references, limit)?;
                        (observation::hierarchy(nodes)?, truncated)
                    };
                    let mut result = self.navigation_result().await?;
                    result["tree"] = tree;
                    result["view"] = json!(view);
                    result["truncated"] = json!(truncated);
                    Ok(result)
                }),
                BrowserCommand::Find(input) => Box::pin(async {
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
                    let limit = input.limit.map_or(DEFAULT_NODES, |limit| limit as usize);
                    let offset = (input.offset.unwrap_or(0) as usize).min(elements.len());
                    let matches =
                        observation.describe_elements(&elements[offset..], references, limit)?;
                    Ok(
                        json!({ "matches": matches, "count": elements.len(), "truncated": elements.len() > offset.saturating_add(limit) }),
                    )
                }),
                BrowserCommand::Read(input) => Box::pin(async {
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
                    let property = input.property.as_deref().unwrap_or("attribute");
                    let mut values = Vec::new();
                    for element in elements.iter().take(MAX_NODES) {
                        let value = match property {
                            "text" | "html" | "text-content" | "value" => {
                                if property == "value" && observation.protected(element) {
                                    return Err(BrowserError::ProtectedValue);
                                }
                                let dom_property = match property {
                                    "text" => "innerText",
                                    "html" => "outerHTML",
                                    "text-content" => "textContent",
                                    _ => "value",
                                };
                                let fallback = if matches!(property, "text" | "html") {
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
                            "visible" | "enabled" | "checked" => {
                                let state = element::state(&self.page, element, &[], false).await?;
                                match property {
                                    "visible" => json!(state.visible),
                                    "enabled" => json!(state.enabled),
                                    _ => json!(state.checked),
                                }
                            }
                            "attribute" => {
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
                            _ => unreachable!("read property is schema validated"),
                        };
                        values.push(value);
                    }
                    if input.all == Some(true) {
                        Ok(json!({ "values": values, "truncated": elements.len() > MAX_NODES }))
                    } else {
                        Ok(json!({ "value": values.remove(0) }))
                    }
                }),
                BrowserCommand::Eval(input) => Box::pin(async {
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
                    Ok(json!({"value": value}))
                }),
                BrowserCommand::Fill(input) => Box::pin(async {
                    self.fill(
                        required_target(&target)?,
                        references,
                        &input.text,
                        &operation,
                    )
                    .await?;
                    self.action_result(
                        &operation,
                        "fill",
                        input.wait_url.as_deref(),
                        navigation.as_mut(),
                    )
                    .await
                }),
                BrowserCommand::Click(input) => Box::pin(async {
                    let button = match input.button.as_deref().unwrap_or("left") {
                        "left" => MouseButton::Left,
                        "middle" => MouseButton::Middle,
                        "right" => MouseButton::Right,
                        _ => unreachable!("mouse button is schema validated"),
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
                        input.count.unwrap_or(1) as i64,
                        modifiers,
                        &operation,
                    )
                    .await?;
                    self.action_result(
                        &operation,
                        "click",
                        input.wait_url.as_deref(),
                        navigation.as_mut(),
                    )
                    .await
                }),
                BrowserCommand::Move(input) => Box::pin(async {
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
                    self.action_result(&operation, "move", None, None).await
                }),
                BrowserCommand::Scroll(input) => Box::pin(async {
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
                    self.action_result(&operation, "scroll", None, None).await
                }),
                BrowserCommand::Type(input) => Box::pin(async {
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
                    if let Some(name) = focused {
                        result["target"] = json!(name);
                    }
                    Ok(result)
                }),
                BrowserCommand::Key(input) => Box::pin(async {
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
                    if let Some(name) = focused {
                        result["target"] = json!(name);
                    }
                    Ok(result)
                }),
                BrowserCommand::Check(input) => Box::pin(async {
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
                    self.action_result(&operation, "check", None, None).await
                }),
                BrowserCommand::Select(input) => Box::pin(async {
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
                    Ok(json!({"operation": "select", "result": values}))
                }),
                BrowserCommand::DialogInspect(_) => Box::pin(async {
                    let dialog = self.state.dialog.borrow().clone();
                    Ok(json!({ "dialog": dialog.as_ref().map(|dialog| json!({
                        "type": dialog.r#type, "message": dialog.message,
                    })) }))
                }),
                BrowserCommand::DialogAccept(_) | BrowserCommand::DialogDismiss(_) => {
                    Box::pin(async {
                        let dialog = self
                            .state
                            .dialog
                            .borrow()
                            .clone()
                            .ok_or(BrowserError::DialogNotFound)?;
                        if let BrowserCommand::DialogAccept(input) = command
                        && (dialog.r#type == chromiumoxide::cdp::browser_protocol::page::DialogType::Alert ||
                            (input.text.is_some() && dialog.r#type != chromiumoxide::cdp::browser_protocol::page::DialogType::Prompt)) {
                            return Err(BrowserError::InvalidDialogAction);
                        }
                        let mut request = HandleJavaScriptDialogParams::new(matches!(
                            command,
                            BrowserCommand::DialogAccept(_)
                        ));
                        if let BrowserCommand::DialogAccept(input) = command {
                            request.prompt_text = input.text.clone();
                        }
                        self.page.execute(request).await?;
                        self.state.dialog.send_if_modified(|current| {
                            if current
                                .as_ref()
                                .is_some_and(|current| std::sync::Arc::ptr_eq(current, &dialog))
                            {
                                *current = None;
                                true
                            } else {
                                false
                            }
                        });
                        let mut releases = self.state.deferred_release.lock().await;
                        while self.state.dialog.borrow().is_none() && !releases.is_empty() {
                            match &releases[0] {
                                super::tab::InputRelease::Mouse(event) => {
                                    self.page.execute(event.clone()).await?;
                                }
                                super::tab::InputRelease::Key(event) => {
                                    self.page.execute(event.clone()).await?;
                                }
                            }
                            releases.remove(0);
                        }
                        Ok(
                            json!({ "type": dialog.r#type, "outcome": if matches!(command, BrowserCommand::DialogAccept(_)) { "accepted" } else { "dismissed" } }),
                        )
                    })
                }
                BrowserCommand::Wait(input) => Box::pin(async {
                    let mut result = json!({"condition":input.url.as_deref().or(input.load.as_deref()).or(input.state.as_deref()).unwrap_or("visible"),"matched":true});
                    if let Some(url) = &input.url {
                        let mut observation = NavigationObservation::subscribe(&self.page).await?;
                        observation.wait_url(&self.page, url).await?;
                        result["url"] = json!(observation.url);
                    } else if let Some(load) = &input.load {
                        super::navigation::wait_current_load(&self.page, load).await?;
                    } else {
                        loop {
                            let observation = Observation::capture(&self.page, references).await?;
                            let elements = observation
                                .resolve_wait(&self.page, required_target(&target)?, references)
                                .await?;
                            let state = input.state.as_deref().unwrap_or("visible");
                            let mut matched = false;
                            for element in &elements {
                                let conditions =
                                    element::state(&self.page, element, &[], false).await?;
                                matched |= match state {
                                    "enabled" => conditions.enabled,
                                    "visible" | "hidden" => conditions.visible,
                                    _ => conditions.attached,
                                };
                            }
                            if matches!(state, "hidden" | "detached") {
                                matched = !matched;
                            }
                            if matched {
                                if let Some(reference) = observation
                                    .describe_elements(&elements, references, 1)?
                                    .first()
                                    .and_then(|node| node.r#ref.as_ref())
                                {
                                    result["ref"] = json!(reference);
                                }
                                break;
                            }
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                    }
                    Ok(result)
                }),
                BrowserCommand::ViewportSet(input) => Box::pin(async {
                    let viewport = super::viewport::Viewport {
                        mode: super::viewport::Mode::Custom,
                        width: input.width as u32,
                        height: input.height as u32,
                        ratio: input.scale.unwrap_or(1.0),
                    };
                    self.set_viewport(viewport).await?;
                    Ok(json!({ "viewport": viewport.report() }))
                }),
                BrowserCommand::ViewportReset(_) => Box::pin(async {
                    let viewport = self.web_viewport();
                    self.set_viewport(viewport).await?;
                    Ok(json!({ "viewport": viewport.report() }))
                }),
                BrowserCommand::ContentRead(input) => Box::pin(async {
                    let content = self
                        .content(input.format.as_deref().unwrap_or("text"), references)
                        .await?;
                    let truncated = input.output.is_none() && content.len() > INLINE_BYTES;
                    let end = if input.output.is_some() {
                        content.len()
                    } else {
                        content.floor_char_boundary(INLINE_BYTES.min(content.len()))
                    };
                    let mut result = self.navigation_result().await?;
                    result
                        .as_object_mut()
                        .expect("metadata object")
                        .remove("tab");
                    result["format"] = json!(input.format.as_deref().unwrap_or("text"));
                    result["content"] = json!(&content[..end]);
                    result["truncated"] = json!(truncated);
                    Ok(result)
                }),
                BrowserCommand::Capabilities(_) => Box::pin(async {
                    Ok(json!({ "capabilities": [
                    { "id": "accessibility", "available": true },
                    { "id": "read-only-eval", "available": true },
                    super::clipboard::capability(&self.page).await?,
                    super::webmcp::capability(&self.page).await?,
                    super::cdp::capability(),
                    { "id": "page-assets", "available": true },
                    { "id": "cross-origin-frames", "available": true }
                ] }))
                }),
                BrowserCommand::Open(_)
                | BrowserCommand::Tabs(_)
                | BrowserCommand::Close(_)
                | BrowserCommand::Screenshot(_)
                | BrowserCommand::Upload(_)
                | BrowserCommand::Download(_)
                | BrowserCommand::ClipboardRead(_)
                | BrowserCommand::ClipboardWrite(_)
                | BrowserCommand::CdpTargets(_)
                | BrowserCommand::CdpDetach(_)
                | BrowserCommand::CdpSend(_)
                | BrowserCommand::CdpEvents(_)
                | BrowserCommand::ContentFetch(_)
                | BrowserCommand::AssetsList(_)
                | BrowserCommand::AssetsExport(_)
                | BrowserCommand::WebmcpList(_)
                | BrowserCommand::WebmcpCall(_) => Box::pin(async {
                    unreachable!("conversation-level operation is dispatched before tab actions")
                }),
            };
            branch.await
        };
        let result = if matches!(
            command,
            BrowserCommand::Drag(_)
                | BrowserCommand::SelectText(_)
                | BrowserCommand::Fill(_)
                | BrowserCommand::Click(_)
                | BrowserCommand::Move(_)
                | BrowserCommand::Scroll(_)
                | BrowserCommand::Type(_)
                | BrowserCommand::Key(_)
                | BrowserCommand::Check(_)
                | BrowserCommand::Select(_)
                | BrowserCommand::Goto(_)
                | BrowserCommand::Reload(_)
                | BrowserCommand::Back(_)
                | BrowserCommand::Forward(_)
        ) {
            // Input branches own their cancellation cleanup and must not be dropped by
            // an enclosing cancellation race while releasing a key or mouse button.
            work.await
        } else {
            operation.run(work).await
        };
        let cleanup = if matches!(
            command,
            BrowserCommand::DialogInspect(_)
                | BrowserCommand::DialogAccept(_)
                | BrowserCommand::DialogDismiss(_)
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
        result.map_err(|error| operation.failure(error, &self.id(), current_url.as_deref()))
    }

    async fn navigation_result(&self) -> Result<Value> {
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
        Ok(json!({ "tab": self.id(), "url": info.url, "title": info.title }))
    }

    pub(super) async fn completed_navigation(&self, url: String) -> Value {
        let mut result = json!({"tab": self.id(), "url": url});
        // A metadata failure cannot reverse completed input. A subsequent
        // navigation's title must not be attributed to the document we observed.
        if let Ok(Ok(metadata)) =
            tokio::time::timeout(super::operation::CONTROL_TIMEOUT, self.navigation_result()).await
            && metadata["url"] == result["url"]
        {
            result["title"] = metadata["title"].clone();
        }
        result
    }

    async fn action_result(
        &self,
        deadline: &Operation<'_>,
        operation: &str,
        wait_url: Option<&str>,
        observation: Option<&mut NavigationObservation>,
    ) -> Result<Value> {
        let mut result = json!({"operation": operation, "result": "completed"});
        if let Some(observation) = observation {
            deadline
                .run(async {
                    if let Some(pattern) = wait_url {
                        observation.wait_url(&self.page, pattern).await?;
                    } else if operation == "click" {
                        observation
                            .wait_load("domcontentloaded", true, deadline)
                            .await?;
                    }
                    Ok(())
                })
                .await
                .map_err(|error| deadline.failure(error, &self.id(), Some(&observation.url)))?;
            result["url"] = json!(observation.url);
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
