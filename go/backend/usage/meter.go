package usage

import (
	"context"
	"iter"
	"log/slog"

	"github.com/wspl/demi/go/backend/storage"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/webapi"
)

type Ledger struct {
	Control      *storage.Control
	User         webapi.UserID
	Conversation webapi.ConversationID
	Provider     webapi.ProviderID
}

func (l Ledger) Record(ctx context.Context, model string, usage core.TokenUsage) {
	if err := l.Control.AppendUsage(ctx, storage.UsageRow{User: l.User, Conversation: l.Conversation, Provider: l.Provider, Model: model, Usage: usage}); err != nil {
		slog.Warn("a usage ledger row was not written", "error", err)
	}
}

// Meter applies the backend policy around an event stream. The provider owner
// supplies response recognition; storage and usage do not duplicate its event
// union. A runtime's Fresh wrapper shares this same Meter.
type Meter struct {
	Limit  *RateLimit
	Ledger Ledger
}

func MeterEvents[E any](ctx context.Context, meter *Meter, model string, run func() iter.Seq[E], response func(E) (core.TokenUsage, bool), refused func(*RateLimited) E) iter.Seq[E] {
	// Admission happens when Run is requested, before polling the stream, as in
	// Rust. A rejected run never calls the provider and consumes no window slot.
	err := meter.Limit.Take()
	if err != nil {
		return func(yield func(E) bool) { yield(refused(err)) }
	}
	stream := run()
	return func(yield func(E) bool) {
		for event := range stream {
			if tokens, ok := response(event); ok {
				meter.Ledger.Record(ctx, model, tokens)
			}
			if !yield(event) {
				return
			}
		}
	}
}
