//! The panel's controls (`crates/plugin-mobile/src/lib.rs`): each control runs
//! the operation the agent's command of the same meaning runs, as a package
//! call for the conversation's user, and holds no simulator logic.
//!
//! The panel opens no device of its own; the agent does. Every control operates
//! what runs without waking it.

use demi_command_declarations::NativeOperation;
use demi_command_package_mobile_protocol::PACKAGE;
use demi_command_package_mobile_protocol::mobile::{
    AppearanceChoice, AppearanceInput, AppearanceResult, CloseInput, CloseResult, DeviceId,
    FailureDocument, Key, KeyInput, KeyResult, ListInput, ListResult, MobileErrorCode,
    MobileOperation, PREFIX, RecordAction, RecordInput, RecordResult, RotateInput, RotateResult,
    Rotation, ScreenshotInput, ScreenshotResult,
};
use demi_plugin_interface::{CallKind, Method, Page, PluginError, PluginPort, PortFailure, PortRefusal, Scope};
use demi_web_api_protocol::error::ErrorCode;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::panel::KIND;
use crate::reason::PanelReason;

/// The operation `name` names, in the mobile package.
pub(crate) fn operation(name: &str) -> NativeOperation {
    NativeOperation {
        package: PACKAGE.into(),
        operation: format!("{PREFIX}{name}"),
    }
}

/// `close { device }`, `screenshot { device }`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OnDevice {
    pub device: String,
}

/// Which way Rotate Left and Rotate Right turn the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Turn {
    Left,
    Right,
}

/// `rotate { device, turn }`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RotateDevice {
    pub device: String,
    pub turn: Turn,
}

/// A button the panel presses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum PanelButton {
    Home,
    Back,
    Lock,
    VolumeUp,
    VolumeDown,
}

impl From<PanelButton> for Key {
    fn from(button: PanelButton) -> Self {
        match button {
            PanelButton::Home => Key::Home,
            PanelButton::Back => Key::Back,
            PanelButton::Lock => Key::Lock,
            PanelButton::VolumeUp => Key::VolumeUp,
            PanelButton::VolumeDown => Key::VolumeDown,
        }
    }
}

/// `press { device, button }`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PressButton {
    pub device: String,
    pub button: PanelButton,
}

/// `record { device, action }`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordDevice {
    pub device: String,
    pub action: RecordAction,
}

/// What Screenshot and a stopped Record answer: the file on the Host the
/// page saves on the user's computer through the file route.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Saved {
    pub path: String,
}

/// The page: its controls, each declared with the operations it calls,
/// and its panel kind.
pub(crate) fn page() -> Page {
    Page::new("@demicodes/plugin-mobile")
        .panel_kind(KIND)
        .method(Method::new::<OnDevice, CloseResult>("close", Scope::Conversation).calls(operation("close")))
        .method(Method::new::<RotateDevice, RotateResult>("rotate", Scope::Conversation).calls(operation("rotate")))
        .method(
            Method::new::<OnDevice, AppearanceResult>("toggleAppearance", Scope::Conversation)
                .calls(operation("appearance")),
        )
        .method(Method::new::<PressButton, KeyResult>("press", Scope::Conversation).calls(operation("key")))
        .method(Method::new::<OnDevice, Saved>("screenshot", Scope::Conversation).calls(operation("screenshot")))
        .method(Method::new::<RecordDevice, RecordResult>("record", Scope::Conversation).calls(operation("record")))
        .reasons::<PanelReason>()
}

/// The Host's simulators, as a look that wakes nothing; none while the
/// conversation's Cloud is stopped, which says nothing of its simulators.
pub(crate) async fn list(port: &PluginPort) -> Result<Option<ListResult>, PluginError> {
    match run::<ListResult, _>(port, MobileOperation::List, ListInput {}, CallKind::Looks).await {
        Ok(listed) => Ok(Some(listed)),
        Err(failure) if stopped(&failure) => Ok(None),
        Err(failure) => Err(refused(failure)),
    }
}

/// Closes the device `device`; one the Host no longer has, or a stopped
/// Cloud's, is closed already.
pub(crate) async fn close(port: &PluginPort, device: &str) -> Result<(), PluginError> {
    let Ok(device) = DeviceId::try_from(device.to_owned()) else {
        return Ok(());
    };
    let input = CloseInput { device };
    match run::<CloseResult, _>(port, MobileOperation::Close, input, CallKind::Operates).await {
        Ok(_) => Ok(()),
        Err(failure) if stopped(&failure) || code(&failure) == Some(MobileErrorCode::DeviceNotFound) => Ok(()),
        Err(failure) => Err(refused(failure)),
    }
}

