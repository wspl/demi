package gates

import (
	"context"
	"fmt"
	"sync"

	"golang.org/x/sync/semaphore"
)

// Serial admits one holder in FIFO order. Its zero value is ready for use.
// It must not be copied after first use.
type Serial struct {
	once    sync.Once
	permits *semaphore.Weighted
}

// semaphore initializes this serial gate on first use.
func (s *Serial) semaphore() *semaphore.Weighted {
	s.once.Do(func() {
		s.permits = semaphore.NewWeighted(1)
	})
	return s.permits
}

// Acquire waits for the next turn, relinquishing its place on cancellation.
func (s *Serial) Acquire(ctx context.Context) (*Permit, error) {
	sem := s.semaphore()
	if err := sem.Acquire(ctx, 1); err != nil {
		return nil, fmt.Errorf("acquire serial gate: %w", err)
	}
	return &Permit{release: func() {
		sem.Release(1)
	}}, nil
}

// TryAcquire takes a turn immediately, or returns nil if anyone holds or waits.
func (s *Serial) TryAcquire() *Permit {
	sem := s.semaphore()
	if !sem.TryAcquire(1) {
		return nil
	}
	return &Permit{release: func() {
		sem.Release(1)
	}}
}

// Permit owns a serial or keyed serial turn. Transfer its pointer to hand it
// to another goroutine; do not copy it.
type Permit struct {
	once    sync.Once
	release func()
}

// Release gives up the turn exactly once, from any goroutine.
func (p *Permit) Release() {
	p.once.Do(p.release)
}

// KeyedSerial serializes each key independently. Its zero value is ready for
// use. Entries remain owned by both holders and waiters; do not copy the gate.
type KeyedSerial[K comparable] struct {
	mu      sync.Mutex // Protects entries and their reference counts, never admission.
	entries map[K]*keyEntry
}

type keyEntry struct {
	permits *semaphore.Weighted
	refs    int
}

// Acquire waits for key's next turn. Cancellation releases the waiting entry.
func (s *KeyedSerial[K]) Acquire(ctx context.Context, key K) (*Permit, error) {
	s.mu.Lock()
	if s.entries == nil {
		s.entries = make(map[K]*keyEntry)
	}
	entry := s.entries[key]
	if entry == nil {
		entry = &keyEntry{permits: semaphore.NewWeighted(1)}
		s.entries[key] = entry
	}
	entry.refs++
	s.mu.Unlock()
	if err := entry.permits.Acquire(ctx, 1); err != nil {
		s.unref(key, entry)
		return nil, fmt.Errorf("acquire keyed serial gate: %w", err)
	}
	return &Permit{release: func() {
		entry.permits.Release(1)
		s.unref(key, entry)
	}}, nil
}

// unref forgets a keyed gate only when neither holders nor waiters own it.
func (s *KeyedSerial[K]) unref(key K, entry *keyEntry) {
	s.mu.Lock()
	defer s.mu.Unlock()
	entry.refs--
	if entry.refs == 0 {
		delete(s.entries, key)
	}
}

// Len reports how many keys have a holder or waiter.
func (s *KeyedSerial[K]) Len() int {
	s.mu.Lock()
	defer s.mu.Unlock()
	return len(s.entries)
}
