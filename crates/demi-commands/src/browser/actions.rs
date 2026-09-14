//! One tab admission covers observation, targeting, input and associated waits.

use std::time::Duration;

use chromiumoxide::{
    cdp::browser_protocol::{
        emulation::SetDeviceMetricsOverrideParams,
        input::{
            DispatchKeyEventParams, DispatchKeyEventType, DispatchMouseEventParams,
            DispatchMouseEventType, MouseButton,
        },
        page::{
            Frame, GetFrameTreeParams, GetLayoutMetricsParams, GetNavigationHistoryParams,
            HandleJavaScriptDialogParams, NavigateToHistoryEntryParams,
        },
        target::GetTargetInfoParams,
    },
    layout::Point,
};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{
    BrowserError, BrowserTab, Result, evaluation,
    observation::Observation,
    operation::{CONTROL_TIMEOUT, Operation, after_cleanup},
    protocol::{BrowserCommand, BrowserTarget, DEFAULT_NODES, INLINE_BYTES, MAX_NODES},
};

impl BrowserTab {
    pub(super) async fn metadata(
        &self,
        cancel: &CancellationToken,
        timeout: std::time::Duration,
    ) -> Result<Value> {
        Operation::new(&self.ended, cancel, timeout)
            .run(async {
                let target = self
                    .page
                    .execute(
                        GetTargetInfoParams::builder()
                            .target_id(self.page.target_id().clone())
                            .build(),
                    )
                    .await?
                    .result
                    .target_info;
                let viewport = self
                    .page
                    .execute(GetLayoutMetricsParams {})
                    .await?
                    .result
                    .css_layout_viewport;
                Ok(
                    json!({ "tab": self.id(), "url": target.url, "title": target.title,
            "viewport": { "width": viewport.client_width, "height": viewport.client_height } }),
                )
            })
            .await
    }

