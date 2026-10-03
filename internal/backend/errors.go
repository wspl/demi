package backend

import (
	"fmt"
	"net/netip"
	"strings"
)

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
	// Kind identifies the failed operation.
	Kind StartErrorKind
	// Path names the path involved in the failed operation.
	Path string
	// Address is the listener address that could not be bound.
	Address netip.AddrPort
	// Err is the underlying failure, when present.
	Err error
}

// Error describes the failed startup step.
func (e *StartError) Error() string {
	switch e.Kind {
	case StartDataDirectory:
		return fmt.Sprintf("the data directory %s cannot be created: %v", e.Path, e.Err)
	case StartSecret, StartServices:
		return e.Err.Error()
	case StartStorage:
		return fmt.Sprintf("storage cannot be opened: %v", e.Err)
	case StartObjects:
		return fmt.Sprintf("the object store cannot be opened: %v", e.Err)
	case StartObjectStore:
		return fmt.Sprintf("DEMI_OBJECT_STORE_CONFIG cannot be used: %v", e.Err)
	case StartShards:
		return fmt.Sprintf("the shard threads cannot start: %v", e.Err)
	case StartListen:
		return fmt.Sprintf("the backend cannot listen on %s: %v", e.Address, e.Err)
	case StartCloud:
		return fmt.Sprintf("the Clouds cannot be recovered: %v", e.Err)
	}
	return fmt.Sprint(e.Err)
}

// Unwrap returns the underlying startup failure.
func (e *StartError) Unwrap() error { return e.Err }

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
	// Kind identifies the failed operation.
	Kind ShutdownErrorKind
	// Err is the underlying failure, when present.
	Err error
}

// Error describes the failed shutdown step.
func (e *ShutdownError) Error() string {
	switch e.Kind {
	case ShutdownEdge:
		return fmt.Sprintf("the listener did not stop cleanly: %v", e.Err)
	case ShutdownStorage:
		return e.Err.Error()
	case ShutdownCloud:
		return fmt.Sprintf("a Cloud was not saved: %v", e.Err)
	case ShutdownMachines:
		return fmt.Sprintf("the machine manager did not reconcile: %v", e.Err)
	}
	return fmt.Sprint(e.Err)
}

// Unwrap returns the underlying shutdown failure.
func (e *ShutdownError) Unwrap() error { return e.Err }

// ShutdownErrors holds every shutdown step that failed, in shutdown order.
type ShutdownErrors struct {
	// Failures contains failed shutdown steps in shutdown order.
	Failures []*ShutdownError
}

// Error describes every failed step in shutdown order.
func (e *ShutdownErrors) Error() string {
	steps := make([]string, len(e.Failures))
	for i, failure := range e.Failures {
		steps[i] = failure.Error()
	}
	return "shutdown failed: " + strings.Join(steps, "; ")
}

// Unwrap exposes every failure to errors.Is and errors.As.
func (e *ShutdownErrors) Unwrap() []error {
	result := make([]error, len(e.Failures))
	for i, failure := range e.Failures {
		result[i] = failure
	}
	return result
}
