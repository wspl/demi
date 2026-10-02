package storetest

import (
	"bytes"
	"context"
	"crypto/sha256"
	"fmt"
	"sync"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
)

// MemoryBlobs is an in-memory blob namespace safe for concurrent calls.
type MemoryBlobs struct {
	// mu protects the namespace; its immutable byte slices never escape.
	mu    sync.Mutex
	blobs map[core.BlobRef]core.B64Bytes
}

// NewMemoryBlobs creates an empty namespace.
func NewMemoryBlobs() *MemoryBlobs { return &MemoryBlobs{blobs: map[core.BlobRef]core.B64Bytes{}} }

// Holds reports whether the namespace holds blob.
func (b *MemoryBlobs) Holds(blob core.BlobRef) bool {
	b.mu.Lock()
	defer b.mu.Unlock()
	_, exists := b.blobs[blob]
	return exists
}

// Forget removes blob from the namespace.
func (b *MemoryBlobs) Forget(blob core.BlobRef) {
	b.mu.Lock()
	defer b.mu.Unlock()
	delete(b.blobs, blob)
}

// Put stores bytes under their SHA-256 unless already held.
func (b *MemoryBlobs) Put(ctx context.Context, data core.B64Bytes) (core.BlobRef, error) {
	if err := ctx.Err(); err != nil {
		return "", err
	}
	blob := core.BlobRef(fmt.Sprintf("%x", sha256.Sum256(data)))
	copied := bytes.Clone(data)
	b.mu.Lock()
	defer b.mu.Unlock()
	if b.blobs == nil {
		b.blobs = map[core.BlobRef]core.B64Bytes{}
	}
	b.blobs[blob] = copied
	return blob, nil
}

// Read returns owned bytes and whether blob exists.
func (b *MemoryBlobs) Read(ctx context.Context, blob core.BlobRef) (core.B64Bytes, bool, error) {
	if err := ctx.Err(); err != nil {
		return nil, false, err
	}
	b.mu.Lock()
	data, exists := b.blobs[blob]
	b.mu.Unlock()
	return bytes.Clone(data), exists, nil
}

var _ store.BlobStore = (*MemoryBlobs)(nil)
