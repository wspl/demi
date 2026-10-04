package cdp

import (
	"encoding/json"
	"fmt"

	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
)

// ErrorKind distinguishes browser failures before mapping to wire error codes.
type ErrorKind uint8

const (
	// KindUnsupportedCapability is a capability the browser or document does not offer (unsupported_capability).
	KindUnsupportedCapability ErrorKind = iota + 1
	// KindStaleInventory is an asset inventory handle that is no longer current (stale_inventory).
	KindStaleInventory
	// KindStaleTools is a WebMCP tool set handle that is no longer current (stale_tools).
	KindStaleTools
	// KindStaleCursor is a cursor that is stale or belongs to another stream (stale_cursor).
	KindStaleCursor
	// KindCDPMethodDenied is a raw CDP method the debugging policy denies; Message names it (cdp_method_denied).
	KindCDPMethodDenied
	// KindPartialFailure is a batch in which some items failed; Details holds what succeeded (partial_failure).
	KindPartialFailure
	// KindSideEffectRejected is a read-only evaluation that would have had a side effect (side_effect_rejected).
	KindSideEffectRejected
	// KindClosed is a browser environment, tab or stream that has closed (browser_lost).
	KindClosed
	// KindConnection is a lost CDP connection; Cause holds the transport failure (browser_lost).
	KindConnection
	// KindOutcomeUnknown is input whose delivery is unknown because the connection was lost after it was sent
	// (outcome_unknown).
	KindOutcomeUnknown
	// KindTabNotFound is a tab or CDP session that does not exist (tab_not_found).
	KindTabNotFound
	// KindTargetNotFound is an element or debugging target that matched nothing (target_not_found).
	KindTargetNotFound
	// KindNotActionable is an element that failed an actionability condition; Details holds the condition
	// (not_actionable).
	KindNotActionable
	// KindHistoryBoundary is a back or forward step past the end of the tab's history (history_boundary).
	KindHistoryBoundary
	// KindNavigationFailed is a failed main-document navigation; Message holds Chrome's reason (navigation_failed).
	KindNavigationFailed
	// KindOutputExists is an output path that already exists; Message holds the path (output_exists).
	KindOutputExists
	// KindResultTooLarge is a result over the browser output limit (result_too_large).
	KindResultTooLarge
	// KindProtectedValue is a read of a password field's value (protected_value).
	KindProtectedValue
	// KindUnavailable is a browser that could not start or lacks a service it needs; Message says why
	// (browser_unavailable).
	KindUnavailable
	// KindRoot is Chrome refusing to run as root with its sandbox on Linux (browser_unavailable).
	KindRoot
	// KindInstallation is a missing or unusable Chrome for Testing installation (browser_unavailable).
	KindInstallation
	// KindAction is a failed action; Cause holds the failure, whose code it reports, and Details its tab, URL
	// and input progress.
	KindAction
	// KindCancelled is a cancelled operation (cancelled).
	KindCancelled
	// KindTimeout is an operation that exceeded its deadline (timeout).
	KindTimeout
	// KindBusy is a tab that another command holds (tab_busy).
	KindBusy
	// KindDialogBlocked is input blocked by an open JavaScript dialog (dialog_blocked).
	KindDialogBlocked
	// KindDialogNotFound is a dialog command on a tab without a JavaScript dialog (dialog_not_found).
	KindDialogNotFound
	// KindInvalidDialogAction is an action the open JavaScript dialog does not accept (invalid_dialog_action).
	KindInvalidDialogAction
	// KindAmbiguous is a target that matched more than one element; Count holds the matches (ambiguous_target).
	KindAmbiguous
	// KindStaleReference is a node reference from an earlier document or another tab (stale_ref).
	KindStaleReference
	// KindInvalidResult is a browser result that is not JSON of the expected shape (unsupported_result).
	KindInvalidResult
	// KindConfiguration is invalid input or configuration; Message says what (invalid_input).
	KindConfiguration
	// KindCDP is a malformed or invalid CDP message (driver_error).
	KindCDP
	// KindIO is a local file system failure; Cause holds it (io_error).
	KindIO
	// KindProfileRetained is a retirement that kept the profile; Path holds it and Cause the failure, whose code
	// it reports.
	KindProfileRetained
	// KindCleanup is a failed cleanup; Cause holds the preceding failure, possibly nil, and Cleanup the
	// cleanup's, and the code is the first one present.
	KindCleanup
)

// BrowserError retains a browser failure and its underlying cause. Message holds
// the variant's text payload, Count its ambiguous match count, Path its retained
// profile, and Details its action, condition or export data. Cleanup retains both
// the preceding Cause (possibly nil) and Cleanup; errors.Is/As reach both.
type BrowserError struct {
	// Kind identifies the browser failure variant.
	Kind ErrorKind
	// Message holds the variant text payload.
	Message string
	// Count holds the ambiguous match count.
	Count uint
	// Path names a retained profile.
	Path string
	// Details holds action, condition or export progress.
	Details browserproto.ErrorDetails
	// Cause retains the preceding failure.
	Cause error
	// Cleanup retains the cleanup failure.
	Cleanup error
}

