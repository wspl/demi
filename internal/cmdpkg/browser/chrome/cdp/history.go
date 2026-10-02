package cdp

//revive:disable:unused-parameter API checkpoint: retain parameter names for dependent implementers.

// Buffer retains browser console or CDP entries with generation-bound cursors.
// The caller serializes access; entries returned by Entries are snapshots.
type Buffer[T any] struct{}

// NewBuffer creates a stream with count and encoded-byte bounds. setSequence
// assigns a position in a copied entry; sequence reads that assigned position.
// Callbacks adapt the existing generated contract types without redeclaring them.
func NewBuffer[T any](prefix string, entryLimit, byteLimit int, sequence func(T) uint64, setSequence func(T, uint64) T) (*Buffer[T], error) {
	panic("not written: k-chrome-cdp")
}

// Push appends at the next position, evicting oldest entries past the limits.
func (b *Buffer[T]) Push(entry T) error { panic("not written: k-chrome-cdp") }

// MarkGap reserves one missing position when an upstream listener loses data.
func (b *Buffer[T]) MarkGap() error { panic("not written: k-chrome-cdp") }

// Entries returns retained entries in stream order.
func (b *Buffer[T]) Entries() []T { panic("not written: k-chrome-cdp") }

// Next returns the position of the next entry.
func (b *Buffer[T]) Next() uint64 { panic("not written: k-chrome-cdp") }

// Cursor binds a position to this stream's generation.
func (b *Buffer[T]) Cursor(position uint64) string { panic("not written: k-chrome-cdp") }

// Position validates a cursor's generation and position.
func (b *Buffer[T]) Position(cursor string) (uint64, error) { panic("not written: k-chrome-cdp") }

// TruncatedSince reports eviction or loss at or after position.
func (b *Buffer[T]) TruncatedSince(position uint64) bool { panic("not written: k-chrome-cdp") }

// HasEvicted reports whether any position has been lost.
func (b *Buffer[T]) HasEvicted() bool { panic("not written: k-chrome-cdp") }
