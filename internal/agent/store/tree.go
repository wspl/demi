package store

import (
	"context"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
)

// SessionStore holds one node's checkpoint and its media's blob namespace.
// Save checks the guard immediately before committing. Once a commit starts,
// cancellation does not abandon it; success means the entire update committed.
type SessionStore interface {
	// Save commits an update and its carried completion receipts atomically.
	Save(ctx context.Context, update CheckpointUpdate, guard CommitGuard) error
	// Load returns a decoded, checked checkpoint, or nil when none exists.
	// Corrupt data stops the load.
	Load(ctx context.Context) (*Checkpoint, error)
	// Blobs returns the conversation owner's blob namespace.
	Blobs() BlobStore
}

// TreeStore holds a conversation's node records and checkpoints.
// Each mutation is one atomic commit. Cancellation cancels admission waits,
// never a transaction that has begun. Implementations support concurrent calls.
type TreeStore interface {
	// Node returns a node's record, or nil when it does not exist.
	Node(ctx context.Context, id core.NodeID) (*NodeRecord, error)
	// Children returns direct children in number order, live and archived alike.
	Children(ctx context.Context, parent core.NodeID) ([]NodeRecord, error)
	// CreateNode commits the record and first checkpoint; an existing node is refused.
	CreateNode(ctx context.Context, record NodeRecord, initial CheckpointUpdate) error
	// SessionStore returns the node's checkpoint store. Saves also mark carried
	// child completions delivered in the same commit.
	SessionStore(id core.NodeID) SessionStore
	// CloseNode closes a node after its final checkpoint, initially undelivered.
	CloseNode(ctx context.Context, id core.NodeID, closed NodeClose) error
	// ReopenNode starts a new round and queues its reviving message atomically.
	ReopenNode(ctx context.Context, id core.NodeID, round uint64, startedAt core.Timestamp, message core.QueuedMessage) error
	// MarkDelivered marks only the named current round delivered.
	MarkDelivered(ctx context.Context, id core.NodeID, round uint64) error
	// DeleteNode deletes the node and all descendants with all their rows.
	DeleteNode(ctx context.Context, id core.NodeID) error
	// NextNumber records the following number before returning this one.
	NextNumber(ctx context.Context, sequence core.Sequence) (uint64, error)
	// CommandOutput returns an ended command's output record, or nil if unknown.
	CommandOutput(ctx context.Context, command core.CommandID) (StoredOutput, error)
}

// CommandOutputDays is how many days an ended command's output is retained.
const CommandOutputDays int64 = 30

// StoredOutput is what a conversation holds of an ended command's output.
//
//sumtype:decl
type StoredOutput interface{ storedOutput() }

// OutputStored carries the kept output.
type OutputStored struct{ Output host.WholeOutput }

// OutputNotStored records why the backend could not store the output.
type OutputNotStored struct{ Reason string }

// OutputRemoved records when retention removed the output.
type OutputRemoved struct{ At core.Timestamp }

func (*OutputStored) storedOutput()    {}
func (*OutputNotStored) storedOutput() {}
func (*OutputRemoved) storedOutput()   {}

// CommitGuard checks that no invocation served by a save has become stale.
// Its zero value serves no invocation. The caller owns the lifetimes and
// serializes invalidation with committing; the guard starts no goroutines.
type CommitGuard struct{ lifetimes []context.Context }

// NewCommitGuard guards the cancellation contexts of the served invocations.
func NewCommitGuard(lifetimes ...context.Context) CommitGuard {
	return CommitGuard{lifetimes: append([]context.Context(nil), lifetimes...)}
}

// Check returns ErrInvalidated when any served invocation has been canceled.
func (g CommitGuard) Check() error {
	for _, lifetime := range g.lifetimes {
		if lifetime.Err() != nil {
			return ErrInvalidated
		}
	}
	return nil
}
