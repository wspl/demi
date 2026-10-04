package server

import (
	"context"
	"errors"
	"fmt"
	"sync"
	"time"

	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/tools"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/host"
)

// TreeStores supplies the tree store of each conversation, by its root.
type TreeStores func(root core.NodeID) store.Tree

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
func DefaultConfig() Config {
	return Config{Session: session.DefaultConfig(), OutboxFrames: 4096, IdleTree: 10 * time.Minute}
}

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
type Server[H host.Host] struct {
	deps    Deps[H]
	mu      sync.Mutex // Protects live trees, their directories and attachment decisions.
	trees   map[core.NodeID]*Tree[H]
	opening gates.KeyedSerial[core.NodeID]
	work    sync.WaitGroup
	closing bool
}

// New constructs a server from the product's dependencies.
func New[H host.Host](deps Deps[H]) *Server[H] {
	return &Server[H]{deps: deps, trees: map[core.NodeID]*Tree[H]{}}
}

// Connect creates a connection for root, whose tree works in cwd when created,
// and its bounded outbox. Resolver resolves files referenced by client frames.
// The socket owner must defer Connection.Detach, even after cancellation.
func (s *Server[H]) Connect(root core.NodeID, cwd string, resolver ContentResolver) (*Connection[H], *Frames) {
	outbox := &outbox{capacity: s.deps.Config.OutboxFrames, changed: make(chan struct{})}
	c := &Connection[H]{server: s, root: root, cwd: cwd, resolver: resolver, outbox: outbox}
	return c, &Frames{outbox: outbox}
}

// Tree returns the conversation's live tree, or nil if it has none.
func (s *Server[H]) Tree(root core.NodeID) *Tree[H] {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.trees[root]
}

// Restore opens root in cwd without a connection, continues its saved work
// and arms its wakeups. A tree already live is left as it is.
// Failure says whether the tree did not open or did not continue.
func (s *Server[H]) Restore(ctx context.Context, root core.NodeID, cwd string) error {
	permit, err := s.opening.Acquire(ctx, root)
	if err != nil {
		return err
	}
	defer permit.Release()
	tree, continuation, continues, err := s.liveOrOpen(ctx, root, cwd)
	if err != nil {
		return fmt.Errorf("the tree did not open: %w", err)
	}
	if continues {
		if err := tree.continueRestored(ctx, continuation); err != nil {
			return fmt.Errorf("the restored tree did not continue: %w", err)
		}
	}
	return nil
}

// PrepareSwitch prepares the live root's next model, building a runtime before
// anything changes when the provider changes. It returns nil when no tree is
// live, and an error wrapping ErrProviderUnavailable when no provider entry has the id. The caller must pass a
// prepared switch to SwitchModel or discard it to release its runtime.
func (s *Server[H]) PrepareSwitch(
	ctx context.Context,
	root core.NodeID,
	model core.ModelSelection,
) (*session.ModelSwitch, error) {
	tree := s.Tree(root)
	if tree == nil {
		return nil, nil
	}
	change := &session.ModelSwitch{Model: model}
	if tree.root.session.NeedsRuntimeFor(model) {
		runtime, err := s.deps.Providers.Runtime(ctx, root, model)
		if err != nil {
			return nil, err
		}
		change.Runtime = runtime
	}
	return change, nil
}

// SwitchModel transfers the prepared runtime to the live root's next request.
// With no live accepting tree, it closes the prepared runtime instead.
func (s *Server[H]) SwitchModel(ctx context.Context, root core.NodeID, change session.ModelSwitch) error {
	if tree := s.Tree(root); tree != nil {
		if err := tree.root.session.UpdateModel(change); err == nil {
			return nil
		}
	}
	return change.Discard(context.WithoutCancel(ctx))
}

// Node returns the live node whose rpc commands the backend dispatches,
// or nil. Its command storage is reached through CommandStorage.
func (s *Server[H]) Node(root, node core.NodeID) *Node[H] {
	if tree := s.Tree(root); tree != nil {
		return tree.Node(node)
	}
	return nil
}

// CommandStorage serves a job's storage message in its recorded history
// generation. Context is the invocation lifetime; a disposed node or an ended
// generation cannot read or write. Failure returns *host.PortError.
func (s *Server[H]) CommandStorage(
	ctx context.Context,
	root core.NodeID,
	caller host.JobCaller,
	op host.StorageOp,
) (host.StorageReply, error) {
	node := s.Node(root, caller.Node)
	if node == nil {
		return nil, &host.PortError{
			Kind:    host.StorageRefused,
			Message: store.ErrInvalidated.Error(),
			Err:     store.ErrInvalidated,
		}
	}
	return node.session.JobStorage(ctx, caller.Generation, op)
}

// PrepareFork captures a seed through source's completed target text from its
// live session or committed checkpoint. Source keeps running either way.
// Failure returns *session.ForkError.
func (s *Server[H]) PrepareFork(
	ctx context.Context,
	source core.NodeID,
	target core.BlockID,
) (store.Checkpoint, error) {
	if tree := s.Tree(source); tree != nil {
		return tree.root.session.PrepareFork(target)
	}
	treeStore := s.deps.Stores(source)
	record, found, err := treeStore.Node(ctx, source)
	if err != nil {
		return store.Checkpoint{}, &session.ForkError{Kind: session.ForkStore, Detail: err.Error(), Cause: err}
	}
	if !found || record.Parent != nil {
		return store.Checkpoint{}, &session.ForkError{Kind: session.ForkNotRoot}
	}
	checkpoint, saved, err := treeStore.Session(source).Load(ctx)
	if err != nil {
		return store.Checkpoint{}, &session.ForkError{Kind: session.ForkStore, Detail: err.Error(), Cause: err}
	}
	if !saved {
		return store.Checkpoint{}, &session.ForkError{Kind: session.ForkNoCheckpoint}
	}
	commands, err := store.RestoreCommandStateHistory(checkpoint.CommandState)
	if err != nil {
		return store.Checkpoint{}, &session.ForkError{Kind: session.ForkStore, Detail: err.Error(), Cause: err}
	}
	return session.ForkSeed(checkpoint.Transcript, commands, checkpoint.State, target)
}

