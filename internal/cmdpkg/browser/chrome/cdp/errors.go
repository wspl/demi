package cdp

//revive:disable:unused-parameter API checkpoint: retain parameter names for dependent implementers.

import (
	"encoding/json"

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
func (e *BrowserError) Error() string { panic("not written: k-chrome-cdp") }

// Unwrap exposes causes without comparing their message strings.
func (e *BrowserError) Unwrap() []error { panic("not written: k-chrome-cdp") }

// Code returns the public failure code through wrappers.
func (e *BrowserError) Code() browserop.BrowserErrorCode { panic("not written: k-chrome-cdp") }

// ProtocolError preserves Chrome's numeric error code, message and optional data.
type ProtocolError struct {
	Code    int64
	Message string
	Data    json.RawMessage
}

// Error describes Chrome's command rejection.
func (e *ProtocolError) Error() string { panic("not written: k-chrome-cdp") }
