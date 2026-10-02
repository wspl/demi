//revive:disable:unused-parameter API checkpoint retains parameter names for callers; bodies follow after merge.

package cmdpkgs

import "os"

// RuntimeError identifies why a service could not be acquired or why it ended.
// Cause is available through errors.Is and errors.As.
type RuntimeError struct {
	Kind   ErrorKind
	Cause  error
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
func (e *RuntimeError) Error() string { panic("not written: r-cmdpkgs") }

// Unwrap returns the underlying artifact, IO, protocol or exit error.
func (e *RuntimeError) Unwrap() error { panic("not written: r-cmdpkgs") }

// ServiceExit reports a service's end and the tail of its standard error.
type ServiceExit struct {
	Service string
	Reason  ExitReason
	Stderr  string
}

// Error describes the exit and includes nonempty standard error.
func (e *ServiceExit) Error() string { panic("not written: r-cmdpkgs") }

// ExitReason identifies natural exit, protocol failure or startup deadline.
type ExitReason struct {
	Kind ExitKind
	// State preserves the OS exit status for a natural exit.
	State *os.ProcessState
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
func (r ExitReason) String() string { panic("not written: r-cmdpkgs") }
