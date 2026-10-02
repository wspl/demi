//revive:disable:unused-parameter API checkpoint retains parameter names for callers; bodies follow after merge.

package cmdpkgstest

import (
	"context"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
)

// NoNumbers refuses requests as a backend does for a conversation not on the device.
type NoNumbers struct{}

// Reserve refuses every request with "this test gives out no conversation numbers".
func (NoNumbers) Reserve(ctx context.Context, conversation string, sequence commandwire.ServiceSequence, count uint32) (uint64, error) {
	panic("not written: r-cmdpkgs")
}

// Fixture supplies the deliberately faulty demi-native-fixture service operations.
// Its zero value is ready to pass to cmdsdk.ServeStdio. Crash and spin operations
// must only run in the fixture child process, never in the test's own process.
type Fixture struct{}

// Operations returns the shared commandwiretest fixture operation catalog.
func (f *Fixture) Operations() []string { panic("not written: r-cmdpkgs") }

// SetNumbers supplies this connection's conversation number source.
func (f *Fixture) SetNumbers(numbers *cmdsdk.Numbers) { panic("not written: r-cmdpkgs") }

// Invoke runs a fixture operation with its supplied IO and cancellation.
func (f *Fixture) Invoke(ctx context.Context, invocation cmdsdk.InvocationContext[commandwire.Invocation]) (commandwire.Completion, error) {
	panic("not written: r-cmdpkgs")
}

// Conversation answers retained-state queries and exercises controlled release failures.
func (f *Fixture) Conversation(ctx context.Context, conversation cmdsdk.ConversationContext) (commandwire.Completion, error) {
	panic("not written: r-cmdpkgs")
}
