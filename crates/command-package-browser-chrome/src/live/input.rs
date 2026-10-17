//! A viewer's input to the tab it watches (`live-view.md` § Input): the live
//! view's input task (`demi_live_view::input`) orders it beside the agent's
//! commands, and this turns it into CDP events. Keys and buttons held down
//! are this viewer's own: its end releases only them.

use std::collections::HashMap;

use chromiumoxide::cdp::browser_protocol::input::{
    DispatchKeyEventParams, DispatchKeyEventType, DispatchMouseEventParams, DispatchMouseEventType,
    DispatchTouchEventParams, DispatchTouchEventType, ImeSetCompositionParams, InsertTextParams,
    MouseButton, TouchPoint,
};

use crate::driver::operation::{BrowserError, CONTROL_TIMEOUT, Result};
use crate::page::keyboard;
use crate::tabs::{dialog::InputRelease, tab::BrowserTab};
use demi_command_package_browser_protocol::live::{LiveModuleMessage, LiveViewerMessage};
use demi_live_view::input::{self, Deliver};
use demi_live_view_protocol::{KeyAction, PointerAction, PointerButton, PointerKind, ViewerInput};

use crate::live::protocol::{TabId, ViewportMode};
use crate::live::{observers, writer::Writer};

pub(crate) type Input = input::Input<BrowserInput>;

/// Delivers one viewer's input to the tab it watches.
pub(crate) struct BrowserInput {
    writer: Writer,
    mac: bool,
}

impl BrowserInput {
    /// Input of a viewer on a Mac when `mac`, whose failures `writer` tells
    /// the viewer.
    pub fn new(writer: Writer, mac: bool) -> Self {
        Self { writer, mac }
    }
}

impl Deliver for BrowserInput {
    type Message = LiveViewerMessage;
    type Target = BrowserTab;
    type Held = Held;
    /// A tab takes a mouse's input, which a tab in Mobile mode receives as
    /// touches.
    const POINTER: PointerKind = PointerKind::Mouse;

    fn merged(
        message: &LiveViewerMessage,
        next: LiveViewerMessage,
    ) -> std::result::Result<LiveViewerMessage, LiveViewerMessage> {
        match (message, next) {
            (
                LiveViewerMessage::Input { tab, input },
                LiveViewerMessage::Input {
                    tab: next_tab,
                    input: next,
                },
            ) if *tab == next_tab => input::merged(input, next)
                .map(|input| LiveViewerMessage::Input { tab: next_tab.clone(), input })
                .map_err(|input| LiveViewerMessage::Input { tab: next_tab, input }),
            (_, next) => Err(next),
        }
    }

    async fn deliver(&self, tab: &BrowserTab, held: &mut Held, message: LiveViewerMessage) {
        if let Err(error) = deliver(tab, held, message, self.mac, &self.writer).await
            && !tab.ended().is_cancelled()
        {
            self.writer.notice("input_failed", &error.to_string()).await;
        }
    }

    async fn release(&self, tab: Option<&BrowserTab>, held: Held) {
        release(tab, held).await;
    }
}

/// What this viewer holds down on one tab.
#[derive(Default)]
pub(crate) struct Held {
    keys: HashMap<String, DispatchKeyEventParams>,
    buttons: Vec<MouseButton>,
    point: (f64, f64),
    touching: bool,
    composing: bool,
}

/// The tab named in an input message, which must be the watched one.
fn target(message: &LiveViewerMessage) -> Option<&TabId> {
    match message {
        LiveViewerMessage::Input { tab, .. }
        | LiveViewerMessage::Paste { tab, .. }
        | LiveViewerMessage::Choice { tab, .. } => Some(tab),
        _ => None,
    }
}

fn button(button: PointerButton) -> MouseButton {
    match button {
        PointerButton::Left => MouseButton::Left,
        PointerButton::Middle => MouseButton::Middle,
        PointerButton::Right => MouseButton::Right,
        PointerButton::None => MouseButton::None,
    }
}

