package server

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"context"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

// ProviderResolver supplies a conversation's model selection and the runtimes
// its sessions infer with. The agent never sees the provider itself.
type ProviderResolver interface {
	// Selection returns the selection held in the conversation's record.
	Selection(ctx context.Context, root core.NodeID) (core.ModelSelection, error)
	// Runtime builds a runtime for a session of root that infers with model.
	// Ownership passes to the caller, which must close it.
	Runtime(ctx context.Context, root core.NodeID, model core.ModelSelection) (provider.Runtime, error)
}
