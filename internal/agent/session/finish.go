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

func newAction() *ActionAnswer {
	return &ActionAnswer{result: &actionResult{done: make(chan struct{})}}
}

func (a *ActionAnswer) finish(end ActionEnd, err error) {
	if a == nil {
		return
	}
	a.result.once.Do(func() {
		a.result.end = end
		a.result.err = err
		close(a.result.done)
	})
}

// wait waits without transferring cancellation to the session's action.
func (a *ActionAnswer) wait(ctx context.Context) (ActionEnd, error) {
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

func newAcceptance() *acceptance { return &acceptance{result: &editResult{done: make(chan struct{})}} }

func (a *acceptance) finish(receipt store.EditReceipt, err error) {
	a.result.once.Do(func() {
		a.result.receipt = receipt
		a.result.err = err
		close(a.result.done)
	})
}

func (a *acceptance) wait(ctx context.Context) (store.EditReceipt, error) {
	select {
	case <-a.result.done:
		return a.result.receipt, a.result.err
	case <-ctx.Done():
		return store.EditReceipt{}, ctx.Err()
	}
}