// InitializeFork stores seed as destination's first checkpoint in one create
// commit. A seed that is not idle or holds waiting work or edit receipts is
// refused with *session.ForkError. Opening the tree assembles its runtime.
func (s *Server[H]) InitializeFork(ctx context.Context, destination core.NodeID, seed store.Checkpoint) error {
	state := seed.State
	if state.Phase != core.SessionPhaseIdle ||
		len(state.Queue)+len(state.AgentInputs)+len(state.Wakeups)+len(state.Edits) != 0 {
		return &session.ForkError{Kind: session.ForkInvalidSeed}
	}
	initial := store.CheckpointUpdate{
		State:         state,
		CommandState:  &seed.CommandState,
		BlockCount:    len(seed.Transcript),
		ChangedBlocks: []store.ChangedBlock{},
	}
	for i, block := range seed.Transcript {
		initial.ChangedBlocks = append(initial.ChangedBlocks, store.ChangedBlock{Index: i, Block: block})
	}
	err := s.deps.Stores(destination).CreateNode(ctx, store.RootRecord(destination, s.deps.Clock.Now()), initial)
	if err != nil {
		return &session.ForkError{Kind: session.ForkStore, Detail: err.Error(), Cause: err}
	}
	return nil
}

// LiveRoots returns an owned snapshot of the live trees' roots.
func (s *Server[H]) LiveRoots() []core.NodeID {
	s.mu.Lock()
	defer s.mu.Unlock()
	roots := make([]core.NodeID, 0, len(s.trees))
	for root := range s.trees {
		roots = append(roots, root)
	}
	return roots
}

// Reload closes a quiescent live tree so its next open takes the current
// toolset. Attached connections receive closed; a working tree returns
// ErrWorking and stays live. With no live tree there is nothing to do.
func (s *Server[H]) Reload(ctx context.Context, root core.NodeID) error {
	permit, err := s.opening.Acquire(ctx, root)
	if err != nil {
		return err
	}
	defer permit.Release()
	tree := s.Tree(root)
	if tree == nil {
		return nil
	}
	if !tree.IsQuiescent() {
		return ErrWorking
	}
	return s.disposeTree(ctx, tree)
}

// Shutdown disposes every live tree and joins evictions already under way.
// The owner supplies a cleanup context that outlives action cancellation.
func (s *Server[H]) Shutdown(ctx context.Context) error {
	s.mu.Lock()
	s.closing = true
	s.mu.Unlock()
	var errs []error
	for _, root := range s.LiveRoots() {
		permit, err := s.opening.Acquire(context.WithoutCancel(ctx), root)
		if err != nil {
			errs = append(errs, err)
			continue
		}
		if tree := s.Tree(root); tree != nil {
			errs = append(errs, s.disposeTree(context.WithoutCancel(ctx), tree))
		}
		permit.Release()
	}
	s.work.Wait()
	return errors.Join(errs...)
}

// liveOrOpen shares a tree under the conversation's opening gate.
func (s *Server[H]) liveOrOpen(
	ctx context.Context,
	root core.NodeID,
	cwd string,
) (*Tree[H], session.Continuation, bool, error) {
	s.mu.Lock()
	tree, closing := s.trees[root], s.closing
	if !closing && tree == nil {
		s.work.Add(1)
	}
	s.mu.Unlock()
	if closing {
		return nil, session.Continuation{}, false, session.ErrClosed
	}
	if tree != nil {
		return tree, session.Continuation{}, false, nil
	}
	defer s.work.Done()
	return s.openTree(ctx, root, cwd)
}

// disposeTree removes a tree after its owned work and final save finish.
func (s *Server[H]) disposeTree(ctx context.Context, tree *Tree[H]) error {
	err := tree.dispose(ctx)
	s.mu.Lock()
	removed := s.trees[tree.id] == tree
	if removed {
		delete(s.trees, tree.id)
	}
	s.mu.Unlock()
	if removed {
		s.deps.StatusChanged(tree.id)
	}
	return err
}

// evict owns a disposal independently of the tree monitor that requested it.
func (s *Server[H]) evict(tree *Tree[H], epoch uint64) <-chan struct{} {
	done := make(chan struct{})
	s.mu.Lock()
	if s.closing {
		s.mu.Unlock()
		close(done)
		return done
	}
	s.work.Add(1)
	s.mu.Unlock()
	go func() {
		defer s.work.Done()
		defer close(done)
		ctx := context.Background()
		permit, err := s.opening.Acquire(ctx, tree.id)
		if err != nil {
			return
		}
		defer permit.Release()
		s.mu.Lock()
		current := tree.attachmentEpoch == epoch
		s.mu.Unlock()
		if current && s.Tree(tree.id) == tree && !tree.IsAttached() && tree.IsQuiescent() {
			if err := s.disposeTree(ctx, tree); err != nil {
				tree.report(err)
			}
		}
	}()
	return done
}
