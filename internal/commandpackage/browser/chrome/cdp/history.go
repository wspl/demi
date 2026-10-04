package cdp

import (
	"math"
	"strconv"
	"strings"

	"github.com/wspl/demi/internal/contract"
)

// Buffer retains browser console or CDP entries with generation-bound cursors.
// The caller serializes access; entries returned by Entries are snapshots.
type Buffer[T any] struct {
	generation                   string
	next                         uint64
	bytes, entryLimit, byteLimit int
	evicted                      *uint64
	entries                      []T
	sizes                        []int
	sequence                     func(T) uint64
	setSequence                  func(T, uint64) T
}

// NewBuffer creates a stream with count and encoded-byte bounds. setSequence
// assigns a position in a copied entry; sequence reads that assigned position.
// Callbacks adapt the existing generated contract types without redeclaring them.
func NewBuffer[T any](
	prefix string,
	entryLimit, byteLimit int,
	sequence func(T) uint64,
	setSequence func(T, uint64) T,
) (*Buffer[T], error) {
	generation, err := Fresh(prefix)
	if err != nil {
		return nil, err
	}
	return &Buffer[T]{
		generation:  generation,
		entryLimit:  entryLimit,
		byteLimit:   byteLimit,
		sequence:    sequence,
		setSequence: setSequence,
	}, nil
}

// Push appends at the next position, evicting oldest entries past the limits.
func (b *Buffer[T]) Push(entry T) error {
	entry = b.setSequence(entry, b.next)
	bytes, err := contract.EncodeJSON(entry)
	if err != nil {
		return &BrowserError{Kind: KindInvalidResult, Message: err.Error(), Cause: err}
	}
	if b.next == math.MaxUint64 {
		return &BrowserError{Kind: KindResultTooLarge}
	}
	b.next++
	b.bytes += len(bytes)
	b.entries = append(b.entries, entry)
	b.sizes = append(b.sizes, len(bytes))
	for len(b.entries) > b.entryLimit || b.bytes > b.byteLimit {
		position := b.sequence(b.entries[0])
		if b.evicted == nil || *b.evicted < position {
			b.evicted = &position
		}
		b.bytes -= b.sizes[0]
		var zero T
		b.entries[0] = zero
		b.entries = b.entries[1:]
		b.sizes = b.sizes[1:]
	}
	return nil
}

// MarkGap reserves one missing position when an upstream listener loses data.
func (b *Buffer[T]) MarkGap() error {
	if b.next == math.MaxUint64 {
		return &BrowserError{Kind: KindResultTooLarge}
	}
	position := b.next
	b.evicted = &position
	b.next++
	return nil
}

// Entries returns retained entries in stream order.
func (b *Buffer[T]) Entries() []T {
	return append([]T{}, b.entries...)
}

// Next returns the position of the next entry.
func (b *Buffer[T]) Next() uint64 {
	return b.next
}

// Cursor binds a position to this stream's generation.
func (b *Buffer[T]) Cursor(position uint64) string {
	return b.generation + ":" + strconv.FormatUint(position, 10)
}

// Position validates a cursor's generation and position.
func (b *Buffer[T]) Position(cursor string) (uint64, error) {
	index := strings.LastIndexByte(cursor, ':')
	if index < 0 {
		return 0, &BrowserError{Kind: KindStaleCursor}
	}
	position, err := strconv.ParseUint(cursor[index+1:], 10, 64)
	if err != nil || cursor[:index] != b.generation || position > b.next {
		return 0, &BrowserError{Kind: KindStaleCursor}
	}
	return position, nil
}

// TruncatedSince reports eviction or loss at or after position.
func (b *Buffer[T]) TruncatedSince(position uint64) bool {
	return b.evicted != nil && position <= *b.evicted
}

// HasEvicted reports whether any position has been lost.
func (b *Buffer[T]) HasEvicted() bool {
	return b.evicted != nil
}
