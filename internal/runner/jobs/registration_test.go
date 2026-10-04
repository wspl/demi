package jobs

import (
	"context"
	"errors"
	"testing"
)

// TestCancelledContextCannotRegisterAfterOwnerCheck forces this ordering:
// owner checks the caller, caller abandons its wait, owner attempts insertion.
// The old uncoordinated insertion leaks live authority and transfers leases to
// a table after their caller has reclaimed them. Register must make that impossible.
func TestCancelledContextCannotRegisterAfterOwnerCheck(t *testing.T) {
	connection, cancelConnection := context.WithCancel(t.Context())
	defer cancelConnection()
	handle, requests := NewConnection(connection, make(chan []byte, 1))
	contexts := &Contexts{}
	table := NewContextTable(contexts)
	defer table.Close()
	caller, cancelCaller := context.WithCancel(t.Context())
	defer cancelCaller()
	executionCtx, cancelExecution := context.WithCancel(t.Context())
	defer cancelExecution()
	execution := &ExecutionContext{ID: "context", JobID: "job", lifetime: executionCtx, cancel: cancelExecution}
	outcome := make(chan error, 1)
	go func() {
		outcome <- handle.RegisterContext(caller, execution, nil)
	}()
	request := (<-requests).(*ContextRequest)
	// This is the connection owner's pre-registration check in the old ordering.
	if err := caller.Err(); err != nil {
		t.Fatal(err)
	}
	cancelCaller()
	if err := <-outcome; !errors.Is(err, context.Canceled) {
		t.Fatalf("registration did not abandon: %v", err)
	}
	request.Register(table)
	if _, err := contexts.Lookup(execution.ID); err == nil {
		t.Fatal("cancelled context became live after the owner's check")
	}
	// A subsequent live registration of the same job must still succeed.
	next := make(chan error, 1)
	go func() {
		next <- handle.RegisterContext(t.Context(), execution, nil)
	}()
	(<-requests).(*ContextRequest).Register(table)
	if err := <-next; err != nil {
		t.Fatal(err)
	}
	if found, err := contexts.Lookup(execution.ID); err != nil || found != execution {
		t.Fatalf("live registration missing: %v", err)
	}
}
