package usershard

import (
	"context"
	"fmt"
	"log/slog"

	"github.com/wspl/demi/internal/agent/server"
	"github.com/wspl/demi/internal/backend/hostaccess"
	"github.com/wspl/demi/internal/backend/providers"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/webapi"
)

type conversationProviders struct {
	shard *Shard
	rate  *providers.RequestRateLimit
}
type conversationRuntime struct {
	source       *conversationProviders
	conversation webapi.ConversationID
	entry        webapi.ProviderID
	selection    core.ModelSelection
	current      *currentRuntime
}
type currentRuntime struct {
	provider provider.Provider
	model    string
	runtime  provider.Runtime
}

func (p *conversationProviders) Selection(ctx context.Context, root core.NodeID) (core.ModelSelection, error) {
	record, err := p.shard.Control().Conversation(ctx, hostaccess.ConversationOf(root))
	if err != nil {
		return core.ModelSelection{}, &server.ResolveError{Kind: server.ResolveFailed, Message: err.Error(), Cause: err}
	}
	if record == nil || record.Model == nil {
		return core.ModelSelection{}, &server.ResolveError{Kind: server.ResolveFailed, Message: "The conversation has no model yet"}
	}
	return *record.Model, nil
}

func (p *conversationProviders) Runtime(ctx context.Context, root core.NodeID, model core.ModelSelection) (provider.Runtime, error) {
	id, err := webapi.ParseProviderID(model.ProviderID)
	if err != nil {
		return nil, &server.ResolveError{Kind: server.ResolveUnknown, Provider: model.ProviderID}
	}
	entry, err := p.shard.services.Vault.Visible(ctx, p.shard.user, id)
	if err != nil {
		return nil, &server.ResolveError{Kind: server.ResolveFailed, Message: err.Error(), Cause: err}
	}
	if entry == nil {
		return nil, &server.ResolveError{Kind: server.ResolveUnknown, Provider: model.ProviderID}
	}
	runtime := &conversationRuntime{source: p, conversation: hostaccess.ConversationOf(root), entry: id, selection: model}
	if failure := runtime.serve(ctx, *entry, model); failure != nil {
		return nil, &server.ResolveError{Kind: server.ResolveFailed, Message: failure.Message}
	}
	return runtime, nil
}

func (r *conversationRuntime) serve(ctx context.Context, entry providers.ProviderEntry, requested core.ModelSelection) *provider.Failure {
	if credential, ok := entry.Credential.(*providers.SubscriptionCredential); ok && credential.Active == nil {
		code := provider.AuthMissing
		return &provider.Failure{Message: "No subscription account configured", Code: &code}
	}
	selection, err := providers.ConfiguredSelection(entry, requested)
	if err != nil {
		return &provider.Failure{Message: err.Error()}
	}
	shard := r.source.shard
	built, err := shard.services.Assembly.ProviderFor(ctx, entry)
	if err != nil {
		return &provider.Failure{Message: err.Error()}
	}
	if r.current == nil || r.current.provider != built || r.current.model != selection.Model.ID {
		var runtime provider.Runtime
		if built.Capabilities().ProcessHost {
			runtime, err = shard.services.Assembly.ProcessRuntime(ctx, entry, entry.Active(), cloudPlacement{shard: shard, conversation: &r.conversation})
		} else {
			runtime, err = built.Runtime(provider.RuntimeEnv{HTTP: shard.http})
		}
		if err != nil {
			return &provider.Failure{Message: err.Error()}
		}
		metered := providers.NewMeteredRuntime(runtime, r.source.rate, providers.Ledger{Control: shard.Control(), User: shard.user, Conversation: r.conversation, Provider: r.entry})
		if err := r.Close(context.WithoutCancel(ctx)); err != nil {
			// Rust close cannot fail; retain its replacement behavior and report cleanup failures.
			slog.Warn("the replaced provider runtime did not close", "error", err)
		}
		r.current = &currentRuntime{provider: built, model: selection.Model.ID, runtime: metered}
	}
	r.selection = selection
	return nil
}

func (r *conversationRuntime) Run(ctx context.Context, request provider.InferenceRequest) provider.Run {
	return func(yield func(provider.Event) bool) {
		shard := r.source.shard
		entry, err := shard.services.Vault.Visible(ctx, shard.user, r.entry)
		if ctx.Err() != nil {
			return
		}
		var failure *provider.Failure
		switch {
		case err != nil:
			failure = &provider.Failure{Message: fmt.Sprintf("The provider entry could not be read: %v", err)}
		case entry == nil:
			// Runtime release is best effort; the removed entry is the refusal the user needs.
			_ = r.Close(context.WithoutCancel(ctx))
			failure = &provider.Failure{Message: fmt.Sprintf("Provider %q is no longer available to this conversation", r.entry)}
		default:
			requested := r.selection
			requested.Model.ID = request.ModelID
			requested.Model.OutputLimit = request.OutputLimit
			requested.Thinking = request.Thinking
			requested.ServiceTierID = request.ServiceTierID
			failure = r.serve(ctx, *entry, requested)
		}
		if ctx.Err() != nil {
			return
		}
		if failure != nil {
			yield(&provider.Error{Failure: *failure})
			return
		}
		request.OutputLimit = r.selection.Model.OutputLimit
		r.current.runtime.Run(ctx, request)(yield)
	}
}
func (r *conversationRuntime) Fresh() provider.Runtime {
	fresh := &conversationRuntime{source: r.source, conversation: r.conversation, entry: r.entry, selection: r.selection}
	if r.current != nil {
		fresh.current = &currentRuntime{provider: r.current.provider, model: r.current.model, runtime: r.current.runtime.Fresh()}
	}
	return fresh
}
func (r *conversationRuntime) Close(ctx context.Context) error {
	current := r.current
	r.current = nil
	if current == nil {
		return nil
	}
	return current.runtime.Close(ctx)
}
func (r *conversationRuntime) RequestLimits(model core.Model) provider.RequestLimits {
	if r.current == nil {
		return provider.RequestLimits{}
	}
	return r.current.runtime.RequestLimits(model)
}
