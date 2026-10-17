//! The commands' default text (`browser.md` § Default text, which the
//! mobile commands follow): short and readable, the screen's text quoted
//! with its control characters escaped, so a label cannot forge a line of
//! the result.

use std::fmt::Write;

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

fn state(simulator: &Simulator) -> String {
    if let Some(handle) = &simulator.open {
        return format!("open here as {handle}");
    }
    if simulator.elsewhere {
        return "open in another conversation".into();
    }
    match simulator.state {
        PowerState::ShutDown => "shut down",
        PowerState::Booting => "booting",
        PowerState::Booted => "booted",
        PowerState::ShuttingDown => "shutting down",
    }
    .into()
}

fn native_id(platform: MobilePlatform, id: &str) -> String {
    match platform {
        MobilePlatform::Ios => format!("simctl id {id}"),
        MobilePlatform::Android => format!("AVD {id}"),
    }
}

pub fn list(result: &ListResult) -> String {
    let mut text = String::new();
    for simulator in &result.devices {
        let model = if simulator.model == simulator.name || simulator.model.is_empty() {
            String::new()
        } else {
            format!(" ({})", simulator.model)
        };
        let _ = writeln!(
            text,
            "{}{model} · {} · {} · {}",
            simulator.name,
            simulator.os,
            state(simulator),
            native_id(simulator.platform, &simulator.native_id)
        );
    }
    if result.devices.is_empty() {
        text.push_str("No simulators on this Host.\n");
    }
    for unavailable in &result.unavailable {
        let _ = writeln!(text, "{}", unavailable.reason);
    }
    text
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
    if result.truncated {
        text.push_str("… more elements left out; the result holds at most 64 KiB.\n");
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
