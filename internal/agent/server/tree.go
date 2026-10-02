package server

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"context"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/host"
)

// Tree is a conversation's live nodes, connection attachments and supervisor.
// It belongs to its Server and exposes synchronized operations, not mutable state.
type Tree[H host.Host] struct{}

// Toolset returns the revision the tree opened with, for product comparison.
func (t *Tree[H]) Toolset() string { panic("not written: a-server") }

// Root returns the root node.
func (t *Tree[H]) Root() *Node[H] { panic("not written: a-server") }

// Node returns the live node, root or child at any depth, or nil.
func (t *Tree[H]) Node(id core.NodeID) *Node[H] { panic("not written: a-server") }

// Admission returns the tree's admission. Every node action holds a lease;
// a target switch or archive reserves the idle tree through the same gate.
func (t *Tree[H]) Admission() *gates.Activity { panic("not written: a-server") }

// Interrupt reserves admission, stops the root's running action, aborts all
// live children and ends the root's shells for a Host transition. It returns
// once every action has released admission. The caller must Release the
// reservation; queued actions remain waiting until then.
func (t *Tree[H]) Interrupt(ctx context.Context) (*gates.Reservation, error) {
	panic("not written: a-server")
}

// IsAttached reports whether any connection is attached.
func (t *Tree[H]) IsAttached() bool { panic("not written: a-server") }

// IsQuiescent reports that no child or command is live and the root has no
// running or waiting action or scheduled wakeup.
func (t *Tree[H]) IsQuiescent() bool { panic("not written: a-server") }
