package cdp

//revive:disable:unused-parameter API checkpoint: retain parameter names for dependent implementers.

import (
	"context"
	"time"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
)

// ControlTimeout bounds driver-owned setup and cleanup.
const ControlTimeout = 5 * time.Second

// Operation shares one deadline across every wait and CDP step.
// Close releases its context registrations; it does not end its parent lifetime.
type Operation struct{}

// NewOperation uses the earlier of ctx's deadline and timeout. The lifetime
// context ends with the tab; its cancellation cause retains transport failure.
func NewOperation(ctx context.Context, lifetime context.Context, timeout time.Duration) *Operation {
	panic("not written: k-chrome-cdp")
}

// OperationUntil uses an absolute shared command deadline.
func OperationUntil(ctx context.Context, lifetime context.Context, deadline time.Time) *Operation {
	panic("not written: k-chrome-cdp")
}

// Context supplies the shared deadline and cancellation to low-level steps.
func (o *Operation) Context() context.Context { panic("not written: k-chrome-cdp") }

// Close releases cancellation registrations and the operation's timer.
func (o *Operation) Close() { panic("not written: k-chrome-cdp") }

// Run runs a cooperative step synchronously and translates cancellation causes.
// The step must use its supplied context and join any work it starts.
func (o *Operation) Run(ctx context.Context, step func(context.Context) error) error {
	panic("not written: k-chrome-cdp")
}

// BeginInput marks input as in flight, with an unknown outcome until delivered.
func (o *Operation) BeginInput() { panic("not written: k-chrome-cdp") }

// InputNotDelivered records that input never reached the page.
func (o *Operation) InputNotDelivered() { panic("not written: k-chrome-cdp") }

// CompleteInput records acknowledged input delivery.
func (o *Operation) CompleteInput() { panic("not written: k-chrome-cdp") }

// Failure preserves the cause and adds tab, URL and input-progress details.
func (o *Operation) Failure(err error, tab string, url *string) error {
	panic("not written: k-chrome-cdp")
}

// Agent returns the calling agent number, rejecting non-agent invocations.
func Agent(invocation *cmdsdk.InvocationContext[commandwire.Invocation]) (uint64, error) {
	panic("not written: k-chrome-cdp")
}

// AfterCleanup preserves the operation error when its required cleanup also fails.
func AfterCleanup(operation, cleanup error) error { panic("not written: k-chrome-cdp") }

// ErrorCode maps wrapped browser failures to their public code.
func ErrorCode(err error) browserop.BrowserErrorCode { panic("not written: k-chrome-cdp") }

// ErrorDetails preserves structured action and cleanup failure context.
func ErrorDetails(err error) browserop.ErrorDetails { panic("not written: k-chrome-cdp") }

// IsDeadline reports whether the primary browser failure exhausted its deadline.
func IsDeadline(err error) bool { panic("not written: k-chrome-cdp") }

// WithDeadlineCause preserves cleanup wrappers around a readiness failure.
func WithDeadlineCause(err, cause error) error { panic("not written: k-chrome-cdp") }