// Error returns the browser failure's user-facing message.
func (e *BrowserError) Error() string {
	switch e.Kind {
	case KindUnsupportedCapability:
		return "unsupported browser capability: " + e.Message
	case KindStaleInventory:
		return "browser asset inventory is stale"
	case KindStaleTools:
		return "browser tool declarations are stale"
	case KindStaleCursor:
		return "browser cursor is stale or belongs to another stream"
	case KindCDPMethodDenied:
		return "CDP method is denied: " + e.Message
	case KindPartialFailure:
		return "some browser items failed"
	case KindSideEffectRejected:
		return "read-only evaluation rejected a possible side effect"
	case KindClosed:
		return "browser environment is closed"
	case KindOutcomeUnknown:
		return "browser input outcome is unknown: " + e.Cause.Error()
	case KindTabNotFound:
		return "browser tab was not found"
	case KindTargetNotFound:
		return "browser target matched no elements"
	case KindNotActionable:
		return e.conditionError()
	case KindHistoryBoundary:
		return "no navigation entry in that direction"
	case KindNavigationFailed:
		return "main document navigation failed: " + e.Message
	case KindOutputExists:
		return "output already exists: " + e.Message
	case KindResultTooLarge:
		return "result exceeds the browser output limit"
	case KindProtectedValue:
		return "password values are protected"
	case KindUnavailable:
		return "browser could not start: " + e.Message
	default:
		return e.driverError()
	}
}

// Unwrap exposes causes without comparing their message strings.
func (e *BrowserError) Unwrap() []error {
	var causes []error
	if e.Cause != nil {
		causes = append(causes, e.Cause)
	}
	if e.Cleanup != nil {
		causes = append(causes, e.Cleanup)
	}
	return causes
}

// Code returns the public failure code through wrappers.
func (e *BrowserError) Code() browserproto.BrowserErrorCode {
	switch e.Kind {
	case KindAction, KindProfileRetained:
		return ErrorCode(e.Cause)
	case KindCleanup:
		if e.Cause != nil {
			return ErrorCode(e.Cause)
		}
		return ErrorCode(e.Cleanup)
	case KindClosed, KindConnection:
		return "browser_lost"
	case KindUnavailable, KindInstallation, KindRoot:
		return "browser_unavailable"
	case KindCDP:
		return "driver_error"
	default:
		return errorCodes[e.Kind]
	}
}

var errorCodes = map[ErrorKind]browserproto.BrowserErrorCode{
	KindUnsupportedCapability: "unsupported_capability",
	KindStaleInventory:        "stale_inventory",
	KindStaleTools:            "stale_tools",
	KindStaleCursor:           "stale_cursor",
	KindCDPMethodDenied:       "cdp_method_denied",
	KindPartialFailure:        "partial_failure",
	KindSideEffectRejected:    "side_effect_rejected",
	KindOutcomeUnknown:        "outcome_unknown",
	KindTabNotFound:           "tab_not_found",
	KindTargetNotFound:        "target_not_found",
	KindNotActionable:         "not_actionable",
	KindHistoryBoundary:       "history_boundary",
	KindNavigationFailed:      "navigation_failed",
	KindOutputExists:          "output_exists",
	KindResultTooLarge:        "result_too_large",
	KindProtectedValue:        "protected_value",
	KindCancelled:             "cancelled",
	KindTimeout:               "timeout",
	KindBusy:                  "tab_busy",
	KindDialogBlocked:         "dialog_blocked",
	KindDialogNotFound:        "dialog_not_found",
	KindInvalidDialogAction:   "invalid_dialog_action",
	KindAmbiguous:             "ambiguous_target",
	KindStaleReference:        "stale_ref",
	KindInvalidResult:         "unsupported_result",
	KindConfiguration:         "invalid_input",
	KindIO:                    "io_error",
}

// ProtocolError preserves Chrome's numeric error code, message and optional data.
type ProtocolError struct {
	// Code holds Chrome's numeric rejection code.
	Code int64
	// Message holds Chrome's rejection text.
	Message string
	// Data retains optional rejection details.
	Data json.RawMessage
}

// Error describes Chrome's command rejection.
func (e *ProtocolError) Error() string { return fmt.Sprintf("%s (%d)", e.Message, e.Code) }

func (e *BrowserError) conditionError() string {
	condition := ""
	if e.Details.Condition != nil {
		condition = *e.Details.Condition
	}
	if e.Details.Interceptor == nil {
		return fmt.Sprintf("element condition failed: %s", condition)
	}
	return fmt.Sprintf("element condition failed: %s; interceptor: %s", condition, quoted(*e.Details.Interceptor))
}

func (e *BrowserError) driverError() string {
	switch e.Kind {
	case KindRoot:
		return "Chrome does not run as root on Linux with its sandbox, which Demi keeps: run the runner as " +
			"an ordinary user"
	case KindInstallation:
		return "Chrome for Testing " + e.Message
	case KindAction:
		return e.Cause.Error()
	case KindCancelled:
		return "browser operation was cancelled"
	case KindTimeout:
		return "browser operation exceeded its deadline"
	case KindBusy:
		return "another operation owns this browser tab"
	case KindDialogBlocked:
		return "browser action is blocked by a JavaScript dialog; inspect and handle the dialog before continuing"
	case KindDialogNotFound:
		return "the browser tab has no JavaScript dialog"
	case KindInvalidDialogAction:
		return "this action is not valid for the current JavaScript dialog"
	case KindAmbiguous:
		return fmt.Sprintf("browser target matched %d elements; exactly one is required", e.Count)
	case KindStaleReference:
		return "browser node reference is stale or belongs to another tab"
	case KindInvalidResult:
		return "browser result is not representable as JSON: " + e.Message
	case KindConfiguration:
		return "invalid browser configuration: " + e.Message
	case KindProfileRetained:
		return fmt.Sprintf("browser profile retained at %s: %v", e.Path, e.Cause)
	case KindCleanup:
		return fmt.Sprintf("browser cleanup failed: %v; preceding operation: %v", e.Cleanup, e.Cause)
	default:
		if e.Message != "" {
			return e.Message
		}
		if e.Cause != nil {
			return e.Cause.Error()
		}
		return e.Message
	}
}
