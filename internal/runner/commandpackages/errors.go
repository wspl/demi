package commandpackages

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"unicode"

	"github.com/wspl/demi/internal/runner/process"
)

// RuntimeError identifies why a service could not be acquired or why it ended.
// Cause is available through errors.Is and errors.As.
type RuntimeError struct {
	// Kind classifies the runtime failure.
	Kind ErrorKind
	// Cause preserves the underlying failure.
	Cause error
	// Detail supplies the category-specific diagnostic.
	Detail string
}

// ErrorKind identifies a runtime failure category.
type ErrorKind uint8

const (
	// ArtifactFailure means copying, downloading or verifying failed.
	ArtifactFailure ErrorKind = iota
	// LocationFailure means the backend location was absent or invalid.
	LocationFailure
	// Cancelled means the runtime operation was cancelled.
	Cancelled
	// IOFailure means process or filesystem IO failed.
	IOFailure
	// ServiceFailure means the command protocol failed.
	ServiceFailure
	// CatalogMismatch means the service does not match its package descriptor.
	CatalogMismatch
	// Deadline means a startup phase exceeded its deadline, named by Detail.
	Deadline
	// Stopped means the registry shut the service down.
	Stopped
	// Exited means the service ended on its own; Cause is a ServiceExit.
	Exited
)

// Error describes the runtime failure.
func (e *RuntimeError) Error() string {
	switch e.Kind {
	case ArtifactFailure:
		return fmt.Sprintf("native artifact: %v", e.Cause)
	case LocationFailure:
		return "native artifact location: " + e.Detail
	case Cancelled:
		return "native runtime was cancelled"
	case CatalogMismatch:
		return "native service does not match its package descriptor"
	case Deadline:
		return fmt.Sprintf("native service did not answer within its %s deadline", e.Detail)
	case Stopped:
		return "native service was shut down"
	default:
		if e.Cause != nil {
			return e.Cause.Error()
		}
		return e.Detail
	}
}

// Unwrap returns the underlying artifact, IO, protocol or exit error.
func (e *RuntimeError) Unwrap() error {
	return e.Cause
}

// ServiceExit reports a service's end and the tail of its standard error.
type ServiceExit struct {
	// Service identifies the service that ended.
	Service string
	// Reason describes how the service ended.
	Reason ExitReason
	// Stderr is the retained standard error tail.
	Stderr string
}

// Error describes the exit and includes nonempty standard error.
func (e *ServiceExit) Error() string {
	text := fmt.Sprintf("native service %s %s", e.Service, e.Reason)
	if tail := strings.TrimRightFunc(e.Stderr, unicode.IsSpace); tail != "" {
		text += "; its standard error ended with:\n" + tail
	}
	return text
}

// ExitReason identifies natural exit, protocol failure or startup deadline.
type ExitReason struct {
	// Kind classifies how the service ended.
	Kind ExitKind
	// State preserves the process owner’s exit record for a natural exit.
	State process.Exit
	// WaitErr is the process owner's wait failure, shown when no OS status was recorded.
	WaitErr error
	// Detail is the protocol diagnostic or startup phase for a forced stop.
	Detail string
}

// ExitKind identifies how a resident service ended.
type ExitKind uint8

const (
	// ProcessExited means the process ended on its own.
	ProcessExited ExitKind = iota
	// ProtocolBroken means the runner stopped a faulty service.
	ProtocolBroken
	// StartupDeadline means the runner stopped a service missing a startup deadline.
	StartupDeadline
)

// String describes how the service ended.
func (r ExitReason) String() string {
	switch r.Kind {
	case ProcessExited:
		return "exited with " + serviceExitStatus(r.State, r.WaitErr)
	case ProtocolBroken:
		return fmt.Sprintf("broke the protocol (%s) and was stopped", r.Detail)
	case StartupDeadline:
		return fmt.Sprintf("did not answer within its %s deadline and was stopped", r.Detail)
	default:
		return r.Detail
	}
}

// runtimeFailure retains causes while exposing the runtime's public error category.
func runtimeFailure(err error) error {
	if err == nil {
		return nil
	}
	var failure *RuntimeError
	if errors.As(err, &failure) {
		return err
	}
	var exit *ServiceExit
	if errors.As(err, &exit) {
		return &RuntimeError{Kind: Exited, Cause: err}
	}
	if errors.Is(err, context.Canceled) {
		return &RuntimeError{Kind: Cancelled, Cause: err}
	}
	return &RuntimeError{Kind: IOFailure, Cause: err}
}
