package cdp

import (
	"encoding/json"
	"fmt"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
)

// ErrorKind distinguishes browser failures before mapping to wire error codes.
type ErrorKind uint8

const (
	// KindUnsupportedCapability identifies the Rust BrowserError::UnsupportedCapability failure.
	KindUnsupportedCapability ErrorKind = iota + 1
	// KindStaleInventory identifies the Rust BrowserError::StaleInventory failure.
	KindStaleInventory
	// KindStaleTools identifies the Rust BrowserError::StaleTools failure.
	KindStaleTools
	// KindStaleCursor identifies the Rust BrowserError::StaleCursor failure.
	KindStaleCursor
	// KindCDPMethodDenied identifies the Rust BrowserError::CDPMethodDenied failure.
	KindCDPMethodDenied
	// KindPartialFailure identifies the Rust BrowserError::PartialFailure failure.
	KindPartialFailure
	// KindSideEffectRejected identifies the Rust BrowserError::SideEffectRejected failure.
	KindSideEffectRejected
	// KindClosed identifies the Rust BrowserError::Closed failure.
	KindClosed
	// KindConnection identifies the Rust BrowserError::Connection failure.
	KindConnection
	// KindOutcomeUnknown identifies the Rust BrowserError::OutcomeUnknown failure.
	KindOutcomeUnknown
	// KindTabNotFound identifies the Rust BrowserError::TabNotFound failure.
	KindTabNotFound
	// KindTargetNotFound identifies the Rust BrowserError::TargetNotFound failure.
	KindTargetNotFound
	// KindNotActionable identifies the Rust BrowserError::NotActionable failure.
	KindNotActionable
	// KindHistoryBoundary identifies the Rust BrowserError::HistoryBoundary failure.
	KindHistoryBoundary
	// KindNavigationFailed identifies the Rust BrowserError::NavigationFailed failure.
	KindNavigationFailed
	// KindOutputExists identifies the Rust BrowserError::OutputExists failure.
	KindOutputExists
	// KindResultTooLarge identifies the Rust BrowserError::ResultTooLarge failure.
	KindResultTooLarge
	// KindProtectedValue identifies the Rust BrowserError::ProtectedValue failure.
	KindProtectedValue
	// KindUnavailable identifies the Rust BrowserError::Unavailable failure.
	KindUnavailable
	// KindRoot identifies the Rust BrowserError::Root failure.
	KindRoot
	// KindInstallation identifies the Rust BrowserError::Installation failure.
	KindInstallation
	// KindAction identifies the Rust BrowserError::Action failure.
	KindAction
	// KindCancelled identifies the Rust BrowserError::Cancelled failure.
	KindCancelled
	// KindTimeout identifies the Rust BrowserError::Timeout failure.
	KindTimeout
	// KindBusy identifies the Rust BrowserError::Busy failure.
	KindBusy
	// KindDialogBlocked identifies the Rust BrowserError::DialogBlocked failure.
	KindDialogBlocked
	// KindDialogNotFound identifies the Rust BrowserError::DialogNotFound failure.
	KindDialogNotFound
	// KindInvalidDialogAction identifies the Rust BrowserError::InvalidDialogAction failure.
	KindInvalidDialogAction
	// KindAmbiguous identifies the Rust BrowserError::Ambiguous failure.
	KindAmbiguous
	// KindStaleReference identifies the Rust BrowserError::StaleReference failure.
	KindStaleReference
	// KindInvalidResult identifies the Rust BrowserError::InvalidResult failure.
	KindInvalidResult
	// KindConfiguration identifies the Rust BrowserError::Configuration failure.
	KindConfiguration
	// KindCDP identifies the Rust BrowserError::Cdp failure.
	KindCDP
	// KindEvents identifies the Rust BrowserError::Events failure.
	KindEvents
	// KindIO identifies the Rust BrowserError::IO failure.
	KindIO
	// KindProfileRetained identifies the Rust BrowserError::ProfileRetained failure.
	KindProfileRetained
	// KindTask identifies the Rust BrowserError::Task failure.
	KindTask
	// KindCleanup identifies the Rust BrowserError::Cleanup failure.
	KindCleanup
)

// BrowserError retains a browser failure and its underlying cause. Message holds
// the variant's text payload, Count its ambiguous match count, Path its retained
// profile, and Details its action, condition or export data. Cleanup retains both
// the preceding Cause (possibly nil) and Cleanup; errors.Is/As reach both.
type BrowserError struct {
	Kind    ErrorKind
	Message string
	Count   uint
	Path    string
	Details browserop.ErrorDetails
	Cause   error
	Cleanup error
}

// Error returns the Rust browser failure's user-facing message.
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
		condition := ""
		if e.Details.Condition != nil {
			condition = *e.Details.Condition
		}
		interceptor := "None"
		if e.Details.Interceptor != nil {
			interceptor = "Some(" + quoted(*e.Details.Interceptor) + ")"
		}
		return fmt.Sprintf("element condition failed: %s; interceptor: %s", condition, interceptor)
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
	case KindRoot:
		return "Chrome does not run as root on Linux with its sandbox, which Demi keeps: run the runner as an ordinary user"
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
	case KindTask:
		return fmt.Sprintf("browser event task failed: %v", e.Cause)
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
func (e *BrowserError) Code() browserop.BrowserErrorCode {
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
	case KindCDP, KindEvents, KindTask:
		return "driver_error"
	default:
		return errorCodes[e.Kind]
	}
}

var errorCodes = map[ErrorKind]browserop.BrowserErrorCode{
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
	Code    int64
	Message string
	Data    json.RawMessage
}

// Error describes Chrome's command rejection.
func (e *ProtocolError) Error() string { return fmt.Sprintf("%s (%d)", e.Message, e.Code) }
