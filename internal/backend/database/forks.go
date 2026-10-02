package database

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"

	"github.com/wspl/demi/internal/webapi"
)

// ForkOperation returns the creation attempt that reserved `id`, in whichever case it is
// spelled.
func (c *ControlService) ForkOperation(ctx context.Context, id webapi.ConversationID) (*ForkOperation, error) {
	panic("not written: b-database")
}

// ReserveFork reserves `operation`'s destination: the attempt that already holds
// it when it is this one, none when another attempt or a conversation
// holds the id.
func (c *ControlService) ReserveFork(ctx context.Context, operation ForkOperation) (*ForkOperation, error) {
	panic("not written: b-database")
}

// PendingForks returns the attempts whose destination is not published.
func (c *ControlService) PendingForks(ctx context.Context) ([]ForkOperation, error) {
	panic("not written: b-database")
}

// PublishFork publishes the destination `id` reserved: its conversation first in
// the owner's sidebar, with the title, target, model and attached hosts
// of the attempt, the title the user's. A destination published already
// is answered as it is. An attached host whose device is gone since is
// left out, as its revocation left the source.
func (c *ControlService) PublishFork(ctx context.Context, id webapi.ConversationID) (ConversationRecord, error) {
	panic("not written: b-database")
}
