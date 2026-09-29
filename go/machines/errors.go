package machines

import (
	"errors"
	"strings"

	"github.com/wspl/demi/go/machinesproto"
)

// Why an operation failed. The reply carries an error's own message, the summary;
// the log follows its causes, which for an error that gathers several failures
// names each of them ([Chain]).
var (
	// ErrWorkerStopped means the device's worker ended before it answered.
	ErrWorkerStopped = errors.New("the device's worker stopped")
	// ErrActiveWriter means a working pair cannot be published while a sandbox
	// may write it.
	ErrActiveWriter = errors.New("Cannot publish working disks with an active writer")
	// ErrNotRunning means the operation needs a running Cloud.
	ErrNotRunning = errors.New("Cloud is not running")
	// ErrNoWorkingManifest means a running Cloud has no working record.
	ErrNoWorkingManifest = errors.New("Cloud working manifest is missing")
)

// A MissingBaseError means a reset names a base that is not imported.
type MissingBaseError struct {
	Base machinesproto.BaseVersion
}

func (e *MissingBaseError) Error() string {
	return "Cloud base " + string(e.Base) + " is not imported"
}

// A partsError is an operation that failed in several places, such as a failed
// start and the failed cleanup after it. Its message is the summary the reply
// carries; each part, with its causes, is in the log.
type partsError struct {
	message string
	parts   []error
}

func (e *partsError) Error() string { return e.message }

func (e *partsError) Unwrap() []error { return e.parts }

func startAndCleanup(parts ...error) error {
	return &partsError{message: "Cloud start and cleanup failed; working storage retained", parts: parts}
}

func checkpointRecovery(parts []error) error {
	return &partsError{message: "Cloud checkpoint recovery failed", parts: parts}
}

func shutdownFailed(parts []error) error {
	return &partsError{message: "Cloud shutdown failed; working state retained", parts: parts}
}

// Chain returns an error's message and, for an error that gathers several
// failures, each of them with its causes, for the log: the reply carries only
// the first.
func Chain(err error) string {
	text := err.Error()
	var parts *partsError
	if !errors.As(err, &parts) {
		return text
	}
	chains := make([]string, len(parts.parts))
	for i, part := range parts.parts {
		chains[i] = Chain(part)
	}
	return text + ": " + strings.Join(chains, "; ")
}
