package claude

import (
	"context"
	"errors"

	"github.com/wspl/demi/go/claudeproto"
)

// An EnsureError says why an operation failed. Its Code is what the caller
// branches on. A cancelled operation is not one: it ends with the error of its
// context, and its invocation ends without an answer.
type EnsureError struct {
	Code    claudeproto.ErrorCode
	Message string
}

func (e *EnsureError) Error() string {
	switch e.Code {
	case claudeproto.InvalidRelease:
		return "invalid release record: " + e.Message
	case claudeproto.InstallFailed:
		return "Claude Code installation failed: " + e.Message
	}
	return e.Message
}

func invalidRelease(message string) error {
	return &EnsureError{Code: claudeproto.InvalidRelease, Message: message}
}

func unsupportedPlatform(message string) error {
	return &EnsureError{Code: claudeproto.UnsupportedPlatform, Message: message}
}

func downloadFailed(message string) error {
	return &EnsureError{Code: claudeproto.DownloadFailed, Message: message}
}

func verificationFailed(message string) error {
	return &EnsureError{Code: claudeproto.VerificationFailed, Message: message}
}

// installFailed makes err, a failure of the disk or of the package artifact, an
// installation that failed; a cancellation and an [*EnsureError] stay as they
// are.
func installFailed(err error) error {
	var ensure *EnsureError
	if errors.As(err, &ensure) || isCancellation(err) {
		return err
	}
	return &EnsureError{Code: claudeproto.InstallFailed, Message: err.Error()}
}

// isCancellation reports whether err is the end of a cancelled context.
func isCancellation(err error) bool {
	return errors.Is(err, context.Canceled)
}
