//! Focused keyboard delivery using Chromiumoxide's key definitions and CDP input.

use super::{
    BrowserError, BrowserTab, Result, element,
    operation::{CONTROL_TIMEOUT, Operation, after_cleanup},
};
use chromiumoxide::{
    cdp::browser_protocol::input::{DispatchKeyEventParams, DispatchKeyEventType},
    keys::get_key_definition,
};
use serde_json::json;

pub(super) struct Key {
    key: String,
    code: String,
    virtual_code: i64,
    text: Option<String>,
    modifier: i64,
}

/// Interpret the browser CLI's modifier names on the Host executing the command.
pub(super) fn modifier(name: &str) -> Option<i64> {
    match name {
        "Alt" => Some(1),
        "Control" => Some(2),
        "Meta" => Some(4),
        "Shift" => Some(8),
        "ControlOrMeta" => Some(if cfg!(target_os = "macos") { 4 } else { 2 }),
        _ => None,
    }
}

/// Combine schema-validated browser pointer modifiers using the Host's key mapping.
pub(super) fn modifiers(names: Option<&[String]>) -> i64 {
    names.into_iter().flatten().fold(0, |mask, name| {
        mask | modifier(name).expect("modifier is schema validated")
    })
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

/// A key from the live view, as the viewer's own browser reported it
/// (`browser-live-view.md` § Input).
pub(super) struct ViewerKey<'a> {
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
pub(super) fn viewer_key(key: &ViewerKey<'_>, mac_viewer: bool) -> DispatchKeyEventParams {
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
pub(super) fn click_modifiers(modifiers: i64, mac_viewer: bool) -> i64 {
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
pub(super) fn paste_shortcut() -> [DispatchKeyEventParams; 2] {
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
pub(super) fn combination(input: &str) -> Result<Vec<Key>> {
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

impl BrowserTab {
    pub(super) async fn focus(
        &self,
        target: &element::TargetElement,
        operation: &Operation<'_>,
    ) -> Result<()> {
        let focused: bool = operation
            .run(element::call(
                &self.page,
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
    pub(super) async fn press(&self, keys: &[Key], operation: &Operation<'_>) -> Result<()> {
        let mut pressed = Vec::new();
        let mut modifiers = 0;
        let mut result = async {
            for key in keys {
                modifiers |= key.modifier;
                self.input(operation, async {
                    pressed.push(key);
                    operation.begin_input();
                    self.page
                        .execute(key.event(DispatchKeyEventType::KeyDown, modifiers))
                        .await?;
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
            if self.state.dialog.borrow().is_some() {
                self.state
                    .deferred_release
                    .lock()
                    .await
                    .push(super::tab::InputRelease::Key(release));
                if result.is_ok() {
                    result = Err(BrowserError::DialogBlocked);
                }
                continue;
            }
            let released = tokio::time::timeout(CONTROL_TIMEOUT, self.page.execute(release))
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

    /// Capture the current browser focus without clicking or changing its selection.
    pub(super) async fn current_focus(
        &self,
        references: &mut super::observation::References,
        operation: &Operation<'_>,
    ) -> Result<(element::TargetElement, String)> {
        use chromiumoxide::cdp::{
            browser_protocol::dom::DescribeNodeParams, js_protocol::runtime::EvaluateParams,
        };
        operation
            .run(async {
                let mut page = self.page.clone();
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
                            .related_page(
                                chromiumoxide::cdp::browser_protocol::target::TargetId::new(
                                    frame.as_ref(),
                                ),
                            )
                            .await?
                        {
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
                let name = if matches!(node.local_name.as_str(), "body" | "html") {
                    "document".to_owned()
                } else {
                    let observation =
                        super::observation::Observation::capture(&self.page, references).await?;
                    if !observation.accessible(&target) {
                        return Ok((target, "document".into()));
                    }
                    observation
                        .describe_elements(std::slice::from_ref(&target), references, 1)?
                        .into_iter()
                        .next()
                        .and_then(|node| node.r#ref)
                        .ok_or(BrowserError::StaleReference)?
                };
                Ok((target, name))
            })
            .await
    }

    /// A retained remote object binds typing to the original element and document.
    async fn typing_focus(
        &self,
        target: &element::TargetElement,
        operation: &Operation<'_>,
    ) -> Result<()> {
        let result: Result<bool> = operation
            .run(element::call(
                &self.page,
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

    pub(super) async fn type_text(
        &self,
        target: &element::TargetElement,
        text: &str,
        operation: &Operation<'_>,
    ) -> Result<()> {
        self.focus(target, operation).await?;
        self.type_focused(Some(target), text, operation).await
    }

    pub(super) async fn type_focused(
        &self,
        target: Option<&element::TargetElement>,
        text: &str,
        operation: &Operation<'_>,
    ) -> Result<()> {
        let mut delivered = 0;
        let result = async {
            for character in text.chars() {
                if let Some(target) = target {
                    self.typing_focus(target, operation).await?;
                }
                self.press(&[Key::character(character)], operation).await?;
                delivered += 1;
            }
            if let Some(target) = target {
                self.typing_focus(target, operation).await?;
            }
            Ok(())
        }
        .await;
        result.map_err(|error: BrowserError| {
            let mut details = error.details();
            details["delivered"] = json!(delivered);
            BrowserError::Action {
                source: Box::new(error),
                details,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn down<'a>(code: &'a str, key: &'a str, key_code: u8) -> ViewerKey<'a> {
        ViewerKey {
            down: true,
            key,
            code,
            key_code,
            modifiers: 0,
            repeat: false,
            location: 0,
            text: None,
            alt_graph: false,
        }
    }

    fn fields(event: &DispatchKeyEventParams) -> (DispatchKeyEventType, &str, &str, i64, i64) {
        (
            event.r#type.clone(),
            event.key.as_deref().unwrap(),
            event.code.as_deref().unwrap(),
            event.windows_virtual_key_code.unwrap(),
            event.modifiers.unwrap(),
        )
    }

    #[test]
    fn printable_keys_carry_their_text_so_the_page_gets_keypress() {
        let event = viewer_key_on(
            &ViewerKey {
                text: Some("a"),
                ..down("KeyA", "a", 65)
            },
            false,
            false,
        );
        assert_eq!(
            fields(&event),
            (DispatchKeyEventType::KeyDown, "a", "KeyA", 65, 0)
        );
        assert_eq!(event.text.as_deref(), Some("a"));
        assert_eq!(event.unmodified_text.as_deref(), Some("a"));
        assert_eq!(event.is_keypad, Some(false));
    }

    #[test]
    fn enter_types_a_carriage_return() {
        assert_eq!(
            viewer_key_on(&down("Enter", "Enter", 13), false, false)
                .text
                .as_deref(),
            Some("\r")
        );
        let numpad = viewer_key_on(
            &ViewerKey {
                location: 3,
                ..down("NumpadEnter", "Enter", 13)
            },
            false,
            false,
        );
        assert_eq!(numpad.text.as_deref(), Some("\r"));
        assert_eq!(numpad.is_keypad, Some(true));
    }

    #[test]
    fn keys_without_a_character_stay_raw() {
        for (code, key, key_code) in [
            ("Tab", "Tab", 9),
            ("Backspace", "Backspace", 8),
            ("ArrowLeft", "ArrowLeft", 37),
            ("Escape", "Escape", 27),
        ] {
            let event = viewer_key_on(&down(code, key, key_code), false, false);
            assert_eq!(event.r#type, DispatchKeyEventType::RawKeyDown);
            assert_eq!(event.text, None);
        }
    }

    #[test]
    fn a_mac_viewer_uses_control_shortcuts_on_a_linux_host() {
        let select_all = viewer_key_on(
            &ViewerKey {
                modifiers: META,
                ..down("KeyA", "a", 65)
            },
            true,
            false,
        );
        assert_eq!(select_all.modifiers, Some(CONTROL));
        assert_eq!(select_all.text, None);
        let redo = viewer_key_on(
            &ViewerKey {
                modifiers: META | SHIFT,
                ..down("KeyZ", "z", 90)
            },
            true,
            false,
        );
        assert_eq!(redo.modifiers, Some(CONTROL | SHIFT));
        let command = viewer_key_on(
            &ViewerKey {
                modifiers: META,
                ..down("MetaLeft", "Meta", 91)
            },
            true,
            false,
        );
        assert_eq!(
            fields(&command),
            (
                DispatchKeyEventType::RawKeyDown,
                "Control",
                "ControlLeft",
                17,
                CONTROL
            )
        );
        let released = viewer_key_on(
            &ViewerKey {
                down: false,
                ..down("MetaLeft", "Meta", 91)
            },
            true,
            false,
        );
        assert_eq!(
            fields(&released),
            (DispatchKeyEventType::KeyUp, "Control", "ControlLeft", 17, 0)
        );
    }

    #[test]
    fn mac_text_navigation_maps_to_the_linux_keys() {
        let key = |code, name, key_code, modifiers| {
            viewer_key_on(
                &ViewerKey {
                    modifiers,
                    ..down(code, name, key_code)
                },
                true,
                false,
            )
        };
        let line_start = key("ArrowLeft", "ArrowLeft", 37, META | SHIFT);
        assert_eq!(
            (line_start.key.as_deref(), line_start.modifiers),
            (Some("Home"), Some(SHIFT))
        );
        let document_end = key("ArrowDown", "ArrowDown", 40, META);
        assert_eq!(
            (document_end.key.as_deref(), document_end.modifiers),
            (Some("End"), Some(CONTROL))
        );
        let word = key("ArrowRight", "ArrowRight", 39, ALT);
        assert_eq!(
            (word.key.as_deref(), word.modifiers),
            (Some("ArrowRight"), Some(CONTROL))
        );
        assert_eq!(
            key("Backspace", "Backspace", 8, ALT).modifiers,
            Some(CONTROL)
        );
        let delete_line = key("Backspace", "Backspace", 8, META);
        assert_eq!(
            (delete_line.modifiers, delete_line.commands),
            (Some(0), Some(vec!["DeleteToBeginningOfLine".into()]))
        );
    }

    #[test]
    fn option_and_altgr_characters_are_still_typed() {
        let option = viewer_key_on(
            &ViewerKey {
                modifiers: ALT,
                text: Some("™"),
                ..down("Digit2", "™", 50)
            },
            true,
            false,
        );
        assert_eq!(
            (option.r#type, option.text.as_deref(), option.modifiers),
            (DispatchKeyEventType::KeyDown, Some("™"), Some(ALT))
        );
        let alt_graph = viewer_key_on(
            &ViewerKey {
                modifiers: ALT | CONTROL,
                text: Some("@"),
                alt_graph: true,
                ..down("KeyQ", "@", 81)
            },
            false,
            false,
        );
        assert_eq!(alt_graph.text.as_deref(), Some("@"));
    }

    #[test]
    fn other_viewers_keep_their_modifiers() {
        for modifiers in [CONTROL, META] {
            let event = viewer_key_on(
                &ViewerKey {
                    modifiers,
                    ..down("KeyA", "a", 65)
                },
                false,
                false,
            );
            assert_eq!(event.modifiers, Some(modifiers));
        }
    }

    #[test]
    fn command_click_becomes_control_click_on_a_linux_host() {
        assert_eq!(
            click_modifiers_on(META | SHIFT, true, false),
            CONTROL | SHIFT
        );
        assert_eq!(click_modifiers_on(META, true, true), META);
        assert_eq!(click_modifiers_on(META, false, false), META);
    }

    #[test]
    fn a_macos_host_receives_editing_commands_for_menu_shortcuts() {
        let key = |code, name, key_code, modifiers| {
            viewer_key_on(
                &ViewerKey {
                    modifiers,
                    ..down(code, name, key_code)
                },
                true,
                true,
            )
        };
        let select_all = key("KeyA", "a", 65, META);
        assert_eq!(select_all.commands, Some(vec!["SelectAll".into()]));
        assert_eq!(select_all.modifiers, Some(META));
        assert_eq!(
            key("KeyZ", "z", 90, META | SHIFT).commands,
            Some(vec!["Redo".into()])
        );
        assert_eq!(key("KeyA", "a", 65, META | SHIFT).commands, None);
        assert_eq!(
            paste_shortcut_on(true)[0].commands,
            Some(vec!["Paste".into()])
        );
        assert_eq!(paste_shortcut_on(false)[0].modifiers, Some(CONTROL));
        assert_eq!(paste_shortcut_on(false)[0].commands, None);
    }
}
