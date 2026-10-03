package providers

import (
	"context"
	"log/slog"
	"sync"
	"time"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/webapi"
)

// RequestsPerWindow is the requests a user may start in sixty seconds.
const RequestsPerWindow = 120

// RequestRateLimit holds one user's requests of the last minute. Share its pointer.
type RequestRateLimit struct {
	// mu protects atomic admission across all of the user's runtimes.
	mu      sync.Mutex
	limit   int
	started []time.Time
}

// Ledger identifies the user, conversation and entry of a runtime's requests.
type Ledger struct {
	// Control persists the request ledger.
	Control *database.ControlService
	// User identifies the user whose request is accounted for.
	User webapi.UserID
	// Conversation identifies the conversation whose request is accounted for.
	Conversation webapi.ConversationID
	// Provider identifies the provider entry bound to this record.
	Provider webapi.ProviderID
}

// MeteredRuntime wraps a session runtime with the user's rate limit and ledger.
// Forks share both; each response reaches the ledger before the agent sees it.
type MeteredRuntime struct {
	inner  provider.Runtime
	limit  *RequestRateLimit
	ledger Ledger
}

// NewRequestRateLimit creates a rolling one-minute request window.
func NewRequestRateLimit(limit int) *RequestRateLimit {
	return &RequestRateLimit{limit: limit}
}

// Take admits and counts a request now, or refuses without counting it.
func (r *RequestRateLimit) Take() error {
	r.mu.Lock()
	defer r.mu.Unlock()
	now := time.Now()
	i := 0
	for i < len(r.started) && now.Sub(r.started[i]) >= time.Minute {
		i++
	}
	r.started = r.started[i:]
	if len(r.started) >= r.limit {
		return &RateLimited{Limit: r.limit}
	}
	r.started = append(r.started, now)
	return nil
}

// NewMeteredRuntime wraps a runtime with shared admission and accounting.
func NewMeteredRuntime(inner provider.Runtime, limit *RequestRateLimit, ledger Ledger) *MeteredRuntime {
	return &MeteredRuntime{inner: inner, limit: limit, ledger: ledger}
}

// Run refuses over-limit requests before they reach the vendor.
func (r *MeteredRuntime) Run(ctx context.Context, request provider.InferenceRequest) provider.Run {
	if err := r.limit.Take(); err != nil {
		return func(yield func(provider.Event) bool) {
			code := provider.RateLimited
			yield(&provider.Error{
				Failure: provider.Failure{Message: err.Error(), Code: &code},
			})
		}
	}
	run := r.inner.Run(ctx, request)
	return func(yield func(provider.Event) bool) {
		for event := range run {
			if response, ok := event.(*provider.Response); ok {
				row := database.UsageRow{
					User:         r.ledger.User,
					Conversation: r.ledger.Conversation,
					Provider:     r.ledger.Provider,
					Model:        request.ModelID,
					Usage:        response.Usage,
				}
				if err := r.ledger.Control.AppendUsage(context.WithoutCancel(ctx), row); err != nil {
					slog.Warn("a usage ledger row was not written", "error", err)
				}
			}
			if !yield(event) {
				return
			}
		}
	}
}

// Fresh forks the runtime while sharing its limit and ledger.
func (r *MeteredRuntime) Fresh() provider.Runtime {
	return NewMeteredRuntime(r.inner.Fresh(), r.limit, r.ledger)
}

// Close releases the wrapped runtime resources.
func (r *MeteredRuntime) Close(ctx context.Context) error {
	return r.inner.Close(ctx)
}

// RequestLimits returns the wrapped runtime request limits.
func (r *MeteredRuntime) RequestLimits(model core.Model) provider.RequestLimits {
	return r.inner.RequestLimits(model)
}
