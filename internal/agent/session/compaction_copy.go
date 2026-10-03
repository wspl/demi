package session

import (
	"context"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/gates"
)

// copyRuntime delegates the compaction session's tools and prompts, but neither
// reacquires the parent's admission nor disposes its environments.
type copyRuntime struct{ Runtime }

// EnterAction avoids reacquiring the parent admission during compaction.
func (copyRuntime) EnterAction(context.Context) (*gates.Lease, error) { return nil, nil }

// ReserveEdit refuses transcript edits in a compaction copy.
func (copyRuntime) ReserveEdit(context.Context) (*gates.Reservation, error) {
	return nil, &ErrorReport{Message: "A session copy is never edited"}
}

// Context omits new context from the conversation being summarized.
func (copyRuntime) Context(context.Context, []SeenContext, core.TurnID) ([]NewContext, error) {
	return nil, nil
}

// Dispose leaves the shared parent environments with their owner.
func (copyRuntime) Dispose(context.Context) error { return nil }

// copyStore gives compaction a private command history and unstored media.
type copyStore struct{}

// Save keeps compaction checkpoints out of the parent store.
func (copyStore) Save(context.Context, store.CheckpointUpdate, store.CommitGuard) error { return nil }

// Load starts the compaction copy without a persisted checkpoint.
func (copyStore) Load(context.Context) (store.Checkpoint, bool, error) {
	return store.Checkpoint{}, false, nil
}

// Blobs returns the unstored media source of the compaction copy.
func (copyStore) Blobs() store.BlobStore { return copyBlobs{} }

type copyBlobs struct{}

// Put identifies media without persisting it in the parent store.
func (copyBlobs) Put(_ context.Context, data core.B64Bytes) (core.BlobRef, error) {
	return core.BlobRefOf(data), nil
}

// Read reports that the compaction copy has no stored media.
func (copyBlobs) Read(context.Context, core.BlobRef) (core.B64Bytes, bool, error) {
	return nil, false, nil
}

type sessionIDs struct{ session *Session }

// NextID serializes identity allocation with the parent session.
func (ids sessionIDs) NextID() string {
	ids.session.mu.Lock()
	defer ids.session.mu.Unlock()
	return ids.session.deps.IDs.NextID()
}

type sessionClock struct{ session *Session }

// Now serializes clock access with the parent session.
func (clock sessionClock) Now() core.Timestamp {
	clock.session.mu.Lock()
	defer clock.session.mu.Unlock()
	return clock.session.deps.Clock.Now()
}
