package storetest

// API checkpoint: named parameters document the interface until bodies are ported.
//revive:disable:unused-parameter

import (
	"context"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
)

// MemoryBlobs is an in-memory blob namespace safe for concurrent calls.
type MemoryBlobs struct{}

// NewMemoryBlobs creates an empty namespace.
func NewMemoryBlobs() *MemoryBlobs { panic("not written: a-store") }

// Holds reports whether the namespace holds blob.
func (b *MemoryBlobs) Holds(blob core.BlobRef) bool { panic("not written: a-store") }

// Forget removes blob from the namespace.
func (b *MemoryBlobs) Forget(blob core.BlobRef) { panic("not written: a-store") }

// Put stores bytes under their SHA-256 unless already held.
func (b *MemoryBlobs) Put(ctx context.Context, data core.B64Bytes) (core.BlobRef, error) {
	panic("not written: a-store")
}

// Read returns owned bytes and whether blob exists.
func (b *MemoryBlobs) Read(ctx context.Context, blob core.BlobRef) (core.B64Bytes, bool, error) {
	panic("not written: a-store")
}

var _ store.BlobStore = (*MemoryBlobs)(nil)
