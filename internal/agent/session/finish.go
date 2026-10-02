package session

import (
	"context"
	"sync"

	"github.com/wspl/demi/internal/agent/store"
)

// actionResult publishes one immutable session action answer to all waiters.
type actionResult struct {
	once sync.Once
	done chan struct{}
	end  ActionEnd
	err  error
}

func newAction() *ActionHandle {
	return &ActionHandle{result: &actionResult{done: make(chan struct{})}}
}
func (a *ActionHandle) finish(end ActionEnd, err error) {
	if a == nil {
		return
	}
	a.result.once.Do(func() {
		a.result.end = end
		a.result.err = err
		close(a.result.done)
	})
}

// waitAction waits without transferring cancellation to the session's action.
func (a *ActionHandle) waitAction(ctx context.Context) (ActionEnd, error) {
	select {
	case <-a.result.done:
		return a.result.end, a.result.err
	case <-ctx.Done():
		return 0, ctx.Err()
	}
}

// editResult publishes an edit's durable decision separately from its turn.
type editResult struct {
	once    sync.Once
	done    chan struct{}
	receipt store.EditReceipt
	err     error
}

func newAcceptance() *Acceptance { return &Acceptance{result: &editResult{done: make(chan struct{})}} }
func (a *Acceptance) finish(receipt store.EditReceipt, err error) {
	a.result.once.Do(func() {
		a.result.receipt = receipt
		a.result.err = err
		close(a.result.done)
	})
}
func (a *Acceptance) wait(ctx context.Context) (store.EditReceipt, error) {
	select {
	case <-a.result.done:
		return a.result.receipt, a.result.err
	case <-ctx.Done():
		return store.EditReceipt{}, ctx.Err()
	}
}
