package commandservice

import (
	"context"
	"errors"
	"fmt"
)

// Errors of the wire. Callers tell them apart with [errors.Is] and
// [errors.As].
var (
	// ErrTooLarge means a payload exceeds the wire's limit for it.
	ErrTooLarge = errors.New("command protocol payload exceeds limit")
	// ErrUnknownRecord means a response record has a kind the wire does not
	// define.
	ErrUnknownRecord = errors.New("unknown response record kind")
	// ErrAfterCompletion means a response carries data after its completion
	// record.
	ErrAfterCompletion = errors.New("response contains data after completion")
	// ErrIncomplete means a response ended without its completion record.
	ErrIncomplete = errors.New("response ended without complete final status")
	// ErrCancellationDeadline means a handler did not return within the
	// cancellation grace after its invocation was cancelled. The service is
	// faulty: its process must exit so that its owner retires it.
	ErrCancellationDeadline = errors.New("handler exceeded cancellation deadline; retire the service process")
	// ErrConversationCleanup means a conversation release failed or ended
	// with a nonzero exit code. The service is faulty, as with
	// [ErrCancellationDeadline].
	ErrConversationCleanup = errors.New("conversation cleanup failed; retire the service process")
	// ErrHandshakeTimeout means the peer did not complete the HTTP/2
	// handshake in time.
	ErrHandshakeTimeout = errors.New("HTTP/2 service handshake timed out")
	// ErrNumbers means a draw of conversation numbers failed: the runner
	// refused it, or the numbers stream ended.
	ErrNumbers = errors.New("conversation numbers")
	// ErrMissingTarget means a package has no artifact for a target; the error
	// that wraps it names the target.
	ErrMissingTarget = errors.New("the package has no artifact")
)

// ErrCancelled means an invocation, or the stream carrying it, was cancelled,
// or a service's context ended before its handshake was complete. It matches
// [context.Canceled] as well.
var ErrCancelled error = cancelledError{}

type cancelledError struct{}

func (cancelledError) Error() string { return "command cancelled" }

func (cancelledError) Is(target error) bool { return target == context.Canceled }

// An InvalidError means a value from outside the process, or one about to
// leave it, breaks the rules of its wire type.
type InvalidError struct {
	// Reason says which field broke which rule, and never quotes the value: a
	// value may be a secret, and errors are logged.
	Reason string
}

func (e *InvalidError) Error() string {
	return "invalid command protocol value: " + e.Reason
}

// A RejectedError means the service refused an HTTP request before running it:
// invalid metadata (400), an unknown operation (404), a second numbers stream
// (409), or a service that is shutting down (503).
type RejectedError struct {
	Status int
}

func (e *RejectedError) Error() string {
	return fmt.Sprintf("service rejected HTTP request with status %d", e.Status)
}

// isCancellation reports whether err means a cancelled call rather than a
// failed one.
func isCancellation(err error) bool {
	return errors.Is(err, context.Canceled)
}
