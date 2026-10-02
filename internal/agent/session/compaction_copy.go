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

func (copyRuntime) EnterAction(context.Context) (*gates.Lease, error) { return nil, nil }
func (copyRuntime) ReserveEdit(context.Context) (*gates.Reservation, error) {
	return nil, &ErrorReport{Message: "A session copy is never edited"}
}
func (copyRuntime) Context(context.Context, []SeenContext, core.TurnID) ([]NewContext, error) {
	return nil, nil
}
func (copyRuntime) Dispose(context.Context) error { return nil }

// copyStore gives compaction a private command history and unstored media.
type copyStore struct{}

func (copyStore) Save(context.Context, store.CheckpointUpdate, store.CommitGuard) error { return nil }
func (copyStore) Load(context.Context) (*store.Checkpoint, error)                       { return nil, nil }
func (copyStore) Blobs() store.BlobStore                                                { return copyBlobs{} }

type copyBlobs struct{}

func (copyBlobs) Put(_ context.Context, data core.B64Bytes) (core.BlobRef, error) {
	return core.BlobRefOf(data), nil
}
func (copyBlobs) Read(context.Context, core.BlobRef) (core.B64Bytes, bool, error) {
	return nil, false, nil
}

type sessionIDs struct{ session *Session }

func (ids sessionIDs) NextID() string {
	ids.session.mu.Lock()
	defer ids.session.mu.Unlock()
	return ids.session.deps.IDs.NextID()
}

type sessionClock struct{ session *Session }

func (clock sessionClock) Now() core.Timestamp {
	clock.session.mu.Lock()
	defer clock.session.mu.Unlock()
	return clock.session.deps.Clock.Now()
}
