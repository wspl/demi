//! The inputs a tab takes while it is recorded, from the agent's commands and
//! from the user's hand in the live view alike: each is told where it is sent
//! to Chrome, so that a recording draws what the page was given, and the
//! agent's commands with how they ended.
//!
//! A place is in CSS pixels where Chrome takes it and in the recording's
//! pixels where it is kept: the capture copies the page's own pixels, at a
//! pixel ratio the page renders at within what the capture codes
//! (`tabs/viewport.rs`, `pixels` and `ratio_for`), so the device pixel ratio
//! as the input is sent is both the factor and the size of a point, a
//! viewport just set included, whose frames come at its new size.

use std::sync::{Arc, Mutex};

use chromiumoxide::cdp::browser_protocol::input::{
    DispatchKeyEventParams, DispatchKeyEventType, DispatchMouseEventParams, DispatchMouseEventType, MouseButton,
};
use demi_shared_recording::Sink;
use demi_shared_recording_protocol::{Input, InputBy, InputButton, InputPhase};
use tokio::time::Instant;

use crate::tabs::tab::BrowserTab;

/// The recordings of a tab that take its inputs: none while it records
/// nothing, which is when a report costs a lock and no more.
#[derive(Default)]
pub struct Inputs {
    sinks: Mutex<Vec<Sink>>,
    /// The agent's commands under way, which may run at once.
    commands: Mutex<Vec<Arc<Command>>>,
}

/// One of the agent's commands under way: the recordings that ran as it
/// began, which alone list it, and when it first gave the page an input,
/// the time they list it at. An input that comes while several run is the
/// first of each that has none yet.
pub struct Command {
    sinks: Vec<Sink>,
    first: Mutex<Option<Instant>>,
}

impl Inputs {
    /// `sink` takes the tab's inputs until its recording ends.
    pub fn add(&self, sink: Sink) {
        self.sinks.lock().expect("the tab's recordings are intact").push(sink);
    }

    /// The recordings running now.
    fn running(&self) -> Vec<Sink> {
        let mut sinks = self.sinks.lock().expect("the tab's recordings are intact");
        sinks.retain(|sink| !sink.ended().is_cancelled());
        sinks.clone()
    }

    /// Notes `at` as the first input of each command under way that has
    /// none yet.
    fn acted(&self, at: Instant) {
        for command in self.commands.lock().expect("the tab's commands are intact").iter() {
            command.first.lock().expect("the command's first input is intact").get_or_insert(at);
        }
    }

    fn report(&self, at: Instant, by: InputBy, input: impl FnOnce(Option<(u32, u32)>) -> Input) {
        if by == InputBy::Agent {
            self.acted(at);
        }
        let sinks = self.running();
        let Some(first) = sinks.first() else {
            return;
        };
        let input = input(first.frame_size());
        for sink in &sinks {
            sink.input(at.into_std(), input.clone());
        }
    }
}

/// The frame pixels of a CSS pixel.
fn scale(tab: &BrowserTab, _frame: Option<(u32, u32)>) -> f64 {
    tab.state().viewport.borrow().current.device_pixel_ratio
}

/// Reports a mouse event as Chrome is given it.
pub(crate) fn mouse(tab: &BrowserTab, event: &DispatchMouseEventParams, by: InputBy) {
    tab.state().inputs.report(Instant::now(), by, |frame| {
        let scale = scale(tab, frame);
        let (x, y) = (event.x * scale, event.y * scale);
        match event.r#type {
            DispatchMouseEventType::MouseWheel => Input::Wheel {
                x,
                y,
                scale,
                dx: event.delta_x.unwrap_or(0.0),
                dy: event.delta_y.unwrap_or(0.0),
                by,
            },
            ref kind => Input::Pointer {
                x,
                y,
                scale,
                phase: match kind {
                    DispatchMouseEventType::MousePressed => InputPhase::Down,
                    DispatchMouseEventType::MouseReleased => InputPhase::Up,
                    _ => InputPhase::Move,
                },
                button: match event.button {
                    Some(MouseButton::Left) => Some(InputButton::Left),
                    Some(MouseButton::Middle) => Some(InputButton::Middle),
                    Some(MouseButton::Right) => Some(InputButton::Right),
                    _ => None,
                },
                by,
            },
        }
    });
}

/// Reports a touch of the tab's page at `x`, `y` in CSS pixels.
pub(crate) fn touch(tab: &BrowserTab, x: f64, y: f64, phase: InputPhase, by: InputBy) {
    tab.state().inputs.report(Instant::now(), by, |frame| {
        let scale = scale(tab, frame);
        Input::Touch {
            x: x * scale,
            y: y * scale,
            scale,
            phase,
            by,
        }
    });
}

