package server

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"context"
	"time"

	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/tools"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
)

// TreeStores supplies the tree store of each conversation, by its root.
type TreeStores func(root core.NodeID) store.TreeStore

// Config controls how the server's trees and connections behave.
// Use DefaultConfig for production defaults; zero values are explicit.
type Config struct {
	Session session.Config
	// OutboxFrames is the most frames a connection's outbox holds; a connection
	// whose client lags this far is closed.
	OutboxFrames int
	// IdleTree is how long a tree stays live once it is detached and quiescent.
	IdleTree time.Duration
}

// DefaultConfig returns the session defaults, 4,096 outbox frames and a
// ten-minute detached, quiescent tree lifetime.
func DefaultConfig() Config { panic("not written: a-server") }

// Deps is what a product gives the agent server: what every node is assembled
// from, and the answers it asks for while a node runs. Dependencies support
// concurrent calls. Callbacks run outside the server's state mutex.
type Deps[H host.Host] struct {
	// Toolsets supplies the commands every node starts from, on which the
	// server grafts its own groups per node, and the named subagent profiles.
	Toolsets tools.ToolsetSource
	// Instructions opens every node's system prompt; a profile's replaces it.
	Instructions string
	// Hosts resolves where a node's shell tools run.
	Hosts tools.HostResolver[H]
	// Context supplies what the model must learn before each request, in order.
	Context   []tools.ContextSource
	Providers ProviderResolver
	// Shells makes each node's shell environment on each Host it uses.
	Shells tools.ShellEnvironmentFactory[H]
	Stores TreeStores
	Clock  core.Clock
	IDs    transcript.IDs
	Config Config
	// StatusChanged is told the root of a live tree that started or stopped
	// working, whose root's phase changed, or that was disposed. The product
	// reads the conversation's status again.
	StatusChanged func(core.NodeID)
}

// Server is the agent server of one user shard. Its methods are safe for
// concurrent use; callers own the returned connections and must detach them.
// Shutdown disposes its trees and joins all work the server owns.
type Server[H host.Host] struct{}

// New constructs a server from the product's dependencies.
func New[H host.Host](deps Deps[H]) *Server[H] { panic("not written: a-server") }

// Connect creates a connection for root, whose tree works in cwd when created,
// and its bounded outbox. Resolver resolves files referenced by client frames.
// The socket owner must defer Connection.Detach, even after cancellation.
func (s *Server[H]) Connect(root core.NodeID, cwd string, resolver ContentResolver) (*Connection[H], *FrameReceiver) {
	panic("not written: a-server")
}

// Tree returns the conversation's live tree, or nil if it has none.
func (s *Server[H]) Tree(root core.NodeID) *Tree[H] { panic("not written: a-server") }

// Restore opens root in cwd without a connection, continues its saved work
// and arms its wakeups. A tree already live is left as it is.
// Failure returns *RestoreError.
func (s *Server[H]) Restore(ctx context.Context, root core.NodeID, cwd string) error {
	panic("not written: a-server")
}

// PrepareSwitch prepares the live root's next model, building a runtime before
// anything changes when the provider changes. It returns nil when no tree is
// live, and *ResolveError when resolution fails. The caller must pass a
// prepared switch to SwitchModel or discard it to release its runtime.
func (s *Server[H]) PrepareSwitch(ctx context.Context, root core.NodeID, model core.ModelSelection) (*session.ModelSwitch, error) {
	panic("not written: a-server")
}

// SwitchModel transfers the prepared runtime to the live root's next request.
// With no live accepting tree, it closes the prepared runtime instead.
func (s *Server[H]) SwitchModel(ctx context.Context, root core.NodeID, change session.ModelSwitch) error {
	panic("not written: a-server")
}

// Node returns the live node whose rpc commands the backend dispatches,
// or nil. Its command storage is reached through CommandStorage.
func (s *Server[H]) Node(root, node core.NodeID) *Node[H] { panic("not written: a-server") }

// CommandStorage serves a job's storage message in its recorded history
// generation. Context is the invocation lifetime; a disposed node or an ended
// generation cannot read or write. Failure returns *host.PortError.
func (s *Server[H]) CommandStorage(ctx context.Context, root core.NodeID, caller host.JobCaller, op host.StorageOp) (host.StorageReply, error) {
	panic("not written: a-server")
}

// PrepareFork captures a seed through source's completed target text from its
// live session or committed checkpoint. Source keeps running either way.
// Failure returns *session.ForkError.
func (s *Server[H]) PrepareFork(ctx context.Context, source core.NodeID, target core.BlockID) (store.Checkpoint, error) {
	panic("not written: a-server")
}

// InitializeFork stores seed as destination's first checkpoint in one create
// commit. A seed that is not idle or holds waiting work or edit receipts is
// refused with *session.ForkError. Opening the tree assembles its runtime.
func (s *Server[H]) InitializeFork(ctx context.Context, destination core.NodeID, seed store.Checkpoint) error {
	panic("not written: a-server")
}

// LiveRoots returns an owned snapshot of the live trees' roots.
func (s *Server[H]) LiveRoots() []core.NodeID { panic("not written: a-server") }

// Reload closes a quiescent live tree so its next open takes the current
// toolset. Attached connections receive closed; a working tree returns
// ErrWorking and stays live. With no live tree there is nothing to do.
func (s *Server[H]) Reload(ctx context.Context, root core.NodeID) error {
	panic("not written: a-server")
}

// Shutdown disposes every live tree and joins evictions already under way.
// The owner supplies a cleanup context that outlives action cancellation.
func (s *Server[H]) Shutdown(ctx context.Context) error { panic("not written: a-server") }
