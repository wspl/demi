package usershardtest

//revive:disable:unused-parameter

import (
	"context"
	"testing"

	"github.com/wspl/demi/internal/backend/usershard"
)

// StepHolds holds a flow at at most one of its named steps. Its zero value is ready.
type StepHolds[S comparable] struct{}

// Hold holds step until Release or test cleanup. Replacing a hold leaves its
// owner responsible for releasing it; cleanup also releases failed tests' holds.
func (h *StepHolds[S]) Hold(t testing.TB, step S) *StepHold { panic("not written: b-usershard") }

// Pass waits while step is held, counting its arrival even if canceled later.
func (h *StepHolds[S]) Pass(ctx context.Context, step S) error { panic("not written: b-usershard") }

// StepHold is a test's hold at one flow step; its test cleanup releases it.
type StepHold struct{}

// UntilArrived waits for count arrivals, including callers that left meanwhile.
func (h *StepHold) UntilArrived(ctx context.Context, count int) error {
	panic("not written: b-usershard")
}

// Release lets waiting and future passes through. It is idempotent.
func (h *StepHold) Release() { panic("not written: b-usershard") }

// Hooks connects independently held runner and page flows to shared services.
// Set Services.Hooks before starting routing or accepting any sockets.
type Hooks struct {
	Hellos StepHolds[usershard.HelloStep]
	Syncs  StepHolds[usershard.SyncStep]
}

// Hello passes the runner flow's step.
func (h *Hooks) Hello(ctx context.Context, step usershard.HelloStep) error {
	panic("not written: b-usershard")
}

// Sync passes the page flow's step.
func (h *Hooks) Sync(ctx context.Context, step usershard.SyncStep) error {
	panic("not written: b-usershard")
}

var _ usershard.FlowHooks = (*Hooks)(nil)
