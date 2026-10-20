//! The `demi mobile` group (`crates/plugin-mobile/command/src/operations.rs`): one native leaf per
//! operation of the `demi.mobile` package, its arguments and `--json` result
//! declared from the `command-package-mobile-protocol` types, so the command
//! line, the runner's check and the operation read one definition.

use demi_command_declarations::NativeOperation;
use demi_command_package_mobile_protocol::PACKAGE;
use demi_command_package_mobile_protocol::mobile::*;
use demi_host_interface::{GroupBuilder, LeafBuilder};

/// The group's summary.
const SUMMARY: &str = "Operate the simulators of the Host running this shell, as a person would, while the user watches and operates them in the work panel. Every coordinate is a pixel of the device's screenshot. Use inspect to obtain element references; never guess them.";
/// The group's entry in the model's capability index.
const ENTRY: &str = "Drives the iOS simulators of a Mac Host and the Android emulators of a Mac, Linux or Windows Host: opens a device (booting it without a window), takes screenshots, reads the screen's UI tree with element references, taps, swipes, types, presses home or lock, rotates, switches light or dark, waits for text and records video; the user watches and uses the same device in their work panel. For iOS, build, install and launch apps, set locale, location or permissions with xcodebuild and xcrun simctl, which take the simctl id open prints.";

const SUCCESS: &str = "a short readable result; one validated JSON value with --json";
const FAILURE: &str = "a mobile error on stderr with nonzero exit status; an input is never replayed automatically";

/// Declares each operation's leaf from its name, its input and result types,
/// the fields its command line gives in order, and its summary.
macro_rules! operations {
    ($($name:literal => $input:ty, $result:ty, [$($positional:literal),*], $summary:expr;)*) => {
        /// Every operation's leaf, in the package's order.
        fn leaves() -> Vec<LeafBuilder> {
            vec![$(leaf::<$input, $result>($name, &[$($positional),*], $summary),)*]
        }

        /// The operation each leaf of the group runs, by the leaf's name.
        pub fn leaf_operations() -> Vec<(&'static str, NativeOperation)> {
            vec![$(($name, operation($name)),)*]
        }
    };
}

operations! {
    "list" => ListInput, ListResult, [], "List this Host's simulators: the simctl id or AVD name open takes, platform, system, state, the handle while open here, model and name; why a platform has none is a warning on stderr.";
    "open" => OpenInput, OpenResult, ["device"], format!("Open a simulator by its name or simctl id, booting it without a window when it is shut down; prints its handle, such as m1, its screenshot size and its simctl id, and shows it in the user's work panel. At most {MAX_DEVICES} per Host."); 
    "close" => CloseInput, CloseResult, ["device"], "Close a device; shuts it down when demi mobile booted it, else leaves it booted.";
    "screenshot" => ScreenshotInput, ScreenshotResult, ["device"], format!("Capture the settled screen: its PNG's bytes on stdout, its long side at most {SCREENSHOT_LONG_SIDE} pixels, so demi mobile screenshot m1 | demi file view shows it to you and > shot.png saves it; or save a new PNG with --output. Waits up to 1.5 s for the screen to stop changing.");
    "inspect" => InspectInput, InspectResult, ["device"], "Read the settled screen's UI tree, alerts and the keyboard included: each element's ref, role, label, value, identifier, frame in screenshot pixels and state.";
    "tap" => TapInput, TapResult, ["device", "at"], "Tap an element ref from the last inspect, or x y in screenshot pixels: tap m1 e7, tap m1 540 1210. Returns once the device took the tap, not once the screen changed.";
    "swipe" => SwipeInput, SwipeResult, ["device", "at"], "Swipe from x1 y1 to x2 y2 in screenshot pixels, or across an element so its content moves up, down, left or right: swipe m1 e4 up.";
    "type" => TypeInput, TypeResult, ["device", "text"], "Type text into the focused field, or tap --into <ref> first; --replace sets the field's text at once instead of adding to it.";
    "key" => KeyInput, KeyResult, ["device", "key"], "Press home, lock, volume-up or volume-down, or the keyboard's enter, delete, tab or escape.";
    "rotate" => RotateInput, RotateResult, ["device", "orientation"], "Turn the device portrait or landscape, or a quarter turn left or right; screenshots and coordinates follow it.";
    "appearance" => AppearanceInput, AppearanceResult, ["device", "appearance"], "Switch the system to light or dark, or toggle it.";
    "wait" => WaitInput, WaitResult, ["device"], "Wait until an element shows --text, an element ref appears (--element) or is gone (--gone), re-reading the tree; 30 s by default.";
    "record" => RecordInput, RecordResult, ["device", "action"], "Start recording what the device shows, or stop and write the MP4 to stdout, or to a new file with --output.";
}

fn operation(name: &str) -> NativeOperation {
    NativeOperation {
        package: PACKAGE.into(),
        operation: format!("{PREFIX}{name}"),
    }
}

/// The `mobile` group.
pub(crate) fn mobile_group() -> GroupBuilder {
    leaves().into_iter().fold(
        GroupBuilder::new("mobile", SUMMARY).index_entry(ENTRY),
        GroupBuilder::leaf,
    )
}

fn leaf<I: schemars::JsonSchema, R: schemars::JsonSchema>(
    name: &'static str,
    positionals: &[&'static str],
    summary: impl Into<String>,
) -> LeafBuilder {
    LeafBuilder::native(name, summary, operation(name))
        .input::<I>()
        .positionals(positionals.iter().copied())
        .json_output::<R>()
        .success_output(match name {
            "screenshot" => {
                "the PNG's bytes on stdout and what it captured on stderr, so pipe it into demi file view to see it or redirect it to save it; with --output, the file saved; --json requires --output"
            }
            "record" => "for stop, the MP4's bytes on stdout, or with --output the file saved",
            _ => SUCCESS,
        })
        .failure_output(FAILURE)
}
