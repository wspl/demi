//revive:disable:unused-parameter API checkpoint retains parameter names; bodies follow after merge.

package jobstest

import (
	"context"
	"encoding/json"
	"testing"

	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/runner/cmdpkgs"
	"github.com/wspl/demi/internal/runner/jobs"
	"github.com/wspl/demi/internal/runner/process"
	"github.com/wspl/demi/internal/runnerwire"
)

// Dispatch supplies a dispatcher, local endpoint and connection owner backed
// by channels. Its cleanup joins workers and releases services and contexts.
type Dispatch struct {
	Services   *cmdpkgs.ServiceRegistry
	Dispatcher *jobs.Dispatcher
	Server     *jobs.Server
	// Outgoing carries encoded frames sent to the simulated backend.
	Outgoing <-chan []byte
}

// NewDispatch installs manifest in root and uses the test executable for aliases.
// It registers cleanup with t, including when construction fails. Pipes remains
// caller-owned and must outlive the fixture.
func NewDispatch(ctx context.Context, t testing.TB, root string, manifest json.RawMessage, pipes *process.PipeClient) *Dispatch {
	panic("not written: r-jobs")
}

// Context creates a live job context and a guard. The fixture also registers
// guard cleanup, so a failed test cannot leave a live registration.
func (d *Dispatch) Context(ctx context.Context, jobID string, command commandwire.CommandContext) (*jobs.ExecutionContext, *ContextGuard, error) {
	panic("not written: r-jobs")
}

// Deliver hands a validated backend message to the channel-backed owner.
func (d *Dispatch) Deliver(ctx context.Context, message runnerwire.Inbound) error {
	panic("not written: r-jobs")
}

// Close stops the endpoint, revokes contexts and joins the connection and services.
// It is idempotent and may be called before the registered test cleanup.
func (d *Dispatch) Close(ctx context.Context) error { panic("not written: r-jobs") }

// ContextGuard owns one fixture context registration and its alias directory.
type ContextGuard struct{}

// Close revokes the context and waits for its removal, then removes aliases.
// Callers first join invocations using it. Repeated calls are harmless.
func (g *ContextGuard) Close(ctx context.Context) error { panic("not written: r-jobs") }
