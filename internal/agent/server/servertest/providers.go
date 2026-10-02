package servertest

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"context"

	"github.com/wspl/demi/internal/agent/server"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
)

// Resolution records a conversation and provider requested by a test.
type Resolution struct {
	Root     core.NodeID
	Provider string
}

// ScriptedProviders supplies runtimes by provider id. Every runtime of one
// provider plays its script; a provider without one is unknown. Every root
// opens with the last selected model, or storetest.TestModel's default selection.
// Its zero value is ready for use and its methods support concurrent calls.
type ScriptedProviders struct{}

// Select chooses the model every conversation's record holds from now on.
func (p *ScriptedProviders) Select(model core.ModelSelection) { panic("not written: a-server") }

// Provide makes every runtime of id play script's shared sequence of turns.
func (p *ScriptedProviders) Provide(id string, script *providertest.ScriptedRuntime) {
	panic("not written: a-server")
}

// ProvideRuntime supplies runtime.Fresh for each session using id.
// The test owns the template runtime; the server owns and closes each fresh one.
func (p *ScriptedProviders) ProvideRuntime(id string, runtime provider.Runtime) {
	panic("not written: a-server")
}

// Calls returns an owned snapshot of resolutions in request order.
func (p *ScriptedProviders) Calls() []Resolution { panic("not written: a-server") }

// Selection returns the last selected model or the test default.
func (p *ScriptedProviders) Selection(ctx context.Context, root core.NodeID) (core.ModelSelection, error) {
	panic("not written: a-server")
}

// Runtime records the resolution and supplies a fresh runtime, or *server.ResolveError.
func (p *ScriptedProviders) Runtime(ctx context.Context, root core.NodeID, model core.ModelSelection) (provider.Runtime, error) {
	panic("not written: a-server")
}

var _ server.ProviderResolver = (*ScriptedProviders)(nil)