/// Runs one delivery, unless a dialog opens first: the page takes no input
/// until someone answers it.
async fn dispatch<T, E>(
    tab: &BrowserTab,
    delivery: impl Future<Output = std::result::Result<T, E>>,
) -> Result<()>
where
    BrowserError: From<E>,
{
    let mut dialog = tab.state().dialog.watch();
    tokio::select! {
        result = delivery => result.map(|_| ()).map_err(BrowserError::from),
        _ = dialog.wait_for(|dialog| dialog.is_some()) => Ok(()),
    }
}

async fn deliver(
    tab: &BrowserTab,
    held: &mut Held,
    message: LiveViewerMessage,
    mac: bool,
    writer: &Writer,
) -> Result<()> {
    if target(&message) != Some(tab.id()) || tab.state().dialog.is_open() {
        return Ok(());
    }
    let page = tab.page();
    match message {
        LiveViewerMessage::Input { input: ViewerInput::Pointer {
            action,
            x,
            y,
            button: name,
            buttons,
            click_count,
            modifiers,
            ..
        }, .. } => {
            held.point = (x, y);
            if tab.viewport().mode == ViewportMode::Mobile {
                // A phone has touches, not a mouse: no hover, one finger.
                let kind = match action {
                    PointerAction::Down if name == PointerButton::Left => {
                        DispatchTouchEventType::TouchStart
                    }
                    PointerAction::Move if held.touching => DispatchTouchEventType::TouchMove,
                    PointerAction::Up if held.touching => DispatchTouchEventType::TouchEnd,
                    _ => return Ok(()),
                };
                held.touching = kind != DispatchTouchEventType::TouchEnd;
                let points = if held.touching {
                    vec![TouchPoint::new(x, y)]
                } else {
                    Vec::new()
                };
                return dispatch(
                    tab,
                    page.execute(DispatchTouchEventParams::new(kind, points)),
                )
                .await;
            }
            let kind = match action {
                PointerAction::Down => DispatchMouseEventType::MousePressed,
                PointerAction::Up => DispatchMouseEventType::MouseReleased,
                PointerAction::Move => DispatchMouseEventType::MouseMoved,
            };
            let pressed = button(name);
            match kind {
                DispatchMouseEventType::MousePressed if !held.buttons.contains(&pressed) => {
                    held.buttons.push(pressed.clone());
                }
                DispatchMouseEventType::MouseReleased => {
                    held.buttons.retain(|held| held != &pressed)
                }
                _ => {}
            }
            let event = DispatchMouseEventParams::builder()
                .r#type(kind)
                .x(x)
                .y(y)
                .button(pressed)
                .buttons(i64::from(buttons))
                .click_count(i64::from(click_count))
                .modifiers(keyboard::click_modifiers(i64::from(modifiers), mac))
                .build()
                .map_err(BrowserError::Configuration)?;
            dispatch(tab, page.execute(event)).await
        }
        LiveViewerMessage::Input { input: ViewerInput::Wheel {
            x,
            y,
            delta_x,
            delta_y,
            modifiers,
            ..
        }, .. } => {
            // With the screen at the viewer's ratio, Chrome scrolls a delta
            // by that many CSS pixels at every ratio.
            let event = DispatchMouseEventParams::builder()
                .r#type(DispatchMouseEventType::MouseWheel)
                .x(x)
                .y(y)
                .delta_x(delta_x)
                .delta_y(delta_y)
                .modifiers(i64::from(modifiers))
                .build()
                .map_err(BrowserError::Configuration)?;
            dispatch(tab, page.execute(event)).await
        }
        LiveViewerMessage::Input { input: ViewerInput::Key {
            action,
            key,
            code,
            key_code,
            modifiers,
            repeat,
            location,
            text,
            alt_graph,
            ..
        }, .. } => {
            let down = action == KeyAction::Down;
            let mut event = keyboard::viewer_key(
                &keyboard::ViewerKey {
                    down,
                    key: &key,
                    code: &code,
                    key_code,
                    modifiers: i64::from(modifiers),
                    repeat,
                    location,
                    text: text.as_deref(),
                    alt_graph,
                },
                mac,
            );
            if down {
                held.keys.insert(code, event.clone());
            } else if let Some(pressed) = held.keys.remove(&code) {
                // Release the key that went down, even if its mapping
                // changed with the modifiers since.
                event.key = pressed.key;
                event.code = pressed.code;
                event.windows_virtual_key_code = pressed.windows_virtual_key_code;
                event.location = pressed.location;
                event.is_keypad = pressed.is_keypad;
            }
            dispatch(tab, page.execute(event)).await
        }
        LiveViewerMessage::Input { input: ViewerInput::Text { text }, .. } => {
            held.composing = false;
            dispatch(tab, page.execute(InsertTextParams::new(text))).await
        }
        LiveViewerMessage::Input { input: ViewerInput::Composition { text }, .. } => {
            if text.is_empty() && !held.composing {
                return Ok(());
            }
            held.composing = !text.is_empty();
            let end = text.encode_utf16().count() as i64;
            dispatch(
                tab,
                page.execute(ImeSetCompositionParams::new(text, end, end)),
            )
            .await
        }
        LiveViewerMessage::Paste { text, html, .. } => {
            held.composing = false;
            dispatch(tab, paste(tab, &text, &html)).await
        }
        LiveViewerMessage::Choice {
            token,
            revision,
            value,
            indices,
            ..
        } => {
            let accepted = observers::choose(tab, &token, revision, &value, &indices).await?;
            writer
                .control(&LiveModuleMessage::Choice { token, accepted })
                .await;
            Ok(())
        }
        _ => Ok(()),
    }
}