/// The modifiers of a key event, by CDP's bits: Alt 1, Control 2, Meta 4,
/// Shift 8, in the order a shortcut names them.
fn modifiers(bits: i64) -> Vec<String> {
    [(2, "Control"), (1, "Alt"), (8, "Shift"), (4, "Meta")]
        .into_iter()
        .filter(|(bit, _)| bits & bit != 0)
        .map(|(_, name)| name.to_owned())
        .collect()
}

/// Reports a key's press as Chrome is given it: a named key, or one with
/// Control or Meta, as the keys; one that types a character, Shift, AltGr
/// and Option included, as typed, its character never kept. A release, and
/// a modifier pressed alone, report nothing.
pub(crate) fn key(tab: &BrowserTab, event: &DispatchKeyEventParams, by: InputBy) {
    if !matches!(event.r#type, DispatchKeyEventType::KeyDown | DispatchKeyEventType::RawKeyDown) {
        return;
    }
    let Some(key) = event.key.as_deref().filter(|key| !key.is_empty()) else {
        return;
    };
    if matches!(key, "Shift" | "Control" | "Alt" | "Meta" | "AltGraph") {
        return;
    }
    let bits = event.modifiers.unwrap_or(0);
    let named = key.chars().count() > 1;
    // A key that puts a character in the page types it, whatever the
    // modifiers: AltGr is Control and Alt on Windows.
    let types = event.text.as_deref().is_some_and(|text| text.chars().any(|character| !character.is_control()));
    tab.state().inputs.report(Instant::now(), by, |frame| {
        let scale = scale(tab, frame);
        if named || !types {
            let mut keys = modifiers(bits);
            keys.push(if named { key.to_owned() } else { key.to_uppercase() });
            Input::Keys { keys, scale, by }
        } else {
            Input::Typed { characters: 1, scale, by }
        }
    });
}

/// Reports `text` typed, inserted or filled at once.
pub(crate) fn typed(tab: &BrowserTab, text: &str, by: InputBy) {
    let characters = u32::try_from(text.chars().count()).unwrap_or(u32::MAX);
    if characters > 0 {
        tab.state().inputs.report(Instant::now(), by, |frame| Input::Typed {
            characters,
            scale: scale(tab, frame),
            by,
        });
    }
}

/// Notes that the agent's command gave the page an input that no other
/// report names, as `select` sets an element's choice itself.
pub(crate) fn acted(tab: &BrowserTab) {
    tab.state().inputs.acted(Instant::now());
}

/// Starts one of the agent's commands: the recordings running now list it,
/// at the next input it gives.
pub(crate) fn command_begins(tab: &BrowserTab) -> Arc<Command> {
    let command = Arc::new(Command {
        sinks: tab.state().inputs.running(),
        first: Mutex::new(None),
    });
    tab.state().inputs.commands.lock().expect("the tab's commands are intact").push(command.clone());
    command
}

/// Hands the agent's command to the recordings that ran as it began, at the
/// first input it gave the page, or at `began` for one that gave none, with
/// why it failed when it did.
pub(crate) fn command(tab: &BrowserTab, command: &Arc<Command>, began: Instant, text: impl FnOnce() -> String, failed: Option<String>) {
    tab.state().inputs.commands.lock().expect("the tab's commands are intact").retain(|other| !Arc::ptr_eq(other, command));
    let at = command.first.lock().expect("the command's first input is intact").unwrap_or(began);
    let sinks: Vec<&Sink> = command.sinks.iter().filter(|sink| !sink.ended().is_cancelled()).collect();
    if sinks.is_empty() {
        return;
    }
    let input = Input::Command { text: text(), failed };
    for sink in sinks {
        sink.input(at.into_std(), input.clone());
    }
}

/// Takes a command off its tab's list when dropped, as when its caller stops
/// waiting for it, so that it notes no later input.
pub(crate) struct Forget<'a>(pub &'a BrowserTab, pub &'a Arc<Command>);

impl Drop for Forget<'_> {
    fn drop(&mut self) {
        self.0.state().inputs.commands.lock().expect("the tab's commands are intact").retain(|other| !Arc::ptr_eq(other, self.1));
    }
}

/// Reports an input the agent sent as a DevTools protocol message, by its
/// method and parameters, as the same input sent by a command is.
pub(crate) fn sent(tab: &BrowserTab, method: &str, params: &serde_json::Value) {
    use chromiumoxide::cdp::browser_protocol::input::InsertTextParams;
    match method {
        "Input.dispatchMouseEvent" => {
            if let Ok(event) = serde_json::from_value::<DispatchMouseEventParams>(params.clone()) {
                mouse(tab, &event, InputBy::Agent);
            }
        }
        "Input.dispatchKeyEvent" => {
            if let Ok(event) = serde_json::from_value::<DispatchKeyEventParams>(params.clone()) {
                key(tab, &event, InputBy::Agent);
            }
        }
        "Input.insertText" => {
            if let Ok(event) = serde_json::from_value::<InsertTextParams>(params.clone()) {
                typed(tab, &event.text, InputBy::Agent);
            }
        }
        _ => {}
    }
}