/// Answers the page call `method` with `params` for `conversation`.
pub(crate) async fn call(
    method: &str,
    params: Map<String, Value>,
    port: &PluginPort,
) -> Result<Value, PluginError> {
    match method {
        "close" => {
            let OnDevice { device } = decode(params)?;
            let input = CloseInput { device: device_id(device)? };
            to_value(operate::<CloseResult, _>(port, MobileOperation::Close, input).await?)
        }
        "rotate" => {
            let RotateDevice { device, turn } = decode(params)?;
            let orientation = match turn {
                Turn::Left => Rotation::Left,
                Turn::Right => Rotation::Right,
            };
            let input = RotateInput {
                device: device_id(device)?,
                orientation,
            };
            to_value(operate::<RotateResult, _>(port, MobileOperation::Rotate, input).await?)
        }
        "toggleAppearance" => {
            let OnDevice { device } = decode(params)?;
            let input = AppearanceInput {
                device: device_id(device)?,
                appearance: AppearanceChoice::Toggle,
            };
            to_value(operate::<AppearanceResult, _>(port, MobileOperation::Appearance, input).await?)
        }
        "press" => {
            let PressButton { device, button } = decode(params)?;
            let input = KeyInput {
                device: device_id(device)?,
                key: button.into(),
            };
            to_value(operate::<KeyResult, _>(port, MobileOperation::Key, input).await?)
        }
        // Without --output, the user's call saves the picture or the MP4 in
        // the conversation's scratch directory and answers its path, which
        // the page saves on the user's computer (`crates/plugin-mobile/src/lib.rs`).
        "screenshot" => {
            let OnDevice { device } = decode(params)?;
            let input = ScreenshotInput {
                device: device_id(device)?,
                output: None,
                overwrite: None,
            };
            let shot = operate::<ScreenshotResult, _>(port, MobileOperation::Screenshot, input).await?;
            to_value(Saved { path: shot.path })
        }
        "record" => {
            let RecordDevice { device, action } = decode(params)?;
            let input = RecordInput {
                device: device_id(device)?,
                action,
                output: None,
                overwrite: None,
            };
            to_value(operate::<RecordResult, _>(port, MobileOperation::Record, input).await?)
        }
        method => Err(PluginError::failed(format!("the simulators have no method {method}"))),
    }
}

/// Runs an operation the panel's control asks of an open device.
async fn operate<T: DeserializeOwned, I: Serialize>(
    port: &PluginPort,
    variant: fn(I) -> MobileOperation,
    input: I,
) -> Result<T, PluginError> {
    run(port, variant, input, CallKind::Operates).await.map_err(refused)
}

/// Runs the operation `variant` with `input` and decodes its answer.
async fn run<T: DeserializeOwned, I: Serialize>(
    port: &PluginPort,
    variant: fn(I) -> MobileOperation,
    input: I,
    kind: CallKind,
) -> Result<T, PortFailure> {
    let Ok(Value::Object(args)) = serde_json::to_value(&input) else {
        unreachable!("a mobile operation's input serializes to an object")
    };
    let name = variant(input).name();
    let result = port.package_call(operation(name), args, kind).await?;
    serde_json::from_value(result).map_err(|error| {
        PortFailure::Refused(PortRefusal::Operation {
            stderr: format!("the simulators answered what the plugin cannot read: {error}"),
        })
    })
}

/// The device a call names; one that is no handle is a device the Host
/// does not have.
fn device_id(device: String) -> Result<DeviceId, PluginError> {
    DeviceId::try_from(device)
        .map_err(|_| refusal(PanelReason::DeviceNotFound, "No such device".to_owned()))
}

fn refusal(reason: PanelReason, message: String) -> PluginError {
    PluginError::refused(reason.to_string(), message)
}

fn stopped(failure: &PortFailure) -> bool {
    matches!(
        failure,
        PortFailure::Refused(PortRefusal::Host {
            code: ErrorCode::HostStopped,
            ..
        })
    )
}

/// The package's own failure code, when the operation failed with one.
fn code(failure: &PortFailure) -> Option<MobileErrorCode> {
    mobile_failure(failure).map(|document| document.error.code)
}

fn mobile_failure(failure: &PortFailure) -> Option<FailureDocument> {
    let PortFailure::Refused(PortRefusal::Operation { stderr }) = failure else {
        return None;
    };
    serde_json::from_str(stderr.trim()).ok()
}

/// A failed call as the page sees it: the package's refusal under its own
/// code; a package whose service did not start as `mobile_unavailable`,
/// with the runner's words; or the host access's refusal, which passes on.
pub(crate) fn refused(failure: PortFailure) -> PluginError {
    if let Some(document) = mobile_failure(&failure) {
        return refusal(document.error.code.into(), document.error.message);
    }
    match failure {
        PortFailure::Refused(PortRefusal::Service { message }) => {
            refusal(PanelReason::MobileUnavailable, message)
        }
        failure => failure.into(),
    }
}

fn decode<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, PluginError> {
    serde_json::from_value(Value::Object(params)).map_err(|error| PluginError::Usage {
        message: error.to_string(),
    })
}

fn to_value(value: impl Serialize) -> Result<Value, PluginError> {
    serde_json::to_value(value).map_err(PluginError::failed)
}
