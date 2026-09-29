package storage

import (
	"context"
	"errors"
	"sync"
	"testing"
	"time"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/webapi"
)

type deletionGate struct {
	ObjectStore
	started chan struct{}
	release chan struct{}
	once    sync.Once
}

func (s *deletionGate) Delete(ctx context.Context, key string) error {
	s.once.Do(func() { close(s.started) })
	select {
	case <-s.release:
	case <-ctx.Done():
		return ctx.Err()
	}
	return s.ObjectStore.Delete(ctx, key)
}

// Cost: two tiny filesystem objects and channel-coordinated operations; no sleeps.
func TestBlobDeletionRejectsCommitAndConcurrentPutRepublishes(t *testing.T) {
	ctx := t.Context()
	clock := &testClock{core.TruncateTimestamp(time.UnixMilli(1700000000000))}
	local, err := OpenLocalObjects(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer local.Close()
	gate := &deletionGate{ObjectStore: local, started: make(chan struct{}), release: make(chan struct{})}
	user, _ := webapi.ParseUserID("owner")
	blobs := NewBlobStores(gate, clock).ForUser(user)
	ref, err := blobs.Put(ctx, []byte("bytes"))
	if err != nil {
		t.Fatal(err)
	}
	clock.advance(24*time.Hour + time.Millisecond)
	deleted := make(chan error, 1)
	go func() {
		ok, err := blobs.DeleteUnused(ctx, ref, 24*time.Hour)
		if err == nil && !ok {
			err = errors.New("unused blob retained")
		}
		deleted <- err
	}()
	<-gate.started
	if err = blobs.CommitUses([]core.BlobRef{ref}); err == nil {
		t.Error("commit accepted a deleting blob")
	}
	put := make(chan error, 1)
	go func() { _, err := blobs.Put(ctx, []byte("bytes")); put <- err }()
	close(gate.release)
	if err = <-deleted; err != nil {
		t.Fatal(err)
	}
	if err = <-put; err != nil {
		t.Fatal(err)
	}
	data, err := blobs.Get(ctx, ref)
	if err != nil || string(data) != "bytes" {
		t.Fatalf("put raced with deletion: %q %v", data, err)
	}
	other, _ := webapi.ParseUserID("other")
	data, err = blobs.stores.ForUser(other).Get(ctx, ref)
	if err != nil || data != nil {
		t.Fatalf("blob crossed users: %q %v", data, err)
	}
}