    pub(super) async fn command(
        &self,
        command: &BrowserCommand,
        cancel: &CancellationToken,
    ) -> Result<Value> {
        let mut references = self
            .state
            .operations
            .try_lock()
            .map_err(|_| BrowserError::Busy)?;
        let operation = Operation::new(&self.ended, cancel, command.timeout());
        let target = command.target();
        if !matches!(
            command,
            BrowserCommand::DialogInspect(_)
                | BrowserCommand::DialogAccept(_)
                | BrowserCommand::DialogDismiss(_)
        ) && self.state.dialog.borrow().is_some()
        {
            return Err(BrowserError::DialogBlocked);
        }
        let navigation_before = if command.wait_url().is_some() {
            Some(
                operation
                    .run(async {
                        Ok(self
                            .page
                            .execute(GetFrameTreeParams {})
                            .await?
                            .result
                            .frame_tree
                            .frame)
                    })
                    .await?,
            )
        } else {
            None
        };
        let work = async {
            match command {
                BrowserCommand::Info(_) => self.metadata(cancel, command.timeout()).await,
                BrowserCommand::Goto(input) => {
                    self.page.goto(input.url.clone()).await?;
                    self.navigation_result().await
                }
                BrowserCommand::Reload(_) => {
                    self.page.reload().await?;
                    self.navigation_result().await
                }
                BrowserCommand::Back(_) | BrowserCommand::Forward(_) => {
                    let history = self
                        .page
                        .execute(GetNavigationHistoryParams {})
                        .await?
                        .result;
                    let index = history.current_index
                        + if matches!(command, BrowserCommand::Back(_)) {
                            -1
                        } else {
                            1
                        };
                    let entry = usize::try_from(index)
                        .ok()
                        .and_then(|index| history.entries.get(index))
                        .ok_or_else(|| {
                            BrowserError::Configuration(
                                "no navigation entry in that direction".into(),
                            )
                        })?;
                    self.page
                        .execute(NavigateToHistoryEntryParams::new(entry.id))
                        .await?;
                    self.navigation_result().await
                }
                BrowserCommand::History(_) => {
                    let history = self
                        .page
                        .execute(GetNavigationHistoryParams {})
                        .await?
                        .result;
                    let entries: Vec<_> = history.entries.iter().enumerate().take(MAX_NODES).map(|(index, entry)| json!({ "index": index, "url": entry.url, "title": entry.title, "current": index as i64 == history.current_index })).collect();
                    Ok(
                        json!({ "entries": entries, "truncated": history.entries.len() > MAX_NODES }),
                    )
                }
                BrowserCommand::Inspect(input) => {
                    let observation = Observation::capture(&self.page, &mut references).await?;
                    let (tree, truncated) = observation.tree(
                        &mut references,
                        input.limit.map_or(DEFAULT_NODES, |limit| limit as usize),
                    )?;
                    let mut result = self.navigation_result().await?;
                    result["tree"] = json!(tree);
                    result["truncated"] = json!(truncated);
                    Ok(result)
                }
                BrowserCommand::Find(input) => {
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
                }
                BrowserCommand::Read(input) => {
                    let observation = Observation::capture(&self.page, &mut references).await?;
                    let elements = observation
                        .resolve(&self.page, required_target(&target)?, &references)
                        .await?;
                    if input.all != Some(true) && elements.len() != 1 {
                        return Err(BrowserError::Ambiguous(elements.len()));
                    }
                    let mut values = Vec::new();
                    for element in elements.iter().take(MAX_NODES) {
                        let value = match input.property.as_str() {
                            "text" => json!(element.inner_text().await?.unwrap_or_default()),
                            "html" => json!(element.outer_html().await?.unwrap_or_default()),
                            "value" => element.property("value").await?.unwrap_or(Value::Null),
                            "attribute" => {
                                let attribute = input.attribute.as_ref().ok_or_else(|| {
                                    BrowserError::Configuration(
                                        "attribute read requires --attribute".into(),
                                    )
                                })?;
                                let attributes = element.attributes().await?;
                                json!(
                                    attributes
                                        .chunks_exact(2)
                                        .find(|pair| &pair[0] == attribute)
                                        .map(|pair| &pair[1])
                                )
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
                }
                BrowserCommand::Eval(input) => Ok(
                    json!({ "value": evaluation::read_only(&self.page, &input.expression).await? }),
                ),
                BrowserCommand::Fill(input) => {
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
                        navigation_before.as_ref(),
                    )
                    .await
                }
                BrowserCommand::Click(input) => {
                    let button = match input.button.as_deref().unwrap_or("left") {
                        "left" => MouseButton::Left,
                        "middle" => MouseButton::Middle,
                        "right" => MouseButton::Right,
                        _ => unreachable!("mouse button is schema validated"),
                    };
                    let mut modifiers = 0;
                    for modifier in input.modifier.iter().flatten() {
                        modifiers |= match modifier.as_str() {
                            "Alt" => 1,
                            "Control" => 2,
                            "Meta" => 4,
                            "Shift" => 8,
                            "ControlOrMeta" => {
                                if cfg!(target_os = "macos") {
                                    4
                                } else {
                                    2
                                }
                            }
                            _ => unreachable!("modifier is schema validated"),
                        };
                    }
                    let point = match &input.xy {
                        Some(xy) => coordinates(xy)?,
                        None => {
                            self.ready_element(
                                required_target(&target)?,
                                &mut references,
                                false,
                                &operation,
                            )
                            .await?
                            .1
                        }
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
                        navigation_before.as_ref(),
                    )
                    .await
                }
                BrowserCommand::Move(input) => {
                    let point = match &input.xy {
                        Some(xy) => coordinates(xy)?,
                        None => {
                            self.ready_element(
                                required_target(&target)?,
                                &mut references,
                                false,
                                &operation,
                            )
                            .await?
                            .1
                        }
                    };
                    operation
                        .run(async {
                            self.page.move_mouse(point).await?;
                            Ok(())
                        })
                        .await?;
                    self.action_result(&operation, "move", None, None).await
                }
                BrowserCommand::Scroll(input) => {
                    let point = match &input.xy {
                        Some(xy) => coordinates(xy)?,
                        None if input.r#ref.is_none()
                            && input.role.is_none()
                            && input.label.is_none()
                            && input.placeholder.is_none()
                            && input.text_match.is_none()
                            && input.test_id.is_none()
                            && input.css.is_none() =>
                        {
                            let viewport = self
                                .page
                                .execute(GetLayoutMetricsParams {})
                                .await?
                                .result
                                .css_layout_viewport;
                            Point {
                                x: viewport.client_width as f64 / 2.0,
                                y: viewport.client_height as f64 / 2.0,
                            }
                        }
                        None => {
                            self.ready_element(
                                required_target(&target)?,
                                &mut references,
                                false,
                                &operation,
                            )
                            .await?
                            .1
                        }
                    };
                    operation
                        .run(async {
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
                            Ok(())
                        })
                        .await?;
                    self.action_result(&operation, "scroll", None, None).await
                }
                BrowserCommand::Type(input) => {
                    let (_, point) = self
                        .ready_element(required_target(&target)?, &mut references, true, &operation)
                        .await?;
                    self.click_at(point, MouseButton::Left, 1, 0, &operation)
                        .await?;
                    // InsertText supports arbitrary Unicode without leaving any pressed key.
                    operation.run(async { self.page.execute(chromiumoxide::cdp::browser_protocol::input::InsertTextParams::new(input.text.clone())).await?; Ok(()) }).await?;
                    self.action_result(&operation, "type", None, None).await
                }
                BrowserCommand::Key(input) => {
                    let (element, _) = self
                        .ready_element(
                            required_target(&target)?,
                            &mut references,
                            false,
                            &operation,
                        )
                        .await?;
                    let definition = chromiumoxide::keys::get_key_definition(&input.key)
                        .ok_or_else(|| {
                            BrowserError::Configuration("unknown keyboard key".into())
                        })?;
                    let result = operation
                        .run(async {
                            element.press_key(&input.key).await?;
                            Ok(())
                        })
                        .await;
                    let cleanup = if result.is_err() && !self.ended.is_cancelled() {
                        let release = DispatchKeyEventParams::builder()
                            .r#type(DispatchKeyEventType::KeyUp)
                            .key(definition.key)
                            .code(definition.code)
                            .windows_virtual_key_code(definition.key_code)
                            .native_virtual_key_code(definition.key_code)
                            .build()
                            .map_err(BrowserError::Configuration)?;
                        tokio::time::timeout(CONTROL_TIMEOUT, self.page.execute(release))
                            .await
                            .map_err(|_| BrowserError::Timeout)
                            .and_then(|result| result.map(|_| ()).map_err(BrowserError::from))
                    } else {
                        Ok(())
                    };
                    after_cleanup(result, cleanup)?;
                    self.action_result(
                        &operation,
                        "key",
                        input.wait_url.as_deref(),
                        navigation_before.as_ref(),
                    )
                    .await
                }
                BrowserCommand::Check(input) => {
                    let (element, point) = self
                        .ready_element(
                            required_target(&target)?,
                            &mut references,
                            false,
                            &operation,
                        )
                        .await?;
                    let checked = operation
                        .run(async { Ok(element.property("checked").await?) })
                        .await?
                        .and_then(|value| value.as_bool())
                        .ok_or_else(|| {
                            BrowserError::Configuration(
                                "check target is not a checkbox or radio".into(),
                            )
                        })?;
                    if checked != input.value {
                        self.click_at(point, MouseButton::Left, 1, 0, &operation)
                            .await?;
                    }
                    self.action_result(&operation, "check", None, None).await
                }
                BrowserCommand::Select(input) => {
                    let (element, _) = self
                        .ready_element(
                            required_target(&target)?,
                            &mut references,
                            false,
                            &operation,
                        )
                        .await?;
                    // Chromiumoxide has no select-options action. Scope the DOM operation
                    // to the element already admitted by the shared actionability check.
                    use chromiumoxide::cdp::js_protocol::runtime::{
                        CallArgument, CallFunctionOnParams,
                    };
                    let result = self
                        .page
                        .evaluate_function(
                            CallFunctionOnParams::builder()
                                .object_id(element.remote_object_id.clone())
                                .function_declaration(include_str!("select-options.js"))
                                .arguments(vec![
                                    CallArgument::builder().value(json!(input.value)).build(),
                                    CallArgument::builder()
                                        .value(json!(input.option_label))
                                        .build(),
                                    CallArgument::builder()
                                        .value(json!(input.option_index))
                                        .build(),
                                ])
                                .return_by_value(true)
                                .build()
                                .map_err(BrowserError::Configuration)?,
                        )
                        .await?
                        .into_value::<Vec<String>>()
                        .map_err(|error| BrowserError::InvalidResult(error.to_string()))?;
                    Ok(
                        json!({ "operation": "select", "result": result, "url": self.page.url().await?.unwrap_or_default() }),
                    )
                }
                BrowserCommand::DialogInspect(_) => {
                    let dialog = self.state.dialog.borrow().clone();
                    Ok(json!({ "dialog": dialog.as_ref().map(|dialog| json!({
                        "type": dialog.r#type, "message": dialog.message,
                        "defaultPrompt": dialog.default_prompt.as_deref().unwrap_or_default(),
                    })) }))
                }
                BrowserCommand::DialogAccept(_) | BrowserCommand::DialogDismiss(_) => {
                    let dialog = self
                        .state
                        .dialog
                        .borrow()
                        .clone()
                        .ok_or(BrowserError::DialogNotFound)?;
                    if let BrowserCommand::DialogAccept(input) = command {
                        if dialog.r#type == chromiumoxide::cdp::browser_protocol::page::DialogType::Alert ||
                            (input.text.is_some() && dialog.r#type != chromiumoxide::cdp::browser_protocol::page::DialogType::Prompt) {
                            return Err(BrowserError::InvalidDialogAction);
                        }
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
                    let mut release = self.state.deferred_release.lock().await;
                    if self.state.dialog.borrow().is_none() {
                        if let Some(event) = release.as_ref() {
                            self.page.execute(event.clone()).await?;
                            *release = None;
                        }
                    }
                    Ok(json!({ "handled": true }))
                }
                BrowserCommand::Wait(input) => {
                    if let Some(url) = &input.url {
                        self.wait_url(url, None).await?;
                    } else {
                        loop {
                            let observation =
                                Observation::capture(&self.page, &mut references).await?;
                            let elements = observation
                                .resolve(&self.page, required_target(&target)?, &references)
                                .await?;
                            let state = input.state.as_deref().unwrap_or("visible");
                            let mut matched = !elements.is_empty();
                            if matches!(state, "visible" | "hidden" | "enabled") && matched {
                                matched = false;
                                for element in elements {
                                    matched |= if state == "enabled" {
                                        element.property("disabled").await? != Some(json!(true))
                                    } else {
                                        element.bounding_box().await?.width > 0.0
                                    };
                                }
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
                        json!({ "matched": true, "url": self.page.url().await?.unwrap_or_default() }),
                    )
                }
                BrowserCommand::ViewportSet(input) => {
                    self.page
                        .execute(SetDeviceMetricsOverrideParams::new(
                            input.width as i64,
                            input.height as i64,
                            1.0,
                            false,
                        ))
                        .await?;
                    Ok(json!({ "width": input.width, "height": input.height }))
                }
                BrowserCommand::ViewportReset(_) => {
                    self.page
                        .execute(SetDeviceMetricsOverrideParams::new(1280, 720, 1.0, false))
                        .await?;
                    Ok(json!({ "width": 1280, "height": 720 }))
                }
                BrowserCommand::ContentRead(input) => {
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
                    result["content"] = json!(&content[..end]);
                    result["truncated"] = json!(truncated);
                    Ok(result)
                }
                BrowserCommand::Capabilities(_) => Ok(json!({ "capabilities": [
                    { "id": "accessibility", "available": true },
                    { "id": "read-only-eval", "available": true },
                    { "id": "cross-origin-frames", "available": false, "reason": "Not yet accepted on all browser platforms" }
                ] })),
                BrowserCommand::Open(_)
                | BrowserCommand::Tabs(_)
                | BrowserCommand::Close(_)
                | BrowserCommand::Screenshot(_) => {
                    unreachable!("resource-level operation is dispatched before tab actions")
                }
            }
        };
        if matches!(
            command,
            BrowserCommand::Fill(_)
                | BrowserCommand::Click(_)
                | BrowserCommand::Move(_)
                | BrowserCommand::Scroll(_)
                | BrowserCommand::Type(_)
                | BrowserCommand::Key(_)
                | BrowserCommand::Check(_)
        ) {
            // Input branches own their cancellation cleanup and must not be dropped by
            // an enclosing cancellation race while releasing a key or mouse button.
            work.await
        } else {
            operation.run(work).await
        }
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

    async fn action_result(
        &self,
        deadline: &Operation<'_>,
        operation: &str,
        wait_url: Option<&str>,
        previous: Option<&Frame>,
    ) -> Result<Value> {
        deadline.run(async {
        if let Some(pattern) = wait_url {
            self.wait_url(pattern, previous).await?;
        }
        Ok(json!({ "operation": operation, "result": "completed", "url": self.page.url().await?.unwrap_or_default() }))
        }).await
    }

    async fn wait_url(&self, pattern: &str, previous: Option<&Frame>) -> Result<()> {
        let matcher = globset::Glob::new(pattern)
            .map_err(|error| BrowserError::Configuration(error.to_string()))?
            .compile_matcher();
        loop {
            let frame = self
                .page
                .execute(GetFrameTreeParams {})
                .await?
                .result
                .frame_tree
                .frame;
            let url = format!(
                "{}{}",
                frame.url,
                frame.url_fragment.as_deref().unwrap_or_default()
            );
            let changed = previous.is_none_or(|previous| {
                previous.loader_id != frame.loader_id
                    || previous.url != frame.url
                    || previous.url_fragment != frame.url_fragment
            });
            if changed && matcher.is_match(url) {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
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
