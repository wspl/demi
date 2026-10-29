//! Focused keyboard delivery using Chromiumoxide's key definitions and CDP input.

use chromiumoxide::{
    cdp::browser_protocol::input::{
        DispatchKeyEventParams, DispatchKeyEventType, InsertTextParams,
    },
    keys::get_key_definition,
};
use demi_shared_recording_protocol::InputBy;
use serde_json::json;

use crate::driver::operation::{BrowserError, CONTROL_TIMEOUT, Operation, Result, after_cleanup};
use crate::tabs::{session::References, tab::BrowserTab};

use crate::page::{
    element,
    protocol::{BrowserTarget, Modifier, ResolvedElement},
};

pub(crate) struct Key {
    key: String,
    code: String,
    virtual_code: i64,
    text: Option<String>,
    modifier: i64,
}

/// A modifier's CDP bit on the Host executing the command.
fn modifier_bit(modifier: Modifier) -> i64 {
    match modifier {
        Modifier::Alt => 1,
        Modifier::Control => 2,
        Modifier::Meta => 4,
        Modifier::Shift => 8,
        Modifier::ControlOrMeta if cfg!(target_os = "macos") => 4,
        Modifier::ControlOrMeta => 2,
    }
}

/// Interpret a modifier's name in a key combination.
pub(crate) fn modifier(name: &str) -> Option<i64> {
    name.parse().ok().map(modifier_bit)
}

/// Combine schema-validated browser pointer modifiers using the Host's key mapping.
pub(crate) fn modifiers(modifiers: Option<&[Modifier]>) -> i64 {
    modifiers
        .into_iter()
        .flatten()
        .fold(0, |mask, modifier| mask | modifier_bit(*modifier))
}

impl Key {
    fn named(name: &str) -> Result<Self> {
        let name = match name {
            "ControlOrMeta" if cfg!(target_os = "macos") => "Meta",
            "ControlOrMeta" => "Control",
            "Space" => " ",
            _ => name,
        };
        let definition = get_key_definition(name)
            .or_else(|| {
                chromiumoxide::keys::USKEYBOARD_LAYOUT
                    .iter()
                    .find(|key| key.code == name)
            })
            .ok_or_else(|| BrowserError::Configuration(format!("unknown keyboard key: {name}")))?;
        Ok(Self {
            key: definition.key.into(),
            code: definition.code.into(),
            virtual_code: definition.key_code,
            text: definition.text.map(String::from).or_else(|| {
                (definition.key.chars().count() == 1).then(|| definition.key.to_owned())
            }),
            modifier: modifier(definition.key).unwrap_or(0),
        })
    }

    fn character(character: char) -> Self {
        let name = character.to_string();
        if let Ok(key) = Self::named(if character == '\n' { "Enter" } else { &name }) {
            return key;
        }
        // The vendored US layout has no definition for arbitrary Unicode.
        // CDP accepts its text with an unidentified physical key.
        Self {
            key: name.clone(),
            code: String::new(),
            virtual_code: 0,
            text: Some(name),
            modifier: 0,
        }
    }

    fn event(&self, kind: DispatchKeyEventType, modifiers: i64) -> DispatchKeyEventParams {
        let down = kind == DispatchKeyEventType::KeyDown;
        // The vendored table has no shift metadata. Its shifted printable
        // definitions follow the base definitions for the same physical code.
        let shifted = (modifiers & 8 != 0)
            .then(|| {
                chromiumoxide::keys::USKEYBOARD_LAYOUT
                    .iter()
                    .rev()
                    .find(|definition| {
                        definition.code == self.code && definition.key.chars().count() == 1
                    })
            })
            .flatten();
        let key = shifted.map_or(self.key.as_str(), |definition| definition.key);
        let mut event = DispatchKeyEventParams::new(kind);
        event.key = Some(key.into());
        event.code = Some(self.code.clone());
        event.windows_virtual_key_code = Some(self.virtual_code);
        // The vendored table contains Windows virtual codes, not Host-native
        // scan codes. Let Chrome derive its native code (on macOS 91 means "8",
        // not Meta, and supplying it can start unintended native key repeats).
        event.modifiers = Some(modifiers);
        if down && modifiers & !8 == 0 {
            event.text = shifted
                .map(|_| key.to_owned())
                .or_else(|| self.text.clone());
            event.unmodified_text = self.text.clone();
        }
        if down {
            event.commands = editing_commands(HOST_MAC, &self.code, modifiers);
        }
        event
    }
}

const ALT: i64 = 1;
const CONTROL: i64 = 2;
const META: i64 = 4;
const SHIFT: i64 = 8;
const HOST_MAC: bool = cfg!(target_os = "macos");