/// The viewer's clipboard goes to the browser's own clipboard, then the
/// Host's paste shortcut, so the page receives a real paste event. Without an
/// isolated browser clipboard the page receives inserted text.
async fn paste(tab: &BrowserTab, text: &str, html: &str) -> Result<()> {
    let written = observers::write_clipboard(tab, text, html).await?;
    if !written {
        tab.page().execute(InsertTextParams::new(text)).await?;
        return Ok(());
    }
    for event in keyboard::paste_shortcut() {
        tab.page().execute(event).await?;
    }
    Ok(())
}

/// Releases what this viewer holds on `tab`. A dialog holds the releases
/// back until someone answers it.
async fn release(tab: Option<&BrowserTab>, held: Held) {
    let Some(tab) = tab else { return };
    if tab.ended().is_cancelled() {
        return;
    }
    let mut releases = Vec::new();
    for (_, pressed) in held.keys {
        let mut event = DispatchKeyEventParams::new(DispatchKeyEventType::KeyUp);
        event.key = pressed.key;
        event.code = pressed.code;
        event.windows_virtual_key_code = pressed.windows_virtual_key_code;
        event.location = pressed.location;
        event.is_keypad = pressed.is_keypad;
        event.modifiers = Some(0);
        releases.push(InputRelease::Key(event));
    }
    for button in held.buttons {
        if let Ok(event) = DispatchMouseEventParams::builder()
            .r#type(DispatchMouseEventType::MouseReleased)
            .x(held.point.0)
            .y(held.point.1)
            .button(button)
            .buttons(0)
            .modifiers(0)
            .build()
        {
            releases.push(InputRelease::Mouse(event));
        }
    }
    if tab.state().dialog.is_open() {
        tab.state().dialog.defer(releases).await;
        return;
    }
    let delivered = tokio::time::timeout(CONTROL_TIMEOUT, async {
        for release in releases {
            match release {
                InputRelease::Key(event) => {
                    tab.page().execute(event).await?;
                }
                InputRelease::Mouse(event) => {
                    tab.page().execute(event).await?;
                }
            }
        }
        if held.touching {
            tab.page()
                .execute(DispatchTouchEventParams::new(
                    DispatchTouchEventType::TouchCancel,
                    Vec::new(),
                ))
                .await?;
        }
        if held.composing {
            tab.page()
                .execute(ImeSetCompositionParams::new("", 0, 0))
                .await?;
        }
        Ok::<_, BrowserError>(())
    })
    .await;
    if let Ok(Err(error)) = delivered
        && !tab.ended().is_cancelled()
    {
        tracing::warn!("live view input release on {}: {error}", tab.id());
    }
}
