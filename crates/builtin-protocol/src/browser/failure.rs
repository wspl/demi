//! How a browser operation fails (`browser.md` § Errors): a code scripts
//! branch on, a message that explains the situation, and typed details.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use super::AssetsExportResult;

closed_set! {
    /// Why an operation failed. Each cause keeps its own code.
    pub enum BrowserErrorCode {
        InvalidInput = "invalid_input",
        TabNotFound = "tab_not_found",
        TabBusy = "tab_busy",
        StaleRef = "stale_ref",
        StaleCursor = "stale_cursor",
        StaleInventory = "stale_inventory",
        StaleTools = "stale_tools",
        TargetNotFound = "target_not_found",
        AmbiguousTarget = "ambiguous_target",
        NotActionable = "not_actionable",
        Timeout = "timeout",
        DialogBlocked = "dialog_blocked",
        DialogNotFound = "dialog_not_found",
        InvalidDialogAction = "invalid_dialog_action",
        HistoryBoundary = "history_boundary",
        NavigationFailed = "navigation_failed",
        ProtectedValue = "protected_value",
        SideEffectRejected = "side_effect_rejected",
        UnsupportedResult = "unsupported_result",
        UnsupportedCapability = "unsupported_capability",
        CdpMethodDenied = "cdp_method_denied",
        OutputExists = "output_exists",
        IoError = "io_error",
        ResultTooLarge = "result_too_large",
        PartialFailure = "partial_failure",
        DriverError = "driver_error",
        BrowserUnavailable = "browser_unavailable",
        BrowserLost = "browser_lost",
        Cancelled = "cancelled",
        OutcomeUnknown = "outcome_unknown",
    }
}

closed_set! {
    /// How far an action's input got when it failed: not delivered, delivered
    /// with a later wait failing, or lost with the connection after delivery
    /// began.
    pub enum ActionProgress {
        NotStarted = "not_started",
        Completed = "completed",
        Unknown = "unknown",
    }
}

/// What a failure knows beyond its code and message.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ErrorDetails {
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "ActionProgress")]
    pub action: Option<ActionProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub tab: Option<String>,
    /// The tab's URL when the action failed.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub url: Option<String>,
    /// The element condition that was not met.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub condition: Option<String>,
    /// What intercepted the pointer instead of the target.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub interceptor: Option<String>,
    /// How many elements an ambiguous target matched.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "usize")]
    pub count: Option<usize>,
    /// How many characters `type` delivered before it failed.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "usize")]
    pub delivered: Option<usize>,
    /// The agent nodes whose debugging connections hold the tab when a
    /// command on it times out.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Vec<String>")]
    pub debugging_callers: Option<Vec<String>>,
    /// What an export wrote before it failed or was interrupted.
    #[serde(flatten)]
    pub export: Option<AssetsExportResult>,
}

/// A failed operation, or a failed item of a batch such as a page of
/// `content.fetch`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BrowserFailure {
    pub code: BrowserErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "ErrorDetails")]
    pub details: Option<ErrorDetails>,
}

/// What a failed operation writes to stderr under `--json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FailureDocument {
    pub error: BrowserFailure,
}
