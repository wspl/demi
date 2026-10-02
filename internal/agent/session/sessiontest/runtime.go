package sessiontest

import (
	"context"

	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/provider"
)

// Runtime supplies session hooks for tests. Set hooks before sharing it with a
// session. A waiting hook must honor its context; the session joins its caller.
type Runtime struct {
	Enter       func(context.Context) (*gates.Lease, error)
	Reserve     func(context.Context) (*gates.Reservation, error)
	Prompt      string
	Before      func(context.Context) (*string, error)
	News        func(context.Context, []session.SeenContext, core.TurnID) ([]session.NewContext, error)
	Definitions []provider.ToolDefinition
	Invoke      func(context.Context, session.ToolInvocation) (session.ToolOutcome, error)
	Close       func(context.Context) error
}

// EnterAction delegates admission or admits immediately.
func (r *Runtime) EnterAction(ctx context.Context) (*gates.Lease, error) {
	if r.Enter != nil {
		return r.Enter(ctx)
	}
	return nil, ctx.Err()
}

// ReserveEdit delegates child reservation or needs none.
func (r *Runtime) ReserveEdit(ctx context.Context) (*gates.Reservation, error) {
	if r.Reserve != nil {
		return r.Reserve(ctx)
	}
	return nil, ctx.Err()
}

// SystemPrompt returns the configured prompt.
func (r *Runtime) SystemPrompt(ctx context.Context) (string, error) { return r.Prompt, ctx.Err() }

// Preamble delegates the configured hook or returns no preamble.
func (r *Runtime) Preamble(ctx context.Context) (*string, error) {
	if r.Before != nil {
		return r.Before(ctx)
	}
	return nil, ctx.Err()
}

// Context delegates context collection or supplies no new context.
func (r *Runtime) Context(ctx context.Context, seen []session.SeenContext, turn core.TurnID) ([]session.NewContext, error) {
	if r.News != nil {
		return r.News(ctx, seen, turn)
	}
	return nil, ctx.Err()
}

// Tools returns the immutable definitions supplied by the test.
func (r *Runtime) Tools() []provider.ToolDefinition { return r.Definitions }

// InvokeTool delegates a call or completes it with empty output.
func (r *Runtime) InvokeTool(ctx context.Context, call session.ToolInvocation) (session.ToolOutcome, error) {
	if r.Invoke != nil {
		return r.Invoke(ctx, call)
	}
	return session.ToolOutcome{Output: []provider.ResultPart{}}, ctx.Err()
}

// Dispose delegates cleanup or has nothing to release.
func (r *Runtime) Dispose(ctx context.Context) error {
	if r.Close != nil {
		return r.Close(ctx)
	}
	return ctx.Err()
}

var _ session.Runtime = (*Runtime)(nil)
