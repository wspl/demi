//! The commands' default text: short and readable, the screen's text quoted
//! with its control characters escaped, so a label cannot forge a line of
//! the result.

use std::fmt::Write;

use demi_shared_types::output::{Cell, Table};

use demi_command_package_mobile_protocol::mobile::{
    Element, ElementSummary, Frame, InspectResult, ListResult, Orientation, MobilePlatform, PowerState,
    Simulator,
};


/// `text` quoted, as JSON quotes a string.
pub fn quoted(text: &str) -> String {
    serde_json::to_string(text).expect("a string serializes")
}

pub fn orientation(orientation: Orientation) -> &'static str {
    match orientation {
        Orientation::Portrait => "portrait",
        Orientation::LandscapeLeft => "landscape left",
        Orientation::PortraitUpsideDown => "portrait upside down",
        Orientation::LandscapeRight => "landscape right",
    }
}

/// A simulator's state as `list` prints it.
fn state(simulator: &Simulator) -> &'static str {
    if simulator.open.is_some() {
        return "open";
    }
    if simulator.elsewhere {
        return "open elsewhere";
    }
    match simulator.state {
        PowerState::ShutDown => "shut down",
        PowerState::Booting => "booting",
        PowerState::Booted => "booted",
        PowerState::ShuttingDown => "shutting down",
    }
}

fn platform(platform: MobilePlatform) -> &'static str {
    match platform {
        MobilePlatform::Ios => "ios",
        MobilePlatform::Android => "android",
    }
}

/// `list`: a row per simulator, its `simctl` id or AVD name first, as
/// `open` takes it, and its name last; the handle while this conversation
/// has it open. Why a platform has none goes to standard error.
pub fn list(result: &ListResult) -> String {
    let mut table = Table::new(["id", "platform", "os", "state", "handle", "model", "name"]);
    for simulator in &result.devices {
        table.row::<Cell>([
            simulator.native_id.as_str().into(),
            platform(simulator.platform).into(),
            simulator.os.as_str().into(),
            state(simulator).into(),
            simulator.open.as_ref().map(ToString::to_string).into(),
            Some(simulator.model.as_str()).filter(|model| !model.is_empty()).into(),
            simulator.name.as_str().into(),
        ]);
    }
    table.render()
}

fn frame(frame: &Frame) -> String {
    format!("[{},{} {}×{}]", frame.x, frame.y, frame.width, frame.height)
}

/// An element as an action's acknowledgement names it, as the tree names
/// it: `button "Sign Up" [ref=e7]`.
pub fn summary(element: &ElementSummary) -> String {
    match &element.label {
        Some(label) => format!("{} {} [ref={}]", element.role, quoted(label), element.r#ref),
        None => format!("{} [ref={}]", element.role, element.r#ref),
    }
}

pub fn inspect(result: &InspectResult, name: &str) -> String {
    let mut text = format!(
        "Device: {} · {name} · {} · {}×{}\n\n",
        result.device,
        orientation(result.orientation),
        result.width,
        result.height
    );
    for element in &result.elements {
        tree(&mut text, element, 0);
    }
    if result.elements.is_empty() {
        text.push_str("The screen shows no elements.\n");
    }
    text
}

fn tree(text: &mut String, element: &Element, depth: usize) {
    let _ = write!(text, "{}- {}", "  ".repeat(depth), element.role);
    if let Some(label) = &element.label {
        let _ = write!(text, " {}", quoted(label));
    }
    let _ = write!(text, " [ref={}]", element.r#ref);
    if let Some(value) = &element.value
        && Some(value) != element.label.as_ref()
    {
        let _ = write!(text, " value={}", quoted(value));
    }
    if let Some(identifier) = &element.identifier
        && Some(identifier) != element.label.as_ref()
    {
        let _ = write!(text, " id={}", quoted(identifier));
    }
    for (flag, word) in [
        (element.enabled == Some(false), "[disabled]"),
        (element.selected == Some(true), "[selected]"),
        (element.focused == Some(true), "[focused]"),
        (element.protected == Some(true), "[protected]"),
    ] {
        if flag {
            let _ = write!(text, " {word}");
        }
    }
    let _ = writeln!(text, " {}", frame(&element.frame));
    for child in &element.children {
        tree(text, child, depth + 1);
    }
}
