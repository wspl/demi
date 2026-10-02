//revive:disable:unused-parameter // API checkpoint keeps parameter names for callers; bodies follow after merge.
package providers

import (
	"context"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/webapi"
)

// RequestsPerWindow is the requests a user may start in sixty seconds.
const RequestsPerWindow = 120

// RequestRateLimit holds one user's requests of the last minute. Share its pointer.
type RequestRateLimit struct{}

// Ledger identifies the user, conversation and entry of a runtime's requests.
type Ledger struct {
	Control      *database.ControlService
	User         webapi.UserID
	Conversation webapi.ConversationID
	Provider     webapi.ProviderID
}

// MeteredRuntime wraps a session runtime with the user's rate limit and ledger.
// Forks share both; each response reaches the ledger before the agent sees it.
type MeteredRuntime struct{}

// NewRequestRateLimit creates a rolling one-minute request window.
func NewRequestRateLimit(limit int) *RequestRateLimit { panic("not written: b-providers") }

// Take admits and counts a request now, or refuses without counting it.
func (r *RequestRateLimit) Take() error { panic("not written: b-providers") }

// NewMeteredRuntime wraps a runtime with shared admission and accounting.
func NewMeteredRuntime(inner provider.Runtime, limit *RequestRateLimit, ledger Ledger) *MeteredRuntime {
	panic("not written: b-providers")
}

// Run refuses over-limit requests before they reach the vendor.
func (r *MeteredRuntime) Run(ctx context.Context, request provider.InferenceRequest) provider.Run {
	panic("not written: b-providers")
}

// Fresh forks the runtime while sharing its limit and ledger.
func (r *MeteredRuntime) Fresh() provider.Runtime { panic("not written: b-providers") }

// Close releases the wrapped runtime resources.
func (r *MeteredRuntime) Close(ctx context.Context) error { panic("not written: b-providers") }

// RequestLimits returns the wrapped runtime request limits.
func (r *MeteredRuntime) RequestLimits(model core.Model) provider.RequestLimits {
	panic("not written: b-providers")
}
