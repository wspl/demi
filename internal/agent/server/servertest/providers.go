package servertest

import (
	"context"
	"slices"
	"sync"

	"github.com/wspl/demi/internal/agent/server"
	"github.com/wspl/demi/internal/agent/store/storetest"
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
type ScriptedProviders struct {
	mu        sync.Mutex // Protects the configured runtimes, selection and call log.
	runtimes  map[string]provider.Runtime
	selection *core.ModelSelection
	calls     []Resolution
}

// Select chooses the model every conversation's record holds from now on.
func (p *ScriptedProviders) Select(model core.ModelSelection) {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.selection = &model
}

// Provide makes every runtime of id play script's shared sequence of turns.
func (p *ScriptedProviders) Provide(id string, script *providertest.ScriptedRuntime) {
	p.ProvideRuntime(id, script)
}

// ProvideRuntime supplies runtime.Fresh for each session using id.
// The test owns the template runtime; the server owns and closes each fresh one.
func (p *ScriptedProviders) ProvideRuntime(id string, runtime provider.Runtime) {
	p.mu.Lock()
	defer p.mu.Unlock()
	if p.runtimes == nil {
		p.runtimes = map[string]provider.Runtime{}
	}
	p.runtimes[id] = runtime
}

// Calls returns an owned snapshot of resolutions in request order.
func (p *ScriptedProviders) Calls() []Resolution {
	p.mu.Lock()
	defer p.mu.Unlock()
	return slices.Clone(p.calls)
}

// Selection returns the last selected model or the test default.
func (p *ScriptedProviders) Selection(_ context.Context, _ core.NodeID) (core.ModelSelection, error) {
	p.mu.Lock()
	defer p.mu.Unlock()
	if p.selection != nil {
		return *p.selection, nil
	}
	return storetest.TestModel(), nil
}

// Runtime records the resolution and supplies a fresh runtime, or server.ProviderUnavailable.
func (p *ScriptedProviders) Runtime(
	_ context.Context,
	root core.NodeID,
	model core.ModelSelection,
) (provider.Runtime, error) {
	p.mu.Lock()
	p.calls = append(p.calls, Resolution{Root: root, Provider: model.ProviderID})
	runtime := p.runtimes[model.ProviderID]
	p.mu.Unlock()
	if runtime == nil {
		return nil, server.ProviderUnavailable(model.ProviderID)
	}
	return runtime.Fresh(), nil
}

var _ server.ProviderResolver = (*ScriptedProviders)(nil)
