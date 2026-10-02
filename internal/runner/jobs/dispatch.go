//revive:disable:unused-parameter API checkpoint retains parameter names; bodies follow after merge.

package jobs

import (
	"context"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/runner/cmdpkgs"
	"github.com/wspl/demi/internal/runner/process"
)

// Dispatcher runs declared commands for local clients and shell builtins alike.
// It validates argv and holds JSON output until it passes the leaf's schema.
// Fields are configured before use and remain unchanged while serving calls.
type Dispatcher struct {
	Contexts *Contexts
	Services *cmdpkgs.ServiceHandle
	Pipes    *process.PipeClient
}

// Operations returns the raw command operation served by the dispatcher.
func (d *Dispatcher) Operations() []string { panic("not written: r-jobs") }

// Invoke authenticates the execution context, parses the declaration and routes
// native work to a service or an RPC call to the backend. Cancellation joins IO.
func (d *Dispatcher) Invoke(ctx context.Context, invocation cmdsdk.InvocationContext[commandwire.LocalInvocation]) (commandwire.Completion, error) {
	panic("not written: r-jobs")
}

var _ cmdsdk.Handler[commandwire.LocalInvocation] = (*Dispatcher)(nil)

// Reported presents a local invocation's end: failures other than cancellation
// are written to stderr and become exit 1. The registration's management handler
// uses the same reporting as declared commands.
func Reported(ctx context.Context, completion commandwire.Completion, err error, output *cmdsdk.Output) (commandwire.Completion, error) {
	panic("not written: r-jobs")
}
