//revive:disable:unused-parameter // API checkpoint keeps parameter names for callers; bodies follow after merge.
package providers

import (
	"context"
	"time"

	"github.com/wspl/demi/internal/webapi"
)

// LoginTiming is how long a login waits for its user and keeps its result.
type LoginTiming struct {
	Lifetime  time.Duration
	Retention time.Duration
}

// LoginFlows owns the backend login tasks; Close cancels and joins them.
type LoginFlows struct{}

// Operations admits one configuration operation per provider. The zero value is ready to use.
type Operations struct{}

// OperationGuard holds an entry until Release; do not copy it.
type OperationGuard struct{}

// DefaultLoginTiming returns ten minutes for both lifetime and retention.
func DefaultLoginTiming() LoginTiming { panic("not written: b-providers") }

// NewLoginFlows creates the backend login owner.
func NewLoginFlows(assembly *Assembly, operations *Operations, timing LoginTiming) *LoginFlows {
	panic("not written: b-providers")
}

// Start starts a device login; only starter can read and cancel it.
func (f *LoginFlows) Start(ctx context.Context, owner, starter webapi.UserID, family, label string, existing *ProviderEntry) (webapi.LoginID, error) {
	panic("not written: b-providers")
}

// State returns a login state only to its starter, or nil.
func (f *LoginFlows) State(id webapi.LoginID, owner webapi.UserID) webapi.LoginState {
	panic("not written: b-providers")
}

// Cancel cancels and joins the starter login, reporting whether it exists.
func (f *LoginFlows) Cancel(ctx context.Context, id webapi.LoginID, owner webapi.UserID) bool {
	panic("not written: b-providers")
}

// Close cancels and joins all login tasks.
func (f *LoginFlows) Close(ctx context.Context) error { panic("not written: b-providers") }

// Reserve returns an entry guard, or nil when another operation holds it.
func (o *Operations) Reserve(id webapi.ProviderID) *OperationGuard { panic("not written: b-providers") }

// Release releases the entry reservation. It is idempotent.
func (g *OperationGuard) Release() { panic("not written: b-providers") }
