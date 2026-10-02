package server

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
)

// Node is one live node: its record, role and session. Only node assembly
// creates its session; supervisors ask that assembly for each child.
type Node[H host.Host] struct{}

// ID returns the node's identity.
func (n *Node[H]) ID() core.NodeID { panic("not written: a-server") }

// Record returns an owned snapshot of the node record.
func (n *Node[H]) Record() store.NodeRecord { panic("not written: a-server") }

// Session returns the node's synchronized session handle.
func (n *Node[H]) Session() *session.Session { panic("not written: a-server") }

// CWD returns the node's working directory.
func (n *Node[H]) CWD() string { panic("not written: a-server") }

// Commands returns the immutable command set the node's shell offers: the
// product's commands with the runtime groups grafted. The backend dispatches
// the node's rpc calls through this set.
func (n *Node[H]) Commands() *host.CommandSet { panic("not written: a-server") }

// JobCaller binds a job started now to this node and its current generation.
func (n *Node[H]) JobCaller() host.JobCaller { panic("not written: a-server") }
