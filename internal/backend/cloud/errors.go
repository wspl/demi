package cloud

import (
	"fmt"
	"log/slog"

	"github.com/wspl/demi/internal/webapiproto"
)

// ErrorKind identifies a Cloud admission or transition failure.
type ErrorKind uint8

const (
	// Closed means the Cloud is shutting down.
	Closed ErrorKind = iota
	// CrashLoop means its runtime was lost too often in a short time.
	CrashLoop
	// AtCapacity means every cross-user capacity permit is held.
	AtCapacity
	// Resetting means another reset holds the Cloud.
	Resetting
	// Failed means a boot, save, reset, storage operation or manager call failed.
	Failed
)

// Error is why the Cloud admits no operation or a transition failed. Use
// errors.As to inspect Kind and errors.Is/As through Unwrap for its cause.
type Error struct {
	// Kind identifies the failure category.
	Kind ErrorKind
	// Err is the underlying failure for Failed, including its user-facing text.
	Err error
}

// Error returns the refusal's product text, or the cause's text for Failed.
func (e *Error) Error() string {
	switch e.Kind {
	case Closed:
		return "Cloud is shutting down"
	case CrashLoop:
		return "Cloud repeatedly failed; reset the environment to recover"
	case AtCapacity:
		return "Cloud capacity is currently full; retry later"
	case Resetting:
		return "Another reset is in progress"
	default:
		return e.Err.Error()
	}
}

// Unwrap returns the underlying failure.
func (e *Error) Unwrap() error { return e.Err }

// Code returns the web error code and HTTP status for a refused operation.
func (e *Error) Code() (webapiproto.ErrorCode, int) {
	switch e.Kind {
	case Closed:
		return webapiproto.ErrorCodeBackendClosing, 503
	case CrashLoop:
		return webapiproto.ErrorCodeCloudCrashLoop, 503
	case AtCapacity:
		return webapiproto.ErrorCodeCloudCapacity, 503
	case Resetting:
		return webapiproto.ErrorCodeCloudResetting, 409
	default:
		return webapiproto.ErrorCodeCloudUnavailable, 503
	}
}

// failed preserves the cause of a Cloud transition failure.
func failed(err error) error {
	if err == nil {
		return nil
	}
	return &Error{Kind: Failed, Err: err}
}

// storageFailed preserves the storage cause with the Cloud's user-facing diagnostic.
func storageFailed(err error) error {
	if err == nil {
		return nil
	}
	slog.Error("the Cloud's records failed", "error", err)
	//nolint:staticcheck // Product text, shown to the user as it is.
	return failed(fmt.Errorf("The Cloud's records could not be read or written: %w", err))
}
