//! One tab admission covers observation, targeting, input and associated waits.

use std::time::Duration;

use chromiumoxide::{
    cdp::browser_protocol::{
        emulation::SetDeviceMetricsOverrideParams,
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
                "this operation requires the browser resource controller".into(),
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
        Operation::new(&self.ended, cancel, timeout)
            .run(async {
                let mut result = self.navigation_result().await?;
                let viewport = self
                    .page
                    .execute(GetLayoutMetricsParams {})
                    .await?
                    .result
                    .css_layout_viewport;
                result["viewport"] =
                    json!({"width": viewport.client_width, "height": viewport.client_height});
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
        let operation = Operation::until(&self.ended, cancel, deadline);
        let mut references = self
            .state
            .operations
            .try_lock()
            .map_err(|_| operation.failure(BrowserError::Busy, &self.id(), None))?;
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
                && input.url.is_some()
                && (input.state.is_some() || target.as_ref().is_some_and(has_target_flags))
            {
                return Err(BrowserError::Configuration(
                    "wait requires exactly one URL or element condition".into(),
                ));
            }

            // Each branch has its own heap allocation; large command futures must
            // not be embedded together on native-service or test-thread stacks.
            let branch: futures_util::future::BoxFuture<'_, Result<Value>> = match command {
                BrowserCommand::Probe(_)
                | BrowserCommand::Drag(_)
                | BrowserCommand::SelectText(_)
                | BrowserCommand::AxAction(_)
                | BrowserCommand::Logs(_) => Box::pin(super::catalog::pending()),
                BrowserCommand::Info(_) => {
                    Box::pin(async { self.metadata(cancel, command.timeout()).await })
                }
                BrowserCommand::Goto(input) => Box::pin(async {
                    let url = self
                        .navigate(
                            Navigation::Url(input.url.clone()),
                            input.load.as_deref().unwrap_or("domcontentloaded"),
                            &operation,
                            &mut references,
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
                            &mut references,
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
                        .navigate(
                            Navigation::History(entry.id),
                            load,
                            &operation,
                            &mut references,
                        )
                        .await?;
                    Ok(self.completed_navigation(url).await)
                }),
                BrowserCommand::History(_) => Box::pin(async {
                    let history = operation
                        .run(async {
                            Ok(self
                                .page
                                .execute(GetNavigationHistoryParams {})
                                .await?
                                .result)
                        })
                        .await?;
                    let entries: Vec<_> = history.entries.iter().enumerate().take(MAX_NODES).map(|(index, entry)| json!({ "index": index, "url": entry.url, "title": entry.title, "current": index as i64 == history.current_index })).collect();
                    Ok(
                        json!({ "entries": entries, "truncated": history.entries.len() > MAX_NODES }),
                    )
                }),
                BrowserCommand::Inspect(input) => Box::pin(async {
                    let observation = Observation::capture(&self.page, &mut references).await?;
                    let (tree, truncated) = observation.tree(
                        &mut references,
                        input.limit.map_or(DEFAULT_NODES, |limit| limit as usize),
                    )?;
                    let mut result = self.navigation_result().await?;
                    result["tree"] = observation::hierarchy(tree)?;
                    result["view"] = json!("accessibility");
                    result["truncated"] = json!(truncated);
                    Ok(result)
                }),
                BrowserCommand::Find(input) => Box::pin(async {
                    let observation = Observation::capture(&self.page, &mut references).await?;
                    let elements = observation
                        .resolve(&self.page, required_target(&target)?, &references)
                        .await?;
                    let limit = input.limit.map_or(DEFAULT_NODES, |limit| limit as usize);
                    let matches =
                        observation.describe_elements(&elements, &mut references, limit)?;
                    Ok(
                        json!({ "matches": matches, "count": elements.len(), "truncated": elements.len() > limit }),
                    )
                }),
                BrowserCommand::Read(input) => Box::pin(async {
                    let observation = Observation::capture(&self.page, &mut references).await?;
                    let elements = observation
                        .resolve(&self.page, required_target(&target)?, &references)
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
                                if property == "value"
                                    && observation.protected(element.backend_node_id)
                                {
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
                    Ok(
                        json!({ "value": evaluation::read_only(&self.page, &input.expression).await? }),
                    )
                }),
                BrowserCommand::Fill(input) => Box::pin(async {
                    self.fill(
                        required_target(&target)?,
                        &mut references,
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
                    let mut modifiers = 0;
                    for name in input.modifier.iter().flatten() {
                        modifiers |=
                            keyboard::modifier(name).expect("modifier is schema validated");
                    }
                    let point = match &input.xy {
                        Some(xy) => operation.run(self.coordinates(xy)).await?,
                        None => self
                            .ready_element(
                                required_target(&target)?,
                                &mut references,
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
                                &mut references,
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
                            self.page.move_mouse(point).await?;
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
                                &mut references,
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
                    let (element, _) = self
                        .ready_element(
                            required_target(&target)?,
                            &mut references,
                            element::TYPE,
                            &operation,
                        )
                        .await?;
                    self.type_text(&element, &input.text, &operation).await?;
                    self.action_result(&operation, "type", None, None).await
                }),
                BrowserCommand::Key(input) => Box::pin(async {
                    let keys = keyboard::combination(&input.key)?;
                    let (element, _) = self
                        .ready_element(
                            required_target(&target)?,
                            &mut references,
                            element::KEY,
                            &operation,
                        )
                        .await?;
                    self.focus(&element, &operation).await?;
                    self.press(&keys, &operation).await?;
                    self.action_result(
                        &operation,
                        "key",
                        input.wait_url.as_deref(),
                        navigation.as_mut(),
                    )
                    .await
                }),
                BrowserCommand::Check(input) => Box::pin(async {
                    let (_, state) = self
                        .ready_element(
                            required_target(&target)?,
                            &mut references,
                            &["checkable"],
                            &operation,
                        )
                        .await?;
                    if state.needs_check(input.value)? {
                        let (element, state) = self
                            .ready_element(
                                required_target(&target)?,
                                &mut references,
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
                            &mut references,
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
                    if let Some(url) = &input.url {
                        let mut observation = NavigationObservation::subscribe(&self.page).await?;
                        observation.wait_url(&self.page, url).await?;
                    } else {
                        loop {
                            let observation =
                                Observation::capture(&self.page, &mut references).await?;
                            let elements = observation
                                .resolve_wait(&self.page, required_target(&target)?, &references)
                                .await?;
                            let state = input.state.as_deref().unwrap_or("visible");
                            let mut matched = false;
                            for element in elements {
                                let conditions =
                                    element::state(&self.page, &element, &[], false).await?;
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
                                break;
                            }
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                    }
                    Ok(
                        json!({ "condition": input.url.as_deref().or(input.state.as_deref()).unwrap_or("visible"), "matched": true }),
                    )
                }),
                BrowserCommand::ViewportSet(input) => Box::pin(async {
                    self.page
                        .execute(SetDeviceMetricsOverrideParams::new(
                            input.width as i64,
                            input.height as i64,
                            1.0,
                            false,
                        ))
                        .await?;
                    Ok(json!({ "width": input.width, "height": input.height }))
                }),
                BrowserCommand::ViewportReset(_) => Box::pin(async {
                    self.page
                        .execute(SetDeviceMetricsOverrideParams::new(1280, 720, 1.0, false))
                        .await?;
                    Ok(json!({ "width": 1280, "height": 720 }))
                }),
                BrowserCommand::ContentRead(input) => Box::pin(async {
                    let content = match input.format.as_deref().unwrap_or("text") {
                        "html" => self
                            .page
                            .find_element("html")
                            .await?
                            .outer_html()
                            .await?
                            .unwrap_or_default(),
                        "text" => self
                            .page
                            .find_element("body")
                            .await?
                            .inner_text()
                            .await?
                            .unwrap_or_default(),
                        "dom" => {
                            let document = self.page.execute(chromiumoxide::cdp::browser_protocol::dom::GetDocumentParams::builder().depth(-1).pierce(true).build()).await?.result;
                            serde_json::to_string(&document)
                                .map_err(|error| BrowserError::InvalidResult(error.to_string()))?
                        }
                        _ => unreachable!("content format is schema validated"),
                    };
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
                    { "id": "cross-origin-frames", "available": false, "reason": "Not yet accepted on all browser platforms" }
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
                | BrowserCommand::CdpSend(_)
                | BrowserCommand::CdpEvents(_)
                | BrowserCommand::ContentFetch(_)
                | BrowserCommand::AssetsList(_)
                | BrowserCommand::AssetsExport(_)
                | BrowserCommand::WebmcpList(_)
                | BrowserCommand::WebmcpCall(_) => Box::pin(async {
                    unreachable!("resource-level operation is dispatched before tab actions")
                }),
            };
            branch.await
        };
        let result = if matches!(
            command,
            BrowserCommand::Fill(_)
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
        // Chrome pauses Runtime requests while a dialog is open. Dialog commands
        // never wait for page-side object cleanup; the next ordinary command
        // releases the retained group, or context/tab destruction releases it.
        let cleanup = if self.state.dialog.borrow().is_some()
            || matches!(
                command,
                BrowserCommand::DialogInspect(_)
                    | BrowserCommand::DialogAccept(_)
                    | BrowserCommand::DialogDismiss(_)
            ) {
            Ok(())
        } else {
            let cleanup = tokio::time::timeout(
                super::operation::CONTROL_TIMEOUT,
                self.page.execute(
                    chromiumoxide::cdp::js_protocol::runtime::ReleaseObjectGroupParams::new(
                        element::OBJECT_GROUP,
                    ),
                ),
            )
            .await
            .map_err(|_| BrowserError::Timeout)
            .and_then(|result| result.map(|_| ()).map_err(BrowserError::from));
            // Destroying the tab or connection already releases its remote objects.
            match cleanup {
                Err(
                    BrowserError::Closed | BrowserError::Connection(_) | BrowserError::TabNotFound,
                ) => Ok(()),
                result => result,
            }
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

    async fn coordinates(&self, input: &str) -> Result<Point> {
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