/// Editing shortcuts on a macOS Host also carry Blink editing commands: CDP
/// key events never reach the Cocoa menus that run them there.
fn editing_commands(host_mac: bool, code: &str, modifiers: i64) -> Option<Vec<String>> {
    if !host_mac || modifiers & (META | CONTROL | ALT) != META {
        return None;
    }
    let command = match (code, modifiers & SHIFT != 0) {
        ("KeyA", false) => "SelectAll",
        ("KeyC", false) => "Copy",
        ("KeyX", false) => "Cut",
        ("KeyV", false) => "Paste",
        ("KeyZ", false) => "Undo",
        ("KeyZ", true) => "Redo",
        _ => return None,
    };
    Some(vec![command.into()])
}

/// A key from the live view, as the user's browser reported it.
pub struct ViewerKey<'a> {
    pub down: bool,
    pub key: &'a str,
    pub code: &'a str,
    pub key_code: u8,
    pub modifiers: i64,
    pub repeat: bool,
    pub location: u8,
    pub text: Option<&'a str>,
    pub alt_graph: bool,
}

/// The key event the Host receives for a viewer's key.
pub fn viewer_key(key: &ViewerKey<'_>, mac_viewer: bool) -> DispatchKeyEventParams {
    viewer_key_on(key, mac_viewer, HOST_MAC)
}

fn viewer_key_on(key: &ViewerKey<'_>, mac_viewer: bool, host_mac: bool) -> DispatchKeyEventParams {
    let mut name = key.key;
    let mut code = key.code.to_owned();
    let mut key_code = i64::from(key.key_code);
    let mut modifiers = key.modifiers;
    let mut commands = None;
    // A Mac viewer's shortcuts, as a Linux or Windows page expects them.
    if mac_viewer && !host_mac {
        let shift = modifiers & SHIFT;
        let meta_key = matches!(key.code, "MetaLeft" | "MetaRight");
        if modifiers & META != 0 {
            match key.code {
                "ArrowLeft" | "ArrowUp" => {
                    (name, code, key_code) = ("Home", "Home".into(), 36);
                    modifiers = shift | if key.code == "ArrowUp" { CONTROL } else { 0 };
                }
                "ArrowRight" | "ArrowDown" => {
                    (name, code, key_code) = ("End", "End".into(), 35);
                    modifiers = shift | if key.code == "ArrowDown" { CONTROL } else { 0 };
                }
                "Backspace" | "Delete" => {
                    modifiers &= !(META | ALT);
                    commands = Some(vec![
                        if key.code == "Backspace" {
                            "DeleteToBeginningOfLine"
                        } else {
                            "DeleteToEndOfLine"
                        }
                        .into(),
                    ]);
                }
                _ => modifiers = (modifiers & !META) | CONTROL,
            }
        }
        if meta_key {
            (name, code, key_code) = ("Control", key.code.replace("Meta", "Control"), 17);
        } else if modifiers & ALT != 0
            && matches!(
                key.code,
                "ArrowLeft" | "ArrowRight" | "Backspace" | "Delete"
            )
        {
            modifiers = (modifiers & !ALT) | CONTROL;
        }
    }
    let kind = if !key.down {
        DispatchKeyEventType::KeyUp
    } else {
        DispatchKeyEventType::RawKeyDown
    };
    let mut event = DispatchKeyEventParams::new(kind);
    event.key = Some(name.into());
    event.code = Some(code.clone());
    event.windows_virtual_key_code = Some(key_code);
    event.modifiers = Some(modifiers);
    event.location = Some(i64::from(key.location));
    event.is_keypad = Some(key.location == 3);
    if !key.down {
        return event;
    }
    // Enter types a carriage return, so Blink runs its keypress defaults:
    // newlines, implicit submission and button activation.
    let enter = matches!(key.code, "Enter" | "NumpadEnter");
    let shortcut = modifiers & (CONTROL | META) != 0 && !key.alt_graph;
    let text = if enter {
        Some("\r")
    } else if shortcut {
        None
    } else {
        key.text
    };
    if let Some(text) = text {
        event.r#type = DispatchKeyEventType::KeyDown;
        event.text = Some(text.into());
        event.unmodified_text = Some(text.into());
    }
    event.auto_repeat = Some(key.repeat);
    event.commands = commands.or_else(|| editing_commands(host_mac, &code, modifiers));
    event
}

/// Command-click from a Mac viewer is Control-click on a Linux or Windows Host.
pub fn click_modifiers(modifiers: i64, mac_viewer: bool) -> i64 {
    click_modifiers_on(modifiers, mac_viewer, HOST_MAC)
}

fn click_modifiers_on(modifiers: i64, mac_viewer: bool, host_mac: bool) -> i64 {
    if mac_viewer && !host_mac && modifiers & META != 0 {
        (modifiers & !META) | CONTROL
    } else {
        modifiers
    }
}

