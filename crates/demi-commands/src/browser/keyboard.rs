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
        event.native_virtual_key_code = Some(self.virtual_code);
        event.modifiers = Some(modifiers);
        if down && modifiers & !8 == 0 {
            event.text = shifted
                .map(|_| key.to_owned())
                .or_else(|| self.text.clone());
            event.unmodified_text = self.text.clone();
        }
        if down && cfg!(target_os = "macos") && modifiers == 4 && self.key.eq_ignore_ascii_case("a")
        {
            event.commands = Some(vec!["selectAll".into()]);
        }
        event
    }
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
        let mut delivered = 0;
        let result = async {
            for character in text.chars() {
                self.typing_focus(target, operation).await?;
                self.press(&[Key::character(character)], operation).await?;
                delivered += 1;
            }
            self.typing_focus(target, operation).await?;
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
