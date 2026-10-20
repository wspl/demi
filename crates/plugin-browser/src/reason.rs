//! Why the `browser` kind's page is told a request, a tab or its view
//! failed: one closed set, which the
//! page's types name, so the page says every reason in words of its own.

use demi_command_package_browser_protocol::browser::BrowserErrorCode;
use demi_web_api_protocol::error::ErrorCode;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Every reason the page's tab methods, its tab list, its tabs' failures
/// and its view's notices give: the browser's own codes, the view's capture
/// codes, the conversation's host access's refusals and the plugin call
/// route's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PanelReason {
    // The browser's own codes.
    InvalidInput,
    TabNotFound,
    TabBusy,
    StaleRef,
    StaleCursor,
    StaleInventory,
    StaleTools,
    TargetNotFound,
    AmbiguousTarget,
    NotActionable,
    Timeout,
    DialogBlocked,
    DialogNotFound,
    InvalidDialogAction,
    HistoryBoundary,
    NavigationFailed,
    ProtectedValue,
    SideEffectRejected,
    UnsupportedResult,
    UnsupportedCapability,
    CdpMethodDenied,
    OutputExists,
    IoError,
    ResultTooLarge,
    PartialFailure,
    DriverError,
    BrowserUnavailable,
    BrowserLost,
    Cancelled,
    OutcomeUnknown,
    // The view's capture notices.
    CaptureUnavailable,
    CaptureFailed,
    CaptureStopped,
    // The conversation's host access (`sessions-and-targets.md` § Host
    // operations).
    ConversationNotFound,
    ConversationArchived,
    ConversationBusy,
    HostStopped,
    DeviceOffline,
    DeviceNotFound,
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

impl From<BrowserErrorCode> for PanelReason {
    fn from(code: BrowserErrorCode) -> Self {
        match code {
            BrowserErrorCode::InvalidInput => Self::InvalidInput,
            BrowserErrorCode::TabNotFound => Self::TabNotFound,
            BrowserErrorCode::TabBusy => Self::TabBusy,
            BrowserErrorCode::StaleRef => Self::StaleRef,
            BrowserErrorCode::StaleCursor => Self::StaleCursor,
            BrowserErrorCode::StaleInventory => Self::StaleInventory,
            BrowserErrorCode::StaleTools => Self::StaleTools,
            BrowserErrorCode::TargetNotFound => Self::TargetNotFound,
            BrowserErrorCode::AmbiguousTarget => Self::AmbiguousTarget,
            BrowserErrorCode::NotActionable => Self::NotActionable,
            BrowserErrorCode::Timeout => Self::Timeout,
            BrowserErrorCode::DialogBlocked => Self::DialogBlocked,
            BrowserErrorCode::DialogNotFound => Self::DialogNotFound,
            BrowserErrorCode::InvalidDialogAction => Self::InvalidDialogAction,
            BrowserErrorCode::HistoryBoundary => Self::HistoryBoundary,
            BrowserErrorCode::NavigationFailed => Self::NavigationFailed,
            BrowserErrorCode::ProtectedValue => Self::ProtectedValue,
            BrowserErrorCode::SideEffectRejected => Self::SideEffectRejected,
            BrowserErrorCode::UnsupportedResult => Self::UnsupportedResult,
            BrowserErrorCode::UnsupportedCapability => Self::UnsupportedCapability,
            BrowserErrorCode::CdpMethodDenied => Self::CdpMethodDenied,
            BrowserErrorCode::OutputExists => Self::OutputExists,
            BrowserErrorCode::IoError => Self::IoError,
            BrowserErrorCode::ResultTooLarge => Self::ResultTooLarge,
            BrowserErrorCode::PartialFailure => Self::PartialFailure,
            BrowserErrorCode::DriverError => Self::DriverError,
            BrowserErrorCode::BrowserUnavailable => Self::BrowserUnavailable,
            BrowserErrorCode::BrowserLost => Self::BrowserLost,
            BrowserErrorCode::Cancelled => Self::Cancelled,
            BrowserErrorCode::OutcomeUnknown => Self::OutcomeUnknown,
        }
    }
}

impl PanelReason {
    /// The host access's refusal `code` as a reason; none for one the page
    /// has no words of its own for, such as an internal error, which it
    /// says by its HTTP status.
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
