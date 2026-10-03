package backend

import "net/netip"

// StartErrorKind identifies the startup step that failed.
type StartErrorKind uint8

const (
	// StartDataDirectory means the data directory could not be created.
	StartDataDirectory StartErrorKind = iota
	// StartSecret means the instance secret could not be loaded.
	StartSecret
	// StartStorage means the databases could not be opened.
	StartStorage
	// StartObjects means the object store could not be opened.
	StartObjects
	// StartObjectStore means the configured S3 file could not be used.
	StartObjectStore
	// StartServices means shared services could not be started.
	StartServices
	// StartShards means shard routing could not be started.
	StartShards
	// StartListen means the listener could not be opened.
	StartListen
	// StartCloud means machine reconciliation or reset recovery failed.
	StartCloud
)

// StartError is why the backend did not start.
type StartError struct {
	Kind    StartErrorKind
	Path    string
	Address netip.AddrPort
	Err     error
}

// Error describes the failed startup step.
func (e *StartError) Error() string { panic("not written: b-backend") }

// Unwrap returns the underlying startup failure.
func (e *StartError) Unwrap() error { panic("not written: b-backend") }

// ShutdownErrorKind identifies the shutdown step that failed.
type ShutdownErrorKind uint8

const (
	// ShutdownEdge means the listener did not stop cleanly.
	ShutdownEdge ShutdownErrorKind = iota
	// ShutdownStorage means storage did not close cleanly.
	ShutdownStorage
	// ShutdownCloud means a user's Cloud could not be saved.
	ShutdownCloud
	// ShutdownMachines means the machine manager did not reconcile at close.
	ShutdownMachines
)

// ShutdownError is a shutdown step that failed; later steps still ran.
type ShutdownError struct {
	Kind ShutdownErrorKind
	Err  error
}

// Error describes the failed shutdown step.
func (e *ShutdownError) Error() string { panic("not written: b-backend") }

// Unwrap returns the underlying shutdown failure.
func (e *ShutdownError) Unwrap() error { panic("not written: b-backend") }

// ShutdownErrors holds every shutdown step that failed, in shutdown order.
type ShutdownErrors struct{ Failures []*ShutdownError }

// Error describes every failed step in shutdown order.
func (e *ShutdownErrors) Error() string { panic("not written: b-backend") }

// Unwrap exposes every failure to errors.Is and errors.As.
func (e *ShutdownErrors) Unwrap() []error { panic("not written: b-backend") }
