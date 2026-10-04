package cdp

import (
	"context"
	"errors"
	"sync"
	"time"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserproto"
	"github.com/wspl/demi/internal/cmdproto"
	"github.com/wspl/demi/internal/cmdsdk"
)

// ControlTimeout bounds driver-owned setup and cleanup.
const ControlTimeout = 5 * time.Second

// Operation shares one deadline across every wait and CDP step.
// Close releases its context registrations; it does not end its parent lifetime.
type Operation struct {
	ctx          context.Context
	lifetime     context.Context
	deadline     time.Time
	cancel       context.CancelFunc
	stop         func() bool
	callbackDone chan struct{}
	closeOnce    sync.Once
	mu           sync.Mutex
	progress     browserproto.ActionProgress
}

// NewOperation uses the earlier of ctx's deadline and timeout. The lifetime
// context ends with the tab; its cancellation cause retains transport failure.
func NewOperation(ctx context.Context, lifetime context.Context, timeout time.Duration) *Operation {
	return OperationUntil(ctx, lifetime, time.Now().Add(timeout))
}

// OperationUntil uses an absolute shared command deadline.
func OperationUntil(ctx context.Context, lifetime context.Context, deadline time.Time) *Operation {
	bounded, cancel := context.WithDeadline(ctx, deadline)
	callbackDone := make(chan struct{})
	stop := context.AfterFunc(lifetime, func() {
		defer close(callbackDone)
		cancel()
	})
	return &Operation{
		ctx:          bounded,
		lifetime:     lifetime,
		deadline:     deadline,
		cancel:       cancel,
		stop:         stop,
		callbackDone: callbackDone,
		progress:     browserproto.ActionProgress("not_started"),
	}
}

// Context supplies the shared deadline and cancellation to low-level steps.
func (o *Operation) Context() context.Context {
	return o.ctx
}

// Close releases cancellation registrations and the operation's timer.
func (o *Operation) Close() {
	o.closeOnce.Do(func() {
		if !o.stop() {
			<-o.callbackDone
		}
		o.cancel()
	})
}

// Run runs a cooperative step synchronously and translates cancellation causes.
// The step must use its supplied context and join any work it starts.
func (o *Operation) Run(ctx context.Context, step func(context.Context) error) error {
	work, cancel := context.WithCancel(o.ctx)
	callbackDone := make(chan struct{})
	stop := context.AfterFunc(ctx, func() {
		defer close(callbackDone)
		cancel()
	})
	defer cancel()
	defer func() {
		if !stop() {
			<-callbackDone
		}
	}()
	if err := o.failureCause(ctx); err != nil {
		return err
	}
	err := step(work)
	if cause := o.failureCause(ctx); cause != nil {
		return cause
	}
	return err
}

// failureCause preserves browser-lifetime loss before invocation cancellation.
func (o *Operation) failureCause(ctx context.Context) error {
	if o.lifetime.Err() != nil {
		cause := context.Cause(o.lifetime)
		if cause != nil && !errors.Is(cause, context.Canceled) {
			return cause
		}
		return &BrowserError{Kind: KindClosed}
	}
	if o.ctx.Err() != nil || ctx.Err() != nil {
		if !time.Now().Before(o.deadline) || errors.Is(o.ctx.Err(), context.DeadlineExceeded) {
			return &BrowserError{Kind: KindTimeout}
		}
		return &BrowserError{Kind: KindCancelled}
	}
	return nil
}

// BeginInput marks input as in flight, with an unknown outcome until delivered.
func (o *Operation) BeginInput() {
	o.setProgress("unknown")
}

// InputNotDelivered records that input never reached the page.
func (o *Operation) InputNotDelivered() {
	o.setProgress("not_started")
}

// CompleteInput records acknowledged input delivery.
func (o *Operation) CompleteInput() {
	o.setProgress("completed")
}

// setProgress updates the browser action's delivery state without holding a lock across IO.
func (o *Operation) setProgress(progress browserproto.ActionProgress) {
	o.mu.Lock()
	o.progress = progress
	o.mu.Unlock()
}

// Failure preserves the cause and adds tab, URL and input-progress details.
func (o *Operation) Failure(err error, tab string, url *string) error {
	o.mu.Lock()
	progress := o.progress
	o.mu.Unlock()
	if progress != "not_started" && connectionLoss(err) {
		err = &BrowserError{Kind: KindOutcomeUnknown, Cause: err}
	}
	details := ErrorDetails(err)
	if details.Action == nil {
		details.Action = &progress
	}
	if details.Tab == nil {
		details.Tab = &tab
	}
	if details.URL == nil {
		details.URL = url
	}
	return &BrowserError{Kind: KindAction, Cause: err, Details: details}
}

// Agent returns the calling agent number, rejecting non-agent invocations.
func Agent(invocation *cmdsdk.InvocationContext[cmdproto.Invocation]) (uint64, error) {
	if agent, ok := invocation.Request.Context.Caller.(*cmdproto.AgentCaller); ok {
		return agent.Number, nil
	}
	return 0, &BrowserError{Kind: KindConfiguration, Message: "browser commands act for an agent"}
}

// AfterCleanup preserves the operation error when its required cleanup also fails.
func AfterCleanup(operation, cleanup error) error {
	if cleanup == nil {
		return operation
	}
	return &BrowserError{Kind: KindCleanup, Cause: operation, Cleanup: cleanup}
}

// ErrorCode maps wrapped browser failures to their public code.
func ErrorCode(err error) browserproto.BrowserErrorCode {
	var failure *BrowserError
	if errors.As(err, &failure) {
		return failure.Code()
	}
	if errors.Is(err, context.DeadlineExceeded) {
		return "timeout"
	}
	if errors.Is(err, context.Canceled) {
		return "cancelled"
	}
	return "driver_error"
}

// ErrorDetails preserves structured action and cleanup failure context.
func ErrorDetails(err error) browserproto.ErrorDetails {
	var e *BrowserError
	if !errors.As(err, &e) {
		return browserproto.ErrorDetails{}
	}
	switch e.Kind {
	case KindAction, KindPartialFailure, KindNotActionable:
		return e.Details
	case KindOutcomeUnknown:
		details := ErrorDetails(e.Cause)
		progress := browserproto.ActionProgress("unknown")
		details.Action = &progress
		return details
	case KindCleanup:
		if e.Cause != nil {
			return ErrorDetails(e.Cause)
		}
	case KindAmbiguous:
		return browserproto.ErrorDetails{Count: &e.Count}
	}
	return browserproto.ErrorDetails{}
}

// IsDeadline reports whether the primary browser failure exhausted its deadline.
func IsDeadline(err error) bool {
	return ErrorCode(err) == "timeout"
}

// WithDeadlineCause preserves cleanup wrappers around a readiness failure.
func WithDeadlineCause(err, cause error) error {
	var e *BrowserError
	if errors.As(err, &e) && e.Kind == KindCleanup {
		wrapped := *e
		wrapped.Cause = WithDeadlineCause(e.Cause, cause)
		return &wrapped
	}
	return cause
}

// connectionLoss follows only the primary browser failure through wrappers.
func connectionLoss(err error) bool {
	var e *BrowserError
	if !errors.As(err, &e) {
		return false
	}
	switch e.Kind {
	case KindConnection:
		return true
	case KindAction, KindProfileRetained:
		return connectionLoss(e.Cause)
	case KindCleanup:
		if e.Cause != nil {
			return connectionLoss(e.Cause)
		}
		return connectionLoss(e.Cleanup)
	}
	return false
}