/// The Host's paste shortcut, down then up.
pub fn paste_shortcut() -> [DispatchKeyEventParams; 2] {
    paste_shortcut_on(HOST_MAC)
}

fn paste_shortcut_on(host_mac: bool) -> [DispatchKeyEventParams; 2] {
    let modifiers = if host_mac { META } else { CONTROL };
    let event = |kind| {
        let mut event = DispatchKeyEventParams::new(kind);
        event.key = Some("v".into());
        event.code = Some("KeyV".into());
        event.windows_virtual_key_code = Some(86);
        event.modifiers = Some(modifiers);
        event
    };
    let mut down = event(DispatchKeyEventType::RawKeyDown);
    down.commands = editing_commands(host_mac, "KeyV", modifiers);
    [down, event(DispatchKeyEventType::KeyUp)]
}

/// Validate the complete browser key combination before focusing or dispatching input.
pub(crate) fn combination(input: &str) -> Result<Vec<Key>> {
    let mut remaining = input;
    let mut keys = Vec::new();
    while remaining != "+" {
        let Some((name, rest)) = remaining.split_once('+') else {
            break;
        };
        if modifier(name).is_none()
            || keys
                .iter()
                .any(|key: &Key| Some(key.modifier) == modifier(name))
        {
            return Err(BrowserError::Configuration(format!(
                "invalid keyboard modifier: {name}"
            )));
        }
        keys.push(Key::named(name)?);
        remaining = rest;
    }
    keys.push(Key::named(remaining)?);
    Ok(keys)
}

pub(crate) async fn focus(
    tab: &BrowserTab,
    target: &element::TargetElement,
    operation: &Operation<'_>,
) -> Result<()> {
    let focused: bool = operation
        .run(element::call(
            tab.page(),
            target,
            include_str!("focus.js"),
            vec![json!(true)],
        ))
        .await?;
    if !focused {
        return Err(BrowserError::NotActionable {
            condition: "focused".into(),
            interceptor: None,
        });
    }
    Ok(())
}

/// Release every attempted key in reverse order, even after cancellation or failure.
pub(crate) async fn press(tab: &BrowserTab, keys: &[Key], operation: &Operation<'_>) -> Result<()> {
    let mut pressed = Vec::new();
    let mut modifiers = 0;
    let mut result = async {
        for key in keys {
            modifiers |= key.modifier;
            tab.input(operation, async {
                pressed.push(key);
                operation.begin_input();
                let down = key.event(DispatchKeyEventType::KeyDown, modifiers);
                crate::live::inputs::key(tab, &down, InputBy::Agent);
                tab.page().execute(down).await?;
                Ok(())
            })
            .await?;
        }
        Ok(())
    }
    .await;
    let mut cleanup = Ok(());
    for key in pressed.into_iter().rev() {
        modifiers &= !key.modifier;
        let release = key.event(DispatchKeyEventType::KeyUp, modifiers);
        if tab.state().dialog.is_open() {
            tab.state()
                .dialog
                .defer(vec![crate::tabs::dialog::InputRelease::Key(release)])
                .await;
            if result.is_ok() {
                result = Err(BrowserError::DialogBlocked);
            }
            continue;
        }
        let released = tokio::time::timeout(CONTROL_TIMEOUT, tab.page().execute(release))
            .await
            .map_err(|_| BrowserError::Timeout)
            .and_then(|result| result.map(|_| ()).map_err(BrowserError::from));
        cleanup = after_cleanup(cleanup, released);
    }
    let result = after_cleanup(result, cleanup);
    if result.is_ok() {
        operation.complete_input();
    }
    result
}

