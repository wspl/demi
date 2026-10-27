//! The `demi mobile` group (`crates/plugin-mobile/command/src/operations.rs`): one native leaf per
//! operation of the `demi.mobile` package, its arguments and `--json` result
//! declared from the `command-package-mobile-protocol` types, so the command
//! line, the runner's check and the operation read one definition.

use demi_command_declarations::NativeOperation;
use demi_command_package_mobile_protocol::PACKAGE;
use demi_command_package_mobile_protocol::mobile::*;
use demi_host_interface::{GroupBuilder, LeafBuilder};

/// The group's summary.
const SUMMARY: &str = "Operate the simulators of the Host running this shell, as a person would, while the user watches and operates them in the work panel. Every coordinate is a pixel of the device's screenshot. Element references come from inspect.";
/// The group's entry in the model's capability index.
const ENTRY: &str = "iOS simulators and Android emulators: open, screenshot, UI tree, tap, swipe, type, rotate, record, install and launch apps, logs, erase.";

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
    "open" => OpenInput, OpenResult, ["device"], format!("Open a simulator by its name or simctl id, booting it without a window when it is shut down; prints its handle, such as m1, its screenshot size and its simctl id, and shows it in the user's work panel."); 
    "close" => CloseInput, CloseResult, ["device"], "Close a device; shuts it down when demi mobile booted it, else leaves it booted.";
    "screenshot" => ScreenshotInput, ScreenshotResult, ["device"], format!("Capture the settled screen: its PNG's bytes on stdout, its long side at most {SCREENSHOT_LONG_SIDE} pixels, so demi mobile screenshot m1 | demi file view shows it to you and > shot.png saves it; or save a new PNG with --output. Waits up to 1.5 s for the screen to stop changing.");
    "inspect" => InspectInput, InspectResult, ["device"], "Read the settled screen's UI tree, alerts and the keyboard included: each element's ref, role, label, value, identifier, frame in screenshot pixels and state.";
    "tap" => TapInput, TapResult, ["device", "at"], "Tap an element, or x y in screenshot pixels: tap m1 text=Continue, tap m1 id=submit, tap m1 e7 (a ref from the last inspect), tap m1 540 1210. text= and id= need no inspect and wait until the element is there, 10 s by default. Returns once the device took the tap; wait waits for what the tap changes.";
    "swipe" => SwipeInput, SwipeResult, ["device", "at"], "Swipe from x1 y1 to x2 y2 in screenshot pixels, or across an element so its content moves up, down, left or right: swipe m1 id=list up.";
    "type" => TypeInput, TypeResult, ["device", "text"], "Type text into the focused field, or tap --into <element> first, such as --into id=email or --into text=Email, checking that it landed and saying what the field holds, or, when the screen's tree names no focused field, as a field an app hides behind boxes it draws, sending the keys unchecked; --replace sets the field's text at once instead of adding to it.";
    "key" => KeyInput, KeyResult, ["device", "key"], "Press home, lock, volume-up or volume-down, or the keyboard's enter, delete, tab or escape.";
    "rotate" => RotateInput, RotateResult, ["device", "orientation"], "Turn the device portrait or landscape, or a quarter turn left or right; screenshots and coordinates follow it.";
    "fold" => FoldInput, FoldResult, ["device", "to"], "Fold or open a foldable: closed, half-open or open, or the hinge's angle from 0 (closed) to 180 (flat) on iPhone Duo; closed or open on an Android foldable. Screenshots and coordinates follow the display that is lit, and refs from an earlier inspect no longer hold.";
    "appearance" => AppearanceInput, AppearanceResult, ["device", "appearance"], "Switch the system to light or dark, or toggle it.";
    "wait" => WaitInput, WaitResult, ["device"], "Wait until an element shows --text, an element appears (--element) or is gone (--gone), such as --gone text=Loading, re-reading the tree; 30 s by default.";
    "record" => RecordInput, RecordResult, ["target", "action"], record_about("device", "m1");
    "install" => InstallInput, InstallResult, ["device", "file"], "Install an app's build, an .apk on an Android emulator or an .app on an iOS simulator, replacing an installed version and keeping its data; prints the app's id, which launch and logs take.";
    "launch" => LaunchInput, LaunchResult, ["device", "app"], "Bring an installed app in front by its id, starting it when it is not running, as a tap on its icon does; --restart stops it first, so it starts from the beginning.";
    "logs" => LogsInput, LogsResult, ["device"], format!("Read the device's log, followed since it opened, without clearing it: the last entries, or those after --after <cursor>, the cursor the last read printed; --app keeps one app's entries across its restarts, --filter those holding a text, --level those at a level or above. An iOS app's own debug messages and what it prints are there when launch started it. The last {} MB are kept.", LOG_BYTES / (1024 * 1024));
    "erase" => EraseInput, EraseResult, ["device"], "Erase the device to its factory state, every app and its data, the settings and the device's identifiers, and start it again under the same handle; takes about a minute.";
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
            "record" => "start and stop: the recording's ID and state; frames: the changed frames as a table; export: one image path per line, or with --mp4 the video's bytes on stdout, or with --output the file saved",
            _ => SUCCESS,
        })
        .failure_output(FAILURE)
}
