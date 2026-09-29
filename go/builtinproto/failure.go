package builtinproto

import "errors"

// How a browser operation fails (docs/browser/browser.md § Errors): a code
// scripts branch on, a message that explains the situation, and typed details.

// A BrowserErrorCode says why an operation failed. Each cause keeps its own
// code.
//
//demi:enum
//demi:describe Why an operation failed. Each cause keeps its own code.
type BrowserErrorCode string

// The values of a [BrowserErrorCode].
const (
	ErrInvalidInput          BrowserErrorCode = "invalid_input"
	ErrTabNotFound           BrowserErrorCode = "tab_not_found"
	ErrTabBusy               BrowserErrorCode = "tab_busy"
	ErrStaleRef              BrowserErrorCode = "stale_ref"
	ErrStaleCursor           BrowserErrorCode = "stale_cursor"
	ErrStaleInventory        BrowserErrorCode = "stale_inventory"
	ErrStaleTools            BrowserErrorCode = "stale_tools"
	ErrTargetNotFound        BrowserErrorCode = "target_not_found"
	ErrAmbiguousTarget       BrowserErrorCode = "ambiguous_target"
	ErrNotActionable         BrowserErrorCode = "not_actionable"
	ErrTimeout               BrowserErrorCode = "timeout"
	ErrDialogBlocked         BrowserErrorCode = "dialog_blocked"
	ErrDialogNotFound        BrowserErrorCode = "dialog_not_found"
	ErrInvalidDialogAction   BrowserErrorCode = "invalid_dialog_action"
	ErrHistoryBoundary       BrowserErrorCode = "history_boundary"
	ErrNavigationFailed      BrowserErrorCode = "navigation_failed"
	ErrProtectedValue        BrowserErrorCode = "protected_value"
	ErrSideEffectRejected    BrowserErrorCode = "side_effect_rejected"
	ErrUnsupportedResult     BrowserErrorCode = "unsupported_result"
	ErrUnsupportedCapability BrowserErrorCode = "unsupported_capability"
	ErrCDPMethodDenied       BrowserErrorCode = "cdp_method_denied"
	ErrOutputExists          BrowserErrorCode = "output_exists"
	ErrIO                    BrowserErrorCode = "io_error"
	ErrResultTooLarge        BrowserErrorCode = "result_too_large"
	ErrPartialFailure        BrowserErrorCode = "partial_failure"
	ErrDriver                BrowserErrorCode = "driver_error"
	ErrBrowserUnavailable    BrowserErrorCode = "browser_unavailable"
	ErrBrowserLost           BrowserErrorCode = "browser_lost"
	ErrCancelled             BrowserErrorCode = "cancelled"
	ErrOutcomeUnknown        BrowserErrorCode = "outcome_unknown"
)

// An ActionProgress says how far an action's input got when it failed: not
// delivered, delivered with a later wait failing, or lost with the connection
// after delivery began.
//
//demi:enum
//demi:describe How far an action's input got when it failed: not delivered, delivered
//demi:describe with a later wait failing, or lost with the connection after delivery
//demi:describe began.
type ActionProgress string

// The values of an [ActionProgress].
const (
	ProgressNotStarted ActionProgress = "not_started"
	ProgressCompleted  ActionProgress = "completed"
	ProgressUnknown    ActionProgress = "unknown"
)

// ErrorDetails is what a failure knows beyond its code and message. Its last
// three members are what an export wrote before it failed or was interrupted
// (an [AssetsExportResult]'s), which come together.
//
//demi:wire open
type ErrorDetails struct {
	Action *ActionProgress `json:"action,omitzero"`
	Tab    *string         `json:"tab,omitzero"`
	// The tab's URL when the action failed.
	URL *string `json:"url,omitzero"`
	// The element condition that was not met.
	Condition *string `json:"condition,omitzero"`
	// What intercepted the pointer instead of the target.
	Interceptor *string `json:"interceptor,omitzero"`
	// How many elements an ambiguous target matched.
	Count *uint `json:"count,omitzero"`
	// How many characters `type` delivered before it failed.
	Delivered *uint `json:"delivered,omitzero"`
	// The numbers of the agents whose debugging connections hold the tab
	// when a command on it times out.
	DebuggingCallers *[]uint64        `json:"debuggingCallers,omitzero"`
	Directory        *string          `json:"directory,omitzero"`
	Manifest         *string          `json:"manifest,omitzero"`
	Files            *[]ExportedAsset `json:"files,omitzero"`
}

// check is the rule across the export's members: they come together.
func (d ErrorDetails) check() error {
	present := 0
	if d.Directory != nil {
		present++
	}
	if d.Manifest != nil {
		present++
	}
	if d.Files != nil {
		present++
	}
	if present != 0 && present != 3 {
		return errors.New("directory, manifest and files come together")
	}
	return nil
}

// A BrowserFailure is a failed operation, or a failed item of a batch such as a
// page of content.fetch.
//
//demi:wire
//demi:describe A failed operation, or a failed item of a batch such as a page of
//demi:describe `content.fetch`.
type BrowserFailure struct {
	Code    BrowserErrorCode `json:"code"`
	Message string           `json:"message"`
	// What a failure knows beyond its code and message.
	Details *ErrorDetails `json:"details,omitzero"`
}

// A FailureDocument is what a failed operation writes to stderr under --json.
//
//demi:wire
type FailureDocument struct {
	Error BrowserFailure `json:"error"`
}