/// Capture the current browser focus without clicking or changing its
/// selection, and what the page tree names it; none when the focus is the
/// document or an element the tree leaves out.
pub(crate) async fn current_focus(
    tab: &BrowserTab,
    references: &mut crate::tabs::session::References,
    operation: &Operation<'_>,
) -> Result<(element::TargetElement, Option<ResolvedElement>)> {
    use chromiumoxide::cdp::{
        browser_protocol::dom::DescribeNodeParams, js_protocol::runtime::EvaluateParams,
    };
    operation
        .run(async {
            let mut page = tab.page().clone();
            let mut context = None;
            let (node, object) = loop {
                let mut request = EvaluateParams::builder()
                    .expression(format!("({})()", include_str!("focused.js")))
                    .object_group(element::OBJECT_GROUP)
                    .return_by_value(false)
                    .build()
                    .map_err(BrowserError::Configuration)?;
                request.context_id = context;
                let result = page.evaluate_expression(request).await?;
                let object = result
                    .object()
                    .object_id
                    .clone()
                    .ok_or(BrowserError::StaleReference)?;
                let node = page
                    .execute(
                        DescribeNodeParams::builder()
                            .object_id(object.clone())
                            .build(),
                    )
                    .await?
                    .result
                    .node;
                if matches!(node.local_name.as_str(), "iframe" | "frame")
                    && let Some(frame) = &node.frame_id
                {
                    if let Some(related) = page
                        .related_page(chromiumoxide::cdp::browser_protocol::target::TargetId::new(
                            frame.as_ref(),
                        ))
                        .await?
                    {
                        references.hold(&related);
                        page = related;
                    }
                    context = Some(
                        page.frame_execution_context(frame.clone())
                            .await?
                            .ok_or(BrowserError::StaleReference)?,
                    );
                    continue;
                }
                break (node, object);
            };
            let target = element::TargetElement {
                page,
                frame_chain: Vec::new(),
                backend_node_id: node.backend_node_id,
                remote_object_id: object,
            };
            if matches!(node.local_name.as_str(), "body" | "html") {
                return Ok((target, None));
            }
            if context.is_none() {
                let named =
                    crate::page::locate::named_in_main(tab.page(), &target, references).await?;
                return Ok((target, named));
            }
            let observation =
                crate::page::observation::Observation::capture(tab.page(), references).await?;
            if !observation.accessible(&target) {
                return Ok((target, None));
            }
            let named = observation.named(&target, references)?;
            Ok((target, Some(named)))
        })
        .await
}

/// A retained remote object binds typing to the original element and document.
async fn typing_focus(
    tab: &BrowserTab,
    target: &element::TargetElement,
    operation: &Operation<'_>,
) -> Result<()> {
    let result: Result<bool> = operation
        .run(element::call(
            tab.page(),
            target,
            include_str!("focus.js"),
            vec![json!(false)],
        ))
        .await;
    match result {
        Ok(true) => Ok(()),
        Ok(false) => Err(BrowserError::NotActionable {
            condition: "typing target changed or lost focus".into(),
            interceptor: None,
        }),
        // CDP does not expose a typed stale execution-context/object error.
        Err(BrowserError::Cdp(chromiumoxide::error::CdpError::Chrome(error)))
            if error.message == "Cannot find context with specified id"
                || error.message == "Could not find object with given id" =>
        {
            Err(BrowserError::NotActionable {
                condition: "typing document changed".into(),
                interceptor: None,
            })
        }
        Err(error) => Err(error),
    }
}

pub(crate) async fn type_text(
    tab: &BrowserTab,
    target: &element::TargetElement,
    text: &str,
    operation: &Operation<'_>,
) -> Result<()> {
    focus(tab, target, operation).await?;
    type_focused(tab, Some(target), text, operation).await
}

pub(crate) async fn type_focused(
    tab: &BrowserTab,
    target: Option<&element::TargetElement>,
    text: &str,
    operation: &Operation<'_>,
) -> Result<()> {
    let mut delivered = 0;
    let result = async {
        for character in text.chars() {
            if let Some(target) = target {
                typing_focus(tab, target, operation).await?;
            }
            press(tab, &[Key::character(character)], operation).await?;
            delivered += 1;
        }
        if let Some(target) = target {
            typing_focus(tab, target, operation).await?;
        }
        Ok(())
    }
    .await;
    result.map_err(|error: BrowserError| {
        let mut details = error.details();
        details.delivered = Some(delivered);
        BrowserError::Action {
            source: Box::new(error),
            details: Box::new(details),
        }
    })
}

/// Fills the element `target` resolves to with `text`; what the page tree
/// names it.
pub(crate) async fn fill(
    tab: &BrowserTab,
    target: &BrowserTarget,
    references: &mut References,
    text: &str,
    operation: &Operation<'_>,
) -> Result<ResolvedElement> {
    let element::Ready {
        element: target,
        state,
        named,
    } = crate::page::element::ready(tab, target, references, element::FILL, operation).await?;
    let native = state.fill_kind == element::FillKind::Native;
    let mode: FillMode = operation
        .run(async {
            if native {
                crate::live::inputs::typed(tab, text, InputBy::Agent);
                operation.begin_input();
            }
            let mode = element::call(
                tab.page(),
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
            press(tab, &combination("Delete")?, operation).await?;
        } else {
            operation
                .run(async {
                    operation.begin_input();
                    crate::live::inputs::typed(tab, text, InputBy::Agent);
                    tab.page().execute(InsertTextParams::new(text)).await?;
                    operation.complete_input();
                    Ok(())
                })
                .await?;
        }
    }
    Ok(named)
}

#[derive(serde::Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum FillMode {
    Invalid,
    Native,
    Text,
}
