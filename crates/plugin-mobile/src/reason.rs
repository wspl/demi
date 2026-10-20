//! Why the `mobile` kind's page is told a request or a tab failed: one
//! closed set, which the page's types name, so the page says every reason
//! in words of its own.

use demi_command_package_mobile_protocol::mobile::MobileErrorCode;
use demi_web_api_protocol::error::ErrorCode;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Every reason the page's methods and its view's
/// notices give: the package's own codes, the view's notices, the
/// conversation's host access's refusals and the plugin call route's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PanelReason {
    // The package's own codes (`crates/command-package-mobile/src/operations.rs`).
    InvalidInput,
    DeviceNotFound,
    AmbiguousDevice,
    DeviceBusy,
    TooManyDevices,
    StaleRef,
    Timeout,
    Unavailable,
    NotRecording,
    Recording,
    OutputExists,
    IoError,
    DriverError,
    DeviceLost,
    Cancelled,
    /// The package's service did not start on the Host.
    MobileUnavailable,
    // The view's notices.
    CaptureFailed,
    CaptureUnavailable,
    InputFailed,
    NoFocusedField,
    // The conversation's host access (`sessions-and-targets.md` § Host
    // operations).
    ConversationNotFound,
    ConversationArchived,
    ConversationBusy,
    HostStopped,
    DeviceOffline,
    CloudUnavailable,
    CloudResetting,
    CloudCapacity,
    CloudCrashLoop,
    HostOperationFailed,
    // The plugin call route's own.
    UnknownPluginMethod,
    PluginDisabled,
}

serde_plain::derive_display_from_serialize!(PanelReason);
serde_plain::derive_fromstr_from_deserialize!(PanelReason);

impl From<MobileErrorCode> for PanelReason {
    fn from(code: MobileErrorCode) -> Self {
        match code {
            MobileErrorCode::InvalidInput => Self::InvalidInput,
            MobileErrorCode::DeviceNotFound => Self::DeviceNotFound,
            MobileErrorCode::AmbiguousDevice => Self::AmbiguousDevice,
            MobileErrorCode::DeviceBusy => Self::DeviceBusy,
            MobileErrorCode::TooManyDevices => Self::TooManyDevices,
            MobileErrorCode::StaleRef => Self::StaleRef,
            MobileErrorCode::Timeout => Self::Timeout,
            MobileErrorCode::Unavailable => Self::Unavailable,
            MobileErrorCode::NotRecording => Self::NotRecording,
            MobileErrorCode::Recording => Self::Recording,
            MobileErrorCode::OutputExists => Self::OutputExists,
            MobileErrorCode::IoError => Self::IoError,
            MobileErrorCode::DriverError => Self::DriverError,
            MobileErrorCode::DeviceLost => Self::DeviceLost,
            MobileErrorCode::Cancelled => Self::Cancelled,
        }
    }
}

impl PanelReason {
    /// The host access's refusal `code` as a reason; none for one the page
    /// has no words of its own for, which it says by its HTTP status.
    pub fn of_host(code: ErrorCode) -> Option<Self> {
        Some(match code {
            ErrorCode::ConversationNotFound => Self::ConversationNotFound,
            ErrorCode::ConversationArchived => Self::ConversationArchived,
            ErrorCode::ConversationBusy => Self::ConversationBusy,
            ErrorCode::HostStopped => Self::HostStopped,
            ErrorCode::DeviceOffline => Self::DeviceOffline,
            ErrorCode::DeviceNotFound => Self::DeviceNotFound,
            ErrorCode::CloudUnavailable => Self::CloudUnavailable,
            ErrorCode::CloudResetting => Self::CloudResetting,
            ErrorCode::CloudCapacity => Self::CloudCapacity,
            ErrorCode::CloudCrashLoop => Self::CloudCrashLoop,
            ErrorCode::HostOperationFailed => Self::HostOperationFailed,
            _ => return None,
        })
    }
}
