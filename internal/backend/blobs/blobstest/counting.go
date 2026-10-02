package blobstest

import (
	"context"
	"runtime"
	"sync/atomic"

	"gocloud.dev/blob"

	"github.com/wspl/demi/internal/backend/blobs"
)

// ObjectCounts counts the operations reaching one object store. Its zero value
// is ready to use; share its pointer across observers rather than copying it.
// It supports concurrent observation and snapshots.
type ObjectCounts struct {
	puts, bytesPut, gets, heads, reading, mostReading, lists, deletes atomic.Uint64
}

// ObjectTally is what reached the object store up to one moment.
type ObjectTally struct {
	Puts uint64
	// BytesPut is the bytes the puts sent.
	BytesPut uint64
	// Gets counts reads of an object's bytes.
	Gets uint64
	// Heads counts asks whether an object exists, which transfer no bytes.
	Heads uint64
	// MostGetsAtOnce is the most reads that were in flight at once.
	MostGetsAtOnce uint64
	// Lists counts listings of a prefix.
	Lists uint64
	// Deletes counts objects asked to be deleted.
	Deletes uint64
}

// Since returns what reached the store since earlier; the most reads at once
// is this tally's.
func (t ObjectTally) Since(earlier ObjectTally) ObjectTally {
	return ObjectTally{Puts: t.Puts - earlier.Puts, BytesPut: t.BytesPut - earlier.BytesPut, Gets: t.Gets - earlier.Gets, Heads: t.Heads - earlier.Heads, MostGetsAtOnce: t.MostGetsAtOnce, Lists: t.Lists - earlier.Lists, Deletes: t.Deletes - earlier.Deletes}
}

// Tally returns a snapshot of the operations observed so far.
func (c *ObjectCounts) Tally() ObjectTally {
	return ObjectTally{Puts: c.puts.Load(), BytesPut: c.bytesPut.Load(), Gets: c.gets.Load(), Heads: c.heads.Load(), MostGetsAtOnce: c.mostReading.Load(), Lists: c.lists.Load(), Deletes: c.deletes.Load()}
}

// Observe returns objects with what reaches it counted here. It borrows objects;
// the caller retains ownership of the underlying bucket and its cleanup.
func (c *ObjectCounts) Observe(objects blobs.Objects) blobs.Objects {
	return &counted{Objects: objects, counts: c}
}

type counted struct {
	blobs.Objects
	counts *ObjectCounts
}

func (c *counted) Attributes(ctx context.Context, key string) (*blob.Attributes, error) {
	c.counts.heads.Add(1)
	return c.Objects.Attributes(ctx, key)
}

func (c *counted) ReadAll(ctx context.Context, key string) ([]byte, error) {
	counts := c.counts
	counts.gets.Add(1)
	reading := counts.reading.Add(1)
	defer counts.reading.Add(^uint64(0))
	for previous := counts.mostReading.Load(); reading > previous; previous = counts.mostReading.Load() {
		if counts.mostReading.CompareAndSwap(previous, reading) {
			break
		}
	}
	// Match the Rust counting fixture's yield so concurrently started reads can overlap.
	runtime.Gosched()
	return c.Objects.ReadAll(ctx, key)
}

func (c *counted) WriteAll(ctx context.Context, key string, data []byte, opts *blob.WriterOptions) error {
	c.counts.puts.Add(1)
	c.counts.bytesPut.Add(uint64(len(data)))
	return c.Objects.WriteAll(ctx, key, data, opts)
}

func (c *counted) List(opts *blob.ListOptions) *blob.ListIterator {
	c.counts.lists.Add(1)
	return c.Objects.List(opts)
}

func (c *counted) Delete(ctx context.Context, key string) error {
	c.counts.deletes.Add(1)
	return c.Objects.Delete(ctx, key)
}
